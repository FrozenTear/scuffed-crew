use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use surrealdb::Surreal;
use surrealdb::engine::local::SurrealKv;
use surrealdb_types::Datetime as SurrealDatetime;
use surrealdb_types::SurrealValue;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, SurrealValue)]
pub struct PersonalMatch {
    /// Local store record id — set on rows read back from SurrealKV, `None`
    /// on freshly parsed captures. Excluded from the JSON snapshot/log
    /// (readers identify games by `session_id`); used to mark exactly the
    /// fetched rows as synced.
    #[serde(skip)]
    #[surreal(default)]
    pub id: Option<surrealdb_types::RecordId>,
    pub hero: String,
    pub map_name: String,
    pub game_mode: String,
    pub role: String,
    pub outcome: String,
    pub elims: u32,
    pub deaths: u32,
    pub assists: u32,
    pub damage: u32,
    pub healing: u32,
    pub mitigation: u32,
    pub played_at: SurrealDatetime,
    #[serde(default)]
    pub synced: bool,
    #[serde(default)]
    pub session_id: String,

    // --- Manual edit overlay (transparency) ---------------------------------
    // The OCR-captured values above are IMMUTABLE. A manual correction records
    // the fixed value in the matching `corrected_*` field and appends the field
    // name to `edited_fields`; the effective (displayed) value is `corrected_*`
    // if present, else the OCR read. This keeps "OCR read X → corrected Y"
    // recoverable for the badge/tooltip and never destroys the original read.
    // All fields are additive Options / empty-by-default, so rows written before
    // this feature deserialize unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[surreal(default)]
    pub corrected_hero: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[surreal(default)]
    pub corrected_role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[surreal(default)]
    pub corrected_map_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[surreal(default)]
    pub corrected_outcome: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[surreal(default)]
    pub corrected_elims: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[surreal(default)]
    pub corrected_deaths: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[surreal(default)]
    pub corrected_assists: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[surreal(default)]
    pub corrected_damage: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[surreal(default)]
    pub corrected_healing: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[surreal(default)]
    pub corrected_mitigation: Option<u32>,
    /// Names of fields carrying a manual correction (e.g. `["elims", "hero"]`).
    /// Non-empty ⇒ the row is edited (drives the badge).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[surreal(default)]
    pub edited_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[surreal(default)]
    pub edited_at: Option<SurrealDatetime>,

    // --- Hero timeline (HS-1) -----------------------------------------------
    /// Consecutive-run hero segments derived from the session's RAW snapshots
    /// (see [`derive_hero_segments`]). The daemon stamps this after each
    /// capture; legacy rows (empty) are derived on the fly in
    /// [`latest_per_game`]. Per-snapshot `hero`/`role` stay raw — this field
    /// carries the swap history, and the primary hero/role for the game is the
    /// snapshot-count majority over these segments ([`primary_hero_role`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[surreal(default)]
    pub heroes_played: Vec<HeroSegment>,
    /// Manual confirm/dismiss decisions for hero segments, by segment index.
    /// Source of truth for the flags inside `heroes_played`, which are
    /// recomputed from these on every re-derivation.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[surreal(default)]
    pub segment_resolutions: Vec<SegmentResolution>,
}

/// Count an unconfirmed hero segment toward the majority (it was a real swap).
pub const SEGMENT_CONFIRM: &str = "confirm";
/// Mark a hero segment as a misread: excluded from display and from the
/// majority vote. The raw snapshots stay untouched.
pub const SEGMENT_DISMISS: &str = "dismiss";

/// One consecutive run of the same hero across a session's snapshots, in
/// capture-time order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, SurrealValue)]
pub struct HeroSegment {
    pub hero: String,
    pub role: String,
    pub first_seen: SurrealDatetime,
    pub last_seen: SurrealDatetime,
    /// Number of captures in the run.
    pub snapshots: u32,
    /// ≥2 snapshots, or manually confirmed. A 1-snapshot segment is recorded
    /// but unconfirmed — a lone minority read is indistinguishable from a
    /// misread (the 07-18 Lijiang "Wuyang" flap), so it needs a human call.
    pub confirmed: bool,
    /// Manual resolution ([`SEGMENT_CONFIRM`] / [`SEGMENT_DISMISS`]), applied
    /// from the match's `segment_resolutions` during derivation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[surreal(default)]
    pub resolution: Option<String>,
}

impl HeroSegment {
    /// Manually dismissed as a misread — excluded from display and majority.
    pub fn is_dismissed(&self) -> bool {
        self.resolution.as_deref() == Some(SEGMENT_DISMISS)
    }
}

/// A manual decision about one derived hero segment, keyed by index into the
/// derived segment list (stable: snapshots only ever append, so existing
/// segments never change position).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, SurrealValue)]
pub struct SegmentResolution {
    pub segment: u32,
    /// [`SEGMENT_CONFIRM`] or [`SEGMENT_DISMISS`].
    pub action: String,
}

impl PersonalMatch {
    /// True when at least one field carries a manual correction.
    pub fn is_edited(&self) -> bool {
        !self.edited_fields.is_empty()
    }

    /// True when `field` (storage name, e.g. `"map_name"`) was manually edited.
    pub fn field_edited(&self, field: &str) -> bool {
        self.edited_fields.iter().any(|f| f == field)
    }

    /// Effective hero: a manual correction wins, else the majority-derived
    /// primary from the hero timeline, else the raw OCR read.
    pub fn display_hero(&self) -> &str {
        if let Some(h) = self.corrected_hero.as_deref() {
            return h;
        }
        primary_hero_role(&self.heroes_played)
            .map(|(h, _)| h)
            .unwrap_or(&self.hero)
    }
    /// Effective role, resolved like [`Self::display_hero`].
    pub fn display_role(&self) -> &str {
        if let Some(r) = self.corrected_role.as_deref() {
            return r;
        }
        primary_hero_role(&self.heroes_played)
            .map(|(_, r)| r)
            .unwrap_or(&self.role)
    }
    pub fn display_map_name(&self) -> &str {
        self.corrected_map_name.as_deref().unwrap_or(&self.map_name)
    }
    pub fn display_outcome(&self) -> &str {
        self.corrected_outcome.as_deref().unwrap_or(&self.outcome)
    }
    pub fn display_elims(&self) -> u32 {
        self.corrected_elims.unwrap_or(self.elims)
    }
    pub fn display_deaths(&self) -> u32 {
        self.corrected_deaths.unwrap_or(self.deaths)
    }
    pub fn display_assists(&self) -> u32 {
        self.corrected_assists.unwrap_or(self.assists)
    }
    pub fn display_damage(&self) -> u32 {
        self.corrected_damage.unwrap_or(self.damage)
    }
    pub fn display_healing(&self) -> u32 {
        self.corrected_healing.unwrap_or(self.healing)
    }
    pub fn display_mitigation(&self) -> u32 {
        self.corrected_mitigation.unwrap_or(self.mitigation)
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
}

impl LocalStore {
    pub async fn open(data_dir: &Path) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let db_path = data_dir.join("stats.surrealkv");
        std::fs::create_dir_all(&db_path)?;

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
        ",
        )
        .await?;

        tracing::info!(path = %db_path.display(), "local store opened");
        Ok(Self {
            db,
            data_dir: data_dir.to_path_buf(),
        })
    }

    pub async fn insert_match(
        &self,
        match_data: PersonalMatch,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let _: Option<PersonalMatch> = self.db.create("personal_match").content(match_data).await?;
        tracing::debug!("match inserted into local store");
        Ok(())
    }

    /// Rows awaiting upload. Holds back `unknown` outcomes: the server only
    /// stores decided games, and one unstorable row at the head of the queue
    /// used to fail the whole batch forever. An outcome back-fill flips the
    /// row to a decided outcome and `synced = false`, releasing it here.
    pub async fn get_unsynced(
        &self,
    ) -> Result<Vec<PersonalMatch>, Box<dyn std::error::Error + Send + Sync>> {
        let mut result = self
            .db
            .query("SELECT * FROM personal_match WHERE synced = false AND outcome != 'unknown' ORDER BY played_at ASC")
            .await?;
        let matches: Vec<PersonalMatch> = result.take(0)?;
        Ok(matches)
    }

    /// Mark exactly these rows as synced — by identity, not queue position, so
    /// rows inserted while an upload was in flight are never marked by mistake.
    pub async fn mark_synced(
        &self,
        ids: Vec<surrealdb_types::RecordId>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if ids.is_empty() {
            return Ok(());
        }
        self.db
            .query("UPDATE $ids SET synced = true")
            .bind(("ids", ids))
            .await?;
        Ok(())
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
    pub async fn append_capture(
        &self,
        session_id: &str,
        capture_time: SurrealDatetime,
        outcome: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.db
            .query("UPDATE match_session SET last_capture_at = $time, capture_count += 1, final_outcome = $outcome WHERE session_id = $sid")
            .bind(("time", capture_time))
            .bind(("outcome", outcome.to_string()))
            .bind(("sid", session_id.to_string()))
            .await?;
        Ok(())
    }

    /// Delete a session and all its capture snapshots (manual cleanup of a
    /// junk/misdetected game). Records a tombstone so the next sync deletes
    /// the game server-side too, and drops the session from `matches.jsonl`
    /// so the GUI's log fallback stops showing it.
    pub async fn delete_session(
        &self,
        session_id: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.db
            .query("DELETE match_session WHERE session_id = $sid; DELETE personal_match WHERE session_id = $sid; CREATE deleted_session SET session_id = $sid, deleted_at = time::now()")
            .bind(("sid", session_id.to_string()))
            .await?;
        rewrite_match_log_session(&self.data_dir, session_id, None);
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
            } => self.set_session_outcome(session_id, outcome).await,
            StoreCommand::DeleteSession { session_id } => self.delete_session(session_id).await,
            StoreCommand::EditMatch { session_id, edit } => self.edit_match(session_id, edit).await,
        }
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
                 UPDATE personal_match SET hero = $hero, role = $role, synced = false \
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
            }),
        );
        Ok(())
    }

    /// Set a session's map and stamp it onto ALL its snapshots, re-queuing
    /// them for sync. A game has exactly one map, so snapshots that disagree
    /// (missed or misread the label) are corrected, not preserved.
    pub async fn set_session_map(
        &self,
        session_id: &str,
        map: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.db
            .query("UPDATE match_session SET map_name = $map WHERE session_id = $sid; UPDATE personal_match SET map_name = $map, synced = false WHERE session_id = $sid AND map_name != $map")
            .bind(("map", map.to_string()))
            .bind(("sid", session_id.to_string()))
            .await?;
        let map = map.to_string();
        rewrite_match_log_session(
            &self.data_dir,
            session_id,
            Some(&|m| {
                m.map_name = map.clone();
            }),
        );
        Ok(())
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
            .query("UPDATE match_session SET final_outcome = $outcome WHERE session_id = $sid; UPDATE personal_match SET outcome = $outcome, synced = false WHERE session_id = $sid")
            .bind(("outcome", outcome.to_string()))
            .bind(("sid", session_id.to_string()))
            .await?;
        let outcome = outcome.to_string();
        rewrite_match_log_session(
            &self.data_dir,
            session_id,
            Some(&|m| {
                m.outcome = outcome.clone();
            }),
        );
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
                     corrected_mitigation = $cmit, edited_fields = $ef, edited_at = $ea, \
                     synced = false",
                )
                .bind(("id", id))
                .bind(("ch", m.corrected_hero.clone()))
                .bind(("cr", m.corrected_role.clone()))
                .bind(("cmap", m.corrected_map_name.clone()))
                .bind(("co", m.corrected_outcome.clone()))
                .bind(("ce", m.corrected_elims))
                .bind(("cd", m.corrected_deaths))
                .bind(("ca", m.corrected_assists))
                .bind(("cdmg", m.corrected_damage))
                .bind(("chl", m.corrected_healing))
                .bind(("cmit", m.corrected_mitigation))
                .bind(("ef", m.edited_fields.clone()))
                .bind(("ea", m.edited_at))
                .await?;
        }
        let edit = edit.clone();
        rewrite_match_log_session(
            &self.data_dir,
            session_id,
            Some(&move |m| {
                apply_match_edit(m, &edit, now);
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
    /// process lock; the caller checks the pid file).
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
        changed = true;
    }
    if let Some(v) = &edit.role {
        m.corrected_role = Some(v.clone());
        mark_edited(&mut m.edited_fields, "role");
        changed = true;
    }
    if let Some(v) = &edit.map_name {
        m.corrected_map_name = Some(v.clone());
        mark_edited(&mut m.edited_fields, "map_name");
        changed = true;
    }
    if let Some(v) = &edit.outcome {
        m.corrected_outcome = Some(v.clone());
        mark_edited(&mut m.edited_fields, "outcome");
        changed = true;
    }
    if let Some(v) = edit.elims {
        m.corrected_elims = Some(v);
        mark_edited(&mut m.edited_fields, "elims");
        changed = true;
    }
    if let Some(v) = edit.deaths {
        m.corrected_deaths = Some(v);
        mark_edited(&mut m.edited_fields, "deaths");
        changed = true;
    }
    if let Some(v) = edit.assists {
        m.corrected_assists = Some(v);
        mark_edited(&mut m.edited_fields, "assists");
        changed = true;
    }
    if let Some(v) = edit.damage {
        m.corrected_damage = Some(v);
        mark_edited(&mut m.edited_fields, "damage");
        changed = true;
    }
    if let Some(v) = edit.healing {
        m.corrected_healing = Some(v);
        mark_edited(&mut m.edited_fields, "healing");
        changed = true;
    }
    if let Some(v) = edit.mitigation {
        m.corrected_mitigation = Some(v);
        mark_edited(&mut m.edited_fields, "mitigation");
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
    let name = format!(
        "cmd_{}_{}.json",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        std::process::id()
    );
    let tmp = dir.join(format!("{name}.tmp"));
    std::fs::write(&tmp, serde_json::to_vec(cmd)?)?;
    std::fs::rename(&tmp, dir.join(name))?;
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

/// The primary hero/role for a game: snapshot-count majority across its hero
/// segments. Dismissed segments never vote; unconfirmed segments vote only
/// when every non-dismissed segment is unconfirmed (a lone flap must not
/// outvote 25 solid captures, but an all-unconfirmed game still needs a
/// label). Ties break to the hero seen latest, then alphabetically for
/// determinism. `None` when nothing can vote (no segments, or all dismissed)
/// — callers fall back to the raw read.
pub fn primary_hero_role(segments: &[HeroSegment]) -> Option<(&str, &str)> {
    let live: Vec<&HeroSegment> = segments.iter().filter(|s| !s.is_dismissed()).collect();
    let voters: Vec<&HeroSegment> = if live.iter().any(|s| s.confirmed) {
        live.iter().copied().filter(|s| s.confirmed).collect()
    } else {
        live
    };
    struct Vote<'a> {
        count: u32,
        latest: chrono::DateTime<chrono::Utc>,
        role: &'a str,
    }
    let mut votes: std::collections::HashMap<&str, Vote> = std::collections::HashMap::new();
    for s in voters {
        let seen: chrono::DateTime<chrono::Utc> = s.last_seen.into();
        let v = votes.entry(s.hero.as_str()).or_insert(Vote {
            count: 0,
            latest: seen,
            role: &s.role,
        });
        v.count += s.snapshots;
        if seen >= v.latest {
            v.latest = seen;
            v.role = &s.role;
        }
    }
    votes
        .into_iter()
        .max_by(|a, b| {
            a.1.count
                .cmp(&b.1.count)
                .then(a.1.latest.cmp(&b.1.latest))
                .then(b.0.cmp(a.0))
        })
        .map(|(hero, v)| (hero, v.role))
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
        } else if let Some(snaps) = legacy.get_mut(&m.session_id) {
            snaps.push(m);
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
mod tests {
    use super::*;
    use chrono::Utc;

    fn snap(session_id: &str, elims: u32) -> PersonalMatch {
        PersonalMatch {
            id: None,
            hero: "Test".into(),
            map_name: String::new(),
            game_mode: String::new(),
            role: "Tank".into(),
            outcome: "victory".into(),
            elims,
            deaths: 0,
            assists: 0,
            damage: 0,
            healing: 0,
            mitigation: 0,
            played_at: SurrealDatetime::from(Utc::now()),
            synced: false,
            session_id: session_id.into(),
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
        }
    }

    #[test]
    fn latest_per_game_keeps_final_snapshot_per_session() {
        // Newest-first input: the first snapshot per session is the final one.
        let rows = vec![snap("b", 30), snap("a", 20), snap("a", 10), snap("a", 5)];
        let games = latest_per_game(rows);
        assert_eq!(games.len(), 2);
        assert_eq!(games[0].session_id, "b");
        assert_eq!(games[1].elims, 20);
    }

    #[test]
    fn command_queue_roundtrip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = StoreCommand::SetOutcome {
            session_id: "s1".into(),
            outcome: "defeat".into(),
        };
        let b = StoreCommand::DeleteSession {
            session_id: "s2".into(),
        };
        queue_command(dir.path(), &a).unwrap();
        queue_command(dir.path(), &b).unwrap();
        let cmds = read_commands(dir.path());
        assert_eq!(cmds.len(), 2);
        assert!(
            matches!(&cmds[0].1, StoreCommand::SetOutcome { session_id, outcome }
            if session_id == "s1" && outcome == "defeat")
        );
        assert!(
            matches!(&cmds[1].1, StoreCommand::DeleteSession { session_id }
            if session_id == "s2")
        );
        // Two-phase: still queued until the caller removes the files...
        assert_eq!(read_commands(dir.path()).len(), 2);
        // ...and gone once each applied command's file is removed.
        for (path, _) in &cmds {
            remove_command_file(path);
        }
        assert!(read_commands(dir.path()).is_empty());
    }

    #[tokio::test]
    async fn delete_session_tombstones_until_acknowledged() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = LocalStore::open(dir.path()).await.expect("open store");
        store.insert_match(snap("s1", 1)).await.unwrap();
        append_match_log(dir.path(), &snap("s1", 1));

        store.delete_session("s1").await.unwrap();
        // Local rows gone, jsonl fallback cleaned, tombstone pending for sync.
        assert_eq!(store.match_count().await.unwrap(), 0);
        assert!(read_match_log(dir.path()).is_empty());
        assert_eq!(
            store.get_pending_tombstones().await.unwrap(),
            vec!["s1".to_string()]
        );

        // Server acknowledged — tombstone cleared, nothing left to send.
        store.clear_tombstones(vec!["s1".into()]).await.unwrap();
        assert!(store.get_pending_tombstones().await.unwrap().is_empty());
        // Clearing all data never leaks tombstones either.
        store.clear_all_data().await.unwrap();
        assert!(store.get_pending_tombstones().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn set_session_hero_writes_through_to_capture_rows() {
        // The 07-18 field case: majority-correct captures with wrong-hero
        // stragglers from post-match row browsing. The repair must reach the
        // personal_match rows (what the GUI and sync read), not just the
        // session row, and must flip only mismatched rows to synced=false.
        let dir = tempfile::tempdir().expect("tempdir");
        let store = LocalStore::open(dir.path()).await.expect("open store");
        for i in 0..3 {
            let mut m = snap("s1", i);
            if i < 2 {
                m.hero = "Mizuki".into();
                m.role = "Support".into();
            } else {
                m.hero = "Tracer".into();
                m.role = "Damage".into();
            }
            store.insert_match(m).await.unwrap();
        }
        // Everything already synced, as after a normal upload cycle.
        let ids: Vec<_> = store
            .get_all_matches()
            .await
            .unwrap()
            .into_iter()
            .filter_map(|m| m.id)
            .collect();
        store.mark_synced(ids).await.unwrap();

        store
            .set_session_hero("s1", "Mizuki", "Support")
            .await
            .unwrap();

        let rows = store.get_all_matches().await.unwrap();
        assert!(
            rows.iter()
                .all(|m| m.hero == "Mizuki" && m.role == "Support")
        );
        // Only the repaired (previously Tracer) row is queued for re-sync.
        assert_eq!(rows.iter().filter(|m| !m.synced).count(), 1);
    }

    #[tokio::test]
    async fn edit_match_overlays_corrections_and_preserves_ocr() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = LocalStore::open(dir.path()).await.expect("open store");
        // Two snapshots of one game, both already synced (post-upload state).
        for i in 0..2 {
            let mut m = snap("s1", 5 + i); // ocr elims 5, 6
            append_match_log(dir.path(), &m);
            m.map_name = "Ilios".into();
            store.insert_match(m).await.unwrap();
        }
        let ids: Vec<_> = store
            .get_all_matches()
            .await
            .unwrap()
            .into_iter()
            .filter_map(|m| m.id)
            .collect();
        store.mark_synced(ids).await.unwrap();

        let edit = MatchEdit {
            elims: Some(30),
            hero: Some("Ana".into()),
            map_name: Some("Nepal".into()),
            ..Default::default()
        };
        store.edit_match("s1", &edit).await.unwrap();

        let rows = store.get_all_matches().await.unwrap();
        assert_eq!(rows.len(), 2);
        for m in &rows {
            // Corrected values overlay; OCR reads stay immutable underneath.
            assert_eq!(m.corrected_elims, Some(30));
            assert_eq!(m.display_elims(), 30);
            assert!(m.elims == 5 || m.elims == 6, "OCR elims preserved");
            assert_eq!(m.display_hero(), "Ana");
            assert_eq!(m.hero, "Test", "OCR hero preserved");
            assert_eq!(m.display_map_name(), "Nepal");
            assert!(m.is_edited());
            assert!(
                m.field_edited("elims") && m.field_edited("hero") && m.field_edited("map_name")
            );
            assert!(m.edited_at.is_some());
            // Re-queued for sync.
            assert!(!m.synced);
        }

        // jsonl fallback carries the same overlay (rewrite parity).
        let logged = read_match_log(dir.path());
        assert_eq!(logged.len(), 2);
        assert!(logged.iter().all(|m| m.display_elims() == 30
            && m.display_hero() == "Ana"
            && m.elims != 30
            && m.is_edited()));
    }

    #[test]
    fn old_rows_without_edit_fields_deserialize_unchanged() {
        // A pre-feature jsonl line has none of the corrected_*/edited_* keys.
        let line = r#"{"hero":"Genji","map_name":"Hanamura","game_mode":"assault","role":"Damage","outcome":"victory","elims":20,"deaths":4,"assists":6,"damage":8000,"healing":0,"mitigation":0,"played_at":"2026-01-01T00:00:00Z","synced":true,"session_id":"legacy1"}"#;
        let m: PersonalMatch = serde_json::from_str(line).expect("legacy row parses");
        assert!(!m.is_edited());
        assert!(m.edited_fields.is_empty());
        assert_eq!(m.edited_at, None);
        // Display falls back to the OCR value when no correction exists.
        assert_eq!(m.display_elims(), 20);
        assert_eq!(m.display_hero(), "Genji");
        assert_eq!(m.corrected_elims, None);
    }

    #[tokio::test]
    async fn set_session_hero_rewrites_match_log() {
        // HW-1a: the hero write-through must also reach matches.jsonl, exactly
        // like the outcome/map back-fills. Without the rewrite the fallback log
        // keeps the stale hero forever — this test fails.
        let dir = tempfile::tempdir().expect("tempdir");
        let store = LocalStore::open(dir.path()).await.expect("open store");
        for i in 0..3 {
            let mut m = snap("s1", i);
            m.hero = if i < 2 {
                "Mizuki".into()
            } else {
                "Tracer".into()
            };
            m.role = if i < 2 {
                "Support".into()
            } else {
                "Damage".into()
            };
            append_match_log(dir.path(), &m);
            store.insert_match(m).await.unwrap();
        }

        store
            .set_session_hero("s1", "Mizuki", "Support")
            .await
            .unwrap();

        let logged = read_match_log(dir.path());
        assert_eq!(logged.len(), 3);
        assert!(
            logged
                .iter()
                .all(|m| m.hero == "Mizuki" && m.role == "Support"),
            "hero repair must reach the jsonl fallback (HW-1a)"
        );
    }

    #[test]
    fn match_log_rewrite_backfills_and_deletes() {
        let dir = tempfile::tempdir().expect("tempdir");
        append_match_log(dir.path(), &snap("keep", 1));
        append_match_log(dir.path(), &snap("fix", 1));
        append_match_log(dir.path(), &snap("fix", 2));
        append_match_log(dir.path(), &snap("gone", 1));

        rewrite_match_log_session(dir.path(), "fix", Some(&|m| m.outcome = "defeat".into()));
        rewrite_match_log_session(dir.path(), "gone", None);

        let rows = read_match_log(dir.path());
        assert_eq!(rows.len(), 3);
        assert!(!rows.iter().any(|m| m.session_id == "gone"));
        assert!(
            rows.iter()
                .filter(|m| m.session_id == "fix")
                .all(|m| m.outcome == "defeat")
        );
        assert!(
            rows.iter()
                .filter(|m| m.session_id == "keep")
                .all(|m| m.outcome == "victory")
        );
    }

    #[test]
    fn majority_map_ignores_empty_and_picks_dominant() {
        let mut rows: Vec<PersonalMatch> = (0..5).map(|_| snap("a", 1)).collect();
        rows[0].map_name = "King's Row".into();
        for r in rows.iter_mut().skip(2) {
            r.map_name = "Circuit Royal".into();
        }
        // rows[1] stays empty — doesn't vote
        assert_eq!(majority_map(&rows).as_deref(), Some("Circuit Royal"));
        assert_eq!(majority_map(&[snap("a", 1)]), None);
    }

    #[test]
    fn majority_hero_ignores_unknown_and_picks_dominant() {
        let mut rows: Vec<PersonalMatch> = (0..5).map(|_| snap("a", 1)).collect();
        rows[0].hero = "illari".into();
        rows[1].hero = "Unknown".into();
        for r in rows.iter_mut().skip(2) {
            r.hero = "Wrecking Ball".into();
        }
        assert_eq!(majority_hero(&rows).as_deref(), Some("Wrecking Ball"));
        assert_eq!(majority_hero(&[]), None);
    }

    #[test]
    fn latest_per_game_passes_legacy_rows_through() {
        let rows = vec![snap("", 1), snap("", 2), snap("a", 3), snap("a", 4)];
        assert_eq!(latest_per_game(rows).len(), 3);
    }
}
