//! Browser file uploads to the `/api/upload/*` routes.
//!
//! The server reads the first multipart field (sent as `file`) and answers
//! `{ "url": "/uploads/<category>/<member>/<name>" }`.

use serde::Deserialize;
use wasm_bindgen::JsCast;

/// Types `POST /api/upload/image` accepts. Used for the `accept` attribute.
pub const IMAGE_UPLOAD_ACCEPT: &str = "image/jpeg,image/png,image/webp,image/gif";

/// Same cap as the server's `IMAGE_MAX_BYTES` (5 MB).
pub const IMAGE_UPLOAD_MAX_BYTES: f64 = 5.0 * 1024.0 * 1024.0;

#[derive(Debug, Clone, Deserialize)]
struct UploadResponse {
    url: String,
}

/// Why a browser upload did not return a URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UploadFailure {
    /// The server answered with a non-2xx status.
    Status(u16),
    /// The request could not be built, sent, or read. Plain user-facing copy.
    Message(&'static str),
}

/// Client-side check before sending an image to `POST /api/upload/image`.
/// `mime` is the browser's `File.type`, `size` is `File.size` in bytes.
pub fn validate_image_upload(mime: &str, size: f64) -> Result<(), &'static str> {
    let mime = mime.trim().to_ascii_lowercase();
    if !IMAGE_UPLOAD_ACCEPT.split(',').any(|t| t == mime) {
        return Err("That file type is not supported. Choose a JPEG, PNG, WebP, or GIF image.");
    }
    if size <= 0.0 {
        return Err("That file is empty. Choose another image.");
    }
    if size > IMAGE_UPLOAD_MAX_BYTES {
        return Err("That image is larger than 5 MB. Choose a smaller file.");
    }
    Ok(())
}

/// First selected file of the `<input type="file">` with this DOM id.
pub fn selected_file(input_id: &str) -> Option<web_sys::File> {
    let document = web_sys::window()?.document()?;
    let input = document
        .get_element_by_id(input_id)?
        .dyn_into::<web_sys::HtmlInputElement>()
        .ok()?;
    input.files()?.get(0)
}

/// Clear a file input so picking the same file again fires `change`.
pub fn clear_file_input(input_id: &str) {
    if let Some(input) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id(input_id))
        .and_then(|el| el.dyn_into::<web_sys::HtmlInputElement>().ok())
    {
        input.set_value("");
    }
}

/// POST `file` as multipart field `file` to `url` (same-origin cookies) and
/// return the stored path from the JSON body.
pub async fn post_file(url: &str, file: &web_sys::File) -> Result<String, UploadFailure> {
    let form_data = web_sys::FormData::new()
        .map_err(|_| UploadFailure::Message("Could not prepare upload."))?;
    form_data
        .append_with_blob("file", file)
        .map_err(|_| UploadFailure::Message("Could not prepare upload."))?;

    let opts = web_sys::RequestInit::new();
    opts.set_method("POST");
    opts.set_body(&form_data.into());
    opts.set_credentials(web_sys::RequestCredentials::SameOrigin);

    let request = web_sys::Request::new_with_str_and_init(url, &opts)
        .map_err(|_| UploadFailure::Message("Could not build upload request."))?;
    let window =
        web_sys::window().ok_or(UploadFailure::Message("Upload failed: no browser window."))?;

    let resp: web_sys::Response =
        wasm_bindgen_futures::JsFuture::from(window.fetch_with_request(&request))
            .await
            .map_err(|_| UploadFailure::Message("Upload request failed."))?
            .unchecked_into();
    if !resp.ok() {
        return Err(UploadFailure::Status(resp.status()));
    }

    let text_promise = resp
        .text()
        .map_err(|_| UploadFailure::Message("Failed to read upload response."))?;
    let text = wasm_bindgen_futures::JsFuture::from(text_promise)
        .await
        .map_err(|_| UploadFailure::Message("Failed to read upload response."))?
        .as_string()
        .unwrap_or_default();
    serde_json::from_str::<UploadResponse>(&text)
        .map(|r| r.url)
        .map_err(|_| UploadFailure::Message("Failed to parse upload response."))
}

/// Relative path under an upload category folder.
///
/// Mirrors site-server `upload_rel_is_safe` in `routes/uploads.rs` (PR 628 /
/// `183741f2`, around line 184): reject empty, `\`, NUL, `?`, `#`, `%`,
/// empty / `.` / `..` segments, and any char outside `[A-Za-z0-9._-]`.
pub fn upload_rel_is_safe(rel: &str) -> bool {
    if rel.is_empty()
        || rel.contains('\\')
        || rel.contains('\0')
        || rel.contains('?')
        || rel.contains('#')
        || rel.contains('%')
    {
        return false;
    }
    let mut any = false;
    for seg in rel.split('/') {
        if seg.is_empty() || seg == "." || seg == ".." {
            return false;
        }
        if !seg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        {
            return false;
        }
        any = true;
    }
    any
}

/// True when `raw` is an uploads cover path the server accepts for
/// `cover_image_url`.
///
/// Mirrors the uploads branch of site-server `normalize_cover_image_url` in
/// `routes/articles.rs` (PR 628 / `183741f2`, around line 128): after trim,
/// no control / whitespace / backslash; prefix `/uploads/images/` or
/// `uploads/images/`; relative part contains `/` and passes
/// [`upload_rel_is_safe`].
pub fn is_cover_image_upload_path(raw: &str) -> bool {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed
        .chars()
        .any(|c| c.is_control() || c.is_whitespace() || c == '\\')
    {
        return false;
    }
    let Some(rel) = trimmed
        .strip_prefix("/uploads/images/")
        .or_else(|| trimmed.strip_prefix("uploads/images/"))
    else {
        return false;
    };
    rel.contains('/') && upload_rel_is_safe(rel)
}

/// True when `raw` is an absolute https cover URL the server accepts for
/// `cover_image_url`.
///
/// Mirrors the https branch of site-server `normalize_cover_image_url` in
/// `routes/articles.rs` (around line 128 / PR 628): after trim, no control /
/// whitespace / backslash; prefix `https://` (ASCII case-insensitive); the
/// remainder must not start with `/` (so `https:///x` is refused); `url::Url`
/// parses with scheme `https` and a non-empty host.
pub fn is_cover_https_url(raw: &str) -> bool {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed
        .chars()
        .any(|c| c.is_control() || c.is_whitespace() || c == '\\')
    {
        return false;
    }
    let Some(rest) = trimmed
        .get(..8)
        .filter(|prefix| prefix.eq_ignore_ascii_case("https://"))
        .map(|_| &trimmed[8..])
    else {
        return false;
    };
    // `url` folds `https:///x` into host `x`; require the authority here.
    !rest.starts_with('/')
        && url::Url::parse(trimmed)
            .is_ok_and(|u| u.scheme() == "https" && u.host_str().is_some_and(|h| !h.is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_each_allowed_type_at_the_cap() {
        for mime in ["image/jpeg", "image/png", "image/webp", "image/gif"] {
            assert_eq!(validate_image_upload(mime, IMAGE_UPLOAD_MAX_BYTES), Ok(()));
        }
    }

    #[test]
    fn type_match_ignores_case_and_padding() {
        assert_eq!(validate_image_upload(" IMAGE/PNG ", 10.0), Ok(()));
    }

    #[test]
    fn rejects_other_types() {
        for mime in ["image/svg+xml", "image/bmp", "text/html", "", "image/png;x"] {
            let err = validate_image_upload(mime, 10.0).unwrap_err();
            assert!(err.contains("JPEG, PNG, WebP, or GIF"), "{mime}");
        }
    }

    #[test]
    fn rejects_over_five_mb() {
        let err = validate_image_upload("image/png", IMAGE_UPLOAD_MAX_BYTES + 1.0).unwrap_err();
        assert!(err.contains("5 MB"));
    }

    #[test]
    fn rejects_empty_file() {
        assert!(validate_image_upload("image/png", 0.0).is_err());
    }

    #[test]
    fn accept_attribute_matches_server_types() {
        assert_eq!(
            IMAGE_UPLOAD_ACCEPT,
            "image/jpeg,image/png,image/webp,image/gif"
        );
    }

    #[test]
    fn error_copy_has_no_em_dash() {
        for (mime, size) in [("text/plain", 1.0), ("image/png", 0.0), ("image/png", 9e9)] {
            assert!(
                !validate_image_upload(mime, size)
                    .unwrap_err()
                    .contains('\u{2014}')
            );
        }
    }

    #[test]
    fn cover_upload_path_accepts_valid_images_path() {
        assert!(is_cover_image_upload_path(
            "/uploads/images/member_abc/cover.png"
        ));
        assert!(is_cover_image_upload_path(
            " /uploads/images/member_other/sub/c-1_x.webp "
        ));
        assert!(is_cover_image_upload_path(
            "uploads/images/member_abc/a.png"
        ));
    }

    #[test]
    fn cover_upload_path_rejects_dotdot_segment() {
        for bad in [
            "/uploads/images/member_abc/../x.png",
            "/uploads/images/../avatars/m/x.png",
            "uploads/images/member_abc/../x.png",
        ] {
            assert!(!is_cover_image_upload_path(bad), "{bad}");
        }
    }

    #[test]
    fn cover_upload_path_rejects_avatar_path() {
        assert!(!is_cover_image_upload_path(
            "/uploads/avatars/member_abc/a.png"
        ));
    }

    #[test]
    fn cover_upload_path_rejects_other_server_rejects() {
        for bad in [
            "/uploads/",
            "/uploads/images/",
            "/uploads/images/a.png",
            "/uploads/images/member_abc/",
            "/uploads/images/member_abc/./x.png",
            "/uploads/images/member_abc//x.png",
            "/uploads/images/member_abc/x.png?v=1",
            "/uploads/images/member_abc/x.png#f",
            "/uploads/images/member_abc/%2e%2e/x.png",
            "/uploads/images/member_abc\\x.png",
            "/uploads/images/member abc/x.png",
            "/uploads/other/member_abc/a.png",
            "/assets/a.png",
        ] {
            assert!(!is_cover_image_upload_path(bad), "{bad}");
        }
    }

    #[test]
    fn upload_rel_is_safe_matches_server_rules() {
        assert!(upload_rel_is_safe("member_abc/cover.png"));
        assert!(upload_rel_is_safe("member_other/sub/c-1_x.webp"));
        assert!(!upload_rel_is_safe(""));
        assert!(!upload_rel_is_safe("member_abc/../x.png"));
        assert!(!upload_rel_is_safe("member_abc/./x.png"));
        assert!(!upload_rel_is_safe("member_abc//x.png"));
        assert!(!upload_rel_is_safe("member_abc/x.png?v=1"));
        assert!(!upload_rel_is_safe("a b/x.png"));
    }

    #[test]
    fn cover_https_url_accepts_https_with_host() {
        for raw in [
            "https://cdn.example.com/a.png",
            "HTTPS://cdn.example.com/a.png",
            "HtTpS://cdn.example.com",
            "  https://cdn.example.com/a.png?w=1#x  ",
        ] {
            assert!(is_cover_https_url(raw), "{raw}");
        }
    }

    #[test]
    fn cover_https_url_rejects_http_and_no_host() {
        for bad in [
            "http://cdn.example.com/a.png",
            "https://",
            "https:///a.png",
            "https://?x",
            "https://#x",
            "https:cdn.example.com/a.png",
            "https:/cdn.example.com/a.png",
            "javascript:alert(1)",
            "",
            "https://cdn.example.com/a b.png",
            "https://cdn.example.com\\a.png",
        ] {
            assert!(!is_cover_https_url(bad), "{bad}");
        }
    }
}
