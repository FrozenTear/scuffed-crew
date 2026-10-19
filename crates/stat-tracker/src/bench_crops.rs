//! Local crops for an accuracy benchmark.
//!
//! The usual cap is [`NORMAL_KEEP`] games under `<data_dir>/debug/benchcrops/`.
//! [`Config::keep_benchmark_crops`] (off by default) raises that cap to
//! [`BENCHMARK_KEEP`] and applies [`DISK_CEILING_BYTES`]. Turning the setting
//! off uses the usual cap on the next prune. The same switch raises the
//! `debug/accepted` and `debug/rejected` rings.
//!
//! [`export_benchmark_crops`] writes a `.tar.gz` on this machine only. It
//! does nothing when the setting is off. A whole scoreboard is blanked, or
//! left out when it cannot be blanked. Per-stat digit crops are copied as
//! they are. Logs carry counts only.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use image::{DynamicImage, ImageEncoder, RgbImage};
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::ocr::preprocess;
use crate::report::blank::{self, Chat};
use crate::report::kit::ReaderRead;
use crate::storage::PersonalMatch;

/// Settings checkbox. Off by default.
pub const SETTING_LABEL: &str = "Keep crops for the accuracy benchmark";

/// Desktop button. Shown only while [`SETTING_LABEL`] is on.
pub const EXPORT_BUTTON_LABEL: &str = "Export crops for benchmark";

/// File name under the home directory when the member does not pick a path.
pub const DEFAULT_EXPORT_NAME: &str = "scuffed-benchmark-crops.tar.gz";

/// Games kept while the setting is off.
pub const NORMAL_KEEP: usize = 30;

/// Games kept while the setting is on.
pub const BENCHMARK_KEEP: usize = 500;

/// Byte ceiling for benchmark crops and the raised debug rings.
pub const DISK_CEILING_BYTES: u64 = 512 * 1024 * 1024;

const STORE_DIR: &str = "benchcrops";
const MANIFEST_FILE: &str = "manifest.json";
const SCOREBOARD_FILE: &str = "scoreboard.png";
const CELLS_DIR: &str = "cells";

/// Stat keys whose digit crops may be exported. A name key is not in this list.
const READ_KEYS: [&str; 10] = [
    "e", "a", "d", "dmg", "h", "mit", "hero", "result", "mode", "map",
];
const STAT_KEYS: [&str; 6] = ["e", "a", "d", "dmg", "h", "mit"];

/// How many items a prune may leave, and how many bytes they may use together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CropCap {
    pub max_items: usize,
    pub max_bytes: u64,
}

/// Cap for the benchmark crop store.
pub fn crop_cap(keep_for_benchmark: bool) -> CropCap {
    if keep_for_benchmark {
        CropCap {
            max_items: BENCHMARK_KEEP,
            max_bytes: DISK_CEILING_BYTES,
        }
    } else {
        CropCap {
            max_items: NORMAL_KEEP,
            max_bytes: u64::MAX,
        }
    }
}

/// Cap for an existing debug ring whose usual size is `normal_keep`.
pub fn ring_cap(keep_for_benchmark: bool, normal_keep: usize) -> CropCap {
    if keep_for_benchmark {
        CropCap {
            max_items: BENCHMARK_KEEP.max(normal_keep),
            max_bytes: DISK_CEILING_BYTES,
        }
    } else {
        CropCap {
            max_items: normal_keep,
            max_bytes: u64::MAX,
        }
    }
}

/// True only while the member has opted in. The desktop button uses this.
pub fn export_button_visible(opted_in: bool) -> bool {
    opted_in
}

/// `ocr-v1` for the classic reader, `cv-v1` for every cv reader id.
pub fn recognizer_tag(stored: &str) -> &'static str {
    let tag = stored.trim();
    if tag.is_empty()
        || tag.eq_ignore_ascii_case("ocr-v1")
        || tag.to_ascii_lowercase().starts_with("ocr")
    {
        "ocr-v1"
    } else {
        "cv-v1"
    }
}

pub fn tracker_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// `<home>/scuffed-benchmark-crops.tar.gz`, or `./` that name when home is unknown.
pub fn default_export_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(DEFAULT_EXPORT_NAME)
}

/// One read in a game manifest. No player names.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManifestRead {
    pub value: String,
    pub confidence: Option<f64>,
}

/// Manifest written into the export for one game.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GameManifest {
    pub tracker_version: String,
    pub recognizer: String,
    pub resolution: String,
    pub map: Option<String>,
    pub hero: Option<String>,
    pub reads: BTreeMap<String, ManifestRead>,
}

/// On-disk manifest. `team_size` stays local so export can blank the board.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct StoredManifest {
    tracker_version: String,
    recognizer: String,
    resolution: String,
    map: Option<String>,
    hero: Option<String>,
    reads: BTreeMap<String, ManifestRead>,
    #[serde(default)]
    team_size: Option<u32>,
}

/// One accepted capture to keep for the benchmark.
pub struct AcceptedCrop {
    pub session_id: String,
    pub width: u32,
    pub height: u32,
    pub map_name: String,
    pub hero: String,
    pub recognizer: String,
    pub team_size: Option<usize>,
    pub row_index: Option<usize>,
    pub scoreboard: std::sync::Arc<DynamicImage>,
    pub reads: BTreeMap<String, ManifestRead>,
}

/// Result of a local export. Counts only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportCounts {
    pub games: usize,
    pub files: usize,
}

#[derive(Debug)]
pub enum ExportError {
    /// The setting is off. No file is written.
    OptInOff,
    Io(io::Error),
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OptInOff => write!(f, "benchmark crop export is off"),
            Self::Io(_) => write!(f, "could not write the benchmark export"),
        }
    }
}

impl std::error::Error for ExportError {}

impl From<io::Error> for ExportError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

fn flag() -> &'static AtomicBool {
    static FLAG: AtomicBool = AtomicBool::new(false);
    &FLAG
}

fn config_path_slot() -> &'static Mutex<Option<PathBuf>> {
    static PATH: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();
    PATH.get_or_init(|| Mutex::new(None))
}

fn store_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|err| err.into_inner())
}

/// Remember where `config.toml` lives so a later prune sees a Settings save.
pub fn install_runtime(config_path: Option<PathBuf>, opted_in: bool) {
    flag().store(opted_in, Ordering::Relaxed);
    *config_path_slot()
        .lock()
        .unwrap_or_else(|err| err.into_inner()) = config_path;
}

/// The setting as saved now, or the value from [`install_runtime`] when the
/// file cannot be read.
pub fn opted_in() -> bool {
    let path = config_path_slot()
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone();
    if let Some(path) = path
        && path.is_file()
        && let Ok(cfg) = Config::read_at(&path)
    {
        return cfg.keep_benchmark_crops;
    }
    flag().load(Ordering::Relaxed)
}

pub fn store_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("debug").join(STORE_DIR)
}

/// Reads for a stored row. Name keys are dropped. A stat the capture gate
/// replaced has no confidence.
pub fn reads_from_match(
    row: &PersonalMatch,
    confidences: &BTreeMap<String, ReaderRead>,
) -> BTreeMap<String, ManifestRead> {
    let mut reads = BTreeMap::new();
    let mut put = |key: &str, value: String| {
        let confidence = confidences.get(key).and_then(|read| {
            if read.value.as_deref() == Some(value.as_str()) {
                finite_confidence(read.confidence)
            } else {
                None
            }
        });
        if let Some(read) = clean_read(key, &ManifestRead { value, confidence }) {
            reads.insert(key.to_string(), read);
        }
    };
    put("e", row.elims.to_string());
    put("a", row.assists.to_string());
    put("d", row.deaths.to_string());
    put("dmg", row.damage.to_string());
    put("h", row.healing.to_string());
    put("mit", row.mitigation.to_string());
    if let Some(hero) = clean_label(&row.hero) {
        put("hero", hero);
    }
    if let Some(map) = clean_label(&row.map_name) {
        put("map", map);
    }
    reads
}

/// Write one game, then prune to `opted_in`'s cap.
pub fn record_accepted(data_dir: &Path, opted_in: bool, crop: AcceptedCrop) -> io::Result<()> {
    let _guard = store_lock();
    let Some(session) = safe_id(&crop.session_id) else {
        return Ok(());
    };
    let dir = store_dir(data_dir).join(session);
    private_dir(&dir)?;
    let team = match crop.team_size {
        Some(5) | Some(6) => crop.team_size,
        _ => None,
    };
    if let (Some(team_size), Some(row)) = (team, crop.row_index) {
        write_digit_crops(&dir.join(CELLS_DIR), &crop.scoreboard, team_size, row)?;
    }
    if let Some(png) = blank_scoreboard_png(&crop.scoreboard.to_rgb8(), team) {
        write_private(&dir, SCOREBOARD_FILE, &png)?;
    } else if dir.join(SCOREBOARD_FILE).is_file() {
        let _ = fs::remove_file(dir.join(SCOREBOARD_FILE));
    }
    let stored = StoredManifest {
        tracker_version: tracker_version().to_string(),
        recognizer: recognizer_tag(&crop.recognizer).to_string(),
        resolution: format!("{}x{}", crop.width, crop.height),
        map: clean_label(&crop.map_name),
        hero: clean_label(&crop.hero),
        reads: crop
            .reads
            .iter()
            .filter_map(|(key, read)| clean_read(key, read).map(|read| (key.clone(), read)))
            .collect(),
        team_size: team.map(|n| n as u32),
    };
    let json = serde_json::to_vec_pretty(&stored).map_err(io::Error::other)?;
    write_private(&dir, MANIFEST_FILE, &json)?;
    let removed = prune_games_locked(data_dir, crop_cap(opted_in))?;
    if removed > 0 {
        tracing::info!(removed, "benchmark crops pruned");
    }
    Ok(())
}

/// Prune the benchmark store and the two board-pixel rings to the cap in effect.
pub fn prune_runtime(data_dir: &Path, accepted_keep: usize, rejected_keep: usize) {
    let on = opted_in();
    let _ = prune_png_ring(
        &data_dir.join("debug").join("accepted"),
        ring_cap(on, accepted_keep),
    );
    let _ = prune_png_ring(
        &data_dir.join("debug").join("rejected"),
        ring_cap(on, rejected_keep),
    );
    let _guard = store_lock();
    if let Ok(removed) = prune_games_locked(data_dir, crop_cap(on))
        && removed > 0
    {
        tracing::info!(removed, "benchmark crops pruned");
    }
}

/// Delete oldest PNGs in `dir` until both limits hold.
pub fn prune_png_ring(dir: &Path, limits: CropCap) -> io::Result<usize> {
    if !dir.is_dir() {
        return Ok(0);
    }
    let mut items = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("png") {
            continue;
        }
        let meta = entry.metadata()?;
        if !meta.is_file() {
            continue;
        }
        items.push(Item {
            mtime: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            bytes: meta.len(),
            path,
        });
    }
    let drop = select_drop(items, limits);
    let removed = drop.len();
    for path in drop {
        let _ = fs::remove_file(path);
    }
    if removed > 0 {
        tracing::info!(removed, "debug crops pruned");
    }
    Ok(removed)
}

/// Delete oldest games until `limits` holds.
pub fn prune_games(data_dir: &Path, limits: CropCap) -> io::Result<usize> {
    let _guard = store_lock();
    prune_games_locked(data_dir, limits)
}

fn prune_games_locked(data_dir: &Path, limits: CropCap) -> io::Result<usize> {
    let root = store_dir(data_dir);
    if !root.is_dir() {
        return Ok(0);
    }
    let mut items = Vec::new();
    for entry in fs::read_dir(&root)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if safe_id(name).is_none() {
            continue;
        }
        items.push(Item {
            mtime: tree_mtime(&path),
            bytes: tree_bytes(&path),
            path,
        });
    }
    let drop = select_drop(items, limits);
    let removed = drop.len();
    for path in drop {
        let _ = fs::remove_dir_all(path);
    }
    Ok(removed)
}

struct Item {
    mtime: SystemTime,
    bytes: u64,
    path: PathBuf,
}

fn select_drop(mut items: Vec<Item>, limits: CropCap) -> Vec<PathBuf> {
    items.sort_by(|a, b| a.mtime.cmp(&b.mtime).then_with(|| a.path.cmp(&b.path)));
    let mut total: u64 = items.iter().map(|item| item.bytes).sum();
    let mut dropped = Vec::new();
    while !items.is_empty() && (items.len() > limits.max_items || total > limits.max_bytes) {
        let old = items.remove(0);
        total = total.saturating_sub(old.bytes);
        dropped.push(old.path);
    }
    dropped
}

/// Write a `.tar.gz` of the kept crops. Refuses when `opted_in` is false
/// and does not create `dest` in that case.
pub fn export_benchmark_crops(
    opted_in: bool,
    data_dir: &Path,
    dest: &Path,
) -> Result<ExportCounts, ExportError> {
    if !opted_in {
        return Err(ExportError::OptInOff);
    }
    let _guard = store_lock();
    let mut files = Vec::new();
    let root = store_dir(data_dir);
    if root.is_dir() {
        let mut games: Vec<PathBuf> = fs::read_dir(&root)
            .map_err(ExportError::Io)?
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| safe_id(name).is_some())
            })
            .collect();
        games.sort();
        for game in games {
            let Some(name) = game.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let Some(stored) = read_manifest(&game) else {
                continue;
            };
            let public = public_manifest(&stored);
            let json = serde_json::to_vec_pretty(&public)
                .map_err(|err| ExportError::Io(io::Error::other(err)))?;
            files.push((format!("games/{name}/{MANIFEST_FILE}"), json));
            if let Ok(png) = fs::read(game.join(SCOREBOARD_FILE))
                && let Some(blanked) = blank_scoreboard_bytes(&png, stored.team_size)
            {
                files.push((format!("games/{name}/{SCOREBOARD_FILE}"), blanked));
            }
            let cells = game.join(CELLS_DIR);
            if cells.is_dir() {
                let mut names: Vec<String> = fs::read_dir(&cells)
                    .map_err(ExportError::Io)?
                    .flatten()
                    .filter_map(|entry| entry.file_name().into_string().ok())
                    .filter(|name| stat_file_name(name))
                    .collect();
                names.sort();
                for file_name in names {
                    let path = cells.join(&file_name);
                    let Ok(meta) = fs::metadata(&path) else {
                        continue;
                    };
                    // A full board stuffed into the cells folder is not a digit crop.
                    if meta.len() > 256 * 1024 {
                        continue;
                    }
                    let Ok(bytes) = fs::read(&path) else {
                        continue;
                    };
                    if !looks_like_digit_crop(&bytes) {
                        continue;
                    }
                    files.push((format!("games/{name}/{CELLS_DIR}/{file_name}"), bytes));
                }
            }
        }
    }
    let games = files
        .iter()
        .filter(|(path, _)| path.ends_with(MANIFEST_FILE))
        .count();
    let file_count = files.len();
    write_tar_gz(dest, &files)?;
    tracing::info!(games, files = file_count, "benchmark crops exported");
    Ok(ExportCounts {
        games,
        files: file_count,
    })
}

fn public_manifest(stored: &StoredManifest) -> GameManifest {
    GameManifest {
        tracker_version: clean_version(&stored.tracker_version),
        recognizer: recognizer_tag(&stored.recognizer).to_string(),
        resolution: clean_resolution(&stored.resolution),
        map: stored.map.as_deref().and_then(clean_label),
        hero: stored.hero.as_deref().and_then(clean_label),
        reads: stored
            .reads
            .iter()
            .filter_map(|(key, read)| clean_read(key, read).map(|read| (key.clone(), read)))
            .collect(),
    }
}

fn read_manifest(dir: &Path) -> Option<StoredManifest> {
    let bytes = fs::read(dir.join(MANIFEST_FILE)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn blank_scoreboard_png(rgb: &RgbImage, team_size: Option<usize>) -> Option<Vec<u8>> {
    let blanked = blank::blank_crop(rgb, team_size, Chat::Unknown);
    if !blanked.has_picture() {
        return None;
    }
    encode_png(&blanked.image).ok()
}

fn blank_scoreboard_bytes(png: &[u8], team_size: Option<u32>) -> Option<Vec<u8>> {
    let team = match team_size {
        Some(5) => Some(5usize),
        Some(6) => Some(6usize),
        _ => None,
    };
    let img = image::load_from_memory(png).ok()?;
    blank_scoreboard_png(&img.to_rgb8(), team)
}

fn write_digit_crops(
    dir: &Path,
    board: &DynamicImage,
    team_size: usize,
    row: usize,
) -> io::Result<()> {
    let Some(row_img) = preprocess::crop_player_row(board, row, team_size) else {
        return Ok(());
    };
    let columns = preprocess::columns_with_offset(0.0);
    for (index, key) in STAT_KEYS.iter().enumerate() {
        let Some(cell) = preprocess::crop_stat_cell(&row_img, index, &columns) else {
            continue;
        };
        let rgb = cell.to_rgb8();
        if rgb.width() > 512 || rgb.height() > 256 {
            continue;
        }
        if let Ok(png) = encode_png(&rgb) {
            private_dir(dir)?;
            write_private(dir, &format!("{key}.png"), &png)?;
        }
    }
    Ok(())
}

fn looks_like_digit_crop(png: &[u8]) -> bool {
    let Ok(img) = image::load_from_memory(png) else {
        return false;
    };
    img.width() > 0 && img.height() > 0 && img.width() <= 512 && img.height() <= 256
}

fn stat_file_name(name: &str) -> bool {
    let Some(stem) = name.strip_suffix(".png") else {
        return false;
    };
    STAT_KEYS.contains(&stem)
}

fn safe_id(raw: &str) -> Option<&str> {
    let ok = !raw.is_empty()
        && raw.len() <= 64
        && raw
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_');
    ok.then_some(raw)
}

fn clean_label(raw: &str) -> Option<String> {
    let text: String = raw
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(64)
        .collect();
    (!text.is_empty()).then_some(text)
}

fn clean_version(raw: &str) -> String {
    let ok = !raw.is_empty()
        && raw.len() <= 32
        && raw
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '+');
    if ok {
        raw.to_string()
    } else {
        tracker_version().to_string()
    }
}

fn clean_resolution(raw: &str) -> String {
    let mut parts = raw.trim().split('x');
    let width = parts.next().and_then(|part| part.parse::<u32>().ok());
    let height = parts.next().and_then(|part| part.parse::<u32>().ok());
    match (parts.next(), width, height) {
        (None, Some(width), Some(height)) => format!("{width}x{height}"),
        _ => "0x0".to_string(),
    }
}

fn clean_read(key: &str, read: &ManifestRead) -> Option<ManifestRead> {
    if !READ_KEYS.contains(&key) {
        return None;
    }
    let value: String = read
        .value
        .chars()
        .filter(|c| !c.is_control())
        .take(64)
        .collect();
    if value.is_empty() {
        return None;
    }
    if STAT_KEYS.contains(&key)
        && !value
            .chars()
            .all(|c| c.is_ascii_digit() || c == ',' || c == '.')
    {
        return None;
    }
    Some(ManifestRead {
        value,
        confidence: read.confidence.and_then(finite_confidence),
    })
}

fn finite_confidence(value: f64) -> Option<f64> {
    value.is_finite().then_some(value.clamp(0.0, 1.0))
}

fn encode_png(img: &RgbImage) -> io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    image::codecs::png::PngEncoder::new(&mut buf)
        .write_image(
            img.as_raw(),
            img.width(),
            img.height(),
            image::ExtendedColorType::Rgb8,
        )
        .map_err(io::Error::other)?;
    Ok(buf)
}

fn private_dir(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn write_private(dir: &Path, name: &str, bytes: &[u8]) -> io::Result<()> {
    let tmp = dir.join(format!(".{name}.partial"));
    {
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let written = opts.open(&tmp).and_then(|mut file| {
            file.write_all(bytes)?;
            file.sync_all()
        });
        if let Err(err) = written {
            let _ = fs::remove_file(&tmp);
            return Err(err);
        }
    }
    if let Err(err) = fs::rename(&tmp, dir.join(name)) {
        let _ = fs::remove_file(&tmp);
        return Err(err);
    }
    Ok(())
}

fn tree_bytes(dir: &Path) -> u64 {
    let mut total = 0u64;
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_file() {
            total = total.saturating_add(meta.len());
        } else if meta.is_dir() {
            total = total.saturating_add(tree_bytes(&entry.path()));
        }
    }
    total
}

fn tree_mtime(dir: &Path) -> SystemTime {
    let mut newest = fs::metadata(dir)
        .and_then(|meta| meta.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let Ok(entries) = fs::read_dir(dir) else {
        return newest;
    };
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if let Ok(modified) = meta.modified()
            && modified > newest
        {
            newest = modified;
        }
        if meta.is_dir() {
            let nested = tree_mtime(&entry.path());
            if nested > newest {
                newest = nested;
            }
        }
    }
    newest
}

fn write_tar_gz(dest: &Path, files: &[(String, Vec<u8>)]) -> Result<(), ExportError> {
    if let Some(parent) = dest.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    let file_name = dest
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(DEFAULT_EXPORT_NAME);
    let tmp = dest.with_file_name(format!(".{file_name}.partial"));
    let result = (|| {
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let file = opts.open(&tmp)?;
        let mut gzip = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        gzip.write_all(&tar_bytes(files))?;
        gzip.finish()?.sync_all()?;
        Ok::<(), io::Error>(())
    })();
    if let Err(err) = result {
        let _ = fs::remove_file(&tmp);
        return Err(ExportError::Io(err));
    }
    if let Err(err) = fs::rename(&tmp, dest) {
        let _ = fs::remove_file(&tmp);
        return Err(ExportError::Io(err));
    }
    Ok(())
}

fn tar_bytes(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (path, bytes) in files {
        out.extend_from_slice(&tar_header(path, bytes.len() as u64));
        out.extend_from_slice(bytes);
        let pad = (512 - (bytes.len() % 512)) % 512;
        out.extend(std::iter::repeat_n(0u8, pad));
    }
    out.extend_from_slice(&[0u8; 1024]);
    out
}

fn tar_header(path: &str, size: u64) -> [u8; 512] {
    let mut header = [0u8; 512];
    let name = path.as_bytes();
    let n = name.len().min(100);
    header[..n].copy_from_slice(&name[..n]);
    put_octal(&mut header[100..108], 0o644);
    put_octal(&mut header[108..116], 0);
    put_octal(&mut header[116..124], 0);
    put_octal(&mut header[124..136], size);
    put_octal(&mut header[136..148], 0);
    header[148..156].fill(b' ');
    header[156] = b'0';
    header[257..263].copy_from_slice(b"ustar\0");
    header[263..265].copy_from_slice(b"00");
    let sum: u32 = header.iter().map(|byte| u32::from(*byte)).sum();
    let checksum = format!("{sum:06o}\0 ");
    header[148..156].copy_from_slice(checksum.as_bytes());
    header
}

fn put_octal(dst: &mut [u8], value: u64) {
    let digits = dst.len() - 1;
    let text = format!("{value:0digits$o}");
    let bytes = text.as_bytes();
    let n = bytes.len().min(digits);
    let start = bytes.len() - n;
    dst[..n].copy_from_slice(&bytes[start..]);
    dst[n] = 0;
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgb;

    fn touch(path: &Path, bytes: usize, age: u64) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, vec![b'x'; bytes]).unwrap();
        let file = fs::File::options().write(true).open(path).unwrap();
        let when = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000 + age);
        file.set_modified(when).unwrap();
    }

    #[test]
    fn cap_switches_with_the_setting() {
        let off = crop_cap(false);
        let on = crop_cap(true);
        assert_eq!(off.max_items, NORMAL_KEEP);
        assert_eq!(off.max_items, 30);
        assert_eq!(on.max_items, BENCHMARK_KEEP);
        assert_eq!(on.max_items, 500);
        assert_eq!(on.max_bytes, DISK_CEILING_BYTES);
        assert!(on.max_bytes < off.max_bytes);
        assert_eq!(ring_cap(false, 60).max_items, 60);
        assert_eq!(ring_cap(false, 30).max_items, 30);
        assert_eq!(ring_cap(true, 60).max_items, 500);
        assert_eq!(ring_cap(true, 30).max_bytes, DISK_CEILING_BYTES);
        assert!(!export_button_visible(false));
        assert!(export_button_visible(true));
        assert_eq!(SETTING_LABEL, "Keep crops for the accuracy benchmark");
        assert_eq!(EXPORT_BUTTON_LABEL, "Export crops for benchmark");
    }

    #[test]
    fn recognizer_tag_is_ocr_v1_or_cv_v1() {
        assert_eq!(recognizer_tag(""), "ocr-v1");
        assert_eq!(recognizer_tag("ocr-v1"), "ocr-v1");
        assert_eq!(recognizer_tag("OCR-v1"), "ocr-v1");
        assert_eq!(recognizer_tag("cv-v1"), "cv-v1");
        assert_eq!(recognizer_tag("cv-v7"), "cv-v1");
        assert_eq!(recognizer_tag("cv-v7-noocr"), "cv-v1");
    }

    #[test]
    fn missing_config_key_leaves_the_setting_off() {
        let cfg: Config = toml::from_str("data_dir = \"/tmp/x\"").unwrap();
        assert!(!cfg.keep_benchmark_crops);
        let cfg: Config =
            toml::from_str("data_dir = \"/tmp/x\"\nkeep_benchmark_crops = true").unwrap();
        assert!(cfg.keep_benchmark_crops);
    }

    #[test]
    fn prune_returns_to_the_normal_cap_when_the_setting_is_off() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..40u64 {
            touch(
                &store_dir(dir.path())
                    .join(format!("game{i:02}"))
                    .join("note.txt"),
                8,
                i,
            );
        }
        let removed = prune_games(dir.path(), crop_cap(false)).unwrap();
        assert_eq!(removed, 10);
        let mut names = game_names(dir.path());
        names.sort();
        assert_eq!(names.len(), NORMAL_KEEP);
        assert!(!names.contains(&"game09".to_string()));
        assert!(names.contains(&"game10".to_string()));
        assert!(names.contains(&"game39".to_string()));
    }

    #[test]
    fn prune_respects_the_byte_ceiling() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..5u64 {
            touch(
                &store_dir(dir.path()).join(format!("g{i}")).join("crop.bin"),
                1000,
                i,
            );
        }
        let limits = CropCap {
            max_items: BENCHMARK_KEEP,
            max_bytes: 2500,
        };
        let removed = prune_games(dir.path(), limits).unwrap();
        assert_eq!(removed, 3);
        let names = game_names(dir.path());
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"g3".to_string()));
        assert!(names.contains(&"g4".to_string()));
        let bytes: u64 = names
            .iter()
            .map(|name| tree_bytes(&store_dir(dir.path()).join(name)))
            .sum();
        assert!(bytes <= 2500, "bytes={bytes}");
    }

    #[test]
    fn png_ring_respects_the_ceiling() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..4u64 {
            touch(&dir.path().join(format!("frame{i}.png")), 800, i);
        }
        touch(&dir.path().join("notes.txt"), 50_000, 0);
        let removed = prune_png_ring(
            dir.path(),
            CropCap {
                max_items: 500,
                max_bytes: 2000,
            },
        )
        .unwrap();
        assert_eq!(removed, 2);
        assert!(dir.path().join("notes.txt").is_file(), "non-png stays");
        assert!(dir.path().join("frame2.png").is_file());
        assert!(dir.path().join("frame3.png").is_file());
        assert!(!dir.path().join("frame0.png").exists());
    }

    #[test]
    fn export_with_opt_in_off_refuses() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out.tar.gz");
        let err = export_benchmark_crops(false, dir.path(), &dest).unwrap_err();
        assert!(matches!(err, ExportError::OptInOff));
        assert!(!dest.exists());
    }

    #[test]
    fn manifest_fields_are_present_and_name_keys_are_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let mut reads = reads_from_match(&sample_row(), &confidences());
        reads.insert(
            "name".into(),
            ManifestRead {
                value: "SecretName".into(),
                confidence: Some(0.99),
            },
        );
        record_accepted(
            dir.path(),
            true,
            AcceptedCrop {
                session_id: "game-a".into(),
                width: 1920,
                height: 1080,
                map_name: "Busan".into(),
                hero: "Ana".into(),
                recognizer: "cv-v7".into(),
                team_size: Some(5),
                row_index: None,
                scoreboard: std::sync::Arc::new(DynamicImage::ImageRgb8(RgbImage::from_pixel(
                    8,
                    8,
                    Rgb([1, 2, 3]),
                ))),
                reads,
            },
        )
        .unwrap();
        let dest = dir.path().join("bench.tar.gz");
        let counts = export_benchmark_crops(true, dir.path(), &dest).unwrap();
        assert_eq!(counts.games, 1);
        assert!(counts.files >= 1);
        let text = manifest_text(&dest, "games/game-a/manifest.json");
        assert!(!text.contains("SecretName"), "{text}");
        assert!(!text.contains("\"name\""));
        let manifest: GameManifest = serde_json::from_str(&text).unwrap();
        assert_eq!(manifest.tracker_version, tracker_version());
        assert_eq!(manifest.recognizer, "cv-v1");
        assert_eq!(manifest.resolution, "1920x1080");
        assert_eq!(manifest.map.as_deref(), Some("Busan"));
        assert_eq!(manifest.hero.as_deref(), Some("Ana"));
        let elims = &manifest.reads["e"];
        assert_eq!(elims.value, "12");
        assert_eq!(elims.confidence, Some(0.8));
        assert!(manifest.reads["dmg"].confidence.is_none());
    }

    #[test]
    fn exported_whole_scoreboard_drops_name_pixels() {
        use crate::report::blank::synth::{self, Layout};
        let dir = tempfile::tempdir().unwrap();
        let game = store_dir(dir.path()).join("board1");
        fs::create_dir_all(game.join(CELLS_DIR)).unwrap();
        let mut board = synth::board(Layout::L1);
        let sentinel = Rgb([255, 0, 128]);
        let (x0, y0, x1, y1) = synth::name_boxes(Layout::L1)[0];
        for y in y0..y1 {
            for x in x0..x1 {
                board.put_pixel(x, y, sentinel);
            }
        }
        let raw = encode_png(&board).unwrap();
        fs::write(game.join(SCOREBOARD_FILE), &raw).unwrap();
        let digit = encode_png(&RgbImage::from_pixel(4, 4, Rgb([9, 9, 9]))).unwrap();
        fs::write(game.join(CELLS_DIR).join("e.png"), &digit).unwrap();
        let stored = StoredManifest {
            tracker_version: tracker_version().to_string(),
            recognizer: "ocr-v1".into(),
            resolution: "2560x1440".into(),
            map: Some("Ilios".into()),
            hero: Some("Ana".into()),
            reads: BTreeMap::from([(
                "e".into(),
                ManifestRead {
                    value: "1".into(),
                    confidence: Some(0.5),
                },
            )]),
            team_size: Some(6),
        };
        fs::write(
            game.join(MANIFEST_FILE),
            serde_json::to_vec_pretty(&stored).unwrap(),
        )
        .unwrap();
        // A full-board image named like a digit crop must not ride along.
        fs::write(game.join(CELLS_DIR).join("a.png"), &raw).unwrap();

        let dest = dir.path().join("out.tar.gz");
        let counts = export_benchmark_crops(true, dir.path(), &dest).unwrap();
        assert!(counts.games >= 1);
        let extracted = dir.path().join("out");
        fs::create_dir_all(&extracted).unwrap();
        let status = std::process::Command::new("tar")
            .args(["-xzf"])
            .arg(&dest)
            .arg("-C")
            .arg(&extracted)
            .status()
            .unwrap();
        assert!(status.success());
        let scoreboard = extracted.join("games/board1/scoreboard.png");
        if scoreboard.is_file() {
            let img = image::open(&scoreboard).unwrap().to_rgb8();
            assert!(
                img.pixels().all(|pixel| *pixel != sentinel),
                "name pixels stayed in the exported scoreboard"
            );
            assert_ne!(fs::read(&scoreboard).unwrap(), raw);
        }
        let cell = fs::read(extracted.join("games/board1/cells/e.png")).unwrap();
        assert_eq!(cell, digit);
        assert!(
            !extracted.join("games/board1/cells/a.png").exists(),
            "a whole board must not export as a digit crop"
        );
        let manifest: GameManifest = serde_json::from_str(
            &fs::read_to_string(extracted.join("games/board1/manifest.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(manifest.recognizer, "ocr-v1");
        assert_eq!(manifest.resolution, "2560x1440");
        assert_eq!(manifest.map.as_deref(), Some("Ilios"));
        assert_eq!(manifest.hero.as_deref(), Some("Ana"));
        assert_eq!(manifest.reads["e"].value, "1");
        assert_eq!(manifest.reads["e"].confidence, Some(0.5));
    }

    #[test]
    fn default_export_path_is_a_file_in_home_or_the_working_directory() {
        let path = default_export_path();
        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some(DEFAULT_EXPORT_NAME)
        );
    }

    fn game_names(data_dir: &Path) -> Vec<String> {
        fs::read_dir(store_dir(data_dir))
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect()
    }

    fn sample_row() -> PersonalMatch {
        PersonalMatch {
            id: None,
            hero: "Ana".into(),
            map_name: "Busan".into(),
            game_mode: String::new(),
            role: "Support".into(),
            outcome: String::new(),
            elims: 12,
            deaths: 2,
            assists: 3,
            damage: 4000,
            healing: 800,
            mitigation: 0,
            played_at: surrealdb_types::Datetime::from(chrono::Utc::now()),
            synced: false,
            sync_rev: 0,
            upload_reject: None,
            session_id: "game-a".into(),
            corrected_hero: None,
            corrected_role: None,
            corrected_map_name: None,
            corrected_outcome: None,
            corrected_elims: None,
            corrected_deaths: None,
            corrected_assists: None,
            corrected_damage: None,
            corrected_healing: None,
            corrected_mitigation: None,
            edited_fields: Vec::new(),
            edited_at: None,
            heroes_played: Vec::new(),
            segment_resolutions: Vec::new(),
            recognizer: "cv-v7".into(),
            suspect_fields: Vec::new(),
        }
    }

    fn confidences() -> BTreeMap<String, ReaderRead> {
        let mut map = BTreeMap::new();
        map.insert(
            "e".into(),
            ReaderRead {
                confidence: 0.8,
                value: Some("12".into()),
            },
        );
        map.insert(
            "dmg".into(),
            ReaderRead {
                confidence: 0.4,
                value: Some("1".into()),
            },
        );
        map
    }

    fn manifest_text(archive: &Path, member: &str) -> String {
        let out = archive.parent().unwrap().join("extracted");
        fs::create_dir_all(&out).unwrap();
        let status = std::process::Command::new("tar")
            .args(["-xzf"])
            .arg(archive)
            .arg("-C")
            .arg(&out)
            .status()
            .unwrap();
        assert!(status.success(), "tar could not read the export");
        fs::read_to_string(out.join(member)).unwrap()
    }
}
