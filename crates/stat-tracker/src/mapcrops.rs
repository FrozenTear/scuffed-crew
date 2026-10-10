//! Tab map-label crops, stored only on disk.
//!
//! Two lossless PNGs per game under `<data_dir>/debug/mapcrops/`:
//! `<session_id>-<width>x<height>-1.png` from the first Tab whose map-label
//! box has text, and `<session_id>-<width>x<height>-2.png` from a later Tab
//! at least 60 seconds after that. Width and height are the capture size.
//! Each file is the full map-label box at native resolution, raw pixels,
//! with no timer trim and no brighten or invert. The reader still trims
//! when it reads. The cap is 100 games, not 100 files. Both files of a game
//! are removed together. These files are never handed to [`crate::sync`]
//! and are not part of the rolling debug rings. Report zips must leave
//! `debug/mapcrops/` out. The helpers here are not called by a report
//! builder yet.

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

/// `Some` when this Tab should add a crop. The label box must fit in the
/// frame and must not be a flat bar. The first such Tab gets slot 1. A
/// later Tab gets slot 2 only once slot 1 is at least 60 seconds old.
/// A blank bar does not take a slot.
pub fn prepare_mapcrop(
    data_dir: &Path,
    session_id: &str,
    frame: &DynamicImage,
) -> Option<MapcropJob> {
    validate_session_id(session_id)?;
    if !preprocess::map_label_has_text(frame) {
        return None;
    }
    let image = preprocess::crop_map_label_box(frame)?;
    let dir = mapcrop_dir(data_dir);
    let slots = session_slots(&dir, session_id);
    let slot = if !slots.has_first {
        1
    } else if slots.has_second || !second_crop_due(slots.first_mtime) {
        return None;
    } else {
        2
    };
    let file_name = crop_file_name(session_id, frame.width(), frame.height(), slot);
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

/// Manifest `files` paths. `debug/mapcrops/` is never listed.
///
/// Nothing in the daemon calls this yet. The real report builder must use
/// it when that flow lands, so a manifest cannot name a map-label crop.
pub fn report_manifest_paths(candidates: &[&str]) -> Vec<String> {
    candidates
        .iter()
        .copied()
        .filter(|path| !excluded_from_report_bundle(path))
        .map(str::to_string)
        .collect()
}

/// Stored zip of `wanted` paths under `data_dir`.
///
/// A path under `debug/mapcrops` is left out even when that file exists and
/// the caller listed it. Nothing in the daemon calls this yet. The real
/// report builder must use the same exclusion when reports land.
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

fn crop_file_name(session_id: &str, width: u32, height: u32, slot: u8) -> String {
    format!("{session_id}-{width}x{height}-{slot}.png")
}

struct Slots {
    has_first: bool,
    first_mtime: Option<SystemTime>,
    has_second: bool,
}

fn session_slots(dir: &Path, session_id: &str) -> Slots {
    let mut slots = Slots {
        has_first: false,
        first_mtime: None,
        has_second: false,
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return slots;
    };
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().into_string().ok() else {
            continue;
        };
        let Some(parsed) = parse_crop_name(&name) else {
            continue;
        };
        if parsed.game_id != session_id {
            continue;
        }
        if parsed.slot == 1 {
            slots.has_first = true;
            if let Ok(modified) = entry.metadata().and_then(|meta| meta.modified()) {
                slots.first_mtime = Some(match slots.first_mtime {
                    Some(have) => have.min(modified),
                    None => modified,
                });
            }
        } else if parsed.slot == 2 {
            slots.has_second = true;
        }
    }
    slots
}

fn second_crop_due(first_mtime: Option<SystemTime>) -> bool {
    let Some(modified) = first_mtime else {
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

struct ParsedCrop {
    game_id: String,
    slot: u8,
}

/// `<session>-<width>x<height>-1.png`, or a legacy `<session>-1.png`.
/// Both slots of one session are one game, whatever the capture size was.
fn parse_crop_name(name: &str) -> Option<ParsedCrop> {
    if name.starts_with('.') {
        return None;
    }
    let stem = name.strip_suffix(".png")?;
    let (rest, slot) = if let Some(rest) = stem.strip_suffix("-1") {
        (rest, 1u8)
    } else {
        let rest = stem.strip_suffix("-2")?;
        (rest, 2)
    };
    if rest.is_empty() {
        return None;
    }
    let game_id = strip_capture_size(rest).unwrap_or(rest);
    if game_id.is_empty() {
        None
    } else {
        Some(ParsedCrop {
            game_id: game_id.to_string(),
            slot,
        })
    }
}

fn strip_capture_size(rest: &str) -> Option<&str> {
    let dash = rest.rfind('-')?;
    let (width, height) = rest[dash + 1..].split_once('x')?;
    if width.is_empty()
        || height.is_empty()
        || width.starts_with('0')
        || height.starts_with('0')
        || !width.bytes().all(|b| b.is_ascii_digit())
        || !height.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    Some(&rest[..dash])
}

fn game_id_from_file_name(name: &str) -> Option<String> {
    parse_crop_name(name).map(|parsed| parsed.game_id)
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
        paint_label_ink(&mut img, map_color);
        paint_name(&mut img, w, h, name_color);
        DynamicImage::ImageRgb8(img)
    }

    /// A small glyph on a dark bar. A solid fill of the box is flat and must not save.
    fn paint_label_ink(img: &mut RgbImage, color: [u8; 3]) {
        let rect = preprocess::map_name_rect(img.width(), img.height()).expect("map rect");
        let bw = 16.min(rect.w);
        let bh = 8.min(rect.h);
        for y in rect.y..rect.y + bh {
            for x in rect.x..rect.x + bw {
                img.put_pixel(x, y, Rgb(color));
            }
        }
    }

    fn frame_flat(w: u32, h: u32, color: [u8; 3]) -> DynamicImage {
        let mut img = RgbImage::from_pixel(w, h, Rgb([0, 0, 0]));
        let rect = preprocess::map_name_rect(w, h).expect("map rect");
        for y in rect.y..rect.y + rect.h {
            for x in rect.x..rect.x + rect.w {
                img.put_pixel(x, y, Rgb(color));
            }
        }
        DynamicImage::ImageRgb8(img)
    }

    fn crop_path(dir: &Path, id: &str, w: u32, h: u32, slot: u8) -> PathBuf {
        dir.join("debug/mapcrops")
            .join(format!("{id}-{w}x{h}-{slot}.png"))
    }

    fn assert_saved_is_raw_box(path: &Path, frame: &DynamicImage) {
        let saved = image::open(path).unwrap().to_rgb8();
        let raw = preprocess::crop_map_label_box(frame)
            .expect("box")
            .to_rgb8();
        let rect = preprocess::map_name_rect(frame.width(), frame.height()).unwrap();
        assert_eq!((saved.width(), saved.height()), (rect.w, rect.h));
        assert_eq!(saved.as_raw(), raw.as_raw());
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
    fn saved_png_is_the_full_label_box_and_is_lossless() {
        let dir = tempfile::tempdir().unwrap();
        let map = [40, 220, 80];
        let names = [240, 20, 20];
        let frame = frame_with_map(1920, 1080, map, names);
        assert!(save_mapcrop(dir.path(), "sess1080", &frame));
        assert!(
            !save_mapcrop(
                dir.path(),
                "sess1080",
                &frame_with_map(1920, 1080, [220, 80, 80], names)
            ),
            "a later Tab in the same minute must not replace the first crop or write the second"
        );

        let path = crop_path(dir.path(), "sess1080", 1920, 1080, 1);
        assert!(!crop_path(dir.path(), "sess1080", 1920, 1080, 2).exists());
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
        assert_saved_is_raw_box(&path, &frame);
        let saved = image::open(&path).unwrap().to_rgb8();
        assert!(saved.pixels().any(|px| px.0 == map));
        assert!(saved.pixels().all(|px| px.0 != names));

        let rejected = dir.path().join("debug/rejected");
        assert!(!rejected.exists(), "map crops do not use the rejected ring");
    }

    #[test]
    fn the_saved_box_keeps_the_timer_the_reader_trims() {
        let dir = tempfile::tempdir().unwrap();
        let mut img = RgbImage::from_pixel(1920, 1080, Rgb([0, 0, 0]));
        let rect = preprocess::map_name_rect(1920, 1080).unwrap();
        let name = [220, 220, 220];
        let timer = [255, 255, 255];
        for y in rect.y..rect.y + rect.h {
            for x in rect.x..rect.x + 80 {
                img.put_pixel(x, y, Rgb(name));
            }
            for x in rect.x + 450..rect.x + 470 {
                img.put_pixel(x, y, Rgb(timer));
            }
        }
        let frame = DynamicImage::ImageRgb8(img);
        let trimmed = preprocess::crop_map_name(&frame).to_rgb8();
        assert!(
            trimmed.width() < rect.w,
            "reader trim width {} should be under the box {}",
            trimmed.width(),
            rect.w
        );
        assert!(save_mapcrop(dir.path(), "sessclock", &frame));
        let path = crop_path(dir.path(), "sessclock", 1920, 1080, 1);
        assert_saved_is_raw_box(&path, &frame);
        let saved = image::open(&path).unwrap().to_rgb8();
        assert_eq!(saved.get_pixel(450, 0).0, timer);
        assert!(trimmed.width() <= 450);
    }

    #[test]
    fn a_blank_top_bar_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!save_mapcrop(
            dir.path(),
            "sessblank",
            &frame_flat(1920, 1080, [0, 0, 0])
        ));
        assert!(
            !save_mapcrop(
                dir.path(),
                "sessblank",
                &frame_flat(1920, 1080, [107, 107, 107])
            ),
            "a flat mid-grey bar is still empty"
        );
        assert!(!save_mapcrop(
            dir.path(),
            "sessblank",
            &frame_flat(1920, 1080, [250, 250, 250])
        ));
        assert!(!dir.path().join("debug/mapcrops").exists());

        let faded = frame_with_map(1920, 1080, [107, 107, 107], [240, 20, 20]);
        assert!(
            save_mapcrop(dir.path(), "sessblank", &faded),
            "a blank first Tab must not consume slot 1"
        );
        let path = crop_path(dir.path(), "sessblank", 1920, 1080, 1);
        assert!(path.is_file());
        assert_saved_is_raw_box(&path, &faded);
    }

    #[test]
    fn a_faded_label_is_saved_raw() {
        let dir = tempfile::tempdir().unwrap();
        let faded = frame_with_map(1920, 1080, [107, 107, 107], [240, 20, 20]);
        assert!(preprocess::map_label_has_text(&faded));
        assert!(save_mapcrop(dir.path(), "lijiang", &faded));
        let path = crop_path(dir.path(), "lijiang", 1920, 1080, 1);
        assert_saved_is_raw_box(&path, &faded);
        let saved = image::open(&path).unwrap().to_rgb8();
        assert!(saved.pixels().any(|px| px.0 == [107, 107, 107]));
        assert!(saved.pixels().all(|px| {
            let luma = (u16::from(px.0[0]) + u16::from(px.0[1]) + u16::from(px.0[2])) / 3;
            luma < 170
        }));
        let raw = preprocess::crop_map_label_box(&faded).unwrap();
        let retry =
            preprocess::map_label_retry_image(&raw).expect("this fade is in the retry range");
        assert_ne!(
            (saved.width(), saved.height()),
            (retry.width(), retry.height()),
            "the saved PNG is the raw box, not the stretched retry"
        );
    }

    #[test]
    fn a_small_fade_saves_when_the_retry_refuses_the_crop() {
        let dir = tempfile::tempdir().unwrap();
        let mut img = RgbImage::from_pixel(1920, 1080, Rgb([0, 0, 0]));
        let rect = preprocess::map_name_rect(1920, 1080).unwrap();
        for y in rect.y..rect.y + 2 {
            for x in rect.x..rect.x + 4 {
                img.put_pixel(x, y, Rgb([107, 107, 107]));
            }
        }
        let frame = DynamicImage::ImageRgb8(img);
        let raw = preprocess::crop_map_label_box(&frame).unwrap();
        assert!(preprocess::map_label_retry_image(&raw).is_none());
        assert!(save_mapcrop(dir.path(), "thin", &frame));
        assert_saved_is_raw_box(&crop_path(dir.path(), "thin", 1920, 1080, 1), &frame);
    }

    #[test]
    fn the_second_crop_waits_60_seconds_and_uses_that_frames_size() {
        let dir = tempfile::tempdir().unwrap();
        let names = [240, 20, 20];
        let first = frame_with_map(1920, 1080, [240, 240, 40], names);
        let second = frame_with_map(2560, 1440, [40, 180, 240], names);
        assert!(save_mapcrop(dir.path(), "sesswait", &first));
        let slot1 = crop_path(dir.path(), "sesswait", 1920, 1080, 1);
        age_file(&slot1, 1_700_000_000);
        assert!(save_mapcrop(dir.path(), "sesswait", &second));
        assert!(
            !save_mapcrop(
                dir.path(),
                "sesswait",
                &frame_with_map(1920, 1080, [220, 80, 80], names)
            ),
            "a third Tab must not replace the second crop"
        );

        let slot2 = crop_path(dir.path(), "sesswait", 2560, 1440, 2);
        assert!(slot2.is_file());
        assert!(!crop_path(dir.path(), "sesswait", 1920, 1080, 2).exists());
        assert_saved_is_raw_box(&slot1, &first);
        assert_saved_is_raw_box(&slot2, &second);
    }

    #[test]
    fn the_folder_keeps_the_newest_100_games_and_leaves_other_debug_files() {
        let dir = tempfile::tempdir().unwrap();
        let small = frame_with_map(640, 360, [220, 220, 220], [240, 20, 20]);
        let wide = frame_with_map(1920, 1080, [180, 40, 220], [240, 20, 20]);
        let crops = dir.path().join("debug/mapcrops");
        for i in 0..MAPCROP_KEEP {
            let id = format!("old{i:03}");
            assert!(save_mapcrop(dir.path(), &id, &small), "{id}");
            age_file(
                &crops.join(format!("{id}-640x360-1.png")),
                1_700_000_000 + i as u64,
            );
        }
        age_file(&crops.join("old000-640x360-1.png"), 1_700_000_000);
        assert!(save_mapcrop(dir.path(), "old000", &wide));
        age_file(&crops.join("old000-1920x1080-2.png"), 1_700_000_050);
        age_file(&crops.join("old001-640x360-1.png"), 1_700_000_100);
        assert!(
            save_mapcrop(dir.path(), "old001", &wide),
            "the second slot of a kept game is still due"
        );
        age_file(&crops.join("old001-1920x1080-2.png"), 1_700_000_150);
        assert_eq!(game_ids(&crops).len(), MAPCROP_KEEP);

        let rejected = dir.path().join("debug/rejected/keep.png");
        std::fs::create_dir_all(rejected.parent().unwrap()).unwrap();
        std::fs::write(&rejected, b"leave-me").unwrap();

        assert!(save_mapcrop(dir.path(), "newest", &small));

        let ids = game_ids(&crops);
        assert_eq!(ids.len(), MAPCROP_KEEP);
        assert!(!ids.iter().any(|id| id == "old000"));
        assert!(!crops.join("old000-640x360-1.png").exists());
        assert!(!crops.join("old000-1920x1080-2.png").exists());
        assert!(crops.join("old001-640x360-1.png").is_file());
        assert!(crops.join("old001-1920x1080-2.png").is_file());
        assert!(crops.join("newest-640x360-1.png").is_file());
        let pngs = std::fs::read_dir(&crops)
            .unwrap()
            .flatten()
            .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("png"))
            .count();
        assert_eq!(pngs, MAPCROP_KEEP + 1);
        assert_eq!(std::fs::read(&rejected).unwrap(), b"leave-me");
    }

    #[test]
    fn crop_names_group_one_session_across_sizes() {
        assert_eq!(
            game_id_from_file_name("ab-cd-1920x1080-1.png").as_deref(),
            Some("ab-cd")
        );
        assert_eq!(
            game_id_from_file_name("ab-cd-3440x1440-2.png").as_deref(),
            Some("ab-cd")
        );
        assert_eq!(
            game_id_from_file_name("ab-cd-1.png").as_deref(),
            Some("ab-cd")
        );
        assert_eq!(
            game_id_from_file_name("ab-cd-2.png").as_deref(),
            Some("ab-cd")
        );
        assert!(game_id_from_file_name("ab-cd.png").is_none());
        assert_eq!(
            game_id_from_file_name("ab-cd-01920x1080-1.png").as_deref(),
            Some("ab-cd-01920x1080")
        );
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
        let frame = frame_with_map(640, 360, [220, 220, 220], [240, 20, 20]);
        assert!(save_mapcrop(dir.path(), "sessreport", &frame));
        let slot1 = crop_path(dir.path(), "sessreport", 640, 360, 1);
        age_file(&slot1, 1_700_000_000);
        assert!(save_mapcrop(dir.path(), "sessreport", &frame));
        let slot2 = crop_path(dir.path(), "sessreport", 640, 360, 2);
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
                "debug/mapcrops/sessreport-640x360-1.png",
                "debug/mapcrops/sessreport-640x360-2.png",
                "Debug/MapCrops/sessreport-640x360-1.png",
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

    #[test]
    fn report_manifest_paths_leave_mapcrops_out() {
        let listed = report_manifest_paths(&[
            "log.txt",
            "debug/rejected/keep.png",
            "debug/mapcrops/sess-1920x1080-1.png",
            "debug/mapcrops/ab-cd-3440x1440-2.png",
            "Debug/MapCrops/sess-1920x1080-1.png",
            r"debug\mapcrops\sess-1.png",
        ]);
        assert_eq!(
            listed,
            vec!["log.txt".to_string(), "debug/rejected/keep.png".to_string(),]
        );
    }
}
