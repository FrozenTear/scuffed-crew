//! First Tab map-label crops, stored only on disk.
//!
//! One lossless PNG per game at `<data_dir>/debug/mapcrops/<session_id>.png`.
//! The file is the top-bar map-name rectangle and nothing else. These files
//! are never handed to [`crate::sync`] and are not part of the rolling
//! `debug/poll`, `debug/rejected`, `debug/accepted`, or `debug/mapmiss` rings.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use image::DynamicImage;

use crate::ocr::preprocess::{self, MapNameRect};

/// How many map-label crops to keep. Older files are removed.
pub const MAPCROP_KEEP: usize = 100;

/// Crop ready to write. The image is already the map-name rectangle.
pub struct MapcropJob {
    dir: PathBuf,
    file_name: String,
    image: DynamicImage,
}

/// `Some` when this session does not yet have a crop and the map-label
/// rectangle fits inside the frame.
pub fn prepare_mapcrop(
    data_dir: &Path,
    session_id: &str,
    frame: &DynamicImage,
) -> Option<MapcropJob> {
    let file_name = mapcrop_file_name(session_id)?;
    let rect = preprocess::map_name_rect(frame.width(), frame.height())?;
    let dir = mapcrop_dir(data_dir);
    if dir.join(&file_name).is_file() {
        return None;
    }
    let image = crop_rect(frame, rect);
    Some(MapcropJob {
        dir,
        file_name,
        image,
    })
}

/// Write `job` if the session file is still absent, then keep the newest
/// [`MAPCROP_KEEP`] crops.
pub fn commit_mapcrop(job: MapcropJob) {
    if std::fs::create_dir_all(&job.dir).is_err() {
        return;
    }
    let dest = job.dir.join(&job.file_name);
    if dest.is_file() {
        return;
    }
    let tmp = job.dir.join(format!(".{}.partial", job.file_name));
    if job
        .image
        .save_with_format(&tmp, image::ImageFormat::Png)
        .is_err()
    {
        let _ = std::fs::remove_file(&tmp);
        return;
    }
    match std::fs::hard_link(&tmp, &dest) {
        Ok(()) => {
            let _ = std::fs::remove_file(&tmp);
        }
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            let _ = std::fs::remove_file(&tmp);
            return;
        }
        Err(_) => {
            if dest.is_file() || std::fs::rename(&tmp, &dest).is_err() {
                let _ = std::fs::remove_file(&tmp);
                return;
            }
        }
    }
    prune_mapcrops(&job.dir, MAPCROP_KEEP);
}

/// Save the map-name crop for `session_id` when it is not already on disk.
/// Returns whether the session file exists afterward and was written by this
/// call's prepare step (a second call returns false).
pub fn save_first_mapcrop(data_dir: &Path, session_id: &str, frame: &DynamicImage) -> bool {
    let Some(job) = prepare_mapcrop(data_dir, session_id, frame) else {
        return false;
    };
    let dest = job.dir.join(&job.file_name);
    commit_mapcrop(job);
    dest.is_file()
}

fn mapcrop_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("debug").join("mapcrops")
}

fn mapcrop_file_name(session_id: &str) -> Option<String> {
    if session_id.is_empty()
        || session_id.len() > 64
        || !session_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return None;
    }
    Some(format!("{session_id}.png"))
}

fn crop_rect(frame: &DynamicImage, rect: MapNameRect) -> DynamicImage {
    frame.crop_imm(rect.x, rect.y, rect.w, rect.h)
}

fn prune_mapcrops(dir: &Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut frames: Vec<(SystemTime, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_str()?;
            if name.starts_with('.') || path.extension().and_then(|ext| ext.to_str()) != Some("png")
            {
                return None;
            }
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((modified, path))
        })
        .collect();
    if frames.len() <= keep {
        return;
    }
    frames.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    let extra = frames.len() - keep;
    for (_, old) in frames.into_iter().take(extra) {
        let _ = std::fs::remove_file(old);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};
    use std::fs::File;
    use std::time::Duration;

    fn frame_with_map(w: u32, h: u32, map_color: [u8; 3], name_color: [u8; 3]) -> DynamicImage {
        let mut img = RgbImage::from_pixel(w, h, Rgb([0, 0, 0]));
        let rect = preprocess::map_name_rect(w, h).expect("map rect");
        for y in rect.y..rect.y + rect.h {
            for x in rect.x..rect.x + rect.w {
                img.put_pixel(x, y, Rgb(map_color));
            }
        }
        paint_name(&mut img, w, h, name_color);
        DynamicImage::ImageRgb8(img)
    }

    fn paint_name(img: &mut RgbImage, w: u32, h: u32, color: [u8; 3]) {
        let (gx, gy, gw, gh) = preprocess::game_rect_16_9(w, h);
        let bx = gx + (gw as f64 * 0.175) as u32;
        let by = gy + (gh as f64 * 0.15) as u32;
        let bw = (gw as f64 * 0.65) as u32;
        let bh = (gh as f64 * 0.70) as u32;
        let x0 = bx + (bw as f64 * 0.15) as u32;
        let x1 = bx + (bw as f64 * 0.38) as u32;
        let y1 = by.saturating_add(bh).min(h);
        for y in by..y1 {
            for x in x0..x1.min(w) {
                img.put_pixel(x, y, Rgb(color));
            }
        }
    }

    #[test]
    fn saved_png_is_only_the_map_label_and_is_lossless() {
        let dir = tempfile::tempdir().unwrap();
        let map = [10, 220, 30];
        let names = [240, 20, 20];
        let frame = frame_with_map(1920, 1080, map, names);
        assert!(save_first_mapcrop(dir.path(), "sess1080", &frame));
        assert!(
            !save_first_mapcrop(
                dir.path(),
                "sess1080",
                &frame_with_map(1920, 1080, [1, 2, 3], names)
            ),
            "a later Tab must not replace the first crop"
        );

        let path = dir.path().join("debug/mapcrops/sess1080.png");
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
        let saved = image::open(&path).unwrap().to_rgb8();
        let rect = preprocess::map_name_rect(1920, 1080).unwrap();
        assert_eq!((saved.width(), saved.height()), (rect.w, rect.h));
        assert!(saved.pixels().all(|px| px.0 == map));
        assert!(saved.pixels().all(|px| px.0 != names));

        let rejected = dir.path().join("debug/rejected");
        assert!(!rejected.exists(), "map crops do not use the rejected ring");
    }

    #[test]
    fn the_folder_keeps_the_newest_100_and_leaves_other_debug_files() {
        let dir = tempfile::tempdir().unwrap();
        let frame = frame_with_map(640, 360, [8, 8, 8], [9, 9, 9]);
        let crops = dir.path().join("debug/mapcrops");
        for i in 0..MAPCROP_KEEP {
            let id = format!("old{i:03}");
            assert!(save_first_mapcrop(dir.path(), &id, &frame), "{id}");
            let path = crops.join(format!("{id}.png"));
            let file = File::options().write(true).open(&path).unwrap();
            let when = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000 + i as u64);
            file.set_modified(when).unwrap();
        }
        let rejected = dir.path().join("debug/rejected/keep.png");
        std::fs::create_dir_all(rejected.parent().unwrap()).unwrap();
        std::fs::write(&rejected, b"leave-me").unwrap();

        assert!(save_first_mapcrop(dir.path(), "newest", &frame));

        let mut names: Vec<String> = std::fs::read_dir(&crops)
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".png") && !name.starts_with('.'))
            .collect();
        names.sort();
        assert_eq!(names.len(), MAPCROP_KEEP);
        assert!(!names.iter().any(|name| name == "old000.png"));
        assert!(names.iter().any(|name| name == "old001.png"));
        assert!(names.iter().any(|name| name == "newest.png"));
        assert_eq!(std::fs::read(&rejected).unwrap(), b"leave-me");
    }

    #[test]
    fn a_missing_region_or_a_bad_id_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let tiny = DynamicImage::ImageRgb8(RgbImage::new(8, 8));
        assert!(!save_first_mapcrop(dir.path(), "tiny", &tiny));
        assert!(!save_first_mapcrop(
            dir.path(),
            "../escape",
            &frame_with_map(640, 360, [1, 1, 1], [2, 2, 2])
        ));
        assert!(!dir.path().join("debug").exists());
        assert!(!dir.path().join("escape.png").exists());
    }
}
