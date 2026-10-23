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
        None => Err("Still looking for a capture method. Try again in a moment".into()),
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
    data_dir: std::path::PathBuf,
) -> Result<PreviewShot, String> {
    let backend = backend_ready(Some(backend))?;
    let img = capture::capture_screen_oneshot(&backend, output.as_deref(), &data_dir)
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

/// Settings → Install reading model. Puts back the model that ships with
/// the release (READ-1). It never trains on this host: that needs text2image,
/// which hangs or segfaults on most current distros.
pub async fn install_reading_model(install_prefix: std::path::PathBuf) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        let dirs = stat_tracker::setup::bundled_tessdata_dirs(Some(&install_prefix));
        let restored = stat_tracker::setup::restore_bundled_koverwatch(
            &dirs,
            &stat_tracker::setup::tessdata_dir(),
        );
        model_install_message(restored, &reinstall_command())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The pinned one-liner for the installed version, which ships the model.
fn reinstall_command() -> String {
    crate::update::install_command_for(None, crate::update::current_version().as_deref())
}

pub(crate) fn model_install_message(
    restored: std::io::Result<stat_tracker::setup::ModelRestore>,
    reinstall: &str,
) -> Result<String, String> {
    use stat_tracker::setup::ModelRestore;
    match restored {
        Ok(ModelRestore::AlreadyCurrent) => {
            Ok("The reading model is installed and matches this release.".into())
        }
        Ok(ModelRestore::Restored { backup: None, .. }) => {
            Ok("Reading model installed from this release.".into())
        }
        Ok(ModelRestore::Restored {
            backup: Some(bak), ..
        }) => Ok(format!(
            "Reading model installed from this release. The one that was there is kept at {}.",
            bak.display()
        )),
        Ok(ModelRestore::NoBundledCopy) => Err(format!(
            "This install has no copy of the reading model to put back. Reinstall the tracker to get it: {reinstall}"
        )),
        Err(e) => Err(format!("Could not install the reading model: {e}")),
    }
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
        return Err("Tracker started while confirming. Stop it first".into());
    }
    let result = async {
        match stat_tracker::storage::LocalStore::open(&data_dir).await {
            Ok(store) => {
                store.clear_all_data().await.map_err(|e| e.to_string())?;
                stat_tracker::storage::clear_match_log(&data_dir);
            }
            Err(e) if stat_tracker::storage::is_store_busy(e.as_ref()) => {
                return Err("Stop the tracker before deleting stored data".into());
            }
            Err(_) => {
                stat_tracker::storage::force_clear_data_dir(&data_dir)
                    .map_err(|e| e.to_string())?;
            }
        }
        // The screen-share permission goes too, so the next share asks
        // again. Same gate as the store: the daemon is stopped here.
        stat_tracker::capture::remove_restore_token(&data_dir).map_err(|e| {
            format!("Match data deleted, but the screen-share permission could not be removed: {e}")
        })?;
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
    fn missing_bundled_model_names_the_reinstall_command() {
        let err = super::model_install_message(
            Ok(stat_tracker::setup::ModelRestore::NoBundledCopy),
            "curl … | bash",
        )
        .expect_err("nothing to restore");
        assert!(err.contains("Reinstall the tracker"), "{err}");
        assert!(err.ends_with("curl … | bash"), "{err}");
    }

    #[tokio::test]
    async fn install_puts_back_the_bundled_model_without_training() {
        // The live path reads the prefix and the user tessdata dir; this
        // runs the same pieces on temp dirs.
        let prefix = tempfile::tempdir().unwrap();
        let bundle = prefix.path().join("share/scuffed-stat-tracker/tessdata");
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::write(bundle.join("koverwatch.traineddata"), b"ci").unwrap();
        let user = tempfile::tempdir().unwrap();
        let dirs = stat_tracker::setup::bundled_tessdata_dirs(Some(prefix.path()));
        let msg = super::model_install_message(
            stat_tracker::setup::restore_bundled_koverwatch(&dirs, user.path()),
            "cmd",
        )
        .expect("restored");
        assert_eq!(msg, "Reading model installed from this release.");
        assert_eq!(
            std::fs::read(user.path().join("koverwatch.traineddata")).unwrap(),
            b"ci"
        );
    }

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

    #[tokio::test]
    async fn clear_store_removes_the_screencast_restore_token() {
        let dir = tempfile::tempdir().expect("tempdir");
        let token = dir.path().join("screencast-restore-token");
        let tmp = dir.path().join("screencast-restore-token.tmp");
        std::fs::write(&token, "tok").unwrap();
        std::fs::write(&tmp, "half").unwrap();
        assert!(stat_tracker::capture::share_restorable(dir.path()));

        clear_store(dir.path().to_path_buf())
            .await
            .expect("clear with the tracker stopped");

        assert!(!stat_tracker::capture::share_restorable(dir.path()));
        assert!(!token.exists(), "restore token left behind");
        assert!(!tmp.exists(), "restore token temp file left behind");
    }

    #[tokio::test]
    async fn clear_store_does_not_wipe_a_locked_store() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _held = stat_tracker::storage::LocalStore::open(dir.path())
            .await
            .expect("open");
        std::fs::write(dir.path().join("live_snapshot.json"), "{}").unwrap();
        std::fs::write(dir.path().join("screencast-restore-token"), "tok").unwrap();
        let err = clear_store(dir.path().to_path_buf())
            .await
            .expect_err("locked store");
        assert!(
            err.contains("Stop the tracker"),
            "busy lock must not be treated as a corrupt store: {err}"
        );
        assert!(
            dir.path().join("stats.surrealkv").exists(),
            "force-clear must not run while the store lock is held"
        );
        assert!(dir.path().join("live_snapshot.json").exists());
        assert!(
            stat_tracker::capture::share_restorable(dir.path()),
            "a refused delete keeps the screen-share permission"
        );
    }
}
