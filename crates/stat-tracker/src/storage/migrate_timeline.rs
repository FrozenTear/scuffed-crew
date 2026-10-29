//! The one-time move of each game's hero timeline into `hero_timeline`.
//!
//! Additive: no row is changed or removed. Every row keeps its own
//! `heroes_played` copy, so an older tracker still shows every game that
//! existed before the move. Only the daemon runs it, after startup
//! maintenance (auto-vacuum) and after its store is open; `LocalStore::open`
//! never does, so vacuum, the GUI's clear and the tools never write a backup.
//!
//! Order:
//! 1. [`prepare_timeline_migration`], before the daemon's open: if the store
//!    has no `store_meta:schema` marker, copy `stats.surrealkv` (closed) to
//!    `stats.surrealkv.pre-timeline-<UTC stamp>/` (via a `.partial` name, so a
//!    complete copy is the only one with the final name). No space or a
//!    failed copy: no migration, the store keeps today's layout.
//! 2. [`LocalStore::migrate_timeline_if_needed`]: one session at a time,
//!    store the timeline of the row the app shows for the game. A game whose
//!    shown row has no stored timeline gets no record and keeps showing what
//!    it shows today. Records are rebuilt from the rows until the marker is
//!    set (reads ignore them before that), so a stop part way through is
//!    finished by the next start with nothing stale.
//! 3. The marker is written last.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use surrealdb_types::{RecordId, SurrealValue};

use super::maintain::{STORE_DIR_NAME, dir_size};
use super::{LocalStore, PersonalMatch, collect_segment_resolutions};

/// Prefix for the pre-migration copy: `stats.surrealkv.pre-timeline-<stamp>`.
/// Not `pre-vacuum-`, so the vacuum backup prune never removes it.
pub const PRE_TIMELINE_BACKUP_PREFIX: &str = "stats.surrealkv.pre-timeline-";

/// Current timeline layout version in `store_meta:schema`.
const TIMELINE_V: u32 = 1;

/// Sessions read per page during the fill.
const FILL_PAGE: usize = 100;

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
#[surreal(crate = "::surrealdb_types")]
struct SchemaMarker {
    #[surreal(default)]
    timeline_v: u32,
}

#[derive(Debug, Deserialize, SurrealValue)]
#[surreal(crate = "::surrealdb_types")]
struct SessionKey {
    session_id: String,
}

fn marker_id() -> RecordId {
    RecordId::new("store_meta", "schema")
}

/// What [`prepare_timeline_migration`] found or did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimelineBackup {
    /// The store is migrated already.
    NotNeeded,
    /// A complete copy of the store exists (`None`: there was no data).
    Ready(Option<PathBuf>),
    /// No migration this start; the store keeps today's layout.
    Failed(String),
}

/// Free bytes on the file system holding `path`.
fn free_bytes(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt as _;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: statvfs is a plain C struct; all-zero is a valid value.
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a valid C string and `st` a valid out pointer.
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    Some(st.f_bavail as u64 * st.f_frsize as u64)
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let dest = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &dest)?;
        } else {
            std::fs::copy(entry.path(), &dest)?;
            // On disk before the copy can count as complete.
            std::fs::File::open(&dest)?.sync_all()?;
        }
    }
    sync_dir(to)
}

/// fsync a directory, so the entries made in it survive a power loss.
fn sync_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::File::open(dir)?.sync_all()
}

/// A complete pre-timeline copy left by an earlier start whose move did not
/// finish (its `.pending` file is still there). It holds the state before
/// any move write. An older copy from a finished move is never reused.
fn existing_backup(data_dir: &Path) -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(data_dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_dir()
                && p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                    n.starts_with(PRE_TIMELINE_BACKUP_PREFIX) && !n.ends_with(".partial")
                })
                && pending_path(p).exists()
        })
        .collect();
    found.sort();
    found.pop()
}

/// Remove every pre-timeline backup (it holds the user's games): "Delete
/// all local match data" and the uninstaller remove it with the store.
pub fn remove_pre_timeline_backups(data_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(data_dir) else {
        return;
    };
    for path in entries.flatten().map(|e| e.path()) {
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with(PRE_TIMELINE_BACKUP_PREFIX))
        {
            let _ = if path.is_dir() {
                std::fs::remove_dir_all(&path)
            } else {
                std::fs::remove_file(&path)
            };
        }
    }
}

/// Step 1, before the daemon opens its store (see the module doc). The
/// daemon must hold the pid guard; nothing else may open the store.
pub async fn prepare_timeline_migration(data_dir: &Path) -> TimelineBackup {
    prepare_with(data_dir, free_bytes).await
}

/// [`prepare_timeline_migration`] with the free-space check passed in.
pub(super) async fn prepare_with(
    data_dir: &Path,
    free_bytes: impl Fn(&Path) -> Option<u64>,
) -> TimelineBackup {
    let live = data_dir.join(STORE_DIR_NAME);
    let has_data = std::fs::read_dir(&live).is_ok_and(|mut d| d.next().is_some());
    if !has_data {
        return TimelineBackup::Ready(None);
    }
    let migrated = match LocalStore::open_inner(data_dir, false).await {
        Ok(store) => store.timeline_migrated(),
        Err(e) => return TimelineBackup::Failed(format!("could not open the store: {e}")),
    };
    // Dropping the handle only starts SurrealKV's close; the daemon's own
    // open right after this must not find the engine still holding LOCK.
    match super::lock_released_within(data_dir, std::time::Duration::from_secs(10)).await {
        Ok(true) => {}
        Ok(false) => return TimelineBackup::Failed("the store did not finish closing".into()),
        Err(e) => return TimelineBackup::Failed(format!("cannot check the store closed: {e}")),
    }
    if migrated {
        return TimelineBackup::NotNeeded;
    }
    if let Some(path) = existing_backup(data_dir) {
        return TimelineBackup::Ready(Some(path));
    }
    remove_partial_backups(data_dir);
    let need = dir_size(&live).saturating_mul(2);
    match free_bytes(data_dir) {
        Some(free) if free >= need => {}
        Some(free) => {
            return TimelineBackup::Failed(format!(
                "not enough free disk for the backup ({free} bytes free, {need} needed)"
            ));
        }
        None => return TimelineBackup::Failed("cannot read the free disk space".into()),
    }
    let stamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let backup = data_dir.join(format!("{PRE_TIMELINE_BACKUP_PREFIX}{stamp}"));
    let partial = data_dir.join(format!("{PRE_TIMELINE_BACKUP_PREFIX}{stamp}.partial"));
    let _ = std::fs::remove_dir_all(&partial);
    let copied = copy_tree(&live, &partial)
        .and_then(|()| sync_dir(data_dir))
        .and_then(|()| std::fs::rename(&partial, &backup))
        .and_then(|()| sync_dir(data_dir));
    if let Err(e) = copied {
        let _ = std::fs::remove_dir_all(&partial);
        return TimelineBackup::Failed(format!("backup copy failed: {e}"));
    }
    crate::fs_mode::tighten_private_dir(&backup);
    // Marks this copy as the one for a move still in progress; removed with
    // the marker. Only such a copy is reused by a later start.
    if let Err(e) = std::fs::write(pending_path(&backup), b"") {
        return TimelineBackup::Failed(format!("backup copy failed: {e}"));
    }
    TimelineBackup::Ready(Some(backup))
}

/// `<backup>.pending`: this backup belongs to a move that has not finished.
fn pending_path(backup: &Path) -> PathBuf {
    let mut name = backup.as_os_str().to_owned();
    name.push(".pending");
    PathBuf::from(name)
}

/// After a finished move, keep only `keep` (the copy this move made) of
/// the finished pre-timeline copies: older ones (left by a move redone
/// after an older tracker's vacuum dropped the tables) hold games the user
/// may have deleted since. A copy still pending is never removed here.
fn prune_finished_backups(data_dir: &Path, keep: &Path) {
    let Ok(entries) = std::fs::read_dir(data_dir) else {
        return;
    };
    for path in entries.flatten().map(|e| e.path()) {
        let finished_copy = path.is_dir()
            && path != keep
            && !pending_path(&path).exists()
            && path.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                n.starts_with(PRE_TIMELINE_BACKUP_PREFIX) && !n.ends_with(".partial")
            });
        if finished_copy {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

/// Copies an interrupted start left half done.
fn remove_partial_backups(data_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(data_dir) else {
        return;
    };
    for path in entries.flatten().map(|e| e.path()) {
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with(PRE_TIMELINE_BACKUP_PREFIX) && n.ends_with(".partial"))
        {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

impl LocalStore {
    /// True once the store holds one timeline per game.
    pub fn timeline_migrated(&self) -> bool {
        self.timeline_migrated
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    pub(super) async fn timeline_marker(
        &self,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        let marker: Option<SchemaMarker> = self
            .db
            .query("SELECT * FROM $rid")
            .bind(("rid", marker_id()))
            .await?
            .take(0)?;
        Ok(marker.is_some_and(|m| m.timeline_v >= TIMELINE_V))
    }

    pub(super) async fn write_timeline_marker(
        &self,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.db
            .query("UPSERT $rid SET timeline_v = $v, migrated_at = time::now() RETURN NONE")
            .bind(("rid", marker_id()))
            .bind(("v", TIMELINE_V))
            .await?
            .check()?;
        Ok(())
    }

    /// Steps 2 and 3 (see the module doc). Returns the records written.
    pub async fn migrate_timeline_if_needed(
        &self,
        backup: &TimelineBackup,
    ) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        let _rev = self.rev_guard();
        if self.timeline_migrated() || self.timeline_marker().await? {
            self.set_timeline_migrated(true);
            return Ok(0);
        }
        match backup {
            TimelineBackup::Ready(_) => {}
            TimelineBackup::Failed(why) => {
                tracing::error!(reason = %why, "hero timeline move skipped: no backup; the store keeps its layout");
                return Ok(0);
            }
            TimelineBackup::NotNeeded => {
                tracing::error!(
                    "hero timeline move skipped: the marker went missing after the backup check"
                );
                return Ok(0);
            }
        }
        let written = self.fill_timelines(None).await?;
        self.write_timeline_marker().await?;
        self.set_timeline_migrated(true);
        if let TimelineBackup::Ready(Some(path)) = backup {
            let _ = std::fs::remove_file(pending_path(path));
            if let Some(data_dir) = path.parent() {
                prune_finished_backups(data_dir, path);
            }
        }
        tracing::info!(
            records = written,
            "hero timelines moved to one record per game"
        );
        Ok(written)
    }

    pub(super) fn set_timeline_migrated(&self, on: bool) {
        self.timeline_migrated
            .store(on, std::sync::atomic::Ordering::Relaxed);
    }

    /// Store a record for each session whose shown row has a stored timeline,
    /// unless an equal record is there already. `stop_after` ends the pass
    /// after that many sessions (a crash in tests). Returns the records
    /// written.
    pub(super) async fn fill_timelines(
        &self,
        stop_after: Option<usize>,
    ) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        let mut after: Option<String> = None;
        let (mut visited, mut written) = (0usize, 0usize);
        loop {
            let keys: Vec<SessionKey> = match &after {
                None => self
                    .db
                    .query(format!(
                        "SELECT session_id FROM match_session ORDER BY session_id ASC LIMIT {FILL_PAGE}"
                    ))
                    .await?
                    .take(0)?,
                Some(last) => self
                    .db
                    .query(format!(
                        "SELECT session_id FROM match_session WHERE session_id > $last \
                         ORDER BY session_id ASC LIMIT {FILL_PAGE}"
                    ))
                    .bind(("last", last.clone()))
                    .await?
                    .take(0)?,
            };
            let Some(last) = keys.last().map(|k| k.session_id.clone()) else {
                break;
            };
            let have = self
                .load_timelines(keys.iter().map(|k| k.session_id.as_str()))
                .await?;
            let mut page_ids: Vec<&str> = keys.iter().map(|k| k.session_id.as_str()).collect();
            page_ids.dedup();
            for sid in page_ids {
                if stop_after.is_some_and(|n| visited >= n) {
                    return Ok(written);
                }
                visited += 1;
                if sid.is_empty() {
                    continue;
                }
                let rows = self.session_rows_raw(sid).await?;
                // The list the game shows today: the own list of the row the
                // app shows (the newest that passes the tracked filter, else
                // the newest). Rows written since alpha.1.1 have no list of their
                // own (a rebuild after an older vacuum dropped the table), so
                // then the newest row that has one. No list on any row: no
                // record, the game shows as today.
                let shown = rows
                    .iter()
                    .rev()
                    .find(|m| super::export::row_is_tracked(m))
                    .or(rows.last())
                    .filter(|m| !m.heroes_played.is_empty())
                    .or_else(|| rows.iter().rev().find(|m| !m.heroes_played.is_empty()));
                let Some(shown) = shown else {
                    continue;
                };
                let resolutions = collect_segment_resolutions(&rows);
                let fresh = (&shown.heroes_played, &resolutions);
                // Rewritten from the rows each pass until the marker is set,
                // so a record left by an interrupted pass is never stale.
                if have
                    .get(sid)
                    .is_some_and(|t| (&t.heroes_played, &t.segment_resolutions) == fresh)
                {
                    continue;
                }
                self.upsert_timeline(sid, &shown.heroes_played, &resolutions)
                    .await?;
                written += 1;
            }
            if keys.len() < FILL_PAGE {
                break;
            }
            after = Some(last);
        }
        Ok(written)
    }

    /// A session's rows oldest first, as stored (not hydrated).
    pub(super) async fn session_rows_raw(
        &self,
        session_id: &str,
    ) -> Result<Vec<PersonalMatch>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self
            .db
            .query("SELECT * FROM personal_match WHERE session_id = $sid ORDER BY played_at ASC")
            .bind(("sid", session_id.to_string()))
            .await?
            .take(0)?)
    }
}
