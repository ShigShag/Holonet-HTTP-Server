use actix_web::{HttpRequest, HttpResponse, Result, web};
use futures::StreamExt;
use path_clean::PathClean;
use std::path::PathBuf;
use tokio::io::AsyncWriteExt;

use crate::State;

pub async fn upload(
    mut payload: web::Payload,
    state: web::Data<State>,
    req: HttpRequest,
) -> Result<HttpResponse> {
    // Determine filename
    let filename = if let Some(fname_header) = req.headers().get("X-Target-File") {
        match fname_header.to_str() {
            Ok(name) if !name.is_empty() => name.to_string(),
            _ => {
                return Ok(HttpResponse::BadRequest().finish());
            }
        }
    } else {
        format!("upload_{}.bin", chrono::Utc::now().timestamp_millis())
    };

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
    let mut file = match tokio::fs::File::create(&full_file_path).await {
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

    // --- 7. Read Payload and Write to File ---
    let mut total_bytes_written: u64 = 0;
    while let Some(chunk_result) = payload.next().await {
        match chunk_result {
            Ok(data) => {
                if let Err(e) = file.write_all(&data).await {
                    log::error!(
                        "Error writing upload chunk to {}: {}",
                        full_file_path.display(),
                        e
                    );
                    // Clean up partially written file
                    drop(file); // Ensure file handle is closed before trying to remove
                    let _ = tokio::fs::remove_file(&full_file_path).await; // Attempt cleanup
                    return Ok(
                        HttpResponse::InternalServerError().body("Error writing file to server")
                    );
                }
                total_bytes_written += data.len() as u64;
            }
            Err(e) => {
                log::error!(
                    "Error receiving upload payload chunk for {}: {}",
                    full_file_path.display(),
                    e
                );
                // Clean up partially written file
                drop(file);
                let _ = tokio::fs::remove_file(&full_file_path).await;
                return Ok(HttpResponse::BadRequest().body("Error receiving upload data"));
            }
        }
    }

    // Flush OS buffer to disk before checking size / finishing
    if let Err(e) = file.sync_all().await {
        log::error!(
            "Error flushing file {} to disk: {}",
            full_file_path.display(),
            e
        );
        // Cleanup? Maybe not fatal, but log it.
    }

    // --- 8. Check if File Size is Zero (Optional but good) ---
    // No need to get metadata again if we tracked bytes written
    if total_bytes_written == 0 {
        log::debug!(
            "Upload rejected: Received empty file for '{}'. Deleting.",
            full_file_path.display()
        );
        drop(file); // Ensure file handle is closed
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
        "Successfully uploaded {} ({} bytes) to {}",
        filename,
        total_bytes_written,
        full_file_path.display()
    );
    Ok(HttpResponse::Ok().finish())
}
