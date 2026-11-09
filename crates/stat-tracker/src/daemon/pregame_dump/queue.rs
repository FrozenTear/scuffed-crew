//! The saver's tie to the queue title reader ([`super::super::queue_title`]):
//! the feed's way in, the pending next mode hint, and the read when a game
//! is first recorded.

use super::*;
use crate::daemon::queue_title::{HintOutcome, QueueTitleFeed, apply_read};
use stat_tracker::shadow::queue_title::Field;

impl PregameDump {
    /// Give the saver the queue title feed. `None` (the reader off) changes
    /// nothing. With a feed and no saver (the pre-game saves and the bans
    /// and picks reader both off) a saver that writes nothing is started
    /// for it.
    pub(crate) fn add_queue_title(
        dump: &mut Option<Self>,
        feed: Option<QueueTitleFeed>,
        data_dir: &std::path::Path,
        poll: Duration,
    ) {
        let Some(feed) = feed else {
            return;
        };
        if dump.is_none() {
            *dump = Self::start(false, None, data_dir, poll);
        }
        if let Some(d) = dump.as_mut() {
            d.queue = Some(feed);
        }
    }

    /// Set the pending next mode hint (the mode banner word read after a
    /// game), for the next game's queue title read to confirm or drop. A
    /// blank hint clears it. Nothing happens with the reader off. The rank
    /// reader's wiring calls this; until it lands only tests do.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn set_next_mode_hint(&mut self, hint: Option<&str>) {
        if let Some(feed) = self.queue.as_mut() {
            feed.pending_hint = hint
                .map(str::trim)
                .filter(|h| !h.is_empty())
                .map(str::to_string);
        }
    }

    /// A game was just recorded as `session_id`: read its queue title once
    /// and put it on the game ([`apply_read`]). With the reader off this
    /// does nothing. Frames that are not this game's, a store error or a
    /// reader that fails give the game the warning, never a failed capture.
    pub(crate) async fn attach_queue_title(
        &mut self,
        store: &crate::storage::LocalStore,
        session_id: &str,
    ) {
        let Some(feed) = self.queue.as_mut() else {
            return;
        };
        if feed.was_read_for(session_id) {
            return;
        }
        feed.mark_read_for(session_id);
        let read = match store.session_started(session_id).await {
            Ok(Some(started)) if feed.fits(started) => feed.read_now().await,
            Ok(_) => {
                tracing::info!(target: "scuffed_stat_tracker", session = session_id, "queue title: no pre-game frames held for this game");
                Field::none()
            }
            Err(e) => {
                tracing::warn!(target: "scuffed_stat_tracker", error = %e, "queue title: game start not read");
                Field::none()
            }
        };
        let existing = match store.session_queue_title(session_id).await {
            Ok(existing) => existing,
            Err(e) => {
                tracing::warn!(target: "scuffed_stat_tracker", error = %e, "queue title: not stored");
                return;
            }
        };
        let (title, hint) = apply_read(existing, &read, &mut feed.pending_hint);
        match &hint {
            HintOutcome::NoHint => {}
            HintOutcome::Confirmed(h) => {
                tracing::info!(target: "scuffed_stat_tracker", hint = %h, "queue title: next mode hint confirmed")
            }
            HintOutcome::Dropped(h) => {
                tracing::info!(target: "scuffed_stat_tracker", hint = %h, "queue title: next mode hint dropped, the read disagrees")
            }
            HintOutcome::Pending(h) => {
                tracing::info!(target: "scuffed_stat_tracker", hint = %h, "queue title: next mode hint still pending, the read was weak")
            }
        }
        if let Some(warning) = title.warning() {
            tracing::warn!(target: "scuffed_stat_tracker", session = session_id, confidence = read.confidence, "{warning}");
        } else {
            tracing::info!(target: "scuffed_stat_tracker", session = session_id, queue = title.value.as_deref().unwrap_or(""), confidence = title.confidence, "queue title read");
        }
        if let Err(e) = store.attach_queue_title(session_id, &title).await {
            tracing::warn!(target: "scuffed_stat_tracker", error = %e, "queue title: not stored");
        }
    }
}
