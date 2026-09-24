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

/// Detect image type from magic bytes (not client Content-Type).
fn sniff_image_ext(data: &[u8]) -> Option<&'static str> {
    if data.len() >= 3 && data[0] == 0xff && data[1] == 0xd8 && data[2] == 0xff {
        return Some("jpg");
    }
    if data.len() >= 8 && data[..8] == [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'] {
        return Some("png");
    }
    if data.len() >= 6 && (data[..6] == *b"GIF87a" || data[..6] == *b"GIF89a") {
        return Some("gif");
    }
    // RIFF....WEBP
    if data.len() >= 12 && data[..4] == *b"RIFF" && data[8..12] == *b"WEBP" {
        return Some("webp");
    }
    None
}

/// Save an uploaded file to disk. Returns the relative URL path.
///
/// Extension is chosen from **magic bytes**, not the client-declared content-type.
/// Declared type is only used as a soft check when present.
pub async fn save_upload(
    upload_dir: &Path,
    category: &str,
    data: &[u8],
    content_type: &str,
    max_bytes: usize,
) -> Result<String, UploadError> {
    if data.len() > max_bytes {
        return Err(UploadError::FileTooLarge { max_bytes });
    }

    let ext = sniff_image_ext(data).ok_or(UploadError::InvalidContentType)?;

    // If client declared a type, it must match the sniffed type (when recognized).
    if let Some(declared) = extension_for(content_type)
        && declared != ext
    {
        return Err(UploadError::InvalidContentType);
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

/// Enforcing CSP for any `/uploads` response that is not a raster image.
///
/// `sandbox` (no allow-scripts) plus `default-src 'none'` stops a navigated
/// HTML or SVG document from running script even if the browser ignores
/// `Content-Disposition`.
const UPLOAD_SANDBOX_CSP: &str = "default-src 'none'; sandbox";

/// Router mounted at `/uploads`. `ServeDir` plus download/sandbox headers.
///
/// State parameter matches `create_router`'s `Router<AppState>` so this can be
/// nested. The directory service does not read that state.
pub fn uploads_router<S>(upload_dir: PathBuf) -> axum::Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    axum::Router::new()
        .fallback_service(tower_http::services::ServeDir::new(upload_dir))
        .layer(axum::middleware::from_fn(upload_response_headers))
}

/// Headers for user-uploaded files.
///
/// `nosniff` is set here as well as on the global security layer in
/// `scuffed-server`. The global layer already covers `/uploads` when that
/// binary is what is serving (the production path). This copy keeps the
/// header on the site-server binary, which does not mount that layer, and
/// makes the upload policy testable without the outer process.
///
/// PNG, JPEG, WebP, and GIF are `Content-Disposition: inline` so avatars and
/// article images still render in `<img>` / CSS. SVG is not in that set:
/// `image/svg+xml` is a document and can carry script when opened directly.
/// The upload API already rejects SVG by magic bytes; this is the response
/// side if a file is on disk anyway. Other types (HTML included) are
/// `Content-Disposition: attachment` and carry [`UPLOAD_SANDBOX_CSP`].
pub async fn upload_response_headers(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        axum::http::header::X_CONTENT_TYPE_OPTIONS,
        axum::http::HeaderValue::from_static("nosniff"),
    );

    let media = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|raw| raw.split(';').next())
        .map(str::trim)
        .unwrap_or("")
        .to_ascii_lowercase();

    if is_inline_raster(&media) {
        headers.insert(
            axum::http::header::CONTENT_DISPOSITION,
            axum::http::HeaderValue::from_static("inline"),
        );
    } else {
        headers.insert(
            axum::http::header::CONTENT_DISPOSITION,
            axum::http::HeaderValue::from_static("attachment"),
        );
        headers.insert(
            axum::http::header::CONTENT_SECURITY_POLICY,
            axum::http::HeaderValue::from_static(UPLOAD_SANDBOX_CSP),
        );
    }

    response
}

fn is_inline_raster(media_type: &str) -> bool {
    matches!(
        media_type,
        "image/png" | "image/jpeg" | "image/webp" | "image/gif"
    )
}

#[cfg(test)]
mod header_tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use tower::ServiceExt;

    async fn served(name: &str, bytes: &[u8]) -> axum::response::Response {
        let dir = std::env::temp_dir().join(format!("scuffed-upload-hdr-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(name), bytes).unwrap();
        let app = uploads_router::<()>(dir.clone());
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/{name}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        response
    }

    fn header_str<'a>(
        response: &'a axum::response::Response,
        name: &header::HeaderName,
    ) -> Option<&'a str> {
        response.headers().get(name).and_then(|v| v.to_str().ok())
    }

    #[tokio::test]
    async fn raster_images_are_inline_and_nosniff() {
        for name in ["a.png", "a.jpg", "a.jpeg", "a.webp", "a.gif"] {
            let response = served(name, b"not-a-real-image").await;
            assert_eq!(response.status(), StatusCode::OK, "{name}");
            assert_eq!(
                header_str(&response, &header::X_CONTENT_TYPE_OPTIONS),
                Some("nosniff"),
                "{name}"
            );
            assert_eq!(
                header_str(&response, &header::CONTENT_DISPOSITION),
                Some("inline"),
                "{name} {:?}",
                response.headers().get(header::CONTENT_TYPE)
            );
            assert!(
                header_str(&response, &header::CONTENT_SECURITY_POLICY).is_none(),
                "{name} should not be sandboxed"
            );
        }
    }

    #[tokio::test]
    async fn html_and_svg_download_and_are_sandboxed() {
        for name in ["page.html", "icon.svg", "note.bin"] {
            let response = served(name, b"<svg><script>alert(1)</script></svg>").await;
            assert_eq!(response.status(), StatusCode::OK, "{name}");
            assert_eq!(
                header_str(&response, &header::X_CONTENT_TYPE_OPTIONS),
                Some("nosniff"),
                "{name}"
            );
            assert_eq!(
                header_str(&response, &header::CONTENT_DISPOSITION),
                Some("attachment"),
                "{name}"
            );
            assert_eq!(
                header_str(&response, &header::CONTENT_SECURITY_POLICY),
                Some(UPLOAD_SANDBOX_CSP),
                "{name}"
            );
        }
    }
}
