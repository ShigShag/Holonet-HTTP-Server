use actix_files::NamedFile;
use actix_web::{HttpRequest, HttpResponse, Responder, Result, web};
use futures_util::StreamExt;
use path_clean::PathClean; // For path cleaning
use serde::Serialize; // For Tera context
use std::path::PathBuf;
use tokio::fs; // Use tokio's async fs for reading directories // For processing directory stream

// Import shared state structs (adjust path if needed)
use crate::State;

// Struct for Tera context when listing directories
#[derive(Serialize)]
struct DirContext {
    current_path: String,        // The requested path relative to the base
    parent_path: Option<String>, // Link to parent dir, if not root
    entries: Vec<DirEntry>,
}

// Struct for individual directory entries
#[derive(Serialize)]
struct DirEntry {
    name: String,
    url: String, // URL relative to the site root
    is_dir: bool,
}

// The main handler
pub async fn display_dir(
    state: web::Data<State>,
    req: HttpRequest,
    tail: web::Path<String>, // Capture the path segment(s) after the base URL
) -> Result<impl Responder> {
    // Use impl Responder for flexibility
    let canonical_base_path: PathBuf = state.base_path.clone();

    let requested_path_str = tail.into_inner();

    // --- 1. Path Construction and Security Check ---
    let cleaned_relative_path = PathBuf::from(requested_path_str).clean();

    log::debug!("cleaned_relative_path: {}", cleaned_relative_path.display());

    // Join the cleaned relative path to the canonical base path
    let requested_absolute_path = canonical_base_path.join(&cleaned_relative_path);

    // Convert to canonical path for security check
    let canonical_requested_path = match requested_absolute_path.canonicalize() {
        Ok(p) => p,
        Err(ref e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Path doesn't exist, return 404 Not Found
            log::debug!("Path not found: {}", requested_absolute_path.display());
            return Ok(HttpResponse::SeeOther()
                .append_header(("Location", "/"))
                .finish());
        }
        Err(e) => {
            // Other error (e.g., permission denied) during canonicalization
            log::error!(
                "Error canonicalizing requested path {}: {}",
                requested_absolute_path.display(),
                e
            );
            return Ok(HttpResponse::InternalServerError().body("Error accessing path information"));
        }
    };

    // Check if the canonical path starts with the canonical base path
    if !canonical_requested_path.starts_with(&canonical_base_path) {
        log::debug!(
            "Path traversal attempt detected: {}",
            canonical_requested_path.display()
        );
        return Ok(HttpResponse::Forbidden().body("Forbidden access"));
    }

    // --- 2. Check if Path is File or Directory ---
    let metadata = match fs::metadata(&canonical_requested_path).await {
        Ok(meta) => meta,
        Err(ref e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Should have been caught by canonicalize check, but double-check
            log::debug!(
                "Metadata check failed (Not Found): {}",
                canonical_requested_path.display()
            );
            return Ok(HttpResponse::SeeOther()
                .append_header(("Location", "/"))
                .finish());
        }
        Err(e) => {
            log::error!(
                "Failed to get metadata for {}: {}",
                canonical_requested_path.display(),
                e
            );
            return Ok(HttpResponse::InternalServerError().body("Failed to read path metadata"));
        }
    };

    // --- 3. Serve File if it's a file ---
    if metadata.is_file() {
        log::debug!("Serving file: {}", canonical_requested_path.display());
        return Ok(NamedFile::open_async(canonical_requested_path)
            .await?
            .into_response(&req));
    }

    // Ok(HttpResponse::Ok().finish()) // Placeholder for actual response

    // --- 4. List Directory Contents if it's a directory ---
    if metadata.is_dir() {
        let mut entries = Vec::new();
        let read_dir = match fs::read_dir(&canonical_requested_path).await {
            Ok(rd) => rd,
            Err(e) => {
                log::error!(
                    "Failed to read directory {}: {}",
                    canonical_requested_path.display(),
                    e
                );
                // Could be permission denied
                return Ok(HttpResponse::Forbidden().body("Cannot read directory listing"));
            }
        };

        // Use `try_for_each` for cleaner async iteration over directory entries
        let mut dir_stream = tokio_stream::wrappers::ReadDirStream::new(read_dir);

        while let Some(entry_result) = dir_stream.next().await {
            // Handle errors in reading directory entries
            let entry_result = match entry_result {
                Ok(entry) => entry,
                Err(e) => {
                    log::debug!("Error reading directory entry: {}", e);
                    continue; // Skip this entry if we can't read it
                }
            };

            // Propagate IO errors as Internal Server Error
            let file_name = entry_result.file_name().to_string_lossy().into_owned();
            let file_type = match entry_result.file_type().await {
                Ok(ft) => ft,
                Err(e) => {
                    log::debug!("Could not get file type for '{}': {}", file_name, e);
                    continue; // Skip this entry if we can't determine type
                }
            };

            let is_dir = file_type.is_dir();

            // Construct the URL relative to the web server root
            // Combine the *original* cleaned relative path with the entry name
            let entry_relative_path = cleaned_relative_path.join(&file_name);
            let url = format!(
                "/{}",
                entry_relative_path.to_string_lossy().replace("\\", "/")
            ); // Ensure forward slashes

            entries.push(DirEntry {
                name: file_name,
                url,
                is_dir,
            });
        }

        // Sort entries alphabetically (directories first, then files)
        entries.sort_by(|a, b| {
            match (a.is_dir, b.is_dir) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()), // Case-insensitive sort by name
            }
        });

        // --- 5. Prepare Tera Context ---
        let current_display_path = cleaned_relative_path.to_string_lossy().replace("\\", "/");

        // Generate parent path link, if not at the root
        let parent_path = if cleaned_relative_path.parent().is_some()
            && !cleaned_relative_path.as_os_str().is_empty()
        {
            cleaned_relative_path
                .parent()
                .map(|p| format!("/{}", p.to_string_lossy().replace("\\", "/")))
                // Handle case where parent is the root ("/")
                .map(|p| if p == "/" { "/".to_string() } else { p })
                // If parent resolves to empty (e.g. was just "./") go to root
                .filter(|p| !p.is_empty() || cleaned_relative_path.components().count() > 1)
                .or(Some("/".to_string()))
        } else {
            None
        };

        let context = DirContext {
            current_path: current_display_path,
            parent_path,
            entries,
        };

        // --- 6. Render Template ---
        let tera = &state.tera;
        let rendered_body = match tera.render(
            "home.html",
            &tera::Context::from_serialize(&context).unwrap(),
        ) {
            Ok(html) => html,
            Err(e) => {
                log::error!(
                    "Tera rendering error for {}: {}",
                    canonical_requested_path.display(),
                    e
                );
                return Ok(
                    HttpResponse::InternalServerError().body("Failed to render directory listing")
                );
            }
        };

        Ok(HttpResponse::Ok()
            .content_type("text/html; charset=utf-8")
            .body(rendered_body))
    } else {
        // Path exists but is not a file or directory (e.g., symlink, socket - handle as needed)
        log::debug!(
            "Path is neither file nor directory: {}",
            canonical_requested_path.display()
        );
        Ok(HttpResponse::NotFound().finish()) // Or maybe Forbidden?
    }
}
