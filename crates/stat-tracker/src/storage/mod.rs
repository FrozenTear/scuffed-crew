use std::path::{Path, PathBuf};
use std::sync::Arc;

mod commands;
mod edits;
mod export;
mod files;
pub mod maintain;
mod migrate;
mod migrate_timeline;
mod rows;
mod rules;
mod timeline;
mod tombstones;
mod upload_queue;

use serde::{Deserialize, Serialize};
use surrealdb::Surreal;
use surrealdb::engine::local::SurrealKv;
use surrealdb_types::Datetime as SurrealDatetime;
use surrealdb_types::SurrealValue;

pub use crate::model::{
    HeroSegment, PersonalMatch, SEGMENT_CLEAR, SEGMENT_CONFIRM, SEGMENT_DISMISS, SegmentResolution,
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

pub use migrate_timeline::{
    PRE_TIMELINE_BACKUP_PREFIX, TimelineBackup, prepare_timeline_migration,
    remove_pre_timeline_backups,
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
    /// The store holds one hero timeline per game (`hero_timeline`, marker
    /// `store_meta:schema`), so new rows are written without their own copy.
    /// Read at open; set by [`LocalStore::migrate_timeline_if_needed`].
    timeline_migrated: Arc<std::sync::atomic::AtomicBool>,
    /// Stored-data revision ([`LocalStore::rev`]): moves on every write.
    rev: Arc<std::sync::atomic::AtomicU64>,
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
        Self::open_inner(data_dir, true).await
    }

    /// [`Self::open`]; `pack_keys` false skips the pack-key pass (a quick
    /// look at the migration marker before the daemon's real open).
    async fn open_inner(
        data_dir: &Path,
        pack_keys: bool,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
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
            DEFINE TABLE IF NOT EXISTS hero_timeline SCHEMALESS;
            DEFINE INDEX IF NOT EXISTS idx_timeline_session ON hero_timeline FIELDS session_id UNIQUE;
            DEFINE TABLE IF NOT EXISTS store_meta SCHEMALESS;
        ",
        )
        .await?;

        tracing::info!(path = %db_path.display(), "local store opened");
        let store = Self {
            db,
            data_dir: data_dir.to_path_buf(),
            writer_lock,
            deleted_here: Arc::default(),
            timeline_migrated: Arc::default(),
            rev: Arc::new(std::sync::atomic::AtomicU64::new(start_rev())),
        };
        // Read only: `open` never migrates (vacuum, the GUI's clear and the
        // tools open stores too). The daemon calls migrate_timeline_if_needed.
        let migrated = store.timeline_marker().await?;
        store
            .timeline_migrated
            .store(migrated, std::sync::atomic::Ordering::Relaxed);
        if pack_keys {
            store.migrate_pack_key_heroes().await?;
        }
        Ok(store)
    }

    /// The stored-data revision: starts at the open time in unix ms (a new
    /// process never repeats one) and moves when a method that writes starts
    /// and again when it returns ([`RevGuard`]), on every path. A reader that
    /// caches by revision and raced a write cached it under the first bump;
    /// the second bump, after the write, makes it read again.
    pub fn rev(&self) -> u64 {
        self.rev.load(std::sync::atomic::Ordering::Acquire)
    }

    pub(super) fn rev_guard(&self) -> RevGuard {
        self.rev.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        RevGuard(self.rev.clone())
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

/// Bumps the store revision again when dropped (see [`LocalStore::rev`]).
pub(super) struct RevGuard(Arc<std::sync::atomic::AtomicU64>);

impl Drop for RevGuard {
    fn drop(&mut self) {
        self.0.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    }
}

fn start_rev() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(1)
}

/// Wait until SurrealKV has closed the store in `data_dir`, for at most
/// `within`. Call it after the last [`LocalStore`] handle is dropped.
///
/// SurrealKV takes an flock on `stats.surrealkv/LOCK` at open and releases
/// it as the last step of its close, after the memtable flush and the WAL
/// cleanup. flock is per open file, so this process can take it on a new fd
/// once the close is done. Our own writer lock is on the directory, not on
/// `LOCK`, and stays held until exit. Returns `false` when the close did not
/// finish in time.
///
/// Read against surrealkv 0.21.2 (`Tree::close` in lsm.rs releases the lock
/// after the flush, WAL close and cleanup). Recheck that order when
/// Cargo.lock moves surrealkv; the storage test
/// `wait_for_engine_close_sees_surrealkv_finish_its_close` fails if a close
/// that frees `LOCK` leaves WAL records to replay.
pub async fn wait_for_engine_close(data_dir: &Path, within: std::time::Duration) -> bool {
    match lock_released_within(data_dir, within).await {
        Ok(closed) => closed,
        Err(err) => {
            // Waiting cannot tell more. At exit, going on is right.
            tracing::warn!(error = %err, "cannot check the store's LOCK; not waiting for its close");
            true
        }
    }
}

/// [`wait_for_engine_close`] without its exit-time choice: `Err` when the
/// `LOCK` file cannot be opened (other than missing) or flock-tested. Vacuum
/// treats that as not closed, so it never swaps a store it could not check.
async fn lock_released_within(
    data_dir: &Path,
    within: std::time::Duration,
) -> std::io::Result<bool> {
    let lock = data_dir.join("stats.surrealkv").join("LOCK");
    let until = tokio::time::Instant::now() + within;
    loop {
        // flock needs no write access.
        match std::fs::File::open(&lock) {
            // Locked and dropped at once: nothing opens the store after this.
            Ok(file) => match file.try_lock() {
                Ok(()) => return Ok(true),
                Err(std::fs::TryLockError::WouldBlock) => {}
                Err(std::fs::TryLockError::Error(err)) => return Err(err),
            },
            // No LOCK file: no engine to wait for.
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(true),
            Err(err) => return Err(err),
        }
        if tokio::time::Instant::now() >= until {
            return Ok(false);
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
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

#[cfg(test)]
mod tests_export;

#[cfg(test)]
mod tests_timeline;
