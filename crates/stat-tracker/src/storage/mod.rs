use std::path::{Path, PathBuf};
use std::sync::Arc;

mod commands;
mod edits;
mod files;
pub mod maintain;
mod migrate;
mod rows;
mod rules;
mod tombstones;
mod upload_queue;

use serde::{Deserialize, Serialize};
use surrealdb::Surreal;
use surrealdb::engine::local::SurrealKv;
use surrealdb_types::Datetime as SurrealDatetime;
use surrealdb_types::SurrealValue;

pub use crate::model::{
    HeroSegment, PersonalMatch, SEGMENT_CONFIRM, SEGMENT_DISMISS, SegmentResolution,
    primary_hero_role,
};

pub use commands::{StoreCommand, queue_command, read_commands, remove_command_file};
pub use files::{
    Snapshot, append_match_log, clear_match_log, force_clear_data_dir, read_match_log,
    read_snapshot,
};
pub use rules::{
    MatchEdit, carry_member_hero_edit, collect_segment_resolutions, derive_hero_segments,
    latest_per_game, majority_hero, majority_map, member_hero_edit,
};

pub use upload_queue::{SyncClaim, UPLOAD_REJECTED_BY_SERVER, format_upload_rejection};

use files::{rewrite_match_log_session, snapshot_path};
use rules::{apply_match_edit, name_recognizer_absent, outcome_is_decided};

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
pub struct MatchSession {
    pub session_id: String,
    pub hero: String,
    pub map_name: String,
    pub role: String,
    pub started_at: SurrealDatetime,
    pub last_capture_at: SurrealDatetime,
    pub capture_count: u32,
    pub final_outcome: String,
}

#[derive(Clone)]
pub struct LocalStore {
    db: Surreal<surrealdb::engine::local::Db>,
    /// Home of the side-channel files (`matches.jsonl`, snapshots) that some
    /// mutations must keep in step with the database.
    data_dir: PathBuf,
    /// Exclusive `flock` on the `stats.surrealkv` directory. Declared after
    /// `db` so the handle drops first; the lock releases on the last clone
    /// unless [`LocalStore::hold_writer_lock_until_exit`] leaked a dup.
    writer_lock: Arc<StoreDirLock>,
    /// Sessions [`LocalStore::delete_session`] removed in this process. A
    /// Tab capture still in flight for one of them must not bring it back.
    /// Not the `deleted_session` table: a pending tombstone there can also
    /// name a game that stays local. A capture cannot outlive its process,
    /// so this does not need to survive a restart.
    deleted_here: Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
}

/// Advisory exclusive lock on the store directory inode.
///
/// SurrealKV is single-writer. The pid file is not a lock. `flock` does not
/// block creating files inside the directory; it only conflicts with another
/// `flock` on this inode, which is what a second `LocalStore::open` takes.
struct StoreDirLock(std::fs::File);

/// `LocalStore::open` refused because another process (or an earlier open in
/// this one) still holds the store-directory flock.
#[derive(Debug)]
pub struct StoreBusy {
    path: PathBuf,
}

impl std::fmt::Display for StoreBusy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "stat store is already open in another process ({})",
            self.path.display()
        )
    }
}

impl std::error::Error for StoreBusy {}

pub fn is_store_busy(err: &(dyn std::error::Error + Send + Sync + 'static)) -> bool {
    err.is::<StoreBusy>()
}

fn lock_store_dir(
    db_path: &Path,
) -> Result<StoreDirLock, Box<dyn std::error::Error + Send + Sync>> {
    let file = std::fs::File::open(db_path)?;
    match file.try_lock() {
        Ok(()) => Ok(StoreDirLock(file)),
        Err(std::fs::TryLockError::WouldBlock) => {
            tracing::warn!(
                path = %db_path.display(),
                "stat store is already open in another process"
            );
            Err(StoreBusy {
                path: db_path.to_path_buf(),
            }
            .into())
        }
        Err(std::fs::TryLockError::Error(err)) => Err(err.into()),
    }
}

impl LocalStore {
    pub async fn open(data_dir: &Path) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let db_path = data_dir.join("stats.surrealkv");
        std::fs::create_dir_all(&db_path)?;
        crate::fs_mode::tighten_private_dir(&db_path);
        // Take the single-writer lock before SurrealKV opens the directory.
        let writer_lock = Arc::new(lock_store_dir(&db_path)?);

        let db =
            Surreal::new::<SurrealKv>(db_path.to_str().ok_or("data_dir path is not valid UTF-8")?)
                .await?;
        db.use_ns("stat_tracker").use_db("local").await?;

        db.query(
            "
            DEFINE TABLE IF NOT EXISTS personal_match SCHEMALESS;
            DEFINE INDEX IF NOT EXISTS idx_synced ON personal_match FIELDS synced;
            DEFINE INDEX IF NOT EXISTS idx_played_at ON personal_match FIELDS played_at;
            DEFINE INDEX IF NOT EXISTS idx_session_id ON personal_match FIELDS session_id;
            DEFINE TABLE IF NOT EXISTS match_session SCHEMALESS;
            DEFINE INDEX IF NOT EXISTS idx_session_id ON match_session FIELDS session_id;
            DEFINE INDEX IF NOT EXISTS idx_last_capture ON match_session FIELDS last_capture_at;
            DEFINE TABLE IF NOT EXISTS deleted_session SCHEMALESS;
            DEFINE INDEX IF NOT EXISTS idx_deleted_sid ON deleted_session FIELDS session_id;
            DEFINE FIELD IF NOT EXISTS recognizer ON personal_match TYPE string DEFAULT 'ocr-v1';
            DEFINE FIELD IF NOT EXISTS suspect_fields ON personal_match TYPE array<string> DEFAULT [];
            DEFINE FIELD IF NOT EXISTS name_recognizer ON personal_match TYPE option<string>;
        ",
        )
        .await?;

        tracing::info!(path = %db_path.display(), "local store opened");
        let store = Self {
            db,
            data_dir: data_dir.to_path_buf(),
            writer_lock,
            deleted_here: Arc::default(),
        };
        store.migrate_pack_key_heroes().await?;
        Ok(store)
    }

    /// Keep the store-directory flock until this process exits.
    ///
    /// Dropping [`LocalStore`] closes its fd before SurrealKV's detached
    /// memtable flush finishes. `File::try_clone` is a `CLOEXEC` dup: it stays
    /// open across that window and is not inherited across `exec`. The daemon
    /// calls this once after the store that must outlive shutdown is open.
    /// Same-process reopen (vacuum, tests) does not, so it can lock again
    /// after the handle drops.
    pub fn hold_writer_lock_until_exit(&self) {
        match self.writer_lock.0.try_clone() {
            Ok(dup) => std::mem::forget(dup),
            Err(e) => tracing::warn!(
                error = %e,
                "could not dup store lock; flock ends when the store handle drops"
            ),
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod tests_rows;

#[cfg(test)]
mod tests_upload_queue;

#[cfg(test)]
mod tests_tombstones;

#[cfg(test)]
mod tests_edits;

#[cfg(test)]
mod tests_hero_timeline;

#[cfg(test)]
mod tests_rules;

#[cfg(test)]
mod tests_migrate;

#[cfg(test)]
mod tests_commands;

#[cfg(test)]
mod tests_files;
