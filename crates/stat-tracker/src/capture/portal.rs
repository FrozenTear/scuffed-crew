use std::path::{Path, PathBuf};

use image::DynamicImage;

pub async fn is_available() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP").is_ok()
}

pub async fn capture() -> Result<DynamicImage, Box<dyn std::error::Error + Send + Sync>> {
    let response = ashpd::desktop::screenshot::ScreenshotRequest::default()
        .interactive(false)
        .modal(false)
        .send()
        .await
        .map_err(|e| format!("portal screenshot request failed: {e}"))?
        .response()
        .map_err(|e| format!("portal screenshot response failed: {e}"))?;

    let uri = response.uri().to_string();
    let path = file_uri_path(&uri).ok_or("unexpected screenshot URI: not a local file")?;
    // The portal wrote this file for us. It goes on every path out of here,
    // a decode error included, so no screenshot is left behind.
    let file = RemoveOnDrop(path);

    let convert_started = std::time::Instant::now();
    let img = image::open(&file.0).map_err(|e| format!("failed to open portal screenshot: {e}"))?;
    crate::perf::record_grab_convert(convert_started.elapsed());
    drop(file);

    tracing::debug!(
        width = img.width(),
        height = img.height(),
        "screen captured via XDG Desktop Portal"
    );

    Ok(img)
}

/// Deletes the file when dropped.
struct RemoveOnDrop(PathBuf);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        remove_quietly(&self.0);
    }
}

fn remove_quietly(path: &Path) {
    if let Err(e) = std::fs::remove_file(path)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        tracing::debug!(error = %e, "could not remove the portal screenshot");
    }
}

/// The local path of a `file://` URI, percent-decoded. The portal encodes
/// spaces and other bytes (GNOME names the file "Screenshot from ...png",
/// sent as `Screenshot%20from%20...`), and before alpha.12 the encoded
/// path was opened as is, failed, and left the screenshot on disk. `None`
/// for any other scheme, a remote host, or a bad escape.
pub fn file_uri_path(uri: &str) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt as _;
    let rest = uri.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    if !rest.starts_with('/') {
        return None;
    }
    let raw = rest.split(['?', '#']).next()?.as_bytes();
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'%' {
            let hex = raw.get(i + 1..i + 3)?;
            let byte = u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?;
            out.push(byte);
            i += 3;
        } else {
            out.push(raw[i]);
            i += 1;
        }
    }
    if out.contains(&0) {
        return None;
    }
    Some(PathBuf::from(std::ffi::OsString::from_vec(out)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_uri_path_decodes_percent_escapes() {
        assert_eq!(
            file_uri_path("file:///home/a/Pictures/Screenshot%20from%202026-10-18%2014-02-11.png"),
            Some(PathBuf::from(
                "/home/a/Pictures/Screenshot from 2026-10-18 14-02-11.png"
            ))
        );
        assert_eq!(
            file_uri_path("file:///tmp/Screenshot_20261018_140211.png"),
            Some(PathBuf::from("/tmp/Screenshot_20261018_140211.png"))
        );
        assert_eq!(
            file_uri_path("file://localhost/tmp/a%C3%A9.png"),
            Some(PathBuf::from("/tmp/a\u{e9}.png"))
        );
    }

    #[test]
    fn file_uri_path_refuses_what_is_not_a_local_file() {
        for uri in [
            "https://example.com/a.png",
            "file://otherhost/tmp/a.png",
            "file:///tmp/a%2.png",
            "file:///tmp/a%zz.png",
            "file:///tmp/a%00.png",
            "/tmp/a.png",
            "",
        ] {
            assert_eq!(file_uri_path(uri), None, "{uri}");
        }
    }

    #[test]
    fn the_screenshot_is_removed_even_when_it_does_not_decode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Screenshot from 2026-10-18.png");
        std::fs::write(&path, b"not a png").unwrap();
        let uri = format!("file://{}", path.to_str().unwrap().replace(' ', "%20"));
        let resolved = file_uri_path(&uri).unwrap();
        assert_eq!(resolved, path);
        let result = {
            let file = RemoveOnDrop(resolved);
            image::open(&file.0)
        };
        assert!(result.is_err());
        assert!(!path.exists(), "a failed decode left the screenshot behind");
        // A file that is already gone is not an error.
        drop(RemoveOnDrop(path));
    }
}
