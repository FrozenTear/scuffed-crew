//! Tab map-label crops, stored only on disk.
//!
//! Two lossless PNGs per game under `<data_dir>/debug/mapcrops/`:
//! `<session_id>-1.png` from the first Tab that shows the map label, and
//! `<session_id>-2.png` from a later Tab at least 60 seconds after that.
//! Each file is the top-bar map-name rectangle and nothing else. The cap is
//! 100 games, not 100 files. These files are never handed to [`crate::sync`],
//! are not part of a report zip, and are not part of the rolling `debug/poll`,
//! `debug/rejected`, `debug/accepted`, or `debug/mapmiss` rings.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use image::DynamicImage;

use crate::ocr::preprocess;

/// How many games to keep. Each game has up to two crops. Older games are
/// removed, both files together.
pub const MAPCROP_KEEP: usize = 100;

/// Wait this long after the first crop before saving the second.
const SECOND_CROP_AFTER: Duration = Duration::from_secs(60);

/// Crop ready to write. The image is already the map-name rectangle.
pub struct MapcropJob {
    dir: PathBuf,
    file_name: String,
    image: DynamicImage,
}

/// `Some` when this Tab should add a crop and the map-label rectangle fits
/// inside the frame. The first Tab gets `-1`. A later Tab gets `-2` only
/// once the first file is at least 60 seconds old.
pub fn prepare_mapcrop(
    data_dir: &Path,
    session_id: &str,
    frame: &DynamicImage,
) -> Option<MapcropJob> {
    validate_session_id(session_id)?;
    preprocess::map_name_rect(frame.width(), frame.height())?;
    let dir = mapcrop_dir(data_dir);
    let first = format!("{session_id}-1.png");
    let second = format!("{session_id}-2.png");
    let file_name = if !dir.join(&first).is_file() {
        first
    } else if dir.join(&second).is_file() || !second_crop_due(&dir.join(&first)) {
        return None;
    } else {
        second
    };
    // Same crop the reader uses, including the timer trim. The full window
    // stays inside the top bar; the trim only shortens the right edge.
    let image = preprocess::crop_map_name(frame);
    Some(MapcropJob {
        dir,
        file_name,
        image,
    })
}

/// Write `job` if that slot is still absent, then keep the newest
/// [`MAPCROP_KEEP`] games.
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

/// Save the map-name crop that this Tab is due to write.
/// Returns whether this call created a file.
pub fn save_mapcrop(data_dir: &Path, session_id: &str, frame: &DynamicImage) -> bool {
    let Some(job) = prepare_mapcrop(data_dir, session_id, frame) else {
        return false;
    };
    let dest = job.dir.join(&job.file_name);
    commit_mapcrop(job);
    dest.is_file()
}

/// True when a report zip must not contain this relative path.
/// `debug/mapcrops` and anything under it are local only.
pub fn excluded_from_report_bundle(rel: &str) -> bool {
    is_mapcrop_report_path(rel)
}

/// Stored zip of `wanted` paths under `data_dir`.
///
/// A path under `debug/mapcrops` is left out even when that file exists and
/// the caller listed it. Other listed files are copied as they are.
pub fn build_report_bundle(data_dir: &Path, wanted: &[&str]) -> Vec<u8> {
    let mut files = Vec::new();
    for rel in wanted {
        if excluded_from_report_bundle(rel) {
            continue;
        }
        if !safe_report_rel(rel) {
            continue;
        }
        let path = data_dir.join(rel);
        if !path.is_file() {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        files.push(((*rel).to_string(), bytes));
    }
    write_stored_zip(&files)
}

fn mapcrop_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("debug").join("mapcrops")
}

fn validate_session_id(session_id: &str) -> Option<()> {
    if session_id.is_empty()
        || session_id.len() > 64
        || !session_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        None
    } else {
        Some(())
    }
}

fn second_crop_due(first: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(first) else {
        return false;
    };
    let Ok(modified) = meta.modified() else {
        return false;
    };
    match SystemTime::now().duration_since(modified) {
        Ok(age) => age >= SECOND_CROP_AFTER,
        Err(_) => false,
    }
}

fn is_mapcrop_report_path(rel: &str) -> bool {
    let segs: Vec<&str> = rel
        .split(['/', '\\'])
        .filter(|seg| !seg.is_empty() && *seg != ".")
        .collect();
    segs.windows(2).any(|pair| {
        pair[0].eq_ignore_ascii_case("debug") && pair[1].eq_ignore_ascii_case("mapcrops")
    })
}

fn safe_report_rel(rel: &str) -> bool {
    !rel.is_empty()
        && !rel.starts_with('/')
        && !rel.starts_with('\\')
        && !rel.contains('\\')
        && !rel
            .split('/')
            .any(|seg| seg.is_empty() || seg == "." || seg == "..")
}

struct GameFiles {
    earliest: SystemTime,
    paths: Vec<PathBuf>,
}

fn game_id_from_file_name(name: &str) -> Option<String> {
    if name.starts_with('.') {
        return None;
    }
    let stem = name.strip_suffix(".png")?;
    if stem.is_empty() {
        return None;
    }
    let id = stem
        .strip_suffix("-1")
        .or_else(|| stem.strip_suffix("-2"))
        .unwrap_or(stem);
    if id.is_empty() {
        None
    } else {
        Some(id.to_string())
    }
}

fn prune_mapcrops(dir: &Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut games: BTreeMap<String, GameFiles> = BTreeMap::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(id) = game_id_from_file_name(name) else {
            continue;
        };
        let modified = entry
            .metadata()
            .ok()
            .and_then(|meta| meta.modified().ok())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let game = games.entry(id).or_insert_with(|| GameFiles {
            earliest: modified,
            paths: Vec::new(),
        });
        if modified < game.earliest {
            game.earliest = modified;
        }
        game.paths.push(path);
    }
    if games.len() <= keep {
        return;
    }
    let mut order: Vec<(SystemTime, String)> = games
        .iter()
        .map(|(id, game)| (game.earliest, id.clone()))
        .collect();
    order.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    let extra = order.len() - keep;
    for (_, id) in order.into_iter().take(extra) {
        if let Some(game) = games.remove(&id) {
            for path in game.paths {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

fn write_stored_zip(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut locals = Vec::new();
    let mut central = Vec::new();
    let mut offset = 0u32;
    for (name, data) in files {
        let name_bytes = name.as_bytes();
        let crc = crc32(data);
        let mut local = Vec::new();
        local.extend_from_slice(&0x04034b50u32.to_le_bytes());
        local.extend_from_slice(&20u16.to_le_bytes());
        local.extend_from_slice(&0u16.to_le_bytes());
        local.extend_from_slice(&0u16.to_le_bytes());
        local.extend_from_slice(&0u16.to_le_bytes());
        local.extend_from_slice(&0u16.to_le_bytes());
        local.extend_from_slice(&crc.to_le_bytes());
        local.extend_from_slice(&(data.len() as u32).to_le_bytes());
        local.extend_from_slice(&(data.len() as u32).to_le_bytes());
        local.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        local.extend_from_slice(&0u16.to_le_bytes());
        local.extend_from_slice(name_bytes);
        local.extend_from_slice(data);
        let mut cd = Vec::new();
        cd.extend_from_slice(&0x02014b50u32.to_le_bytes());
        cd.extend_from_slice(&20u16.to_le_bytes());
        cd.extend_from_slice(&20u16.to_le_bytes());
        cd.extend_from_slice(&0u16.to_le_bytes());
        cd.extend_from_slice(&0u16.to_le_bytes());
        cd.extend_from_slice(&0u16.to_le_bytes());
        cd.extend_from_slice(&0u16.to_le_bytes());
        cd.extend_from_slice(&crc.to_le_bytes());
        cd.extend_from_slice(&(data.len() as u32).to_le_bytes());
        cd.extend_from_slice(&(data.len() as u32).to_le_bytes());
        cd.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        cd.extend_from_slice(&0u16.to_le_bytes());
        cd.extend_from_slice(&0u16.to_le_bytes());
        cd.extend_from_slice(&0u16.to_le_bytes());
        cd.extend_from_slice(&0u16.to_le_bytes());
        cd.extend_from_slice(&0u32.to_le_bytes());
        cd.extend_from_slice(&offset.to_le_bytes());
        cd.extend_from_slice(name_bytes);
        offset += local.len() as u32;
        locals.extend(local);
        central.extend(cd);
    }
    let mut out = locals;
    let cd_offset = out.len() as u32;
    out.extend(&central);
    let cd_size = central.len() as u32;
    out.extend_from_slice(&0x06054b50u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&cd_size.to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};
    use std::fs::File;

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

    fn age_file(path: &Path, secs: u64) {
        let file = File::options().write(true).open(path).unwrap();
        file.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
            .unwrap();
    }

    fn zip_entry_names(bytes: &[u8]) -> Vec<String> {
        let mut names = Vec::new();
        let mut i = 0;
        while i + 30 <= bytes.len() && bytes[i..i + 4] == [0x50, 0x4b, 0x03, 0x04] {
            let name_len = u16::from_le_bytes([bytes[i + 26], bytes[i + 27]]) as usize;
            let extra_len = u16::from_le_bytes([bytes[i + 28], bytes[i + 29]]) as usize;
            let comp_size =
                u32::from_le_bytes([bytes[i + 18], bytes[i + 19], bytes[i + 20], bytes[i + 21]])
                    as usize;
            let start = i + 30;
            let name = std::str::from_utf8(&bytes[start..start + name_len])
                .unwrap()
                .to_string();
            names.push(name);
            i = start + name_len + extra_len + comp_size;
        }
        names
    }

    fn game_ids(dir: &Path) -> Vec<String> {
        let mut ids: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .filter_map(|entry| game_id_from_file_name(entry.file_name().to_str()?))
            .collect();
        ids.sort();
        ids.dedup();
        ids
    }

    #[test]
    fn saved_png_is_only_the_map_label_and_is_lossless() {
        let dir = tempfile::tempdir().unwrap();
        let map = [10, 220, 30];
        let names = [240, 20, 20];
        let frame = frame_with_map(1920, 1080, map, names);
        assert!(save_mapcrop(dir.path(), "sess1080", &frame));
        assert!(
            !save_mapcrop(
                dir.path(),
                "sess1080",
                &frame_with_map(1920, 1080, [1, 2, 3], names)
            ),
            "a later Tab in the same minute must not replace the first crop or write the second"
        );

        let path = dir.path().join("debug/mapcrops/sess1080-1.png");
        assert!(!dir.path().join("debug/mapcrops/sess1080-2.png").exists());
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
    fn the_second_crop_waits_60_seconds_and_is_written_once() {
        let dir = tempfile::tempdir().unwrap();
        let first = [10, 180, 40];
        let second = [20, 40, 200];
        let names = [240, 20, 20];
        assert!(save_mapcrop(
            dir.path(),
            "sesswait",
            &frame_with_map(1920, 1080, first, names)
        ));
        let slot1 = dir.path().join("debug/mapcrops/sesswait-1.png");
        age_file(&slot1, 1_700_000_000);
        assert!(save_mapcrop(
            dir.path(),
            "sesswait",
            &frame_with_map(1920, 1080, second, names)
        ));
        assert!(
            !save_mapcrop(
                dir.path(),
                "sesswait",
                &frame_with_map(1920, 1080, [1, 2, 3], names)
            ),
            "a third Tab must not replace the second crop"
        );

        let slot2 = dir.path().join("debug/mapcrops/sesswait-2.png");
        let saved = image::open(&slot2).unwrap().to_rgb8();
        assert!(saved.pixels().all(|px| px.0 == second));
        let kept = image::open(&slot1).unwrap().to_rgb8();
        assert!(kept.pixels().all(|px| px.0 == first));
    }

    #[test]
    fn the_folder_keeps_the_newest_100_games_and_leaves_other_debug_files() {
        let dir = tempfile::tempdir().unwrap();
        let frame = frame_with_map(640, 360, [8, 8, 8], [9, 9, 9]);
        let crops = dir.path().join("debug/mapcrops");
        for i in 0..MAPCROP_KEEP {
            let id = format!("old{i:03}");
            assert!(save_mapcrop(dir.path(), &id, &frame), "{id}");
            age_file(&crops.join(format!("{id}-1.png")), 1_700_000_000 + i as u64);
        }
        age_file(&crops.join("old001-1.png"), 1_700_000_000);
        assert!(
            save_mapcrop(dir.path(), "old001", &frame),
            "the second slot of a kept game is still due"
        );
        age_file(&crops.join("old001-2.png"), 1_700_000_001);
        assert_eq!(game_ids(&crops).len(), MAPCROP_KEEP);

        let rejected = dir.path().join("debug/rejected/keep.png");
        std::fs::create_dir_all(rejected.parent().unwrap()).unwrap();
        std::fs::write(&rejected, b"leave-me").unwrap();

        assert!(save_mapcrop(dir.path(), "newest", &frame));

        let ids = game_ids(&crops);
        assert_eq!(ids.len(), MAPCROP_KEEP);
        assert!(!ids.iter().any(|id| id == "old000"));
        assert!(!crops.join("old000-1.png").exists());
        assert!(crops.join("old001-1.png").is_file());
        assert!(crops.join("old001-2.png").is_file());
        assert!(crops.join("newest-1.png").is_file());
        assert_eq!(std::fs::read(&rejected).unwrap(), b"leave-me");
    }

    #[test]
    fn a_missing_region_or_a_bad_id_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let tiny = DynamicImage::ImageRgb8(RgbImage::new(8, 8));
        assert!(!save_mapcrop(dir.path(), "tiny", &tiny));
        assert!(!save_mapcrop(
            dir.path(),
            "../escape",
            &frame_with_map(640, 360, [1, 1, 1], [2, 2, 2])
        ));
        assert!(!dir.path().join("debug").exists());
        assert!(!dir.path().join("escape.png").exists());
    }

    #[test]
    fn a_report_bundle_built_while_mapcrops_exist_has_no_mapcrops_entries() {
        let dir = tempfile::tempdir().unwrap();
        let frame = frame_with_map(640, 360, [8, 8, 8], [9, 9, 9]);
        assert!(save_mapcrop(dir.path(), "sessreport", &frame));
        let slot1 = dir.path().join("debug/mapcrops/sessreport-1.png");
        age_file(&slot1, 1_700_000_000);
        assert!(save_mapcrop(dir.path(), "sessreport", &frame));
        let slot2 = dir.path().join("debug/mapcrops/sessreport-2.png");
        assert!(slot1.is_file() && slot2.is_file());

        let rejected = dir.path().join("debug/rejected/keep.png");
        std::fs::create_dir_all(rejected.parent().unwrap()).unwrap();
        std::fs::write(&rejected, b"leave-me").unwrap();
        std::fs::write(dir.path().join("log.txt"), b"synthetic log line\n").unwrap();

        let zip = build_report_bundle(
            dir.path(),
            &[
                "log.txt",
                "debug/rejected/keep.png",
                "debug/mapcrops/sessreport-1.png",
                "debug/mapcrops/sessreport-2.png",
                "Debug/MapCrops/sessreport-1.png",
            ],
        );
        let names = zip_entry_names(&zip);
        assert!(names.iter().any(|name| name == "log.txt"), "{names:?}");
        assert!(
            names.iter().any(|name| name == "debug/rejected/keep.png"),
            "{names:?}"
        );
        assert!(
            names
                .iter()
                .all(|name| !name.to_ascii_lowercase().contains("mapcrops")),
            "{names:?}"
        );
        assert!(slot1.is_file() && slot2.is_file());
    }
}
