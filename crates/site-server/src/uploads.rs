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
                    "Invalid content type. Only JPEG, PNG, WebP, GIF, and AVIF are allowed"
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
    ("image/avif", "avif"),
];

fn extension_for(content_type: &str) -> Option<&'static str> {
    ALLOWED_TYPES
        .iter()
        .find(|(ct, _)| *ct == content_type)
        .map(|(_, ext)| *ext)
}

/// Detect image type from magic bytes (not client Content-Type).
pub(crate) fn sniff_image_ext(data: &[u8]) -> Option<&'static str> {
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
    if is_avif_ftyp(data) {
        return Some("avif");
    }
    None
}

/// AVIF: a leading ISO BMFF `ftyp` box whose major brand or one of its
/// compatible brands is `avif` (still image) or `avis` (image sequence).
///
/// The box must be well formed: a 32-bit size of at least 16 bytes (size,
/// type, major brand, minor version), a whole number of 4-byte brands, and
/// no larger than the data. A 64-bit (`size == 1`) or to-end (`size == 0`)
/// `ftyp` is refused; real AVIF encoders never write one.
fn is_avif_ftyp(data: &[u8]) -> bool {
    if data.len() < 16 || data[4..8] != *b"ftyp" {
        return false;
    }
    let size = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
    if size < 16 || size > data.len() || !size.is_multiple_of(4) {
        return false;
    }
    let major = [data[8], data[9], data[10], data[11]];
    let (compatible, _) = data[16..size].as_chunks::<4>();
    std::iter::once(&major)
        .chain(compatible)
        .any(|brand| brand == b"avif" || brand == b"avis")
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
/// PNG, JPEG, WebP, GIF, and AVIF are `Content-Disposition: inline` so avatars and
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
        "image/png" | "image/jpeg" | "image/webp" | "image/gif" | "image/avif"
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
        for name in ["a.png", "a.jpg", "a.jpeg", "a.webp", "a.gif", "a.avif"] {
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
    async fn avif_is_inline_with_avif_content_type_and_nosniff() {
        let response = served("a.avif", super::test_avif::TINY_AVIF).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            header_str(&response, &header::CONTENT_TYPE),
            Some("image/avif")
        );
        assert_eq!(
            header_str(&response, &header::X_CONTENT_TYPE_OPTIONS),
            Some("nosniff")
        );
        assert_eq!(
            header_str(&response, &header::CONTENT_DISPOSITION),
            Some("inline")
        );
        assert!(
            header_str(&response, &header::CONTENT_SECURITY_POLICY).is_none(),
            "avif should not be sandboxed"
        );
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

/// Test-only AVIF fixture shared by the sniff, dimension, and serving tests.
#[cfg(test)]
pub(crate) mod test_avif {
    /// Real 2x1 AVIF (Pillow encoder, solid fill). Brands: major `avif`,
    /// compatible `avif mif1 miaf MA1B`. One `ispe` box.
    pub(crate) const TINY_AVIF: &[u8] = include_bytes!("../tests/data/tiny-2x1.avif");

    /// The fixture with its `ispe` box rewritten to declare `w` x `h`. The
    /// rest of the file (and its real 2x1 payload) is unchanged, so this is a
    /// header that lies about its size, which is what a pixel-flood upload is.
    pub(crate) fn avif_with_dims(w: u32, h: u32) -> Vec<u8> {
        let mut out = TINY_AVIF.to_vec();
        let at = out
            .windows(4)
            .position(|win| win == b"ispe")
            .expect("fixture has an ispe box");
        // `ispe` tag, then version/flags (4 bytes), then width and height.
        out[at + 8..at + 12].copy_from_slice(&w.to_be_bytes());
        out[at + 12..at + 16].copy_from_slice(&h.to_be_bytes());
        out
    }

    /// A bare `ftyp` box with the given major and compatible brands.
    pub(crate) fn ftyp(major: &[u8; 4], compatible: &[&[u8; 4]]) -> Vec<u8> {
        let size = 16 + 4 * compatible.len();
        let mut out = Vec::with_capacity(size);
        out.extend_from_slice(&(size as u32).to_be_bytes());
        out.extend_from_slice(b"ftyp");
        out.extend_from_slice(major);
        out.extend_from_slice(&0u32.to_be_bytes());
        for brand in compatible {
            out.extend_from_slice(*brand);
        }
        out
    }
}

#[cfg(test)]
mod sniff_tests {
    use super::test_avif::{TINY_AVIF, ftyp};
    use super::*;

    #[test]
    fn real_avif_fixture_sniffs_as_avif() {
        assert_eq!(sniff_image_ext(TINY_AVIF), Some("avif"));
        assert_eq!(extension_for("image/avif"), Some("avif"));
    }

    #[test]
    fn avif_and_avis_major_brands_are_accepted() {
        assert_eq!(sniff_image_ext(&ftyp(b"avif", &[])), Some("avif"));
        assert_eq!(sniff_image_ext(&ftyp(b"avis", &[])), Some("avif"));
    }

    #[test]
    fn avif_and_avis_compatible_brands_are_accepted() {
        assert_eq!(
            sniff_image_ext(&ftyp(b"mif1", &[b"miaf", b"avif"])),
            Some("avif")
        );
        assert_eq!(
            sniff_image_ext(&ftyp(b"msf1", &[b"iso8", b"avis"])),
            Some("avif")
        );
    }

    #[test]
    fn ftyp_with_other_brands_is_rejected() {
        // HEIC, plain MP4, and a QuickTime movie all start with `ftyp`.
        assert_eq!(sniff_image_ext(&ftyp(b"heic", &[b"mif1", b"heic"])), None);
        assert_eq!(sniff_image_ext(&ftyp(b"isom", &[b"iso2", b"mp41"])), None);
        assert_eq!(sniff_image_ext(&ftyp(b"qt  ", &[b"qt  "])), None);
    }

    #[test]
    fn malformed_ftyp_box_is_rejected() {
        // Brand outside the declared box size does not count.
        let mut outside = ftyp(b"mif1", &[]);
        outside.extend_from_slice(b"avif");
        assert_eq!(sniff_image_ext(&outside), None);

        // Box size larger than the data.
        let mut too_big = ftyp(b"avif", &[]);
        too_big[..4].copy_from_slice(&64u32.to_be_bytes());
        assert_eq!(sniff_image_ext(&too_big), None);

        // 64-bit and to-end sizes, and a size below the minimum header.
        for size in [0u32, 1, 8, 12] {
            let mut bad = ftyp(b"avif", &[b"avif"]);
            bad[..4].copy_from_slice(&size.to_be_bytes());
            assert_eq!(sniff_image_ext(&bad), None, "size {size}");
        }

        // Size not a whole number of brands.
        let mut ragged = ftyp(b"avif", &[b"avif"]);
        ragged.push(0);
        ragged[..4].copy_from_slice(&21u32.to_be_bytes());
        assert_eq!(sniff_image_ext(&ragged), None);

        // Truncated before the major brand ends.
        assert_eq!(sniff_image_ext(&TINY_AVIF[..12]), None);
    }

    #[tokio::test]
    async fn save_upload_stores_avif_and_checks_declared_type() {
        let dir = std::env::temp_dir().join(format!("scuffed-upload-avif-{}", Uuid::new_v4()));

        let url = save_upload(&dir, "images", TINY_AVIF, "image/avif", 1024 * 1024)
            .await
            .expect("avif upload is stored");
        assert!(url.starts_with("/uploads/images/"), "{url}");
        assert!(url.ends_with(".avif"), "{url}");
        let rel = url.strip_prefix("/uploads/").unwrap();
        assert_eq!(std::fs::read(dir.join(rel)).unwrap(), TINY_AVIF);

        // Declared PNG but the bytes are AVIF: mismatch is refused.
        assert!(matches!(
            save_upload(&dir, "images", TINY_AVIF, "image/png", 1024 * 1024).await,
            Err(UploadError::InvalidContentType)
        ));
        // Declared AVIF but the bytes are PNG.
        let png = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];
        assert!(matches!(
            save_upload(&dir, "images", &png, "image/avif", 1024 * 1024).await,
            Err(UploadError::InvalidContentType)
        ));
        // Same byte cap as every other format.
        assert!(matches!(
            save_upload(&dir, "images", TINY_AVIF, "image/avif", TINY_AVIF.len() - 1).await,
            Err(UploadError::FileTooLarge { .. })
        ));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
