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
//! The newest [`KEEP`] sessions are kept. The directory is private (0700,
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

pub const KIT_DIR: &str = "report-kit";
/// Sessions kept. Older kits are deleted when a new session writes one.
pub const KEEP: usize = 20;

pub const SCOREBOARD_PNG: &str = "scoreboard.png";
pub const OUTCOME_PNG: &str = "outcome.png";
pub const READS_JSON: &str = "reads.json";

/// One read in `reads.json`, the same shape as a manifest read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KitRead {
    pub value: Option<String>,
    /// The tracker does not keep a per-field confidence today, so this is
    /// `null` unless a reader supplies one.
    pub confidence: Option<f64>,
    /// The Tesseract (`ocr-v1`) read before the digit reader merged.
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
    /// Reads of one accepted capture. `before` is the match as the Tesseract
    /// pass parsed it (the `ocr_v1` read), `after` is what was stored.
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
        let mut put = |key: &str, value: Option<String>, ocr: Option<String>| {
            reads.insert(
                key.to_string(),
                KitRead {
                    value,
                    confidence: None,
                    ocr_v1: ocr,
                    suspect: suspects.iter().any(|f| f == key),
                },
            );
        };
        put("mode", text(&after.game_mode), text(&before.game_mode));
        put("result", text(&after.outcome), text(&before.outcome));
        put("hero", text(&after.hero), text(&before.hero));
        for (key, a, b) in [
            ("e", after.elims, before.elims),
            ("a", after.assists, before.assists),
            ("d", after.deaths, before.deaths),
            ("dmg", after.damage, before.damage),
            ("h", after.healing, before.healing),
            ("mit", after.mitigation, before.mitigation),
        ] {
            put(key, Some(a.to_string()), Some(b.to_string()));
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
        let mut f = opts.open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, dir.join(name))
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

/// Sessions whose portrait memory is kept in memory.
const MEMORY_SESSIONS: usize = 4;

/// The portrait memory of the newest sessions, keyed by session directory
/// ([`blank::PortraitMemory`]: unblanked portrait cells, in memory only).
fn portrait_memory() -> &'static std::sync::Mutex<Vec<(PathBuf, blank::PortraitMemory)>> {
    static M: std::sync::OnceLock<std::sync::Mutex<Vec<(PathBuf, blank::PortraitMemory)>>> =
        std::sync::OnceLock::new();
    M.get_or_init(Default::default)
}

/// Blank `frame` against the session's earlier captures and remember it.
fn blank_seen(
    dir: &Path,
    frame: &DynamicImage,
    team_size: Option<usize>,
    at: DateTime<Utc>,
) -> blank::BlankedBoard {
    let mut all = portrait_memory().lock().unwrap_or_else(|e| e.into_inner());
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

/// Record an accepted scoreboard capture: cut and blank the board (its
/// portraits checked against the session's earlier captures), and replace
/// the session's `scoreboard.png` and `reads.json`. A board the blanker
/// keeps nothing of writes no kit.
pub fn record_scoreboard(
    data_dir: &Path,
    session_id: &str,
    frame: &DynamicImage,
    team_size: Option<usize>,
    reads: &KitReads,
) -> io::Result<()> {
    let Some(dir) = session_dir(data_dir, session_id) else {
        return Ok(());
    };
    let board = blank_seen(&dir, frame, team_size, reads.captured_at);
    if !board.kept_anything() {
        return Ok(());
    }
    let png = encode_png(&board.image)?;
    let mut reads = reads.clone();
    reads.scoreboard_sha256 = super::bundle::sha256_hex(&png);
    let json = serde_json::to_vec_pretty(&reads).map_err(io::Error::other)?;
    let _guard = write_lock();
    // An older capture that finishes late never replaces a newer one.
    if read_reads(&dir).is_some_and(|cur| cur.captured_at > reads.captured_at) {
        return Ok(());
    }
    let new_session = !dir.exists();
    private_dir(&kit_root(data_dir))?;
    private_dir(&dir)?;
    write_private(&dir, SCOREBOARD_PNG, &png)?;
    write_private(&dir, READS_JSON, &json)?;
    if new_session {
        prune_locked(data_dir, KEEP);
    }
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
    private_dir(&kit_root(data_dir))?;
    private_dir(&dir)?;
    write_private(&dir, OUTCOME_PNG, png)
}

fn modified(path: &Path) -> std::time::SystemTime {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .unwrap_or(std::time::UNIX_EPOCH)
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
    for _ in 0..3 {
        let Some(reads) = read_reads(&dir) else {
            return Kit::default();
        };
        // A scoreboard blanked under an older table (or with no stamp) is
        // never reported: the table may have missed names (section 4).
        if reads.blank_table != blank::BLANK_TABLE_VERSION {
            return Kit::default();
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

/// Delete a session's kit (the member deleted the game).
pub fn remove(data_dir: &Path, session_id: &str) {
    if let Some(dir) = session_dir(data_dir, session_id) {
        let _guard = write_lock();
        let _ = fs::remove_dir_all(dir);
    }
}

/// Keep the newest `keep` session kits and delete the rest.
pub fn prune(data_dir: &Path, keep: usize) {
    let _guard = write_lock();
    prune_locked(data_dir, keep);
}

fn prune_locked(data_dir: &Path, keep: usize) {
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
    for (_, p) in dirs.into_iter().skip(keep) {
        let _ = fs::remove_dir_all(p);
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
        prune(tmp.path(), 3);
        let mut left = sessions(tmp.path());
        left.sort();
        assert_eq!(left, vec!["s2", "s3", "s4"]);
    }

    #[test]
    fn only_a_rank_screen_hit_gives_an_outcome_crop() {
        use crate::detect::match_end::OutcomeSource;
        for src in [
            None,
            Some(OutcomeSource::Banner),
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
        for src in [
            None,
            Some(S::Banner),
            Some(S::ResultWord),
            Some(S::EndTitle),
        ] {
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
}
