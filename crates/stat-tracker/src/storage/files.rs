//! The on-disk files beside the store: the append-only `matches.jsonl` log
//! (a record nothing in the app reads) and the `live_snapshot.json` export
//! the GUI and report bundle read.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{MatchSession, PersonalMatch};

/// The daemon's live export of the full store, for readers (the GUI) that
/// can't open SurrealKV while the daemon has it locked.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Snapshot {
    /// All matches, newest first (mirrors `get_all_matches`).
    pub matches: Vec<PersonalMatch>,
    /// All sessions, most recently captured first (mirrors `get_all_sessions`).
    pub sessions: Vec<MatchSession>,
    /// Deleted games whose server delete (tombstone) has not synced yet
    /// (`get_pending_tombstones`). Absent in older snapshots.
    #[serde(default)]
    pub pending_deletes: Vec<String>,
}

pub(super) fn snapshot_path(data_dir: &Path) -> PathBuf {
    data_dir.join("live_snapshot.json")
}

/// Read the daemon's live snapshot. `None` if it doesn't exist yet or is
/// unreadable — callers fall back to the append-only match log.
pub fn read_snapshot(data_dir: &Path) -> Option<Snapshot> {
    let content = std::fs::read(snapshot_path(data_dir)).ok()?;
    serde_json::from_slice(&content).ok()
}

fn match_log_path(data_dir: &Path) -> PathBuf {
    data_dir.join("matches.jsonl")
}

/// Rewrite `matches.jsonl` rows of one session (atomic tmp+rename): apply
/// `update` to each, or drop them entirely when `update` is `None`. The log is
/// append-only at capture time; this keeps corrections and deletes in step
/// (A5). Nothing in the app reads the log, and every call reads, parses and
/// rewrites the whole file, so call it only for rare corrections, never per
/// capture. Unparseable lines (schema drift) are preserved verbatim.
pub(super) fn rewrite_match_log_session(
    data_dir: &Path,
    session_id: &str,
    update: Option<&dyn Fn(&mut PersonalMatch)>,
) {
    let path = match_log_path(data_dir);
    let Ok(content) = std::fs::read_to_string(&path) else {
        return;
    };
    let mut out = String::with_capacity(content.len());
    let mut changed = false;
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<PersonalMatch>(line) {
            Ok(mut m) if m.session_id == session_id => {
                changed = true;
                if let Some(f) = update {
                    f(&mut m);
                    match serde_json::to_string(&m) {
                        Ok(json) => {
                            out.push_str(&json);
                            out.push('\n');
                        }
                        Err(_) => {
                            out.push_str(line);
                            out.push('\n');
                        }
                    }
                }
                // update == None → session deleted, drop the line
            }
            _ => {
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    if !changed {
        return;
    }
    let tmp = path.with_extension("jsonl.tmp");
    if std::fs::write(&tmp, out).is_err() || std::fs::rename(&tmp, &path).is_err() {
        tracing::debug!(session_id, "failed to rewrite match log");
    }
}

pub fn append_match_log(data_dir: &Path, m: &PersonalMatch) {
    use std::io::Write;
    let path = match_log_path(data_dir);
    let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };
    if let Ok(json) = serde_json::to_string(m) {
        let _ = writeln!(f, "{json}");
    }
}

/// Remove the on-disk exports (append-only log + live snapshot). Called by
/// every clear path — a stale snapshot would resurrect cleared data in the
/// GUI's locked-store view.
pub fn clear_match_log(data_dir: &Path) {
    for path in [match_log_path(data_dir), snapshot_path(data_dir)] {
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
    }
}

pub fn force_clear_data_dir(data_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let db_path = data_dir.join("stats.surrealkv");
    if db_path.exists() {
        std::fs::remove_dir_all(&db_path)?;
    }
    clear_match_log(data_dir);
    super::remove_pre_timeline_backups(data_dir);
    tracing::info!("force-cleared data directory");
    Ok(())
}

pub fn read_match_log(data_dir: &Path) -> Vec<PersonalMatch> {
    let path = match_log_path(data_dir);
    let Ok(content) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let mut matches: Vec<PersonalMatch> = content
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    matches.sort_by(|a, b| {
        let da: chrono::DateTime<chrono::Utc> = b.played_at.into();
        let db_time: chrono::DateTime<chrono::Utc> = a.played_at.into();
        da.cmp(&db_time)
    });
    matches
}
