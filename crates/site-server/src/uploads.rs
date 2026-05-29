use std::path::{Path, PathBuf};

use tokio::fs;
use uuid::Uuid;

#[derive(Debug)]
pub enum UploadError {
    InvalidContentType,
    FileTooLarge { max_bytes: usize },
    IoError(std::io::Error),
}

impl std::fmt::Display for UploadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UploadError::InvalidContentType => {
                write!(
                    f,
                    "Invalid content type. Only JPEG, PNG, WebP, and GIF are allowed"
                )
            }
            UploadError::FileTooLarge { max_bytes } => {
                write!(
                    f,
                    "File too large. Maximum size is {} MB",
                    max_bytes / (1024 * 1024)
                )
            }
            UploadError::IoError(e) => write!(f, "IO error: {e}"),
        }
    }
}

const ALLOWED_TYPES: &[(&str, &str)] = &[
    ("image/jpeg", "jpg"),
    ("image/png", "png"),
    ("image/webp", "webp"),
    ("image/gif", "gif"),
];

fn extension_for(content_type: &str) -> Option<&'static str> {
    ALLOWED_TYPES
        .iter()
        .find(|(ct, _)| *ct == content_type)
        .map(|(_, ext)| *ext)
}

/// Save an uploaded file to disk. Returns the relative URL path.
pub async fn save_upload(
    upload_dir: &Path,
    category: &str,
    data: &[u8],
    content_type: &str,
    max_bytes: usize,
) -> Result<String, UploadError> {
    let ext = extension_for(content_type).ok_or(UploadError::InvalidContentType)?;

    if data.len() > max_bytes {
        return Err(UploadError::FileTooLarge { max_bytes });
    }

    let dir = upload_dir.join(category);
    fs::create_dir_all(&dir)
        .await
        .map_err(UploadError::IoError)?;

    let filename = format!("{}.{ext}", Uuid::new_v4());
    let path = dir.join(&filename);

    fs::write(&path, data).await.map_err(UploadError::IoError)?;

    Ok(format!("/uploads/{category}/{filename}"))
}

/// Get the upload directory from config, creating it if needed.
pub async fn ensure_upload_dir(dir: &PathBuf) -> Result<(), std::io::Error> {
    fs::create_dir_all(dir).await
}
