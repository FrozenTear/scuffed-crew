//! The saver's tie to the bans and picks reader: the read when a game is
//! first recorded.

use super::*;

impl PregameDump {
    /// A game was just recorded as `session_id` (its first or a later
    /// capture): when the frames held for the reader are this game's, read
    /// them, put the result on the game and publish the status sentence.
    /// Nothing here can fail the capture: an error is logged and the game
    /// keeps an empty result.
    pub(crate) async fn attach_bans_picks(
        &mut self,
        store: &crate::storage::LocalStore,
        session_id: &str,
    ) {
        let Some(feed) = self.bans.as_mut() else {
            return;
        };
        if !feed.has_frames() {
            return;
        }
        let started = match store.session_started(session_id).await {
            Ok(Some(at)) => at,
            Ok(None) => return,
            Err(e) => {
                tracing::warn!(target: "scuffed_stat_tracker", error = %e, "bans and picks: game start not read, nothing attached");
                return;
            }
        };
        if !feed.fits(started) {
            return;
        }
        let outcome = feed.read_now().await;
        let Some(json) = outcome.stored else {
            tracing::info!(target: "scuffed_stat_tracker", notice = %outcome.notice, "bans and picks not read, the game is recorded without them");
            return;
        };
        if let Err(e) = store.attach_bans_picks(session_id, &json).await {
            tracing::warn!(target: "scuffed_stat_tracker", error = %e, "bans and picks: result not stored");
        }
    }
}
