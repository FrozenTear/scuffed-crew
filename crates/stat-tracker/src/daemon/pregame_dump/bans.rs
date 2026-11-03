//! The saver's tie to the bans and picks reader: the read when a game is
//! first recorded.

use super::*;
use stat_tracker::model::BansPicksStatus;

/// Put `status` on the game. A failure is logged: the game is recorded
/// either way.
async fn record_status(
    store: &crate::storage::LocalStore,
    session_id: &str,
    status: BansPicksStatus,
) {
    if let Err(e) = store.attach_bans_picks_status(session_id, &status).await {
        tracing::warn!(target: "scuffed_stat_tracker", error = %e, "bans and picks: status not stored");
    }
}

/// Why a game has no bans and picks when none of its pre-game screens were
/// held for the reader.
const NO_SCREENS_WHY: &str = "no pre-game screens were seen for this game";

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
            record_status(store, session_id, BansPicksStatus::ReaderOff).await;
            return;
        };
        // A later capture of a game already read has nothing more to say.
        if feed.was_read_for(session_id) {
            return;
        }
        if !feed.has_frames() {
            feed.publish_not_read(NO_SCREENS_WHY);
            record_status(store, session_id, BansPicksStatus::NoFrames).await;
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
            feed.publish_not_read(NO_SCREENS_WHY);
            record_status(store, session_id, BansPicksStatus::NoFrames).await;
            return;
        }
        let outcome = feed.read_now().await;
        feed.mark_read_for(session_id);
        record_status(store, session_id, outcome.status.clone()).await;
        let Some(json) = outcome.stored else {
            tracing::info!(target: "scuffed_stat_tracker", notice = %outcome.notice, "bans and picks not read, the game is recorded without them");
            return;
        };
        if let Err(e) = store.attach_bans_picks(session_id, &json).await {
            tracing::warn!(target: "scuffed_stat_tracker", error = %e, "bans and picks: result not stored");
        }
    }
}
