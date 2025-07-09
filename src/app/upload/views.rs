use actix_web::{HttpRequest, HttpResponse, Result, web};
use base64::{Engine as _, engine::general_purpose};
use futures::StreamExt;
use path_clean::PathClean;
use sanitize_filename;
use std::path::PathBuf;
use tokio::io::AsyncWriteExt;

use crate::State;

pub async fn upload(
    mut payload: web::Payload,
    state: web::Data<State>,
    req: HttpRequest,
) -> Result<HttpResponse> {
    // Determine filename

    // Check if base64 file name was send
    let filename: String = if let Some(b64_file_name) = req.headers().get("X-Target-File-B64") {
        match b64_file_name.to_str() {
            Ok(base64_string) => match general_purpose::STANDARD.decode(base64_string) {
                Ok(vec_u8_file_name) => match String::from_utf8(vec_u8_file_name.clone()) {
                    Ok(file_name) => file_name,
                    Err(err) => return Ok(HttpResponse::BadRequest().body(format!("{}", err))),
                },
                Err(err) => return Ok(HttpResponse::BadRequest().body(format!("{}", err))),
            },
            Err(err) => return Ok(HttpResponse::BadRequest().body(format!("{}", err))),
        }
    } else if let Some(ascii_file_name) = req.headers().get("X-Target-File") {
        match ascii_file_name.to_str() {
            Ok(name) if !name.is_empty() => name.to_string(),
            _ => {
                return Ok(HttpResponse::BadRequest().finish());
            }
        }
    } else {
        format!("upload_{}.bin", chrono::Utc::now().timestamp_millis())
    };

    // Sanitize the filename
    let filename = sanitize_filename::sanitize(&filename);

    // Determine Target Directory from Header (Optional - Only if uploaded from frontend)
    let target_subdir_str = req
        .headers()
        .get("X-Target-Dir")
        .and_then(|h| h.to_str().ok())
        .unwrap_or(""); // Default to empty string (root) if header is missing/invalid

    let cleaned_target_subdir = PathBuf::from(target_subdir_str).clean();
    let full_target_dir = state.base_path.join(&cleaned_target_subdir);

    let canonical_full_target_dir = match full_target_dir.canonicalize() {
        Ok(p) => p,
        Err(err) => {
            log::debug!(
                "Upload rejected: Target directory '{}' could not be canonicalized or does not exist: {}",
                full_target_dir.display(),
                err
            );
            return Ok(HttpResponse::BadRequest()
                .body("Target directory does not exist or is inaccessible"));
        }
    };

    if !canonical_full_target_dir.starts_with(state.base_path.clone()) {
        log::debug!(
            "Upload rejected: Path traversal attempt via X-Target-Dir header. Target: '{}', Cleaned Subdir: '{}'",
            canonical_full_target_dir.display(),
            cleaned_target_subdir.display()
        );
        return Ok(HttpResponse::Forbidden().body("Forbidden target directory"));
    }

    // Ensure the target is actually a directory after canonicalization
    if !canonical_full_target_dir.is_dir() {
        log::debug!(
            "Upload rejected: Target path '{}' is not a directory.",
            canonical_full_target_dir.display()
        );
        return Ok(HttpResponse::BadRequest().body("Target path is not a directory"));
    }

    // Build full path
    let full_file_path = canonical_full_target_dir.join(&filename);

    log::debug!("Attempting to upload to: {}", full_file_path.display());

    // --- 6. Open File for Writing ---
    let file = match tokio::fs::File::create(&full_file_path).await {
        Ok(f) => f,
        Err(e) => {
            log::error!(
                "Failed to create file for upload at {}: {}",
                full_file_path.display(),
                e
            );
            // Use InternalServerError as this is likely a server-side FS issue
            return Ok(HttpResponse::InternalServerError().body("Failed to create file on server"));
        }
    };

    let mut total_bytes_written: u64 = 0;

    // Set chunk size to 4 Megabytes
    let chunk_size = 1024 * 1024 * 4;

    // Create a buffer to read in the data
    let mut read_buffer: Vec<u8> = Vec::new();

    let mut writer = file;

    // Stream payload
    while let Some(chunk) = payload.next().await {
        match chunk {
            Ok(data) => {
                read_buffer.extend_from_slice(&data);
                // If we exceed the chunk size it is time to write
                if read_buffer.len() >= chunk_size {
                    writer.write_all(&read_buffer).await.map_err(|e| {
                        log::error!("Write error: {}", e);
                        actix_web::error::ErrorInternalServerError("Write failure")
                    })?;
                    total_bytes_written += read_buffer.len() as u64;
                    read_buffer.clear();
                }
            }
            Err(err) => {
                log::error!("{}", err);
                return Ok(HttpResponse::BadRequest().finish());
            }
        }
    }

    // Write any remaining data in the buffer
    if !read_buffer.is_empty() {
        writer.write_all(&read_buffer).await.map_err(|e| {
            log::error!("Write error: {}", e);
            actix_web::error::ErrorInternalServerError("Write failure")
        })?;
        total_bytes_written += read_buffer.len() as u64;
    }

    // Flush buffer
    writer.flush().await.map_err(|e| {
        log::error!("Flush error: {}", e);
        actix_web::error::ErrorInternalServerError("Flush failure")
    })?;

    // --- 8. Check if File Size is Zero (Optional but good) ---
    // No need to get metadata again if we tracked bytes written
    if total_bytes_written == 0 {
        log::debug!(
            "Upload rejected: Received empty file for '{}'. Deleting.",
            full_file_path.display()
        );
        if let Err(e) = tokio::fs::remove_file(&full_file_path).await {
            log::error!(
                "Failed to delete empty uploaded file {}: {}",
                full_file_path.display(),
                e
            );
            // Continue to return bad request, but log the cleanup error
        }
        return Ok(HttpResponse::BadRequest().body("Empty file upload rejected"));
    }

    log::info!(
        "Successfully uploaded {:?} ({} bytes) to {}",
        filename,
        total_bytes_written,
        full_file_path.display()
    );
    Ok(HttpResponse::Ok().finish())
}
