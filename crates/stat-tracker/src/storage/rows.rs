//! The capture write path and the reads: `insert_match`, sessions,
//! captures, counts, and the SurrealQL projection rows. The live snapshot
//! export is in `export.rs`.

use serde::Deserialize;
use surrealdb_types::{Datetime as SurrealDatetime, SurrealValue};

use super::{
    LocalStore, MatchSession, PersonalMatch, name_recognizer_absent, outcome_is_decided,
    rewrite_match_log_session,
};

impl LocalStore {
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
                // The caller logged this row as unknown. Keep the
                // matches.jsonl record in step with the store.
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
    pub(super) async fn drop_if_deleted_meanwhile(
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
        let mut matches: Vec<PersonalMatch> = result.take(0)?;
        self.hydrate_rows(&mut matches).await?;
        Ok(matches)
    }

    /// [`Self::get_all_matches`] as stored, without the `hero_timeline`
    /// records: vacuum copies rows verbatim.
    pub(super) async fn get_all_matches_raw(
        &self,
    ) -> Result<Vec<PersonalMatch>, Box<dyn std::error::Error + Send + Sync>> {
        let mut result = self
            .db
            .query("SELECT * FROM personal_match ORDER BY played_at DESC")
            .await?;
        Ok(result.take(0)?)
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
        let mut matches: Vec<PersonalMatch> = result.take(0)?;
        self.hydrate_rows(&mut matches).await?;
        Ok(matches)
    }

    pub async fn clear_all_data(&self) -> Result<(), Box<dyn std::error::Error>> {
        // Local reset only — deliberately does NOT tombstone: clearing a
        // machine must not wipe the account's server history.
        self.db.query("DELETE personal_match").await?;
        self.db.query("DELETE match_session").await?;
        self.db.query("DELETE deleted_session").await?;
        // The marker stays: an empty store already has the new layout, so
        // the next start takes no backup and moves nothing.
        self.db.query("DELETE hero_timeline").await?;
        tracing::info!("cleared all match data from local store");
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

#[derive(Debug, Deserialize, SurrealValue)]
struct SessionStamp {
    last_capture_at: SurrealDatetime,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct SessionOutcome {
    final_outcome: String,
}
