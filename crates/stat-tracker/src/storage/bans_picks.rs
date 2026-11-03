//! The bans and picks result on a game: [`LocalStore::attach_bans_picks`]
//! and the carry that keeps it on every snapshot of the match.

use chrono::{DateTime, Utc};

use super::{LocalStore, rewrite_match_log_session};

impl LocalStore {
    /// When the game's first snapshot was taken; `None` for an unknown game.
    pub async fn session_started(
        &self,
        session_id: &str,
    ) -> Result<Option<DateTime<Utc>>, Box<dyn std::error::Error + Send + Sync>> {
        let snaps = self.get_session_snapshots(session_id).await?;
        Ok(snaps.first().map(|m| m.played_at.into()))
    }

    /// Store `json` (the cv-v1 result, `PersonalMatch::bans_picks`) on every
    /// snapshot of `session_id`. `false`, nothing written, when the game is
    /// unknown or already has a result.
    pub async fn attach_bans_picks(
        &self,
        session_id: &str,
        json: &str,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        let _rev = self.rev_guard();
        let snaps = self.get_session_snapshots(session_id).await?;
        if snaps.is_empty() || snaps.iter().any(|m| m.bans_picks.is_some()) {
            return Ok(false);
        }
        self.db
            .query("UPDATE personal_match SET bans_picks = $bp WHERE session_id = $sid")
            .bind(("bp", json.to_string()))
            .bind(("sid", session_id.to_string()))
            .await?
            .check()?;
        let json = json.to_string();
        rewrite_match_log_session(
            &self.data_dir,
            session_id,
            Some(&|m| m.bans_picks = Some(json.clone())),
        );
        Ok(true)
    }

    /// The result already stored for this match, for a later snapshot that
    /// has none of its own.
    pub(super) async fn session_bans_picks(
        &self,
        session_id: &str,
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let snaps = self.get_session_snapshots(session_id).await?;
        Ok(snaps.into_iter().rev().find_map(|row| row.bans_picks))
    }
}
