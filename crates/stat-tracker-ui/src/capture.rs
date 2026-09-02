//! One-shot capture preview through the existing capture backends.
//!
//! This is not a software layout `--preview` path and not `preview.rs`.

use image::DynamicImage;
use stat_tracker::capture::{self, CaptureBackend};

#[derive(Debug, Clone)]
pub struct PreviewShot {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub captured_at: String,
}

/// Same 800×450 thumbnail the Dioxus preview used.
pub fn thumbnail_rgba(img: &DynamicImage) -> (u32, u32, Vec<u8>) {
    let thumb = img.thumbnail(800, 450);
    let rgba = thumb.to_rgba8();
    (rgba.width(), rgba.height(), rgba.into_raw())
}

pub fn backend_ready(backend: Option<CaptureBackend>) -> Result<CaptureBackend, String> {
    match backend {
        None => Err("Still looking for a capture method — try again in a moment".into()),
        Some(CaptureBackend::None) => Err("No capture method is available on this computer".into()),
        Some(b) => Ok(b),
    }
}

pub async fn list_outputs(backend: CaptureBackend) -> Vec<String> {
    capture::list_outputs(backend).await.unwrap_or_default()
}

pub async fn capture_now(
    backend: CaptureBackend,
    output: Option<String>,
) -> Result<PreviewShot, String> {
    let backend = backend_ready(Some(backend))?;
    let img = capture::capture_screen_output(&backend, output.as_deref())
        .await
        .map_err(|e| format!("Capture failed: {e}"))?;
    let (width, height, rgba) = tokio::task::spawn_blocking(move || thumbnail_rgba(&img))
        .await
        .map_err(|e| format!("Could not prepare the preview: {e}"))?;
    Ok(PreviewShot {
        width,
        height,
        rgba,
        captured_at: chrono::Local::now().format("%H:%M:%S").to_string(),
    })
}

pub fn tessdata_installed() -> bool {
    stat_tracker::setup::tessdata_dir()
        .join("koverwatch.traineddata")
        .exists()
}

pub async fn install_reading_model() -> Result<String, String> {
    tokio::task::spawn_blocking(|| {
        stat_tracker::setup::ensure_koverwatch_tessdata()
            .map(|_| "Reading model is installed".to_string())
            .map_err(|e| format!("Could not install the reading model: {e}"))
    })
    .await
    .map_err(|e| e.to_string())?
}

pub async fn rebuild_reading_model() -> Result<String, String> {
    tokio::task::spawn_blocking(|| {
        stat_tracker::setup::regenerate_koverwatch_tessdata()
            .map(|_| "Reading model rebuilt".to_string())
            .map_err(|e| format!("Could not rebuild the reading model: {e}"))
    })
    .await
    .map_err(|e| e.to_string())?
}

pub async fn vacuum_store(data_dir: std::path::PathBuf) -> Result<String, String> {
    if crate::daemon::is_daemon_running(&data_dir) {
        return Err("Stop the tracker before compacting stored data".into());
    }
    let before = stat_tracker::storage::maintain::store_size_bytes(&data_dir);
    let (matches, sessions, _tombstones) = stat_tracker::storage::LocalStore::vacuum(&data_dir)
        .await
        .map_err(|e| format!("Could not compact stored data: {e}"))?;
    let after = stat_tracker::storage::maintain::store_size_bytes(&data_dir);
    Ok(format!(
        "Compacted {matches} games, {sessions} sessions ({:.1} → {:.1} MB)",
        before as f64 / 1e6,
        after as f64 / 1e6
    ))
}

pub async fn clear_store(data_dir: std::path::PathBuf) -> Result<String, String> {
    if crate::daemon::is_daemon_running(&data_dir) {
        return Err("Tracker started while confirming — stop it first".into());
    }
    let result = async {
        match stat_tracker::storage::LocalStore::open(&data_dir).await {
            Ok(store) => {
                store.clear_all_data().await.map_err(|e| e.to_string())?;
                stat_tracker::storage::clear_match_log(&data_dir);
            }
            Err(_) => {
                stat_tracker::storage::force_clear_data_dir(&data_dir)
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok::<(), String>(())
    }
    .await;
    result.map(|()| "All local match data deleted".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgba};

    #[test]
    fn backend_ready_rejects_missing_and_none() {
        assert!(backend_ready(None).is_err());
        assert!(backend_ready(Some(CaptureBackend::None)).is_err());
        assert_eq!(
            backend_ready(Some(CaptureBackend::Wayshot)).unwrap(),
            CaptureBackend::Wayshot
        );
    }

    #[test]
    fn thumbnail_fits_preview_budget() {
        let img = DynamicImage::ImageRgba8(ImageBuffer::<Rgba<u8>, _>::from_pixel(
            1920,
            1080,
            Rgba([1, 2, 3, 255]),
        ));
        let (w, h, rgba) = thumbnail_rgba(&img);
        assert!(w <= 800 && h <= 450, "{w}x{h}");
        assert_eq!(rgba.len(), (w * h * 4) as usize);
    }
}
