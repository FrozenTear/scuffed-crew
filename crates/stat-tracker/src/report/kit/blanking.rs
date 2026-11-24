//! Blanking at capture time: the process-wide portrait memory of the
//! newest sessions and the scoreboard writes that go through it.

use std::io;
use std::path::{Path, PathBuf};

use image::DynamicImage;

use super::super::blank;
use super::KitReads;
use super::store::{
    READS_JSON, SCOREBOARD_PNG, encode_png, kit_root, private_dir, prune_locked, read_reads,
    retention, session_dir, was_removed, write_lock, write_private,
};

/// Sessions whose portrait memory is kept in memory. Tests run many
/// sessions at once through the one process-wide memory.
#[cfg(not(test))]
const MEMORY_SESSIONS: usize = 4;
#[cfg(test)]
const MEMORY_SESSIONS: usize = 256;

/// The portrait memory of the newest sessions, keyed by session directory
/// ([`blank::PortraitMemory`]: unblanked portrait cells, in memory only).
pub(super) fn portrait_memory() -> &'static std::sync::Mutex<Vec<(PathBuf, blank::PortraitMemory)>>
{
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
    reads.scoreboard_sha256 = super::super::hash::sha256_hex(&png);
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
/// `load` returns such a kit with reads and no scoreboard, so the game
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
