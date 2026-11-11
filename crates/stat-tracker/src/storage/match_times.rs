//! A game's real start and end ([`LocalStore::attach_match_times`]) and the
//! carry that keeps them on every snapshot of the match. Same shape as the
//! bans and picks status (`storage::bans_picks`), without its log rewrite.

use crate::model::MatchTimes;

use super::LocalStore;

impl LocalStore {
    /// Store `times` (`PersonalMatch::match_times`) on every snapshot of
    /// `session_id`, as the daemon knows them now. `false`, nothing written,
    /// when the game is unknown or every snapshot already has these times.
    ///
    /// The open game's times replace what is stored: a result the board
    /// shows was wrong clears the end it set ([`MatchTimes::ended_at`]
    /// back to `None`). Local only: the upload never sends them, so this
    /// leaves the upload queue as it is.
    ///
    /// `matches.jsonl` is not rewritten: nothing reads it, and this runs on
    /// the event loop after every stored capture. The store and
    /// `active_game.json` keep the times across a restart.
    pub async fn attach_match_times(
        &self,
        session_id: &str,
        times: &MatchTimes,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        super::require_session_id(session_id)?;
        let _rev = self.rev_guard();
        let snaps = self.get_session_snapshots(session_id).await?;
        if snaps.is_empty() || snaps.iter().all(|m| m.match_times.as_ref() == Some(times)) {
            return Ok(false);
        }
        self.db
            .query("UPDATE personal_match SET match_times = $mt WHERE session_id = $sid")
            .bind(("mt", times.clone()))
            .bind(("sid", session_id.to_string()))
            .await?
            .check()?;
        Ok(true)
    }
}
