//! Per-session report kit: what a game report needs, kept by the daemon
//! while the game is captured.
//!
//! `<data_dir>/report-kit/<session_id>/` holds:
//!
//! - `scoreboard.png`: the newest accepted scoreboard of the session, blanked
//!   at capture time ([`super::blank::blanked_scoreboard`]). No unblanked
//!   frame is written for reports, and a board the blanker keeps nothing of
//!   writes no kit.
//! - `outcome.png`: the result word box of the competitive rank screen
//!   ([`super::blank::outcome_band`]), when the tracker saw that screen.
//! - `reads.json`: the reads of the capture `scoreboard.png` came from.
//!
//! Up to [`Retention::max_games`] sessions are kept, within
//! [`Retention::max_bytes`] in total, oldest trimmed first. The directory is private (0700,
//! files 0600) and every file is replaced atomically. `reads.json` carries
//! the hash of the `scoreboard.png` it was written with, and [`load`] only
//! returns a matching pair: the app reads a kit while the daemon replaces it.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use image::{DynamicImage, ImageEncoder, RgbImage};
use serde::{Deserialize, Serialize};

use super::blank::{self, TeamLayout};
pub use crate::config::{DEFAULT_MAX_GAMES, DEFAULT_MAX_MB, Retention};

pub const KIT_DIR: &str = "report-kit";

static RETENTION_GAMES: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(DEFAULT_MAX_GAMES);
static RETENTION_BYTES: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(DEFAULT_MAX_MB * 1024 * 1024);

/// Set the retention the daemon prunes with after each kit write.
pub fn set_retention(r: Retention) {
    use std::sync::atomic::Ordering::Relaxed;
    RETENTION_GAMES.store(r.max_games.max(1), Relaxed);
    RETENTION_BYTES.store(r.max_bytes, Relaxed);
}

pub fn retention() -> Retention {
    use std::sync::atomic::Ordering::Relaxed;
    Retention {
        max_games: RETENTION_GAMES.load(Relaxed),
        max_bytes: RETENTION_BYTES.load(Relaxed),
    }
}

/// A kit older than this (14 days) is deleted on the next prune.
pub const MAX_AGE: std::time::Duration = std::time::Duration::from_secs(14 * 24 * 60 * 60);

pub const SCOREBOARD_PNG: &str = "scoreboard.png";
pub const OUTCOME_PNG: &str = "outcome.png";
pub const READS_JSON: &str = "reads.json";
/// Player names read on this session's boards. Local only: never put in a
/// report zip, only handed to the log scrubber so those names are removed.
pub const NAMES_JSON: &str = "names.json";
/// Most names kept per session (two full 6v6 boards plus misreads).
const NAMES_MAX: usize = 64;

/// One read in `reads.json`, the same shape as a manifest read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KitRead {
    pub value: Option<String>,
    /// The new reader's confidence for this read, 0 to 1
    /// ([`KitReads::with_confidences`]). `null` under ocr-v1, for a stat
    /// the capture gate replaced, and for a result filled from the stored
    /// game ([`KitReads::fill_result`]).
    pub confidence: Option<f64>,
    /// The read before the capture gate (spec section 2). `reader =
    /// "ocr-v1"`: the Tesseract read. `reader = "new"`: Tesseract reads no
    /// stat cell, so for the six stats this is the digit read (for a held
    /// cell, the stored value). [`KitReads::recognizer`] tells the two
    /// apart. The wire name is unchanged.
    pub ocr_v1: Option<String>,
    pub suspect: bool,
}

/// `reads.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KitReads {
    pub captured_at: DateTime<Utc>,
    pub width: u32,
    pub height: u32,
    /// `null` when the board was blanked with both layouts.
    pub team_size: Option<u32>,
    pub map: Option<String>,
    pub mode: Option<String>,
    pub result: Option<String>,
    /// The stored row's recognizer tag: the matcher id (`cv-v5`) for a
    /// capture whose stats came from the digit reader, `ocr-v1` for a
    /// Tesseract capture. Under `reader = "new"` every stored capture has
    /// at least four digit-read stats, so this is always the matcher id
    /// there, and the stats' `ocr_v1` are then digit reads too.
    pub recognizer: String,
    /// [`blank::BLANK_TABLE_VERSION`] the scoreboard was blanked with. A kit
    /// from another table is not reported ([`load`] drops its scoreboard).
    #[serde(default)]
    pub blank_table: u32,
    /// SHA-256 (hex) of the `scoreboard.png` written together with these
    /// reads. The daemon replaces the two files one after the other, so a
    /// reader in another process can land between the two renames; [`load`]
    /// only returns a pair whose hash matches.
    #[serde(default)]
    pub scoreboard_sha256: String,
    /// Keyed by the manifest read keys (`mode`, `result`, `hero`, `e`, `a`,
    /// `d`, `dmg`, `h`, `mit`).
    pub reads: BTreeMap<String, KitRead>,
}

impl KitReads {
    /// Reads of one accepted capture. `before` is the match as it was read,
    /// before the capture gate changed a stat (the `ocr_v1` read: the
    /// Tesseract pass under `reader = "ocr-v1"`, the digit reads under
    /// `reader = "new"`); `after` is what was stored.
    pub fn from_capture(
        before: &crate::storage::PersonalMatch,
        after: &crate::storage::PersonalMatch,
        width: u32,
        height: u32,
        layout: TeamLayout,
        captured_at: DateTime<Utc>,
    ) -> Self {
        let suspects = crate::reader_apply::upload_suspect_fields(&after.suspect_fields);
        let text = |s: &str| {
            let s = s.trim();
            (!s.is_empty()).then(|| s.chars().take(64).collect::<String>())
        };
        let mut reads = BTreeMap::new();
        // Spec section 2: a read is suspect when the tracker flags it, when
        // its value is null, or (stats) when the stored value and the
        // Tesseract read disagree.
        let mut put = |key: &str, value: Option<String>, ocr: Option<String>, disagree: bool| {
            let suspect = suspects.iter().any(|f| f == key) || value.is_none() || disagree;
            reads.insert(
                key.to_string(),
                KitRead {
                    value,
                    confidence: None,
                    ocr_v1: ocr,
                    suspect,
                },
            );
        };
        put(
            "mode",
            text(&after.game_mode),
            text(&before.game_mode),
            false,
        );
        put("result", text(&after.outcome), text(&before.outcome), false);
        put("hero", text(&after.hero), text(&before.hero), false);
        for (key, a, b) in [
            ("e", after.elims, before.elims),
            ("a", after.assists, before.assists),
            ("d", after.deaths, before.deaths),
            ("dmg", after.damage, before.damage),
            ("h", after.healing, before.healing),
            ("mit", after.mitigation, before.mitigation),
        ] {
            put(key, Some(a.to_string()), Some(b.to_string()), a != b);
        }
        Self {
            captured_at,
            width,
            height,
            team_size: layout.team_size(),
            map: text(&after.map_name),
            mode: text(&after.game_mode),
            result: text(&after.outcome),
            blank_table: blank::BLANK_TABLE_VERSION,
            scoreboard_sha256: String::new(),
            recognizer: if after.recognizer.is_empty() {
                "ocr-v1".to_string()
            } else {
                after.recognizer.clone()
            },
            reads,
        }
    }
}

/// One field as the active reader read it: its confidence and its own
/// value (before the capture gate), for [`KitReads::with_confidences`].
#[derive(Debug, Clone, PartialEq)]
pub struct ReaderRead {
    pub confidence: f64,
    pub value: Option<String>,
}

impl ReaderRead {
    pub fn from_field(f: &crate::shadow::FieldRead) -> Self {
        Self {
            confidence: f64::from(f.confidence),
            value: f.value.as_ref().map(|v| match v {
                crate::shadow::Value::Int(n) => n.to_string(),
                crate::shadow::Value::Text(t) => t.trim().to_string(),
            }),
        }
    }
}

/// True unless `result` is a decided outcome (victory, defeat, draw).
pub fn result_is_unknown(result: &str) -> bool {
    let r = result.trim();
    !["victory", "defeat", "draw"]
        .iter()
        .any(|o| r.eq_ignore_ascii_case(o))
}

impl KitReads {
    /// Fill each read's `confidence` from the active reader's (keyed by read
    /// key). A stat the capture gate replaced (the stored value is not the
    /// reader's own read) gets none: the confidence belongs to the read, not
    /// to the stored value. A reader read that disagrees with Tesseract
    /// (`ocr_v1`) keeps its confidence. NaN and infinite give none; other
    /// values are clamped to 0..1.
    pub fn with_confidences(mut self, confidences: &BTreeMap<String, ReaderRead>) -> Self {
        for (key, read) in self.reads.iter_mut() {
            let Some(r) = confidences.get(key) else {
                continue;
            };
            let c = r.confidence;
            let gated = matches!(key.as_str(), "e" | "a" | "d" | "dmg" | "h" | "mit")
                && (r.value.is_none() || read.value != r.value);
            if !gated && c.is_finite() {
                read.confidence = Some(c.clamp(0.0, 1.0));
            }
        }
        self
    }

    /// The game's result as stored when the game closed, for a kit whose
    /// Tab capture came before the result was known. The Tab read stays in
    /// `ocr_v1`. A known kit result is never replaced.
    pub fn fill_result(&mut self, stored_outcome: &str) {
        let known = |s: &str| !result_is_unknown(s);
        if !known(stored_outcome) || self.result.as_deref().is_some_and(known) {
            return;
        }
        let outcome = stored_outcome.trim().chars().take(64).collect::<String>();
        self.result = Some(outcome.clone());
        let read = self.reads.entry("result".into()).or_insert(KitRead {
            value: None,
            confidence: None,
            ocr_v1: None,
            suspect: false,
        });
        read.value = Some(outcome);
        read.confidence = None;
        read.suspect = false;
    }
}

/// What [`load`] found for one session.
#[derive(Debug, Clone, Default)]
pub struct Kit {
    pub scoreboard_png: Option<Vec<u8>>,
    pub outcome_png: Option<Vec<u8>>,
    pub reads: Option<KitReads>,
}

/// A session id is used as a directory name only when it is plain: ASCII
/// letters, digits, `-`, `_`, 1 to 64 characters. Anything else has no kit.
fn safe_session(session_id: &str) -> Option<&str> {
    let ok = !session_id.is_empty()
        && session_id.len() <= 64
        && session_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    ok.then_some(session_id)
}

pub fn kit_root(data_dir: &Path) -> PathBuf {
    data_dir.join(KIT_DIR)
}

pub fn session_dir(data_dir: &Path, session_id: &str) -> Option<PathBuf> {
    safe_session(session_id).map(|s| kit_root(data_dir).join(s))
}

/// Serialises kit writes in this process: accepted captures hand their kit
/// to blocking tasks, and those can finish out of order.
static WRITE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn write_lock() -> std::sync::MutexGuard<'static, ()> {
    WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn read_reads(dir: &Path) -> Option<KitReads> {
    fs::read(dir.join(READS_JSON))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
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

/// Write `bytes` to `dir/name` through a private temp file and a rename.
fn write_private(dir: &Path, name: &str, bytes: &[u8]) -> io::Result<()> {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = dir.join(format!(".{name}.{}.{seq}.tmp", std::process::id()));
    {
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let written = opts.open(&tmp).and_then(|mut f| {
            f.write_all(bytes)?;
            f.sync_all()
        });
        if let Err(e) = written {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
    }
    if let Err(e) = fs::rename(&tmp, dir.join(name)) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    // the rename itself survives a crash only once the directory is synced
    #[cfg(unix)]
    fs::File::open(dir)?.sync_all()?;
    Ok(())
}

/// Lossless 8-bit RGB PNG with no ancillary chunks.
pub fn encode_png(img: &RgbImage) -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(
            img.as_raw(),
            img.width(),
            img.height(),
            image::ExtendedColorType::Rgb8,
        )
        .map_err(io::Error::other)?;
    Ok(out)
}

/// Session directories deleted while the daemon runs: a capture still in
/// flight when the member deleted the game never writes its kit again.
fn removed() -> &'static std::sync::Mutex<std::collections::HashSet<PathBuf>> {
    static R: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<PathBuf>>> =
        std::sync::OnceLock::new();
    R.get_or_init(Default::default)
}

fn was_removed(dir: &Path) -> bool {
    removed()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains(dir)
}

/// Sessions whose portrait memory is kept in memory. Tests run many
/// sessions at once through the one process-wide memory.
#[cfg(not(test))]
const MEMORY_SESSIONS: usize = 4;
#[cfg(test)]
const MEMORY_SESSIONS: usize = 256;

/// The portrait memory of the newest sessions, keyed by session directory
/// ([`blank::PortraitMemory`]: unblanked portrait cells, in memory only).
fn portrait_memory() -> &'static std::sync::Mutex<Vec<(PathBuf, blank::PortraitMemory)>> {
    static M: std::sync::OnceLock<std::sync::Mutex<Vec<(PathBuf, blank::PortraitMemory)>>> =
        std::sync::OnceLock::new();
    M.get_or_init(Default::default)
}

/// Drop a session's portrait memory: the game ended or was deleted.
pub fn forget_portraits(data_dir: &Path, session_id: &str) {
    if let Some(dir) = session_dir(data_dir, session_id) {
        let mut all = portrait_memory().lock().unwrap_or_else(|e| e.into_inner());
        all.retain(|(d, _)| *d != dir);
    }
}

/// Blank `frame` against the session's earlier captures and remember it.
fn blank_seen(
    dir: &Path,
    frame: &DynamicImage,
    team_size: Option<usize>,
    at: blank::CaptureTime,
) -> blank::BlankedBoard {
    let mut all = portrait_memory().lock().unwrap_or_else(|e| e.into_inner());
    // a session with no capture for longer than a reference may be old
    // holds nothing usable
    all.retain(|(_, m)| m.latest().is_none_or(|t| at.wall - t <= blank::REF_MAX_AGE));
    let i = match all.iter().position(|(d, _)| d == dir) {
        Some(i) => i,
        None => {
            if all.len() >= MEMORY_SESSIONS {
                all.remove(0);
            }
            all.push((dir.to_path_buf(), blank::PortraitMemory::new()));
            all.len() - 1
        }
    };
    blank::blanked_scoreboard_seen(frame, team_size, &mut all[i].1, at)
}

/// [`record_scoreboard_at`] for a capture grabbed now, stamped
/// `reads.captured_at`.
pub fn record_scoreboard(
    data_dir: &Path,
    session_id: &str,
    frame: &DynamicImage,
    team_size: Option<usize>,
    reads: &KitReads,
) -> io::Result<()> {
    let at = blank::CaptureTime {
        wall: reads.captured_at,
        mono: std::time::Instant::now(),
    };
    record_scoreboard_at(data_dir, session_id, frame, team_size, reads, at)
}

/// Record an accepted scoreboard capture: cut and blank the board (its
/// portraits checked against the session's earlier captures, aged from
/// `grabbed`, the frame grab or the Tab press that asked for it), and
/// replace the session's `scoreboard.png` and `reads.json`. A board the
/// blanker keeps nothing of writes no kit.
pub fn record_scoreboard_at(
    data_dir: &Path,
    session_id: &str,
    frame: &DynamicImage,
    team_size: Option<usize>,
    reads: &KitReads,
    grabbed: blank::CaptureTime,
) -> io::Result<()> {
    let Some(dir) = session_dir(data_dir, session_id) else {
        return Ok(());
    };
    let board = blank_seen(&dir, frame, team_size, grabbed);
    if !board.has_picture() {
        // No picture is safe to keep, but the reads still link a report to
        // this game. Written only when the session has no picture yet, so a
        // good picture is never dropped for a later board that blanked to
        // nothing.
        tracing::info!(
            session_id,
            reason = board
                .board
                .as_ref()
                .err()
                .map(String::as_str)
                .unwrap_or("all fill"),
            "report kit: board blanked to nothing, saving the reads without a picture"
        );
        return record_reads_only(data_dir, &dir, reads);
    }
    let png = encode_png(&board.image)?;
    let mut reads = reads.clone();
    reads.scoreboard_sha256 = super::bundle::sha256_hex(&png);
    let json = serde_json::to_vec_pretty(&reads).map_err(io::Error::other)?;
    let _guard = write_lock();
    if was_removed(&dir) {
        return Ok(());
    }
    // An older capture that finishes late never replaces a newer one.
    if read_reads(&dir).is_some_and(|cur| cur.captured_at > reads.captured_at) {
        return Ok(());
    }
    private_dir(&kit_root(data_dir))?;
    private_dir(&dir)?;
    // The scoreboard first, then the reads that carry its hash: a reader
    // between the two renames (or a crash there) sees a pair that does not
    // match, and [`load`] reports nothing.
    write_private(&dir, SCOREBOARD_PNG, &png)?;
    write_private(&dir, READS_JSON, &json)?;
    prune_locked(data_dir, retention());
    Ok(())
}

/// Save `reads.json` with no `scoreboard.png` (empty `scoreboard_sha256`).
/// [`load`] returns such a kit with reads and no scoreboard, so the game
/// can still be reported and linked by session id.
fn record_reads_only(data_dir: &Path, dir: &Path, reads: &KitReads) -> io::Result<()> {
    let mut reads = reads.clone();
    reads.scoreboard_sha256 = String::new();
    let json = serde_json::to_vec_pretty(&reads).map_err(io::Error::other)?;
    let _guard = write_lock();
    if was_removed(dir) {
        return Ok(());
    }
    if let Some(cur) = read_reads(dir)
        && (!cur.scoreboard_sha256.is_empty() || cur.captured_at > reads.captured_at)
    {
        return Ok(());
    }
    private_dir(&kit_root(data_dir))?;
    private_dir(dir)?;
    write_private(dir, READS_JSON, &json)?;
    prune_locked(data_dir, retention());
    Ok(())
}

/// Whether a polled frame may give `outcome.png` (spec section 3).
///
/// Only when the rank screen detector fired on this frame AND the scoreboard
/// detector did not accept it. The rank word match is loose (a title such as
/// "QUICKDRAW" contains DRAW), so a Tab-held scoreboard could fire it, and
/// on a scoreboard the word box holds the member's portrait, badge, and
/// title next to their name.
pub fn rank_outcome_allowed(
    source: Option<crate::detect::match_end::OutcomeSource>,
    preflight: crate::detect::hero_portrait::ScoreboardPreflight,
    rows_look_like_scoreboard: bool,
) -> bool {
    source == Some(crate::detect::match_end::OutcomeSource::RankScreen)
        && preflight == crate::detect::hero_portrait::ScoreboardPreflight::NotAScoreboard
        && !rows_look_like_scoreboard
}

/// The `outcome.png` bytes for one polled frame, or `None`
/// ([`rank_outcome_allowed`] decides).
pub fn rank_outcome_png(
    source: Option<crate::detect::match_end::OutcomeSource>,
    frame: &DynamicImage,
) -> Option<Vec<u8>> {
    if source != Some(crate::detect::match_end::OutcomeSource::RankScreen) {
        return None;
    }
    let board = crate::ocr::preprocess::crop_scoreboard(frame);
    let (preflight, scan) = crate::detect::hero_portrait::preflight_scoreboard(&board);
    if !rank_outcome_allowed(source, preflight, scan.looks_like_scoreboard()) {
        return None;
    }
    encode_png(&blank::outcome_band(frame)).ok()
}

/// Store the bytes from [`rank_outcome_png`] as the session's `outcome.png`.
pub fn record_outcome_png(data_dir: &Path, session_id: &str, png: &[u8]) -> io::Result<()> {
    let Some(dir) = session_dir(data_dir, session_id) else {
        return Ok(());
    };
    let _guard = write_lock();
    if was_removed(&dir) {
        return Ok(());
    }
    private_dir(&kit_root(data_dir))?;
    private_dir(&dir)?;
    write_private(&dir, OUTCOME_PNG, png)?;
    prune_locked(data_dir, retention());
    Ok(())
}

fn modified(path: &Path) -> std::time::SystemTime {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .unwrap_or(std::time::UNIX_EPOCH)
}

/// Add `names` (player names read on a stored board) to the session's
/// local [`NAMES_JSON`]. Only when the session already has a kit folder,
/// so names never outlive their kit. Blank and duplicate names are dropped.
pub fn record_names(data_dir: &Path, session_id: &str, names: &[String]) -> io::Result<()> {
    let Some(dir) = session_dir(data_dir, session_id) else {
        return Ok(());
    };
    let _guard = write_lock();
    if !dir.is_dir() || was_removed(&dir) {
        return Ok(());
    }
    let mut all = read_names(&dir);
    for name in names.iter().map(|n| n.trim()).filter(|n| !n.is_empty()) {
        if all.len() >= NAMES_MAX {
            break;
        }
        if !all.iter().any(|have| have == name) {
            all.push(name.to_string());
        }
    }
    let json = serde_json::to_vec(&all).map_err(io::Error::other)?;
    write_private(&dir, NAMES_JSON, &json)
}

fn read_names(dir: &Path) -> Vec<String> {
    fs::read(dir.join(NAMES_JSON))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// The player names stored for one session (the scrub list of its report).
pub fn names(data_dir: &Path, session_id: &str) -> Vec<String> {
    session_dir(data_dir, session_id)
        .map(|dir| read_names(&dir))
        .unwrap_or_default()
}

/// The stored names of every session with a kit folder, without
/// duplicates. A log report's window can cover any recent game.
pub fn all_names(data_dir: &Path) -> Vec<String> {
    let Ok(rd) = fs::read_dir(kit_root(data_dir)) else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    for e in rd.filter_map(Result::ok) {
        let Some(name) = e.file_name().into_string().ok() else {
            continue;
        };
        if safe_session(&name).is_none() {
            continue;
        }
        for n in read_names(&e.path()) {
            if !out.contains(&n) {
                out.push(n);
            }
        }
    }
    out
}

/// Sessions with a kit, newest first.
pub fn sessions(data_dir: &Path) -> Vec<String> {
    let Ok(rd) = fs::read_dir(kit_root(data_dir)) else {
        return Vec::new();
    };
    let mut found: Vec<(std::time::SystemTime, String)> = rd
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            safe_session(&name)?;
            let path = e.path();
            has_kit(data_dir, &name).then(|| (modified(&path.join(SCOREBOARD_PNG)), name))
        })
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    found.into_iter().map(|(_, s)| s).collect()
}

/// True when the session has a blanked scoreboard, under the current blank
/// table, to report.
pub fn has_kit(data_dir: &Path, session_id: &str) -> bool {
    load(data_dir, session_id).scoreboard_png.is_some()
}

/// Read a session's kit. Missing or unreadable parts are `None`.
pub fn load(data_dir: &Path, session_id: &str) -> Kit {
    let Some(dir) = session_dir(data_dir, session_id) else {
        return Kit::default();
    };
    // The daemon may be between the two renames of a pair: read again.
    for attempt in 0..3 {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        let Some(reads) = read_reads(&dir) else {
            return Kit::default();
        };
        // A scoreboard blanked under an older table (or with no stamp) is
        // never reported: the table may have missed names (section 4).
        if reads.blank_table != blank::BLANK_TABLE_VERSION {
            return Kit::default();
        }
        if reads.scoreboard_sha256.is_empty() {
            // Reads saved without a picture (the board blanked to nothing).
            return Kit {
                scoreboard_png: None,
                outcome_png: fs::read(dir.join(OUTCOME_PNG)).ok(),
                reads: Some(reads),
            };
        }
        let Ok(png) = fs::read(dir.join(SCOREBOARD_PNG)) else {
            return Kit::default();
        };
        if reads.scoreboard_sha256.is_empty()
            || super::bundle::sha256_hex(&png) != reads.scoreboard_sha256
        {
            continue;
        }
        return Kit {
            scoreboard_png: Some(png),
            outcome_png: fs::read(dir.join(OUTCOME_PNG)).ok(),
            reads: Some(reads),
        };
    }
    Kit::default()
}

/// True when `kit` passes the checks [`load`] makes: reads blanked with
/// this [`blank::BLANK_TABLE_VERSION`] and a scoreboard whose SHA-256 is
/// the one in those reads. [`super::bundle::build_game_report`] checks again
/// at the sink, since a [`Kit`] can also be built by hand.
pub fn is_trusted(kit: &Kit) -> bool {
    match (&kit.reads, &kit.scoreboard_png) {
        (Some(reads), Some(png)) => {
            reads.blank_table == blank::BLANK_TABLE_VERSION
                && !reads.scoreboard_sha256.is_empty()
                && super::bundle::sha256_hex(png) == reads.scoreboard_sha256
        }
        (Some(reads), None) => {
            reads.blank_table == blank::BLANK_TABLE_VERSION && reads.scoreboard_sha256.is_empty()
        }
        _ => false,
    }
}

/// True when the session can be reported as a game: it has saved reads,
/// with or without a picture.
pub fn has_reads(data_dir: &Path, session_id: &str) -> bool {
    load(data_dir, session_id).reads.is_some()
}

/// Delete a session's kit (the member deleted the game), its portrait
/// memory, and any kit write still in flight for it.
pub fn remove(data_dir: &Path, session_id: &str) {
    forget_portraits(data_dir, session_id);
    if let Some(dir) = session_dir(data_dir, session_id) {
        let _guard = write_lock();
        removed()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(dir.clone());
        let _ = fs::remove_dir_all(dir);
    }
}

/// Trim session kits to `keep`: oldest first, newest always kept.
pub fn prune(data_dir: &Path, keep: Retention) {
    let _guard = write_lock();
    prune_locked(data_dir, keep);
}

fn prune_locked(data_dir: &Path, keep: Retention) {
    let root = kit_root(data_dir);
    let Ok(rd) = fs::read_dir(&root) else {
        return;
    };
    let mut dirs: Vec<(std::time::SystemTime, PathBuf)> = rd
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .map(|p| (modified(&p), p))
        .collect();
    dirs.sort_by_key(|d| std::cmp::Reverse(d.0));
    let now = std::time::SystemTime::now();
    let mut total: u64 = 0;
    // Strictly oldest first: once a game is dropped, every older one goes
    // too, so an older game is never kept while a newer one is deleted.
    let mut dropping = false;
    for (i, (at, p)) in dirs.into_iter().enumerate() {
        // The newest kit is always kept, by every limit.
        if i > 0 && !dropping {
            let size = dir_bytes(&p);
            let too_old = now.duration_since(at).is_ok_and(|age| age > MAX_AGE);
            let too_big = total.saturating_add(size) > keep.max_bytes;
            dropping = i >= keep.max_games || too_old || too_big;
            if !dropping {
                total += size;
            }
        } else if i == 0 {
            total = dir_bytes(&p);
        }
        if dropping {
            let _ = fs::remove_dir_all(p);
        }
    }
}

fn dir_bytes(dir: &Path) -> u64 {
    fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter_map(|e| e.metadata().ok())
                .filter(|m| m.is_file())
                .map(|m| m.len())
                .sum()
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod names_tests {
    use super::*;

    #[test]
    fn board_names_are_kept_per_session_and_only_with_a_kit_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path();
        record_names(data, "s1", &["QXOther".into()]).unwrap();
        assert!(names(data, "s1").is_empty(), "no kit folder, no names");
        let dir = session_dir(data, "s1").unwrap();
        private_dir(&dir).unwrap();
        record_names(data, "s1", &["QXOther".into(), " ".into(), "Jo".into()]).unwrap();
        record_names(data, "s1", &["Jo".into(), "QX0ther".into()]).unwrap();
        assert_eq!(names(data, "s1"), vec!["QXOther", "Jo", "QX0ther"]);
        private_dir(&session_dir(data, "s2").unwrap()).unwrap();
        record_names(data, "s2", &["Zed".into(), "Jo".into()]).unwrap();
        let mut all = all_names(data);
        all.sort();
        assert_eq!(all, vec!["Jo", "QX0ther", "QXOther", "Zed"]);
        assert!(names(data, "../s1").is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reads() -> KitReads {
        let mut r = BTreeMap::new();
        r.insert(
            "e".into(),
            KitRead {
                value: Some("12".into()),
                confidence: None,
                ocr_v1: Some("12".into()),
                suspect: false,
            },
        );
        KitReads {
            captured_at: "2026-10-12T08:00:00Z".parse().unwrap(),
            width: 2560,
            height: 1440,
            team_size: Some(6),
            map: Some("Ilios".into()),
            mode: Some("Control".into()),
            result: None,
            recognizer: "cv-v5".into(),
            blank_table: crate::report::blank::BLANK_TABLE_VERSION,
            scoreboard_sha256: String::new(),
            reads: r,
        }
    }

    use crate::report::blank::synth::{self, Layout};

    /// A 2560x1440 frame with a synthetic 6v6 scoreboard.
    fn frame() -> DynamicImage {
        synth::frame(Layout::L1, &synth::VALUES)
    }

    /// [`frame`] with the stat values of row 0 changed, so its blanked board
    /// differs.
    fn frame_variant(i: u32) -> DynamicImage {
        let mut v = synth::VALUES;
        v[0][0] = i;
        synth::frame(Layout::L1, &v)
    }

    /// A dark, flat frame: what the scoreboard preflight calls "not a
    /// scoreboard", like the rank screen's dark background.
    fn rank_like_frame() -> DynamicImage {
        DynamicImage::ImageRgb8(RgbImage::from_pixel(1920, 1080, image::Rgb([24, 20, 18])))
    }

    #[test]
    fn blanking_changes_only_name_boxes_of_the_report_copy() {
        // The blackout is on the copy saved for the report only: the frame
        // the reader uses is unchanged. The copy is an allowlist: every name
        // box is filled, and pixels outside them (hero icons, stat cells)
        // are still there from the original.
        let tmp = tempfile::tempdir().unwrap();
        let source = frame();
        let before = source.to_rgb8();
        record_scoreboard(tmp.path(), "s-1", &source, Some(6), &reads()).unwrap();
        assert_eq!(source.to_rgb8(), before, "reader frame was changed");
        let png = load(tmp.path(), "s-1").scoreboard_png.expect("scoreboard");
        let copy = image::load_from_memory(&png).unwrap().to_rgb8();
        let (cx, cy, cw, ch) = crate::ocr::preprocess::crop_scoreboard_rect(2560, 1440);
        let original = image::imageops::crop_imm(&before, cx, cy, cw, ch).to_image();
        let boxes = synth::name_boxes(Layout::L1);
        let in_box = |x: u32, y: u32| {
            boxes
                .iter()
                .any(|&(x0, y0, x1, y1)| x >= x0 && x < x1 && y >= y0 && y < y1)
        };
        let mut kept = 0;
        for (x, y, px) in copy.enumerate_pixels() {
            if in_box(x, y) {
                assert_eq!(*px, blank::FILL, "name pixel ({x},{y}) not blanked");
            } else if *px != blank::FILL && *px == *original.get_pixel(x, y) {
                kept += 1;
            }
        }
        assert!(kept > 0, "no hero icon or stat pixel was kept");
    }

    #[test]
    fn scoreboard_is_blanked_private_and_loads_back() {
        let tmp = tempfile::tempdir().unwrap();
        record_scoreboard(tmp.path(), "s-1", &frame(), Some(6), &reads()).unwrap();
        let kit = load(tmp.path(), "s-1");
        let png = kit.scoreboard_png.expect("scoreboard");
        let img = image::load_from_memory(&png).unwrap().to_rgb8();
        let (_, _, cw, ch) = crate::ocr::preprocess::crop_scoreboard_rect(2560, 1440);
        assert_eq!(img.dimensions(), (cw, ch));
        for (x0, y0, x1, y1) in synth::name_boxes(Layout::L1) {
            for y in y0..y1 {
                for x in x0..x1 {
                    assert_eq!(*img.get_pixel(x, y), blank::FILL);
                }
            }
        }
        let mut want = reads();
        want.scoreboard_sha256 = crate::report::bundle::sha256_hex(&png);
        assert_eq!(kit.reads, Some(want));
        assert!(has_kit(tmp.path(), "s-1"));
        assert_eq!(sessions(tmp.path()), vec!["s-1".to_string()]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let dir = session_dir(tmp.path(), "s-1").unwrap();
            assert_eq!(
                fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
                0o700
            );
            for f in [SCOREBOARD_PNG, READS_JSON] {
                assert_eq!(
                    fs::metadata(dir.join(f)).unwrap().permissions().mode() & 0o777,
                    0o600
                );
            }
        }
    }

    #[test]
    fn a_kit_from_an_older_blank_table_is_never_reported() {
        let tmp = tempfile::tempdir().unwrap();
        let mut old = reads();
        old.blank_table = blank::BLANK_TABLE_VERSION - 1;
        record_scoreboard(tmp.path(), "s-old", &frame(), Some(6), &old).unwrap();
        assert!(!has_kit(tmp.path(), "s-old"));
        assert!(load(tmp.path(), "s-old").scoreboard_png.is_none());
        assert!(sessions(tmp.path()).is_empty());
        // A reads.json from before the stamp existed has no field at all.
        let dir = session_dir(tmp.path(), "s-old").unwrap();
        let mut v: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.join(READS_JSON)).unwrap()).unwrap();
        v.as_object_mut().unwrap().remove("blank_table");
        fs::write(dir.join(READS_JSON), serde_json::to_vec(&v).unwrap()).unwrap();
        assert!(!has_kit(tmp.path(), "s-old"));
    }

    #[test]
    fn removing_a_session_deletes_its_kit_only() {
        let tmp = tempfile::tempdir().unwrap();
        for sid in ["keep", "gone"] {
            record_scoreboard(tmp.path(), sid, &frame(), Some(6), &reads()).unwrap();
        }
        remove(tmp.path(), "gone");
        remove(tmp.path(), "../keep");
        assert!(!session_dir(tmp.path(), "gone").unwrap().exists());
        assert_eq!(sessions(tmp.path()), vec!["keep".to_string()]);
    }

    #[test]
    fn a_mismatched_pair_is_not_reported() {
        let tmp = tempfile::tempdir().unwrap();
        record_scoreboard(tmp.path(), "s-1", &frame(), Some(6), &reads()).unwrap();
        assert!(has_kit(tmp.path(), "s-1"));
        // The daemon is between the two renames: a new board, the old reads.
        let dir = session_dir(tmp.path(), "s-1").unwrap();
        let other = encode_png(&RgbImage::from_pixel(8, 8, image::Rgb([1, 2, 3]))).unwrap();
        fs::write(dir.join(SCOREBOARD_PNG), other).unwrap();
        assert!(!has_kit(tmp.path(), "s-1"));
        assert!(load(tmp.path(), "s-1").reads.is_none());
        // A reads.json without a hash is never reported either.
        let mut v: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.join(READS_JSON)).unwrap()).unwrap();
        v.as_object_mut().unwrap().remove("scoreboard_sha256");
        fs::write(dir.join(READS_JSON), serde_json::to_vec(&v).unwrap()).unwrap();
        assert!(!has_kit(tmp.path(), "s-1"));
    }

    #[test]
    fn an_older_capture_never_replaces_a_newer_one() {
        let tmp = tempfile::tempdir().unwrap();
        let older = reads();
        let mut newer = reads();
        newer.captured_at = older.captured_at + chrono::Duration::seconds(5);
        let newer_frame = frame_variant(9);
        record_scoreboard(tmp.path(), "s-1", &newer_frame, Some(6), &newer).unwrap();
        record_scoreboard(tmp.path(), "s-1", &frame(), Some(6), &older).unwrap();
        let kit = load(tmp.path(), "s-1");
        assert_eq!(kit.reads.unwrap().captured_at, newer.captured_at);
        let want = encode_png(&blank::blanked_scoreboard(&newer_frame, Some(6)).image).unwrap();
        assert_eq!(kit.scoreboard_png.unwrap(), want);
    }

    #[test]
    fn concurrent_captures_leave_the_newest_matching_pair() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        let base = reads().captured_at;
        let writers: Vec<_> = (0..8u8)
            .map(|i| {
                let dir = dir.clone();
                std::thread::spawn(move || {
                    let mut r = reads();
                    r.captured_at = base + chrono::Duration::seconds(i64::from(i));
                    let f = frame_variant(u32::from(i) + 1);
                    record_scoreboard(&dir, "s-1", &f, Some(6), &r).unwrap();
                })
            })
            .collect();
        let reader = {
            let dir = dir.clone();
            std::thread::spawn(move || {
                let mut last = None;
                for _ in 0..300 {
                    let kit = load(&dir, "s-1");
                    if let (Some(png), Some(r)) = (kit.scoreboard_png, kit.reads) {
                        assert_eq!(super::super::bundle::sha256_hex(&png), r.scoreboard_sha256);
                        assert!(
                            last.is_none_or(|l| r.captured_at >= l),
                            "kit went back in time"
                        );
                        last = Some(r.captured_at);
                    }
                }
            })
        };
        for w in writers {
            w.join().unwrap();
        }
        reader.join().unwrap();
        let r = load(&dir, "s-1").reads.unwrap();
        assert_eq!(r.captured_at, base + chrono::Duration::seconds(7));
    }

    #[test]
    fn png_has_no_metadata_chunks() {
        let png = encode_png(&RgbImage::new(4, 4)).unwrap();
        let mut i = 8;
        let mut kinds = Vec::new();
        while i + 8 <= png.len() {
            let len = u32::from_be_bytes(png[i..i + 4].try_into().unwrap()) as usize;
            kinds.push(String::from_utf8_lossy(&png[i + 4..i + 8]).to_string());
            i += 12 + len;
        }
        assert!(
            kinds
                .iter()
                .all(|k| ["IHDR", "IDAT", "IEND"].contains(&k.as_str())),
            "{kinds:?}"
        );
    }

    #[test]
    fn reader_confidences_reach_the_kit_except_for_a_gated_stat() {
        let mut r = reads();
        let read = |value: &str, ocr: &str| KitRead {
            value: Some(value.into()),
            confidence: None,
            ocr_v1: Some(ocr.into()),
            suspect: false,
        };
        r.reads.insert("e".into(), read("10", "10"));
        r.reads.insert("d".into(), read("3", "3"));
        r.reads.insert("mode".into(), read("Control", "Control"));
        // the reader read d as 8; the gate stored 3
        let c = conf(&[("e", 0.97, "10"), ("d", 0.9, "8"), ("mode", 1.5, "Control")]);
        let r = r.with_confidences(&c);
        assert_eq!(r.reads["e"].confidence, Some(0.97));
        assert_eq!(r.reads["d"].confidence, None, "the gate replaced this read");
        assert_eq!(r.reads["mode"].confidence, Some(1.0), "clamped");
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["reads"]["e"]["confidence"], 0.97);
    }

    fn conf(entries: &[(&str, f64, &str)]) -> BTreeMap<String, ReaderRead> {
        entries
            .iter()
            .map(|&(k, c, v)| {
                (
                    k.to_string(),
                    ReaderRead {
                        confidence: c,
                        value: Some(v.to_string()),
                    },
                )
            })
            .collect()
    }

    /// QA on #316: a cv read that Tesseract read differently was kept by
    /// the gate, so it keeps its confidence.
    #[test]
    fn a_cv_read_that_disagrees_with_tesseract_keeps_its_confidence() {
        let mut r = reads();
        r.reads.insert(
            "dmg".into(),
            KitRead {
                value: Some("8432".into()),
                confidence: None,
                ocr_v1: Some("8482".into()),
                suspect: true,
            },
        );
        let r = r.with_confidences(&conf(&[("dmg", 0.91, "8432")]));
        assert_eq!(r.reads["dmg"].confidence, Some(0.91));
    }

    #[test]
    fn nan_and_infinite_confidences_are_null_and_others_are_clamped() {
        let mut r = reads();
        for k in ["e", "a", "d", "h"] {
            r.reads.insert(
                k.into(),
                KitRead {
                    value: Some("5".into()),
                    confidence: None,
                    ocr_v1: Some("5".into()),
                    suspect: false,
                },
            );
        }
        let c = conf(&[
            ("e", f64::NAN, "5"),
            ("a", f64::INFINITY, "5"),
            ("d", -0.4, "5"),
            ("h", f64::NEG_INFINITY, "5"),
        ]);
        let r = r.with_confidences(&c);
        assert_eq!(r.reads["e"].confidence, None);
        assert_eq!(r.reads["a"].confidence, None);
        assert_eq!(r.reads["d"].confidence, Some(0.0));
        assert_eq!(r.reads["h"].confidence, None);
        let json = serde_json::to_string(&r).unwrap();
        assert!(!json.contains("NaN") && !json.contains("inf"));
    }

    #[test]
    fn only_victory_defeat_and_draw_are_known_results() {
        for r in ["victory", " Defeat ", "DRAW"] {
            assert!(!result_is_unknown(r), "{r}");
        }
        for r in ["", "unknown", "Unknown", "  ", "win"] {
            assert!(result_is_unknown(r), "{r:?}");
        }
        let mut k = reads();
        k.result = Some("".into());
        k.fill_result("unknown");
        assert_eq!(k.result.as_deref(), Some(""));
        k.fill_result("victory");
        assert_eq!(k.result.as_deref(), Some("victory"));
    }

    #[test]
    fn a_kit_from_before_the_result_takes_the_stored_result() {
        let mut r = reads();
        r.result = Some("unknown".into());
        r.fill_result("defeat");
        assert_eq!(r.result.as_deref(), Some("defeat"));
        assert_eq!(r.reads["result"].value.as_deref(), Some("defeat"));
        assert!(!r.reads["result"].suspect);
        // a known kit result is never replaced, and unknown never fills
        r.fill_result("victory");
        assert_eq!(r.result.as_deref(), Some("defeat"));
        let mut none = reads();
        none.result = None;
        none.fill_result("unknown");
        assert_eq!(none.result, None);
    }

    #[test]
    fn unsafe_session_ids_get_no_kit() {
        let tmp = tempfile::tempdir().unwrap();
        for sid in ["", "../x", "a/b", "a b", &"x".repeat(65)] {
            record_scoreboard(tmp.path(), sid, &frame(), Some(6), &reads()).unwrap();
            assert!(!has_kit(tmp.path(), sid));
        }
        assert!(sessions(tmp.path()).is_empty());
        assert!(!tmp.path().join("x").exists());
    }

    #[test]
    fn only_the_newest_sessions_are_kept() {
        let tmp = tempfile::tempdir().unwrap();
        for i in 0..5 {
            record_scoreboard(tmp.path(), &format!("s{i}"), &frame(), Some(6), &reads()).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
        prune(tmp.path(), Retention::new(3, 500));
        let mut left = sessions(tmp.path());
        left.sort();
        assert_eq!(left, vec!["s2", "s3", "s4"]);
    }

    /// A kit folder of `bytes`, written `i` steps after the first.
    fn sized_kit(data: &Path, sid: &str, bytes: usize) {
        let dir = session_dir(data, sid).unwrap();
        private_dir(&kit_root(data)).unwrap();
        private_dir(&dir).unwrap();
        write_private(&dir, OUTCOME_PNG, &vec![0u8; bytes]).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(15));
    }

    /// Kit folders left on disk, sorted.
    fn kit_dirs(data: &Path) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(kit_root(data))
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn the_defaults_are_fifty_games_and_500_mb() {
        let r = Retention::default();
        assert_eq!(r.max_games, 50);
        assert_eq!(r.max_bytes, 500 * 1024 * 1024);
        assert_eq!(Retention::new(0, 1).max_games, 1);
    }

    #[test]
    fn the_size_cap_trims_the_oldest_games_first() {
        let tmp = tempfile::tempdir().unwrap();
        let mb = 1024 * 1024;
        for i in 0..5 {
            sized_kit(tmp.path(), &format!("s{i}"), mb);
        }
        // 3 MB cap: the three newest fit, s0 and s1 go.
        prune(tmp.path(), Retention::new(50, 3));
        assert_eq!(kit_dirs(tmp.path()), vec!["s2", "s3", "s4"]);
    }

    #[test]
    fn the_game_cap_trims_in_age_order_not_name_order() {
        let tmp = tempfile::tempdir().unwrap();
        for sid in ["c", "a", "d", "b"] {
            sized_kit(tmp.path(), sid, 10);
        }
        prune(tmp.path(), Retention::new(2, 500));
        assert_eq!(kit_dirs(tmp.path()), vec!["b", "d"]);
    }

    #[test]
    fn a_smaller_older_game_is_not_kept_after_a_newer_one_is_dropped() {
        let tmp = tempfile::tempdir().unwrap();
        let mb = 1024 * 1024;
        sized_kit(tmp.path(), "oldest_small", 10);
        sized_kit(tmp.path(), "middle_big", 2 * mb);
        sized_kit(tmp.path(), "newest", mb);
        // 2 MB cap: newest fits, middle_big does not, so oldest_small goes too.
        prune(tmp.path(), Retention::new(50, 2));
        assert_eq!(kit_dirs(tmp.path()), vec!["newest"]);
    }

    #[test]
    fn the_newest_game_stays_past_the_age_limit() {
        let tmp = tempfile::tempdir().unwrap();
        sized_kit(tmp.path(), "old", 10);
        sized_kit(tmp.path(), "new", 10);
        let past = std::time::SystemTime::now() - MAX_AGE - std::time::Duration::from_secs(3600);
        let older = past - std::time::Duration::from_secs(60);
        for (sid, at) in [("new", past), ("old", older)] {
            fs::File::open(session_dir(tmp.path(), sid).unwrap())
                .unwrap()
                .set_modified(at)
                .unwrap();
        }
        prune(tmp.path(), Retention::default());
        assert_eq!(kit_dirs(tmp.path()), vec!["new"]);
    }

    #[test]
    fn the_newest_game_stays_even_when_it_alone_is_over_the_cap() {
        let tmp = tempfile::tempdir().unwrap();
        sized_kit(tmp.path(), "old", 10);
        sized_kit(tmp.path(), "new", 2 * 1024 * 1024);
        prune(tmp.path(), Retention::new(50, 1));
        assert_eq!(kit_dirs(tmp.path()), vec!["new"]);
    }

    #[test]
    fn only_a_rank_screen_hit_gives_an_outcome_crop() {
        use crate::detect::match_end::OutcomeSource;
        for src in [
            None,
            Some(OutcomeSource::ResultWord),
            Some(OutcomeSource::EndTitle),
        ] {
            assert!(
                rank_outcome_png(src, &rank_like_frame()).is_none(),
                "{src:?}"
            );
        }
        assert!(rank_outcome_png(Some(OutcomeSource::RankScreen), &rank_like_frame()).is_some());
    }

    #[test]
    fn a_scoreboard_never_gives_an_outcome_crop_even_when_rank_screen_fired() {
        use crate::detect::hero_portrait::ScoreboardPreflight as P;
        use crate::detect::match_end::OutcomeSource as S;
        // R235-2: a Tab-held board whose title contains DRAW fires RankScreen.
        for pre in [P::Ready(5), P::Ready(6), P::TeamSizeUncertain] {
            for rows in [false, true] {
                assert!(
                    !rank_outcome_allowed(Some(S::RankScreen), pre, rows),
                    "{pre:?} {rows}"
                );
            }
        }
        assert!(!rank_outcome_allowed(
            Some(S::RankScreen),
            P::NotAScoreboard,
            true
        ));
        for src in [None, Some(S::ResultWord), Some(S::EndTitle)] {
            assert!(
                !rank_outcome_allowed(src, P::NotAScoreboard, false),
                "{src:?}"
            );
        }
        assert!(rank_outcome_allowed(
            Some(S::RankScreen),
            P::NotAScoreboard,
            false
        ));
    }

    /// QA harness on real frames, not in CI (members' frames never go in the
    /// repo): `SCUFFED_QA_FRAMES=<dir> cargo test -p scuffed-stat-tracker
    /// --lib real_frames_ -- --ignored --nocapture`. Every frame the
    /// scoreboard detector accepts must give no outcome crop, whatever the
    /// outcome detectors say.
    #[test]
    #[ignore]
    fn real_frames_scoreboards_never_give_an_outcome_crop() {
        use crate::detect::match_end::OutcomeSource;
        let dir = std::env::var("SCUFFED_QA_FRAMES").expect("set SCUFFED_QA_FRAMES");
        let (mut boards, mut rank_hits) = (0, 0);
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "png") {
                continue;
            }
            let Ok(img) = image::open(&path) else {
                continue;
            };
            let board = crate::ocr::preprocess::crop_scoreboard(&img);
            let (pre, scan) = crate::detect::hero_portrait::preflight_scoreboard(&board);
            let is_board = pre != crate::detect::hero_portrait::ScoreboardPreflight::NotAScoreboard
                || scan.looks_like_scoreboard();
            let real = crate::detect::match_end::detect_outcome_signal(&img).map(|(_, s)| s);
            let forced = rank_outcome_png(Some(OutcomeSource::RankScreen), &img);
            if is_board {
                boards += 1;
                assert!(
                    forced.is_none(),
                    "{}: scoreboard gave an outcome crop",
                    path.display()
                );
            }
            if real == Some(OutcomeSource::RankScreen) && rank_outcome_png(real, &img).is_some() {
                rank_hits += 1;
                println!("rank crop: {}", path.display());
            }
        }
        println!("scoreboards checked: {boards}, rank crops: {rank_hits}");
    }

    #[test]
    fn a_real_scoreboard_frame_writes_no_outcome_crop() {
        // Q235-1: on a scoreboard the rank box holds the member's portrait,
        // badge, and title. The detectors must not call it a rank screen.
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("test-data/vision-baseline");
        let mut checked = 0;
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            if !name.ends_with(".png") || !(name.contains("tab_board") || name.contains("postgame"))
            {
                continue;
            }
            let img = image::open(&path).unwrap();
            let src = crate::detect::match_end::detect_outcome_signal(&img).map(|(_, s)| s);
            assert!(rank_outcome_png(src, &img).is_none(), "{name}: {src:?}");
            checked += 1;
        }
        assert!(checked > 0, "no scoreboard fixtures in {}", dir.display());
    }

    #[test]
    fn outcome_is_the_rank_word_box() {
        use crate::detect::match_end::OutcomeSource;
        let tmp = tempfile::tempdir().unwrap();
        let png = rank_outcome_png(Some(OutcomeSource::RankScreen), &rank_like_frame()).unwrap();
        record_outcome_png(tmp.path(), "s-1", &png).unwrap();
        // An outcome alone is not a reportable kit.
        assert!(!has_kit(tmp.path(), "s-1"));
        assert!(load(tmp.path(), "s-1").outcome_png.is_none());
        record_scoreboard(tmp.path(), "s-1", &frame(), Some(6), &reads()).unwrap();
        let png = load(tmp.path(), "s-1").outcome_png.expect("outcome");
        let img = image::load_from_memory(&png).unwrap();
        assert_eq!(
            (img.width(), img.height()),
            (
                1920 * 250 / 1000 - 1920 * 10 / 1000,
                1080 * 225 / 1000 - 1080 * 145 / 1000
            )
        );
    }

    #[test]
    fn every_write_path_keeps_only_the_newest_sessions() {
        // K1: games that only ever wrote an outcome crop are pruned too
        let tmp = tempfile::tempdir().unwrap();
        for i in 0..DEFAULT_MAX_GAMES + 3 {
            record_outcome_png(tmp.path(), &format!("o{i:02}"), b"png").unwrap();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let dirs = fs::read_dir(kit_root(tmp.path())).unwrap().count();
        assert_eq!(dirs, DEFAULT_MAX_GAMES);
        assert!(session_dir(tmp.path(), "o00").is_some_and(|d| !d.exists()));
        assert!(
            session_dir(tmp.path(), &format!("o{:02}", DEFAULT_MAX_GAMES + 2))
                .is_some_and(|d| d.exists())
        );
    }

    #[test]
    fn a_write_in_flight_never_recreates_a_deleted_kit() {
        // K7: the member deleted the game while its capture was blanking
        let tmp = tempfile::tempdir().unwrap();
        record_scoreboard(tmp.path(), "gone-1", &frame(), Some(6), &reads()).unwrap();
        assert!(has_kit(tmp.path(), "gone-1"));
        remove(tmp.path(), "gone-1");
        record_scoreboard(tmp.path(), "gone-1", &frame(), Some(6), &reads()).unwrap();
        record_outcome_png(tmp.path(), "gone-1", b"png").unwrap();
        assert!(!session_dir(tmp.path(), "gone-1").unwrap().exists());
    }

    fn pm(stats: [u32; 6], hero: &str) -> crate::storage::PersonalMatch {
        crate::storage::PersonalMatch {
            id: None,
            hero: hero.into(),
            map_name: "Ilios".into(),
            game_mode: "Control".into(),
            role: "Support".into(),
            outcome: String::new(),
            elims: stats[0],
            deaths: stats[2],
            assists: stats[1],
            damage: stats[3],
            healing: stats[4],
            mitigation: stats[5],
            played_at: surrealdb_types::Datetime::from(Utc::now()),
            synced: false,
            sync_rev: 0,
            upload_reject: None,
            session_id: "s".into(),
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
            recognizer: "cv-v5".into(),
            suspect_fields: Vec::new(),
            name_recognizer: None,
        }
    }

    #[test]
    fn from_capture_keeps_the_tesseract_read_and_flags_what_the_spec_says() {
        // K2: ocr_v1 is the read before the gate held a stat; a read is
        // suspect when flagged, null, or (stats) when stored and ocr_v1
        // disagree
        let before = pm([12, 3, 4, 5000, 0, 900], "Ana");
        let mut after = pm([12, 3, 4, 5100, 0, 900], "Ana");
        after.suspect_fields = vec!["mit".into()];
        let at: DateTime<Utc> = "2026-10-12T20:00:00Z".parse().unwrap();
        let r = KitReads::from_capture(&before, &after, 2560, 1440, TeamLayout::Six, at);
        assert_eq!(r.captured_at, at);
        let get = |k: &str| r.reads.get(k).unwrap().clone();
        assert_eq!(get("dmg").value.as_deref(), Some("5100"));
        assert_eq!(get("dmg").ocr_v1.as_deref(), Some("5000"));
        assert!(get("dmg").suspect, "stored and ocr_v1 disagree");
        assert!(!get("e").suspect);
        assert!(get("mit").suspect, "flagged by the tracker");
        assert!(get("result").value.is_none());
        assert!(get("result").suspect, "a null read is suspect");
        assert!(!get("hero").suspect);
        assert_eq!(get("hero").ocr_v1.as_deref(), Some("Ana"));
        assert_eq!(r.reads.len(), 9);
    }

    fn memory_holds(data_dir: &Path, session_id: &str) -> bool {
        let dir = session_dir(data_dir, session_id).unwrap();
        portrait_memory()
            .lock()
            .unwrap()
            .iter()
            .any(|(d, m)| *d == dir && m.holds_cells())
    }

    /// Fill the session's portrait memory the way the blanker does with
    /// portrait copying on, whatever [`blank::COPY_PORTRAITS`] says.
    fn remember_gated(data_dir: &Path, session_id: &str) {
        let dir = session_dir(data_dir, session_id).unwrap();
        let mut memory = blank::PortraitMemory::new();
        let at = blank::CaptureTime {
            wall: reads().captured_at,
            mono: std::time::Instant::now(),
        };
        let _ = blank::blanked_scoreboard_gated(&frame(), Some(6), &mut memory, at);
        let mut all = portrait_memory().lock().unwrap();
        all.retain(|(d, _)| *d != dir);
        all.push((dir, memory));
    }

    #[test]
    fn deleting_or_ending_a_game_drops_its_portrait_memory() {
        // R3-4 / K4: no unblanked portrait cell of a deleted or ended game
        // stays in memory. R2-5: the memory is filled with portrait copying
        // on, so removal is exercised even while the switch is off.
        let tmp = tempfile::tempdir().unwrap();
        record_scoreboard(tmp.path(), "mem-del", &frame(), Some(6), &reads()).unwrap();
        record_scoreboard(tmp.path(), "mem-end", &frame(), Some(6), &reads()).unwrap();
        if !blank::COPY_PORTRAITS {
            // portraits off: the daemon path keeps no cell at all
            assert!(!memory_holds(tmp.path(), "mem-del"));
            assert!(!memory_holds(tmp.path(), "mem-end"));
        }
        remember_gated(tmp.path(), "mem-del");
        remember_gated(tmp.path(), "mem-end");
        remember_gated(tmp.path(), "mem-keep");
        assert!(memory_holds(tmp.path(), "mem-del"));
        assert!(memory_holds(tmp.path(), "mem-end"));
        remove(tmp.path(), "mem-del");
        assert!(!memory_holds(tmp.path(), "mem-del"));
        assert!(memory_holds(tmp.path(), "mem-end"));
        forget_portraits(tmp.path(), "mem-end");
        assert!(!memory_holds(tmp.path(), "mem-end"));
        assert!(
            memory_holds(tmp.path(), "mem-keep"),
            "only the named session is dropped"
        );
        forget_portraits(tmp.path(), "mem-keep");
    }
}
