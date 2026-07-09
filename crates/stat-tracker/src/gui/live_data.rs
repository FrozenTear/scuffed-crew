//! Shared GUI data loading: open SurrealKV when free, else snapshot/log.
//!
//! Avoids copy-pasted open-store-or-snapshot policies across panels and skips
//! re-parsing `live_snapshot.json` when its mtime is unchanged.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use stat_tracker::storage::{self, LocalStore, PersonalMatch, Snapshot};

/// Result of a live-data fetch for a GUI panel.
#[derive(Clone, Debug)]
pub struct LiveMatches {
    pub matches: Vec<PersonalMatch>,
    /// True when the store could not be opened (daemon holds the lock) and we
    /// fell back to snapshot/jsonl.
    pub db_locked: bool,
}

struct SnapshotCache {
    path: PathBuf,
    mtime: SystemTime,
    matches: Vec<PersonalMatch>,
}

static SNAPSHOT_CACHE: Mutex<Option<SnapshotCache>> = Mutex::new(None);

fn snapshot_mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Read matches from the live snapshot, reusing a process-wide cache when the
/// file mtime has not changed.
pub fn read_snapshot_matches_cached(data_dir: &Path) -> Option<Vec<PersonalMatch>> {
    let path = data_dir.join("live_snapshot.json");
    let mtime = snapshot_mtime(&path)?;

    if let Ok(guard) = SNAPSHOT_CACHE.lock()
        && let Some(cached) = guard.as_ref()
        && cached.path == path
        && cached.mtime == mtime
    {
        return Some(cached.matches.clone());
    }

    let snap: Snapshot = storage::read_snapshot(data_dir)?;
    let matches = snap.matches;
    if let Ok(mut guard) = SNAPSHOT_CACHE.lock() {
        *guard = Some(SnapshotCache {
            path,
            mtime,
            matches: matches.clone(),
        });
    }
    Some(matches)
}

/// Load all personal_match rows for GUI panels.
///
/// Prefer the live store; on lock failure use the daemon snapshot (mtime-cached)
/// and finally the append-only log.
pub async fn fetch_live_matches(data_dir: &Path) -> LiveMatches {
    match LocalStore::open(data_dir).await {
        Ok(store) => {
            let matches = store.get_all_matches().await.unwrap_or_default();
            LiveMatches {
                matches,
                db_locked: false,
            }
        }
        Err(_) => {
            let matches = read_snapshot_matches_cached(data_dir)
                .unwrap_or_else(|| storage::read_match_log(data_dir));
            LiveMatches {
                matches,
                db_locked: true,
            }
        }
    }
}
