use std::path::{Path, PathBuf};
use std::sync::Arc;

pub mod maintain;

use serde::{Deserialize, Serialize};
use surrealdb::Surreal;
use surrealdb::engine::local::SurrealKv;
use surrealdb_types::Datetime as SurrealDatetime;
use surrealdb_types::SurrealValue;

pub use crate::model::{
    HeroSegment, PersonalMatch, SEGMENT_CONFIRM, SEGMENT_DISMISS, SegmentResolution,
    primary_hero_role,
};

/// Identity and payload revision captured when a row was read for upload.
///
/// [`LocalStore::mark_synced`] commits `synced = true` only while `sync_rev`
/// still equals this value. A local write during the upload bumps `sync_rev`
/// in the same statement that changes the payload, so the compare-and-swap
/// misses and the row stays queued.
#[derive(Debug, Clone)]
pub struct SyncClaim {
    pub id: surrealdb_types::RecordId,
    pub sync_rev: u64,
    pub session_id: String,
}

impl SyncClaim {
    /// Claims for rows that have a store id (fresh parses, which have none,
    /// cannot be marked).
    pub fn capture(rows: &[PersonalMatch]) -> Vec<Self> {
        rows.iter()
            .filter_map(|m| {
                m.id.clone().map(|id| SyncClaim {
                    id,
                    sync_rev: m.sync_rev,
                    session_id: m.session_id.clone(),
                })
            })
            .collect()
    }

    /// Empty `session_id` rows are legacy one-offs: each is its own group so a
    /// revision miss on one does not unmark the others.
    fn group_key(&self) -> String {
        if self.session_id.is_empty() {
            format!("id:{:?}", self.id)
        } else {
            self.session_id.clone()
        }
    }

    /// This claim is the uploaded row, or another snapshot of its session.
    /// An empty `session_id` matches only the same store id.
    pub fn covers_row(&self, row: &PersonalMatch) -> bool {
        if row.session_id.is_empty() {
            row.id.as_ref() == Some(&self.id)
        } else {
            self.session_id == row.session_id
        }
    }
}

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

/// Display name and role for a pack file key or a hero the table already names.
///
/// `wrecking-ball` becomes `Wrecking Ball` / Tank. A stored display name such
/// as `Wrecking Ball` keeps that text and still returns the table role, so a
/// wrong role can be corrected. A folded alias such as `Lucio` keeps its
/// stored spelling. `None` when the text is not a known hero.
fn known_hero_role(raw: &str) -> Option<(String, String)> {
    let from_pack = scuffed_types::hero_for_pack_file(raw);
    let display = from_pack
        .map(|row| row.display)
        .or_else(|| scuffed_types::hero_key_to_name(raw))?;
    let role = scuffed_types::role_for_hero_name(display)?.to_string();
    let hero = if from_pack.is_some() || raw.trim() == display {
        display.to_string()
    } else {
        raw.trim().to_string()
    };
    Some((hero, role))
}

/// Write `hero` and `role` from the hero table when this text is a known hero.
fn apply_known_hero(hero: &mut String, role: &mut String) -> bool {
    let Some((next_hero, next_role)) = known_hero_role(hero) else {
        return false;
    };
    let mut changed = false;
    if *hero != next_hero {
        *hero = next_hero;
        changed = true;
    }
    if *role != next_role {
        *role = next_role;
        changed = true;
    }
    changed
}

/// Rewrite pack-key heroes, and fix a wrong role on a canonical hero.
///
/// Returns whether any field changed. The row is marked unsynced only when
/// the hero or role that would be uploaded changes.
fn rewrite_pack_key_fields(row: &mut PersonalMatch) -> bool {
    let before_hero = row.display_hero().to_string();
    let before_role = row.display_role().to_string();
    let mut changed = false;
    changed |= apply_known_hero(&mut row.hero, &mut row.role);
    if let Some(current) = row.corrected_hero.clone() {
        let mut hero = current;
        let had_role = row.corrected_role.is_some();
        let mut role = row.corrected_role.clone().unwrap_or_default();
        if apply_known_hero(&mut hero, &mut role) {
            let hero_changed = row.corrected_hero.as_deref() != Some(hero.as_str());
            let role_changed = had_role && row.corrected_role.as_deref() != Some(role.as_str());
            if hero_changed || role_changed {
                row.corrected_hero = Some(hero);
                if had_role || hero_changed {
                    row.corrected_role = Some(role);
                }
                changed = true;
            }
        }
    }
    for segment in &mut row.heroes_played {
        changed |= apply_known_hero(&mut segment.hero, &mut segment.role);
    }
    if !changed {
        return false;
    }
    if before_hero != row.display_hero() || before_role != row.display_role() {
        row.synced = false;
        row.sync_rev = row.sync_rev.saturating_add(1);
        row.upload_reject = None;
    }
    true
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

    /// Rewrite saved pack-key heroes to the display name, and set the role
    /// from the hero table. A row that already says `Wrecking Ball` keeps
    /// that name and still gets Tank when the stored role is wrong.
    ///
    /// Runs on every open. A second run finds nothing to change and writes
    /// nothing. A game is queued for upload only when the hero or role that
    /// would be sent actually changes.
    pub async fn migrate_pack_key_heroes(
        &self,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let matches = self.get_all_matches().await?;
        let mut rewritten = 0usize;
        let mut reupload = 0usize;
        let mut sessions = std::collections::BTreeSet::new();
        for mut row in matches {
            let before_hero = row.display_hero().to_string();
            let before_role = row.display_role().to_string();
            if !rewrite_pack_key_fields(&mut row) {
                continue;
            }
            let queue = before_hero != row.display_hero() || before_role != row.display_role();
            self.write_rewritten_match(&row, queue).await?;
            if !row.session_id.is_empty() {
                sessions.insert(row.session_id.clone());
            }
            rewritten += 1;
            if queue {
                reupload += 1;
            }
        }
        for session in self.get_all_sessions().await? {
            let Some((hero, role)) = known_hero_role(&session.hero) else {
                continue;
            };
            if session.hero == hero && session.role == role {
                continue;
            }
            self.db
                .query(
                    "UPDATE match_session SET hero = $hero, role = $role WHERE session_id = $sid",
                )
                .bind(("hero", hero.to_string()))
                .bind(("role", role))
                .bind(("sid", session.session_id.clone()))
                .await?;
            sessions.insert(session.session_id);
        }
        for session_id in &sessions {
            rewrite_match_log_session(
                self.data_dir.as_path(),
                session_id,
                Some(&|row| {
                    rewrite_pack_key_fields(row);
                }),
            );
        }
        if rewritten > 0 {
            tracing::info!(rewritten, reupload, "rewrote saved pack key heroes");
        }
        Ok(())
    }

    async fn write_rewritten_match(
        &self,
        row: &PersonalMatch,
        reupload: bool,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let Some(id) = row.id.clone() else {
            return Ok(());
        };
        let mut sql =
            String::from("UPDATE $id SET hero = $hero, role = $role, heroes_played = $hp");
        if reupload {
            sql.push_str(", synced = false, upload_reject = '', sync_rev = (sync_rev ?? 0) + 1");
        }
        if row.corrected_hero.is_some() {
            sql.push_str(", corrected_hero = $ch, corrected_role = $cr");
        }
        let mut query = self
            .db
            .query(sql)
            .bind(("id", id))
            .bind(("hero", row.hero.clone()))
            .bind(("role", row.role.clone()))
            .bind(("hp", row.heroes_played.clone()));
        if let Some(hero) = row.corrected_hero.clone() {
            query = query
                .bind(("ch", hero))
                .bind(("cr", row.corrected_role.clone().unwrap_or_default()));
        }
        query.await?;
        Ok(())
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

    pub async fn insert_match(
        &self,
        mut match_data: PersonalMatch,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if !match_data.session_id.is_empty() {
            // A capture that was in flight when its game was deleted must not
            // bring the game back. The caller already logged the row to
            // `matches.jsonl`; drop the session from there too.
            if self.deleted_here(&match_data.session_id) {
                tracing::info!(
                    session_id = %match_data.session_id,
                    "row for a deleted game not stored"
                );
                rewrite_match_log_session(&self.data_dir, &match_data.session_id, None);
                return Ok(());
            }
            // A capture requested before the result was known carries
            // `unknown`. When the session was decided meanwhile (a poll, a
            // GUI edit), the new row takes that result, or it would be the
            // one row of the game held back from upload (MAIN-1).
            if !outcome_is_decided(&match_data.outcome)
                && let Some(outcome) = self.session_decided_outcome(&match_data.session_id).await?
            {
                match_data.outcome = outcome.clone();
                // The caller logged this row as unknown. Keep the fallback
                // log in step with the store.
                rewrite_match_log_session(
                    &self.data_dir,
                    &match_data.session_id,
                    Some(&|m| {
                        if !outcome_is_decided(&m.outcome) {
                            m.outcome = outcome.clone();
                        }
                    }),
                );
            }
        }
        // A later snapshot that did not read a name must keep the recognizer
        // already stored for this match. A resend that omits it clears the
        // server's copy.
        if name_recognizer_absent(&match_data.name_recognizer)
            && !match_data.session_id.is_empty()
            && let Some(kept) = self.session_name_recognizer(&match_data.session_id).await?
        {
            match_data.name_recognizer = Some(kept);
        }
        let created: Option<PersonalMatch> =
            self.db.create("personal_match").content(match_data).await?;
        self.drop_if_deleted_meanwhile(created).await?;
        tracing::debug!("match inserted into local store");
        Ok(())
    }

    /// The `deleted_here` check in [`Self::insert_match`] runs before an
    /// await. A delete that marks and commits during that await would miss
    /// the row created after it, and the game would come back. Re-check
    /// after the write and remove just that row.
    async fn drop_if_deleted_meanwhile(
        &self,
        created: Option<PersonalMatch>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let Some(row) = created else {
            return Ok(());
        };
        if row.session_id.is_empty() || !self.deleted_here(&row.session_id) {
            return Ok(());
        }
        if let Some(id) = row.id {
            self.db
                .query("DELETE $rid")
                .bind(("rid", id))
                .await?
                .check()?;
        }
        rewrite_match_log_session(&self.data_dir, &row.session_id, None);
        tracing::info!(
            session_id = %row.session_id,
            "row for a game deleted during the write removed"
        );
        Ok(())
    }

    /// [`Self::delete_session`] removed this session in this process.
    fn deleted_here(&self, session_id: &str) -> bool {
        self.deleted_here
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(session_id)
    }

    /// The session row's outcome, when it is a decided one.
    async fn session_decided_outcome(
        &self,
        session_id: &str,
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let mut result = self
            .db
            .query("SELECT final_outcome FROM match_session WHERE session_id = $sid")
            .bind(("sid", session_id.to_string()))
            .await?;
        let rows: Vec<SessionOutcome> = result.take(0)?;
        Ok(rows
            .into_iter()
            .map(|row| row.final_outcome)
            .find(|outcome| outcome_is_decided(outcome)))
    }

    /// The recognizer already stored for this match, newest snapshot first.
    async fn session_name_recognizer(
        &self,
        session_id: &str,
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let snaps = self.get_session_snapshots(session_id).await?;
        Ok(snaps.into_iter().rev().find_map(|row| {
            let id = row.name_recognizer?;
            (!id.is_empty()).then_some(id)
        }))
    }

    /// Rows awaiting upload. Holds back `unknown` outcomes: the server only
    /// stores decided games, and one unstorable row at the head of the queue
    /// used to fail the whole batch forever. An outcome back-fill flips the
    /// row to a decided outcome and `synced = false`, releasing it here.
    ///
    /// A closed session with no result is not sent. [`Self::get_unsynced_unknown`]
    /// loads those rows so the caller can mark them synced with no request.
    /// A GUI `SetOutcome` writes a real outcome and `synced = false`, which
    /// releases the row here.
    ///
    /// Rows with [`PersonalMatch::upload_rejection`] set are quarantined.
    /// They stay local until a retry clears the message.
    pub async fn get_unsynced(
        &self,
    ) -> Result<Vec<PersonalMatch>, Box<dyn std::error::Error + Send + Sync>> {
        self.pending_upload_rows(false).await
    }

    /// Unsynced rows whose outcome is still `unknown`. The server cannot store
    /// them. Callers mark these synced locally and do not put them in a request.
    pub async fn get_unsynced_unknown(
        &self,
    ) -> Result<Vec<PersonalMatch>, Box<dyn std::error::Error + Send + Sync>> {
        self.pending_upload_rows(true).await
    }

    async fn pending_upload_rows(
        &self,
        unknown_only: bool,
    ) -> Result<Vec<PersonalMatch>, Box<dyn std::error::Error + Send + Sync>> {
        let sql = if unknown_only {
            "SELECT * FROM personal_match WHERE synced = false AND outcome = 'unknown' ORDER BY played_at ASC"
        } else {
            "SELECT * FROM personal_match WHERE synced = false AND outcome != 'unknown' ORDER BY played_at ASC"
        };
        let mut result = self.db.query(sql).await?;
        let matches: Vec<PersonalMatch> = result.take(0)?;
        if unknown_only {
            Ok(matches)
        } else {
            Ok(matches
                .into_iter()
                .filter(|row| row.upload_rejection().is_none())
                .collect())
        }
    }

    /// Remember that the server rejected these rows. `synced` stays false
    /// so a later edit or retry can upload them, and [`Self::get_unsynced`]
    /// skips them until then. `code` is the JSON `code` when the body sent
    /// one. The stored text is the game-list line from
    /// [`format_upload_rejection`]. A row whose `sync_rev` changed during
    /// the upload is left alone: the local edit is the payload that should
    /// be retried.
    pub async fn quarantine_upload(
        &self,
        claims: &[SyncClaim],
        code: Option<&str>,
        message: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let message = format_upload_rejection(code, message);
        let mut touched = std::collections::HashSet::new();
        for claim in claims {
            let mut response = self
                .db
                .query(
                    "UPDATE $id SET upload_reject = $msg \
                     WHERE (sync_rev ?? 0) = $rev AND synced = false RETURN AFTER",
                )
                .bind(("id", claim.id.clone()))
                .bind(("msg", message.clone()))
                .bind(("rev", claim.sync_rev))
                .await?;
            let updated: Vec<PersonalMatch> = response.take(0)?;
            if !updated.is_empty() && !claim.session_id.is_empty() {
                touched.insert(claim.session_id.clone());
            }
        }
        for session_id in touched {
            let message = message.clone();
            rewrite_match_log_session(
                &self.data_dir,
                &session_id,
                Some(&move |row| {
                    row.upload_reject = Some(message.clone());
                }),
            );
        }
        Ok(())
    }

    /// Put a quarantined session back on the upload queue.
    pub async fn clear_upload_reject(
        &self,
        session_id: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.db
            .query("UPDATE personal_match SET upload_reject = '' WHERE session_id = $sid")
            .bind(("sid", session_id.to_string()))
            .await?;
        rewrite_match_log_session(
            &self.data_dir,
            session_id,
            Some(&|row| {
                row.upload_reject = None;
            }),
        );
        Ok(())
    }

    /// Mark exactly these rows as synced — by identity, not queue position, so
    /// rows inserted while an upload was in flight are never marked by mistake.
    ///
    /// Each claim carries the `sync_rev` observed at read time. The update is
    /// a compare-and-swap: `synced = true` lands only when that revision is
    /// unchanged. A session is one server row (the newest snapshot). If any
    /// claimed snapshot in the session changed during the upload, every claim
    /// in that session is left `synced = false` — marking the unchanged
    /// siblings would let the next sync upload the changed older snapshot
    /// alone and last-write the newer stats off the server.
    pub async fn mark_synced(
        &self,
        claims: &[SyncClaim],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if claims.is_empty() {
            return Ok(());
        }
        let mut groups: std::collections::HashMap<String, Vec<usize>> =
            std::collections::HashMap::new();
        for (i, claim) in claims.iter().enumerate() {
            groups.entry(claim.group_key()).or_default().push(i);
        }
        for idxs in groups.values() {
            let mut marked: Vec<usize> = Vec::new();
            let mut all_ok = true;
            for &i in idxs {
                if self
                    .cas_set_synced(&claims[i].id, claims[i].sync_rev, true)
                    .await?
                {
                    marked.push(i);
                } else {
                    all_ok = false;
                }
            }
            if !all_ok {
                tracing::info!(
                    session_id = %claims[idxs[0]].session_id,
                    "sync left session unsynced: a snapshot changed during upload"
                );
                for i in marked {
                    self.cas_set_synced(&claims[i].id, claims[i].sync_rev, false)
                        .await?;
                }
            }
        }
        Ok(())
    }

    /// Set `synced` only when `sync_rev` is still `rev`. Returns whether this
    /// call applied the write. A missing `sync_rev` (legacy row) compares as 0.
    async fn cas_set_synced(
        &self,
        id: &surrealdb_types::RecordId,
        rev: u64,
        synced: bool,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        let mut response = self
            .db
            .query("UPDATE $id SET synced = $synced WHERE (sync_rev ?? 0) = $rev RETURN AFTER")
            .bind(("id", id.clone()))
            .bind(("synced", synced))
            .bind(("rev", rev))
            .await?;
        let updated: Vec<PersonalMatch> = response.take(0)?;
        Ok(updated.iter().any(|m| m.synced == synced))
    }

    pub async fn match_count(&self) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        let mut result = self
            .db
            .query("SELECT count() AS total FROM personal_match GROUP ALL")
            .await?;
        let row: Option<CountRow> = result.take(0)?;
        Ok(row.map(|r| r.total).unwrap_or(0))
    }

    pub async fn get_all_matches(
        &self,
    ) -> Result<Vec<PersonalMatch>, Box<dyn std::error::Error + Send + Sync>> {
        let mut result = self
            .db
            .query("SELECT * FROM personal_match ORDER BY played_at DESC")
            .await?;
        let matches: Vec<PersonalMatch> = result.take(0)?;
        Ok(matches)
    }

    /// Create the session row, idempotently: if a row with this `session_id`
    /// already exists (a previous capture created it but its snapshot insert
    /// failed, so the caller retries "creation"), it is overwritten rather
    /// than duplicated — there is no unique index protecting `session_id`.
    pub async fn create_session(
        &self,
        session: &MatchSession,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // A capture that was in flight when its game was deleted must not
        // bring the session row back.
        if self.deleted_here(&session.session_id) {
            tracing::info!(
                session_id = %session.session_id,
                "session row for a deleted game not created"
            );
            return Ok(());
        }
        self.db
            .query(
                "UPSERT match_session SET hero = $hero, map_name = $map, role = $role, \
                 started_at = $started, last_capture_at = $last, capture_count = $count, \
                 final_outcome = $outcome, session_id = $sid WHERE session_id = $sid",
            )
            .bind(("hero", session.hero.clone()))
            .bind(("map", session.map_name.clone()))
            .bind(("role", session.role.clone()))
            .bind(("started", session.started_at))
            .bind(("last", session.last_capture_at))
            .bind(("count", session.capture_count))
            .bind(("outcome", session.final_outcome.clone()))
            .bind(("sid", session.session_id.clone()))
            .await?;
        Ok(())
    }

    /// Append a capture to an existing session: bump the capture count and time,
    /// and refresh the final outcome (the active game owns the authoritative value).
    ///
    /// An `unknown` outcome never replaces a decided one. The capture's outcome
    /// was taken when the Tab was pressed; a result decided while it ran (an
    /// end-reel poll, a GUI edit) stays (MAIN-1).
    pub async fn append_capture(
        &self,
        session_id: &str,
        capture_time: SurrealDatetime,
        outcome: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // A carried board is written after the current row and carries an
        // older timestamp. Do not move last_capture_at backwards. Sessions
        // are listed by that stamp.
        let mut existing = self
            .db
            .query("SELECT last_capture_at FROM match_session WHERE session_id = $sid")
            .bind(("sid", session_id.to_string()))
            .await?;
        let rows: Vec<SessionStamp> = existing.take(0)?;
        let time = match rows.first() {
            Some(row) => {
                let have: chrono::DateTime<chrono::Utc> = row.last_capture_at.into();
                let incoming: chrono::DateTime<chrono::Utc> = capture_time.into();
                if have > incoming {
                    row.last_capture_at
                } else {
                    capture_time
                }
            }
            None => capture_time,
        };
        // Fixed fragments only; the outcome is a bind parameter either way.
        let query = if outcome_is_decided(outcome) {
            "UPDATE match_session SET last_capture_at = $time, capture_count += 1, final_outcome = $outcome WHERE session_id = $sid"
        } else {
            "UPDATE match_session SET last_capture_at = $time, capture_count += 1 WHERE session_id = $sid"
        };
        self.db
            .query(query)
            .bind(("time", time))
            .bind(("outcome", outcome.to_string()))
            .bind(("sid", session_id.to_string()))
            .await?;
        Ok(())
    }

    /// Delete a session and all its capture snapshots (manual cleanup of a
    /// junk/misdetected game). Records a tombstone so the next sync deletes
    /// the game server-side too, and drops the session from `matches.jsonl`
    /// so the GUI's log fallback stops showing it.
    ///
    /// The three writes run in one transaction and every statement is
    /// checked. Unchecked, a failed tombstone write still returned Ok after
    /// the local rows were gone: the game vanished from the app but nothing
    /// was left to delete it from the site.
    pub async fn delete_session(
        &self,
        session_id: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Marked before the delete, so a capture insert cannot land between
        // the delete and the mark. A failed delete (the query or any checked
        // statement) unmarks it, or every later row of that game would be
        // dropped for the rest of the process.
        let newly_marked = self
            .deleted_here
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(session_id.to_string());
        let deleted = match self
            .db
            .query(
                "BEGIN TRANSACTION; \
                 DELETE match_session WHERE session_id = $sid; \
                 DELETE personal_match WHERE session_id = $sid; \
                 CREATE deleted_session SET session_id = $sid, deleted_at = time::now(); \
                 COMMIT TRANSACTION;",
            )
            .bind(("sid", session_id.to_string()))
            .await
        {
            Ok(response) => response.check().map(|_| ()),
            Err(e) => Err(e),
        };
        if let Err(e) = deleted {
            if newly_marked {
                self.deleted_here
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(session_id);
            }
            return Err(e.into());
        }
        rewrite_match_log_session(&self.data_dir, session_id, None);
        Ok(())
    }

    /// Queue server deletes for sessions that stay local from now on (an
    /// untracked map or mode after a re-read or an edit). An earlier snapshot
    /// of such a game may already be on the site; the tombstone removes it.
    /// A session that already has a pending tombstone is not queued twice.
    /// A server delete of a session it never stored is a no-op.
    pub async fn queue_tombstones(
        &self,
        session_ids: &[String],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut sids: Vec<String> = session_ids
            .iter()
            .filter(|sid| !sid.is_empty())
            .cloned()
            .collect();
        sids.sort();
        sids.dedup();
        if sids.is_empty() {
            return Ok(());
        }
        let pending = self.get_pending_tombstones().await?;
        sids.retain(|sid| !pending.contains(sid));
        for sid in sids {
            self.db
                .query("CREATE deleted_session SET session_id = $sid, deleted_at = time::now()")
                .bind(("sid", sid))
                .await?
                .check()?;
        }
        Ok(())
    }

    /// Session ids deleted locally but not yet propagated to the server.
    pub async fn get_pending_tombstones(
        &self,
    ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
        let mut result = self
            .db
            .query("SELECT session_id FROM deleted_session")
            .await?;
        let rows: Vec<DeletedSession> = result.take(0)?;
        Ok(rows.into_iter().map(|r| r.session_id).collect())
    }

    /// Drop tombstones the server has acknowledged.
    pub async fn clear_tombstones(
        &self,
        session_ids: Vec<String>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if session_ids.is_empty() {
            return Ok(());
        }
        self.db
            .query("DELETE deleted_session WHERE session_id IN $sids")
            .bind(("sids", session_ids))
            .await?;
        Ok(())
    }

    /// Apply a queued store command (see [`StoreCommand`]).
    pub async fn apply_command(
        &self,
        cmd: &StoreCommand,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        match cmd {
            StoreCommand::SetOutcome {
                session_id,
                outcome,
            } => {
                // The raw outcome makes the row eligible for the next sync.
                // The overlay marks it edited so the upload carries edited=true.
                self.set_session_outcome(session_id, outcome).await?;
                self.edit_match(
                    session_id,
                    &MatchEdit {
                        outcome: Some(outcome.clone()),
                        ..MatchEdit::default()
                    },
                )
                .await
            }
            StoreCommand::DeleteSession { session_id } => self.delete_session(session_id).await,
            StoreCommand::EditMatch { session_id, edit } => self.edit_match(session_id, edit).await,
            StoreCommand::ResolveSegment {
                session_id,
                segment,
                action,
            } => {
                self.resolve_hero_segment(session_id, *segment, action)
                    .await
            }
            StoreCommand::RetryUpload { session_id } => self.clear_upload_reject(session_id).await,
            StoreCommand::RetryRejectedUploads => self.clear_rejected_uploads().await,
        }
    }

    /// Clear every quarantined upload and put those rows back on the queue.
    pub async fn clear_rejected_uploads(
        &self,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut result = self
            .db
            .query(
                "SELECT session_id FROM personal_match \
                 WHERE upload_reject != NONE AND upload_reject != ''",
            )
            .await?;
        let rows: Vec<DeletedSession> = result.take(0)?;
        let mut sessions = Vec::new();
        for row in rows {
            if !sessions.iter().any(|id| id == &row.session_id) {
                sessions.push(row.session_id);
            }
        }
        if sessions.is_empty() {
            return Ok(());
        }
        self.db
            .query("UPDATE personal_match SET upload_reject = '' WHERE session_id IN $sids")
            .bind(("sids", sessions.clone()))
            .await?;
        for session_id in &sessions {
            rewrite_match_log_session(
                &self.data_dir,
                session_id,
                Some(&|row| {
                    row.upload_reject = None;
                }),
            );
        }
        Ok(())
    }

    /// MANUAL repair helper: stamp a hero/role onto a session and ALL its
    /// snapshots. Destroys the per-snapshot raw reads by design — explicit
    /// human intent overrides the captured values, exactly like
    /// `set_session_map`. The automatic detection path must NOT use this (it
    /// used to, which recorded a 97%-Ana game as Rein when a late swap won
    /// last-write; it now derives a hero timeline instead — see
    /// [`Self::refresh_session_hero_timeline`]).
    pub async fn set_session_hero(
        &self,
        session_id: &str,
        hero: &str,
        role: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Write through to the capture rows like set_session_map/outcome do:
        // the GUI and the sync path read personal_match, not match_session, so
        // a session-only update is invisible to both (the 07-18 "Tracer" game).
        // The `hero != $hero` guard keeps re-sync churn out of the every-capture
        // refresh path.
        self.db
            .query(
                "UPDATE match_session SET hero = $hero, role = $role WHERE session_id = $sid; \
                 UPDATE personal_match SET hero = $hero, role = $role, synced = false, \
                 upload_reject = '', sync_rev = (sync_rev ?? 0) + 1 \
                 WHERE session_id = $sid AND (hero != $hero OR role != $role)",
            )
            .bind(("hero", hero.to_string()))
            .bind(("role", role.to_string()))
            .bind(("sid", session_id.to_string()))
            .await?;
        // Keep the append-only log in step too (HW-1a jsonl parity): the GUI's
        // last-resort fallback reads matches.jsonl, so a hero repair that never
        // reached it left the fallback showing the stale hero forever, exactly
        // like the outcome/map back-fills before rewrite_match_log_session.
        let (hero, role) = (hero.to_string(), role.to_string());
        rewrite_match_log_session(
            &self.data_dir,
            session_id,
            Some(&|m| {
                m.hero = hero.clone();
                m.role = role.clone();
                m.upload_reject = None;
            }),
        );
        Ok(())
    }

    /// AUTOMATIC hero path (HS-1). Re-derive the session's hero timeline from
    /// its RAW snapshots and stamp the result (`heroes_played`) onto every
    /// snapshot row — WITHOUT touching the per-snapshot `hero`/`role` reads,
    /// unlike [`Self::set_session_hero`]. The derived primary (majority over
    /// the segments) then drives `display_hero`/`display_role`, so a late hero
    /// swap is recorded as its own segment instead of a last-write that
    /// mislabels the whole game (the 97%-Ana-recorded-as-Rein bug).
    ///
    /// Runs after every capture, so it mirrors the churn-avoidance of the
    /// other back-fills: it writes a row only when its timeline actually
    /// changed, and re-queues it for sync (`synced = false`) only when the
    /// row's EFFECTIVE (uploaded) hero/role would change — a stable-hero game
    /// is a no-op after its first snapshot. Persists across the same layers as
    /// `set_session_*`: the SurrealKV `personal_match` rows and the
    /// `matches.jsonl` fallback. (`match_session` carries no timeline field,
    /// and the GUI's `live_snapshot.json` is re-exported by the daemon after
    /// the capture completes.)
    pub async fn refresh_session_hero_timeline(
        &self,
        session_id: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let snaps = self.get_session_snapshots(session_id).await?;
        if snaps.is_empty() {
            return Ok(());
        }
        // Resolutions are stamped uniformly across a session's rows; gather the
        // union so a partially-written row can't drop a decision.
        let resolutions = collect_segment_resolutions(&snaps);
        let segments = derive_hero_segments(&snaps, &resolutions);

        for m in &snaps {
            let Some(id) = m.id.clone() else { continue };
            if m.heroes_played == segments {
                continue; // already current — no write, no sync churn
            }
            // Re-queue for sync only when the value the upload sends
            // (`display_hero`/`display_role`) actually changes. Compare via the
            // real display methods so this stays lock-step with the sync path
            // (a `corrected_hero` pins the effective value regardless of the
            // timeline, so such rows never re-queue).
            let old_hero = m.display_hero().to_string();
            let old_role = m.display_role().to_string();
            let mut probe = m.clone();
            probe.heroes_played = segments.clone();
            let requeue = old_hero.as_str() != probe.display_hero()
                || old_role.as_str() != probe.display_role();

            if requeue {
                self.db
                    .query(
                        "UPDATE $id SET heroes_played = $hp, synced = false, \
                         upload_reject = '', sync_rev = (sync_rev ?? 0) + 1",
                    )
                    .bind(("id", id))
                    .bind(("hp", segments.clone()))
                    .await?;
            } else {
                self.db
                    .query("UPDATE $id SET heroes_played = $hp")
                    .bind(("id", id))
                    .bind(("hp", segments.clone()))
                    .await?;
            }
        }
        // jsonl parity: the GUI's last-resort fallback reads matches.jsonl, so
        // stamp the derived timeline there too (raw hero/role untouched).
        let segments_for_log = segments.clone();
        rewrite_match_log_session(
            &self.data_dir,
            session_id,
            Some(&move |m| {
                m.heroes_played = segments_for_log.clone();
            }),
        );
        Ok(())
    }

    /// MANUAL confirm/dismiss of one derived hero segment (HS-1). Records the
    /// decision in `segment_resolutions` (keyed by segment index; a repeat on
    /// the same index replaces it, and an unrecognized action clears it) on
    /// EVERY snapshot row — uniform, like `heroes_played` — then re-derives the
    /// timeline via [`Self::refresh_session_hero_timeline`] so the flags and
    /// the effective primary update (and any resulting hero change re-queues
    /// for sync). Handles the 1-snapshot flap (e.g. Lijiang "Wuyang"/"Mizuki"):
    /// confirm keeps it as a real swap, dismiss drops it from display+majority.
    pub async fn resolve_hero_segment(
        &self,
        session_id: &str,
        segment: u32,
        action: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let snaps = self.get_session_snapshots(session_id).await?;
        if snaps.is_empty() {
            return Ok(());
        }
        let mut resolutions = collect_segment_resolutions(&snaps);
        resolutions.retain(|r| r.segment != segment);
        if action == SEGMENT_CONFIRM || action == SEGMENT_DISMISS {
            resolutions.push(SegmentResolution {
                segment,
                action: action.to_string(),
            });
        }
        resolutions.sort_by_key(|r| r.segment);

        // Persist the resolution set on all rows. This field is never uploaded
        // (only its effect via the derived primary is), so it does NOT flip
        // `synced` here — the re-derive below re-queues exactly the rows whose
        // effective hero changed.
        for m in &snaps {
            let Some(id) = m.id.clone() else { continue };
            if m.segment_resolutions == resolutions {
                continue;
            }
            self.db
                .query("UPDATE $id SET segment_resolutions = $sr")
                .bind(("id", id))
                .bind(("sr", resolutions.clone()))
                .await?;
        }
        let res_for_log = resolutions.clone();
        rewrite_match_log_session(
            &self.data_dir,
            session_id,
            Some(&move |m| {
                m.segment_resolutions = res_for_log.clone();
            }),
        );

        // Re-derive with the updated resolutions and re-stamp heroes_played.
        self.refresh_session_hero_timeline(session_id).await
    }

    /// Set a session's map and stamp it onto ALL its snapshots, re-queuing
    /// them for sync. A game has exactly one map, so snapshots that disagree
    /// (missed or misread the label) are corrected, not preserved.
    pub async fn set_session_map(
        &self,
        session_id: &str,
        map: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mode = crate::parse::stored_game_mode(map);
        self.db
            .query(
                "UPDATE match_session SET map_name = $map WHERE session_id = $sid; \
                 UPDATE personal_match SET map_name = $map, game_mode = $mode, synced = false, \
                 upload_reject = '', sync_rev = (sync_rev ?? 0) + 1 \
                 WHERE session_id = $sid AND (map_name != $map OR game_mode != $mode)",
            )
            .bind(("map", map.to_string()))
            .bind(("mode", mode.clone()))
            .bind(("sid", session_id.to_string()))
            .await?;
        self.clear_session_suspects(session_id, "map_name").await?;
        let map = map.to_string();
        rewrite_match_log_session(
            &self.data_dir,
            session_id,
            Some(&|m| {
                m.map_name = map.clone();
                m.game_mode = mode.clone();
                m.upload_reject = None;
                crate::suspect::clear_edited_suspects(&mut m.suspect_fields, "map_name");
            }),
        );
        Ok(())
    }

    /// [`Self::set_session_map`] that leaves a member's own map choice alone.
    ///
    /// A row that carries a member's map choice
    /// ([`PersonalMatch::has_member_map_edit`]) keeps the `map_name` read
    /// under the choice and the `game_mode` stored with it. The session and
    /// every other row take `map`, as in [`Self::set_session_map`]: a map
    /// choice is stamped on the rows that exist when it is made, so the rows
    /// captured after it (the newest is the game's row) have none and must
    /// not keep an untrusted or empty map. Returns how many rows were left
    /// alone. With none, this is [`Self::set_session_map`].
    pub async fn set_session_map_keeping_member_choices(
        &self,
        session_id: &str,
        map: &str,
    ) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        let snaps = self.get_session_snapshots(session_id).await?;
        let kept = snaps.iter().filter(|m| m.has_member_map_edit()).count();
        if kept == 0 {
            self.set_session_map(session_id, map).await?;
            return Ok(0);
        }
        let mode = crate::parse::stored_game_mode(map);
        self.db
            .query("UPDATE match_session SET map_name = $map WHERE session_id = $sid")
            .bind(("map", map.to_string()))
            .bind(("sid", session_id.to_string()))
            .await?;
        for m in &snaps {
            let Some(id) = m.id.clone() else { continue };
            if m.has_member_map_edit() {
                continue;
            }
            // the two writes of `set_session_map`, for this row only
            if m.map_name != map || m.game_mode != mode {
                self.db
                    .query(
                        "UPDATE $id SET map_name = $map, game_mode = $mode, synced = false, \
                         upload_reject = '', sync_rev = (sync_rev ?? 0) + 1",
                    )
                    .bind(("id", id.clone()))
                    .bind(("map", map.to_string()))
                    .bind(("mode", mode.clone()))
                    .await?;
            }
            let mut suspects = m.suspect_fields.clone();
            crate::suspect::clear_edited_suspects(&mut suspects, "map_name");
            if suspects != m.suspect_fields {
                self.db
                    .query("UPDATE $id SET suspect_fields = $sf")
                    .bind(("id", id))
                    .bind(("sf", suspects))
                    .await?;
            }
        }
        let map = map.to_string();
        rewrite_match_log_session(
            &self.data_dir,
            session_id,
            Some(&|m| {
                if m.has_member_map_edit() {
                    return;
                }
                m.map_name = map.clone();
                m.game_mode = mode.clone();
                m.upload_reject = None;
                crate::suspect::clear_edited_suspects(&mut m.suspect_fields, "map_name");
            }),
        );
        Ok(kept)
    }

    /// Back-fill a detected outcome onto a session and every capture snapshot it
    /// contains, re-queuing those snapshots for sync so the corrected result
    /// (e.g. a VICTORY read off the accolade screen after the stats were already
    /// captured with `unknown`) propagates to the server.
    pub async fn set_session_outcome(
        &self,
        session_id: &str,
        outcome: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.db
            .query(
                "UPDATE match_session SET final_outcome = $outcome WHERE session_id = $sid; \
                 UPDATE personal_match SET outcome = $outcome, synced = false, \
                 upload_reject = '', sync_rev = (sync_rev ?? 0) + 1 WHERE session_id = $sid",
            )
            .bind(("outcome", outcome.to_string()))
            .bind(("sid", session_id.to_string()))
            .await?;
        self.clear_session_suspects(session_id, "outcome").await?;
        let outcome = outcome.to_string();
        rewrite_match_log_session(
            &self.data_dir,
            session_id,
            Some(&|m| {
                m.outcome = outcome.clone();
                m.upload_reject = None;
                crate::suspect::clear_edited_suspects(&mut m.suspect_fields, "outcome");
            }),
        );
        Ok(())
    }

    /// Drop one edited field's suspect names on every snapshot of a session.
    async fn clear_session_suspects(
        &self,
        session_id: &str,
        storage_field: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let snaps = self.get_session_snapshots(session_id).await?;
        for m in snaps {
            let Some(id) = m.id.clone() else { continue };
            let mut fields = m.suspect_fields.clone();
            crate::suspect::clear_edited_suspects(&mut fields, storage_field);
            if fields == m.suspect_fields {
                continue;
            }
            self.db
                .query("UPDATE $id SET suspect_fields = $sf")
                .bind(("id", id))
                .bind(("sf", fields))
                .await?;
        }
        Ok(())
    }

    /// Apply a manual stat correction to a game. Writes the fixed values into
    /// the `corrected_*` overlay on EVERY snapshot of the session (leaving the
    /// immutable OCR fields untouched), records which fields were edited, and
    /// re-queues the rows for sync. Editing all snapshots — not just the final
    /// one — means whichever snapshot the server-side per-session upsert lands
    /// last still carries the correction, and mirrors the write-through the
    /// outcome/map/hero back-fills already use. The `matches.jsonl` fallback is
    /// rewritten with the same overlay so the GUI's last-resort view agrees.
    pub async fn edit_match(
        &self,
        session_id: &str,
        edit: &MatchEdit,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let now = SurrealDatetime::from(chrono::Utc::now());
        let snaps = self.get_session_snapshots(session_id).await?;
        for mut m in snaps {
            let Some(id) = m.id.clone() else { continue };
            if !apply_match_edit(&mut m, edit, now) {
                continue; // empty edit — nothing to persist
            }
            self.db
                .query(
                    "UPDATE $id SET corrected_hero = $ch, corrected_role = $cr, \
                     corrected_map_name = $cmap, corrected_outcome = $co, \
                     corrected_elims = $ce, corrected_deaths = $cd, corrected_assists = $ca, \
                     corrected_damage = $cdmg, corrected_healing = $chl, \
                     corrected_mitigation = $cmit, game_mode = $mode, edited_fields = $ef, \
                     edited_at = $ea, suspect_fields = $sf, \
                     synced = false, upload_reject = '', sync_rev = (sync_rev ?? 0) + 1",
                )
                .bind(("id", id))
                .bind(("ch", m.corrected_hero.clone()))
                .bind(("cr", m.corrected_role.clone()))
                .bind(("cmap", m.corrected_map_name.clone()))
                .bind(("mode", m.game_mode.clone()))
                .bind(("co", m.corrected_outcome.clone()))
                .bind(("ce", m.corrected_elims))
                .bind(("cd", m.corrected_deaths))
                .bind(("ca", m.corrected_assists))
                .bind(("cdmg", m.corrected_damage))
                .bind(("chl", m.corrected_healing))
                .bind(("cmit", m.corrected_mitigation))
                .bind(("ef", m.edited_fields.clone()))
                .bind(("ea", m.edited_at))
                .bind(("sf", m.suspect_fields.clone()))
                .await?;
        }
        let edit = edit.clone();
        rewrite_match_log_session(
            &self.data_dir,
            session_id,
            Some(&move |m| {
                if apply_match_edit(m, &edit, now) {
                    m.upload_reject = None;
                }
            }),
        );
        Ok(())
    }

    /// Re-create a tombstone row (vacuum copies pending ones into the fresh
    /// store so un-acked server-side deletes still propagate).
    async fn insert_tombstone(
        &self,
        session_id: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.db
            .query("CREATE deleted_session SET session_id = $sid, deleted_at = time::now()")
            .bind(("sid", session_id.to_string()))
            .await?;
        Ok(())
    }

    pub async fn get_all_sessions(
        &self,
    ) -> Result<Vec<MatchSession>, Box<dyn std::error::Error + Send + Sync>> {
        let mut result = self
            .db
            .query("SELECT * FROM match_session ORDER BY last_capture_at DESC")
            .await?;
        let sessions: Vec<MatchSession> = result.take(0)?;
        Ok(sessions)
    }

    pub async fn get_session_snapshots(
        &self,
        session_id: &str,
    ) -> Result<Vec<PersonalMatch>, Box<dyn std::error::Error + Send + Sync>> {
        let mut result = self
            .db
            .query("SELECT * FROM personal_match WHERE session_id = $sid ORDER BY played_at ASC")
            .bind(("sid", session_id.to_string()))
            .await?;
        let matches: Vec<PersonalMatch> = result.take(0)?;
        Ok(matches)
    }

    /// Rewrite all live rows into a brand-new store and swap it in, leaving
    /// the old directory as `stats.surrealkv.pre-vacuum-<stamp>`.
    ///
    /// SurrealKV keeps every historical version of every record; the tracker
    /// only ever needs latest state, so weeks of captures leave the store
    /// ~99% dead versions (observed 87 MB for ~1 MB of live rows). SurrealDB's
    /// periodic embedded housekeeping range-scans then spend entire cores
    /// skipping those versions — two threads at 100% around the clock on an
    /// idle daemon (2026-07-15). The daemon must NOT be running (single-
    /// writer: `open` flocks the store directory; the caller also checks the
    /// pid file).
    ///
    /// Returns (matches, sessions, tombstones) copied.
    pub async fn vacuum(
        data_dir: &Path,
    ) -> Result<(usize, usize, usize), Box<dyn std::error::Error + Send + Sync>> {
        let live_path = data_dir.join("stats.surrealkv");
        let fresh_dir = data_dir.join("vacuum.tmp");
        if fresh_dir.exists() {
            std::fs::remove_dir_all(&fresh_dir)?;
        }

        let (matches, sessions, tombstones) = {
            let old = LocalStore::open(data_dir).await?;
            (
                old.get_all_matches().await?,
                old.get_all_sessions().await?,
                old.get_pending_tombstones().await?,
            )
        };

        let fresh = LocalStore::open(&fresh_dir).await?;
        for mut m in matches.iter().cloned() {
            // Record ids belong to the old store; fresh inserts mint new ones.
            m.id = None;
            fresh.insert_match(m).await?;
        }
        for s in &sessions {
            fresh.create_session(s).await?;
        }
        for t in &tombstones {
            fresh.insert_tombstone(t).await?;
        }
        drop(fresh);
        // Dropping the handle only SIGNALS engine shutdown — the memtable
        // flush runs detached (engine/local/native.rs router loop), and there
        // is no awaitable close in the SDK. Renaming the directory mid-flush
        // makes the flush ENOENT. Every commit is WAL-synced so nothing can
        // be lost either way, but give the flush time to land at the stable
        // path so the store swaps in fully compacted and error-free.
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        let stamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
        let backup = data_dir.join(format!("stats.surrealkv.pre-vacuum-{stamp}"));
        std::fs::rename(&live_path, &backup)?;
        std::fs::rename(fresh_dir.join("stats.surrealkv"), &live_path)?;
        let _ = std::fs::remove_dir_all(&fresh_dir);
        // Keep only the newest pre-vacuum backup (this one) so data_dir does
        // not accumulate multi-hundred-MB copies across repeated vacua.
        if let Ok(n) =
            maintain::prune_pre_vacuum_backups(data_dir, maintain::PRE_VACUUM_BACKUPS_KEEP)
            && n > 0
        {
            tracing::info!(
                pruned = n,
                keep = maintain::PRE_VACUUM_BACKUPS_KEEP,
                "pruned older pre-vacuum backups after vacuum"
            );
        }
        tracing::info!(
            matches = matches.len(),
            sessions = sessions.len(),
            tombstones = tombstones.len(),
            backup = %backup.display(),
            "store vacuumed"
        );
        Ok((matches.len(), sessions.len(), tombstones.len()))
    }

    pub async fn clear_all_data(&self) -> Result<(), Box<dyn std::error::Error>> {
        // Local reset only — deliberately does NOT tombstone: clearing a
        // machine must not wipe the account's server history.
        self.db.query("DELETE personal_match").await?;
        self.db.query("DELETE match_session").await?;
        self.db.query("DELETE deleted_session").await?;
        tracing::info!("cleared all match data from local store");
        Ok(())
    }

    /// Export all matches and sessions to `live_snapshot.json` (atomic via
    /// tmp+rename). SurrealKV is single-process, so while the daemon holds the
    /// store open the GUI cannot read it — this snapshot is the GUI's live
    /// data source, refreshed by the daemon after every mutation. Unlike
    /// `matches.jsonl` (append-only insert log), it reflects back-filled
    /// outcomes and sync-state changes.
    pub async fn export_snapshot(
        &self,
        data_dir: &Path,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let snapshot = Snapshot {
            matches: self.get_all_matches().await?,
            sessions: self.get_all_sessions().await?,
        };
        let json = serde_json::to_vec(&snapshot)?;
        let tmp = data_dir.join("live_snapshot.json.tmp");
        std::fs::write(&tmp, &json)?;
        std::fs::rename(&tmp, snapshot_path(data_dir))?;
        Ok(())
    }

    pub async fn last_capture_time(&self) -> Option<String> {
        let mut result = self
            .db
            .query("SELECT played_at FROM personal_match ORDER BY played_at DESC LIMIT 1")
            .await
            .ok()?;
        let row: Option<LastCaptureRow> = result.take(0).ok()?;
        row.map(|r| {
            let dt: chrono::DateTime<chrono::Utc> = r.played_at.into();
            dt.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
    }
}

#[derive(Deserialize, SurrealValue)]
struct CountRow {
    total: usize,
}

#[derive(Deserialize, SurrealValue)]
struct LastCaptureRow {
    played_at: SurrealDatetime,
}

/// The daemon's live export of the full store, for readers (the GUI) that
/// can't open SurrealKV while the daemon has it locked.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    /// All matches, newest first (mirrors `get_all_matches`).
    pub matches: Vec<PersonalMatch>,
    /// All sessions, most recently captured first (mirrors `get_all_sessions`).
    pub sessions: Vec<MatchSession>,
}

fn snapshot_path(data_dir: &Path) -> PathBuf {
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

/// Local record of a session deleted while (possibly) already synced — the
/// next sync sends these so the server row disappears too.
#[derive(Debug, Serialize, Deserialize, SurrealValue)]
struct DeletedSession {
    session_id: String,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct SessionStamp {
    last_capture_at: SurrealDatetime,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct SessionOutcome {
    final_outcome: String,
}

/// A stored outcome label that names a result (`victory`, `defeat`, `draw`).
fn outcome_is_decided(label: &str) -> bool {
    label
        .parse::<crate::model::MatchOutcome>()
        .is_ok_and(crate::model::MatchOutcome::is_decided)
}

/// Game-list line for a row the server refused. The desktop list shows this
/// text next to the rejected flag.
pub const UPLOAD_REJECTED_BY_SERVER: &str = "Upload rejected by server";

/// Drop a leading `matches[<n>]: ` from the server's batch error. The index
/// is the position inside one HTTP body, which changes every time the batch
/// is split, so it is not a stable description of the row.
fn strip_match_index_prefix(message: &str) -> &str {
    let Some(rest) = message.strip_prefix("matches[") else {
        return message;
    };
    let Some(end) = rest.find("]:") else {
        return message;
    };
    let index = &rest[..end];
    if index.is_empty() || !index.bytes().all(|byte| byte.is_ascii_digit()) {
        return message;
    }
    rest[end + 2..].trim_start()
}

/// Plain wording stored on a quarantined row and shown in the game list.
/// A batch index is dropped. A server `code` is kept in front of the message
/// when the body sent one and the message does not already contain it.
pub fn format_upload_rejection(code: Option<&str>, message: &str) -> String {
    let mut detail = strip_match_index_prefix(message.trim()).trim().to_string();
    if let Some(rest) = detail.strip_prefix(UPLOAD_REJECTED_BY_SERVER) {
        detail = rest.trim().trim_start_matches(':').trim().to_string();
    }
    if let Some(code) = code.map(str::trim).filter(|code| !code.is_empty())
        && !detail.contains(code)
    {
        detail = if detail.is_empty() {
            code.to_string()
        } else {
            format!("{code}: {detail}")
        };
    }
    let text = if detail.is_empty() {
        UPLOAD_REJECTED_BY_SERVER.to_string()
    } else {
        format!("{UPLOAD_REJECTED_BY_SERVER}: {detail}")
    };
    clip_chars(&text, 400)
}

fn clip_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let end = text
        .char_indices()
        .nth(max)
        .map(|(index, _)| index)
        .unwrap_or(text.len());
    format!("{}...", &text[..end])
}

/// Rewrite `matches.jsonl` rows of one session (atomic tmp+rename): apply
/// `update` to each, or drop them entirely when `update` is `None`. The log is
/// append-only at capture time, so without this back-fills and deletes never
/// reached the GUI's last-resort fallback — it showed `unknown` outcomes and
/// deleted games forever (A5). Rare (once per correction), so the O(file)
/// rewrite is fine; unparseable lines (schema drift) are preserved verbatim.
fn rewrite_match_log_session(
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

/// A store mutation requested by the GUI. SurrealKV is single-process, so
/// while the daemon holds the store the GUI can't write — it queues commands
/// as JSON files in `<data_dir>/commands/`, which the daemon applies within a
/// few seconds (and then refreshes the live snapshot). When no daemon runs,
/// the GUI applies commands directly.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum StoreCommand {
    /// Manually set a session's outcome ("victory"/"defeat"/"draw"/"unknown"),
    /// back-filled onto all its snapshots and re-queued for sync.
    SetOutcome { session_id: String, outcome: String },
    /// Remove a session and all its snapshots.
    DeleteSession { session_id: String },
    /// Apply a manual stat correction to a game (see [`MatchEdit`]). The
    /// corrected values overlay the immutable OCR reads and re-queue the game
    /// for sync.
    EditMatch { session_id: String, edit: MatchEdit },
    /// Confirm or dismiss a derived hero segment of a game (HS-1), keyed by
    /// segment index. `action` is [`SEGMENT_CONFIRM`] or [`SEGMENT_DISMISS`].
    /// Persists the decision and re-derives the hero timeline.
    ///
    // HS-1 follow-up: the daemon-side command path (queue -> apply ->
    // resolve_hero_segment) is wired and tested, but the tracker GUI does not
    // yet render the per-segment confirm/dismiss buttons that would emit this
    // command. Until then a resolution can only be driven by dropping a
    // `resolve_segment` command file into <data_dir>/commands/.
    ResolveSegment {
        session_id: String,
        segment: u32,
        action: String,
    },
    /// Clear a quarantined upload error and queue the session again.
    RetryUpload { session_id: String },
    /// Clear every quarantined upload and queue those sessions again.
    RetryRejectedUploads,
}

/// A manual correction to a game's stats. Only `Some(_)` fields are applied;
/// each corresponds to an OCR-captured field that stays untouched underneath.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MatchEdit {
    pub hero: Option<String>,
    pub role: Option<String>,
    pub map_name: Option<String>,
    pub outcome: Option<String>,
    pub elims: Option<u32>,
    pub deaths: Option<u32>,
    pub assists: Option<u32>,
    pub damage: Option<u32>,
    pub healing: Option<u32>,
    pub mitigation: Option<u32>,
}

impl MatchEdit {
    /// True when no field is set — an empty edit is a no-op.
    pub fn is_empty(&self) -> bool {
        *self == MatchEdit::default()
    }
}

fn mark_edited(fields: &mut Vec<String>, name: &str) {
    if !fields.iter().any(|f| f == name) {
        fields.push(name.to_string());
    }
}

/// The member's own hero pick on a session's rows: the newest row that
/// carries a `corrected_hero`. [`LocalStore::edit_match`] writes the same
/// pick onto every row the session had at that time.
pub fn member_hero_edit(rows: &[PersonalMatch]) -> Option<&PersonalMatch> {
    rows.iter().rev().find(|m| m.corrected_hero.is_some())
}

/// Copy a member's hero pick from `edited` (a row of the same session) onto
/// a capture row written after the edit, so the pick is never overridden by
/// that row's read: `corrected_hero`, `corrected_role` when the edit set one,
/// the `hero` / `role` edit marks, `edited_at`, and the cleared `hero`
/// suspect. The row's own read (`hero`, `role`) stays as read. Returns
/// whether `edited` carried a pick.
pub fn carry_member_hero_edit(edited: &PersonalMatch, row: &mut PersonalMatch) -> bool {
    let Some(hero) = edited.corrected_hero.clone() else {
        return false;
    };
    row.corrected_hero = Some(hero);
    mark_edited(&mut row.edited_fields, "hero");
    crate::suspect::clear_edited_suspects(&mut row.suspect_fields, "hero");
    if let Some(role) = edited.corrected_role.clone() {
        row.corrected_role = Some(role);
        mark_edited(&mut row.edited_fields, "role");
    }
    if row.edited_at.is_none() {
        row.edited_at = edited.edited_at;
    }
    true
}

/// Overlay a [`MatchEdit`] onto a row: for every `Some(_)` field, set the
/// matching `corrected_*` value (OCR fields untouched) and record the field
/// name. Re-editing accumulates the union of edited fields and overwrites
/// earlier corrections for the same field. Returns whether anything changed;
/// stamps `edited_at` only then.
fn apply_match_edit(m: &mut PersonalMatch, edit: &MatchEdit, now: SurrealDatetime) -> bool {
    let mut changed = false;
    if let Some(v) = &edit.hero {
        m.corrected_hero = Some(v.clone());
        mark_edited(&mut m.edited_fields, "hero");
        crate::suspect::clear_edited_suspects(&mut m.suspect_fields, "hero");
        changed = true;
        let guessed = crate::parse::guess_role_public(v);
        if edit.role.is_none()
            && !crate::parse::hero_is_unknown_label(v)
            && guessed != m.display_role()
        {
            m.corrected_role = Some(guessed);
            mark_edited(&mut m.edited_fields, "role");
        }
    }
    if let Some(v) = &edit.role {
        m.corrected_role = Some(v.clone());
        mark_edited(&mut m.edited_fields, "role");
        changed = true;
    }
    if let Some(v) = &edit.map_name {
        m.corrected_map_name = Some(v.clone());
        mark_edited(&mut m.edited_fields, "map_name");
        crate::suspect::clear_edited_suspects(&mut m.suspect_fields, "map_name");
        changed = true;
        let derived = crate::parse::stored_game_mode(v);
        if !derived.is_empty() {
            m.game_mode = derived;
        }
    }
    if let Some(v) = &edit.outcome {
        m.corrected_outcome = Some(v.clone());
        mark_edited(&mut m.edited_fields, "outcome");
        crate::suspect::clear_edited_suspects(&mut m.suspect_fields, "outcome");
        changed = true;
    }
    if let Some(v) = edit.elims {
        m.corrected_elims = Some(v);
        mark_edited(&mut m.edited_fields, "elims");
        crate::suspect::clear_edited_suspects(&mut m.suspect_fields, "elims");
        changed = true;
    }
    if let Some(v) = edit.deaths {
        m.corrected_deaths = Some(v);
        mark_edited(&mut m.edited_fields, "deaths");
        crate::suspect::clear_edited_suspects(&mut m.suspect_fields, "deaths");
        changed = true;
    }
    if let Some(v) = edit.assists {
        m.corrected_assists = Some(v);
        mark_edited(&mut m.edited_fields, "assists");
        crate::suspect::clear_edited_suspects(&mut m.suspect_fields, "assists");
        changed = true;
    }
    if let Some(v) = edit.damage {
        m.corrected_damage = Some(v);
        mark_edited(&mut m.edited_fields, "damage");
        crate::suspect::clear_edited_suspects(&mut m.suspect_fields, "damage");
        changed = true;
    }
    if let Some(v) = edit.healing {
        m.corrected_healing = Some(v);
        mark_edited(&mut m.edited_fields, "healing");
        crate::suspect::clear_edited_suspects(&mut m.suspect_fields, "healing");
        changed = true;
    }
    if let Some(v) = edit.mitigation {
        m.corrected_mitigation = Some(v);
        mark_edited(&mut m.edited_fields, "mitigation");
        crate::suspect::clear_edited_suspects(&mut m.suspect_fields, "mitigation");
        changed = true;
    }
    if changed {
        m.edited_at = Some(now);
    }
    changed
}

fn commands_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("commands")
}

/// Queue a command for the daemon to apply (atomic via tmp+rename).
pub fn queue_command(
    data_dir: &Path,
    cmd: &StoreCommand,
) -> Result<(), Box<dyn std::error::Error>> {
    let dir = commands_dir(data_dir);
    std::fs::create_dir_all(&dir)?;
    crate::fs_mode::tighten_private_dir(data_dir);
    crate::fs_mode::tighten_private_dir(&dir);
    let name = format!(
        "cmd_{}_{}.json",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        std::process::id()
    );
    let tmp = dir.join(format!("{name}.tmp"));
    let dest = dir.join(&name);
    std::fs::write(&tmp, serde_json::to_vec(cmd)?)?;
    crate::fs_mode::tighten_private_file(&tmp);
    std::fs::rename(&tmp, &dest)?;
    crate::fs_mode::tighten_private_file(&dest);
    Ok(())
}

/// First phase of the command queue: list queued commands with their backing
/// files, oldest first, WITHOUT removing them. The caller applies each command
/// and then calls [`remove_command_file`] — two-phase, so a crash between read
/// and apply retries the command instead of silently losing a manual edit.
/// Unparseable files are removed immediately (never retried forever).
pub fn read_commands(data_dir: &Path) -> Vec<(PathBuf, StoreCommand)> {
    let Ok(entries) = std::fs::read_dir(commands_dir(data_dir)) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect();
    files.sort();
    let mut cmds = Vec::new();
    for f in files {
        match std::fs::read(&f)
            .map_err(|e| e.to_string())
            .and_then(|c| serde_json::from_slice::<StoreCommand>(&c).map_err(|e| e.to_string()))
        {
            Ok(cmd) => cmds.push((f, cmd)),
            Err(e) => {
                tracing::warn!(file = %f.display(), error = %e, "discarding bad command file");
                let _ = std::fs::remove_file(&f);
            }
        }
    }
    cmds
}

/// Second phase of the command queue: drop a command file once its command has
/// been successfully applied.
pub fn remove_command_file(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// Gather the union of `segment_resolutions` across a session's snapshots,
/// deduped by segment index (last row wins) and sorted. The daemon stamps
/// resolutions uniformly onto every row, so in practice all rows agree — this
/// merge just tolerates a row that missed a write.
pub fn collect_segment_resolutions(snapshots: &[PersonalMatch]) -> Vec<SegmentResolution> {
    let mut by_index: std::collections::BTreeMap<u32, String> = std::collections::BTreeMap::new();
    for m in snapshots {
        for r in &m.segment_resolutions {
            by_index.insert(r.segment, r.action.clone());
        }
    }
    by_index
        .into_iter()
        .map(|(segment, action)| SegmentResolution { segment, action })
        .collect()
}

/// Aggregate a session's snapshots into consecutive same-hero runs (sorted by
/// capture time here, so any input order works). "Unknown"/empty hero reads
/// neither open nor split a run. A run of ≥2 snapshots is confirmed; a
/// 1-snapshot run is recorded but unconfirmed. Manual `resolutions` (by
/// segment index) are then applied: confirm forces `confirmed`, dismiss marks
/// the segment a misread.
pub fn derive_hero_segments(
    snapshots: &[PersonalMatch],
    resolutions: &[SegmentResolution],
) -> Vec<HeroSegment> {
    let mut ordered: Vec<&PersonalMatch> = snapshots.iter().collect();
    ordered.sort_by_key(|m| {
        let dt: chrono::DateTime<chrono::Utc> = m.played_at.into();
        dt
    });
    let mut segments: Vec<HeroSegment> = Vec::new();
    for m in ordered {
        if m.hero.is_empty() || m.hero == "Unknown" {
            continue;
        }
        match segments.last_mut() {
            Some(seg) if seg.hero == m.hero => {
                seg.last_seen = m.played_at;
                seg.snapshots += 1;
            }
            _ => segments.push(HeroSegment {
                hero: m.hero.clone(),
                role: m.role.clone(),
                first_seen: m.played_at,
                last_seen: m.played_at,
                snapshots: 1,
                confirmed: false,
                resolution: None,
            }),
        }
    }
    for seg in &mut segments {
        seg.confirmed = seg.snapshots >= 2;
    }
    for r in resolutions {
        if let Some(seg) = segments.get_mut(r.segment as usize) {
            seg.resolution = Some(r.action.clone());
            match r.action.as_str() {
                SEGMENT_CONFIRM => seg.confirmed = true,
                SEGMENT_DISMISS => seg.confirmed = false,
                _ => {}
            }
        }
    }
    segments
}

/// The majority hero across a session's snapshots. Individual captures can
/// mislabel — the career panel shows the SPECTATED hero while the player is
/// dead, and portrait matching can misfire — but across 20+ captures the
/// player's real hero dominates. "Unknown" rows don't vote; ties break
/// alphabetically for determinism.
pub fn majority_hero(snapshots: &[PersonalMatch]) -> Option<String> {
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for m in snapshots {
        if m.hero != "Unknown" && !m.hero.is_empty() {
            *counts.entry(m.hero.as_str()).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(a.0)))
        .map(|(hero, _)| hero.to_string())
}

/// Majority map across a session's snapshots (empty reads don't vote).
/// Repair helper for sessions whose map was missed at creation or polluted
/// by misreads.
pub fn majority_map(snapshots: &[PersonalMatch]) -> Option<String> {
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for m in snapshots {
        if !m.map_name.is_empty() {
            *counts.entry(m.map_name.as_str()).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(a.0)))
        .map(|(map, _)| map.to_string())
}

/// `true` when this row has no name recognizer to send. An empty string is
/// the same as absent: the upload omits the field.
fn name_recognizer_absent(id: &Option<String>) -> bool {
    id.as_deref().is_none_or(str::is_empty)
}

/// Collapse capture snapshots (multiple Tab presses during one match) to one
/// row per game. Snapshots of the same game share a `session_id`, and the
/// newest snapshot carries the final scoreboard — so with newest-first input
/// (the order of `get_all_matches`, the live snapshot, and `read_match_log`)
/// the first row seen per session wins. Rows without a session_id (legacy
/// data) pass through individually. Games whose rows predate the hero
/// timeline (empty `heroes_played`) get their segments derived on the fly
/// from the session snapshots present in the input, so legacy data shows the
/// same swap history and majority primary as fresh captures.
pub fn latest_per_game(matches: Vec<PersonalMatch>) -> Vec<PersonalMatch> {
    let mut seen = std::collections::HashSet::new();
    let mut legacy: std::collections::HashMap<String, Vec<PersonalMatch>> =
        std::collections::HashMap::new();
    let mut games: Vec<PersonalMatch> = Vec::new();
    for m in matches {
        if m.session_id.is_empty() {
            games.push(m);
            continue;
        }
        if seen.insert(m.session_id.clone()) {
            if m.heroes_played.is_empty() {
                legacy.insert(m.session_id.clone(), vec![m.clone()]);
            }
            games.push(m);
        } else {
            // The newest snapshot is the uploaded row. If it did not read a
            // name, keep the recognizer from an earlier snapshot of the same
            // match so a resend does not clear it on the server.
            if let Some(game) = games.iter_mut().find(|g| g.session_id == m.session_id)
                && name_recognizer_absent(&game.name_recognizer)
                && !name_recognizer_absent(&m.name_recognizer)
            {
                game.name_recognizer = m.name_recognizer.clone();
            }
            if let Some(snaps) = legacy.get_mut(&m.session_id) {
                snaps.push(m);
            }
        }
    }
    for g in &mut games {
        if let Some(snaps) = legacy.remove(&g.session_id) {
            g.heroes_played = derive_hero_segments(&snaps, &g.segment_resolutions);
        }
    }
    games
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
mod tests_commands_files;
