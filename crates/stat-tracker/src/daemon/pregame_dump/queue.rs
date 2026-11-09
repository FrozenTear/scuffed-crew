//! The saver's tie to the queue title reader ([`super::super::queue_title`]):
//! the pending next mode hint, and the read when a game is first recorded.
//! The feed comes in through [`PregameDump::with_readers`].

use super::*;
use crate::daemon::queue_title::{HintOutcome, PendingHint, ReadGame, apply_read};
use stat_tracker::shadow::queue_title::Field;

impl PregameDump {
    /// Set the pending next mode hint: the mode banner word read on the
    /// rank screen of `source_session`, which names the queue of the game
    /// after it. The next game's read settles it
    /// ([`crate::daemon::queue_title::settle_hint`]); `source_session`
    /// itself never does. A blank hint clears it, a new one replaces the
    /// one pending (a log line says so). Nothing happens with the reader
    /// off. The rank reader's wiring calls this; until it lands only tests
    /// do.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn set_next_mode_hint(&mut self, hint: Option<&str>, source_session: &str) {
        let Some(feed) = self.queue.as_mut() else {
            return;
        };
        let hint = hint
            .map(str::trim)
            .filter(|h| !h.is_empty())
            .map(|text| PendingHint {
                text: text.to_string(),
                source: source_session.to_string(),
                at: Utc::now(),
            });
        if let Some(old) = feed.pending_hint.take() {
            tracing::info!(target: "scuffed_stat_tracker", hint = %old.text, source = %old.source, "queue title: next mode hint replaced before a game settled it");
        }
        feed.pending_hint = hint;
    }

    /// A game was just recorded as `session_id`: read its queue title once
    /// and put it on the game ([`apply_read`]). With the reader off this
    /// does nothing. A game that already has a queue title is not read
    /// again. Only the open game (`open_session`, when there is one) is
    /// read: a Tab stored under another session leaves the held frames for
    /// the open game. A store error returns with the game not marked read,
    /// so its next capture tries again; frames that are not this game's or
    /// a reader that fails give the game the warning. Never a failed
    /// capture.
    pub(crate) async fn attach_queue_title(
        &mut self,
        store: &crate::storage::LocalStore,
        session_id: &str,
        open_session: Option<&str>,
    ) {
        let Some(feed) = self.queue.as_mut() else {
            return;
        };
        if feed.was_read_for(session_id) {
            return;
        }
        if let Some(open) = open_session.filter(|open| *open != session_id) {
            tracing::info!(target: "scuffed_stat_tracker", session = session_id, open, "queue title: not read, the capture is not the open game's");
            return;
        }
        let existing = match store.session_queue_title(session_id).await {
            Ok(existing) => existing,
            Err(e) => {
                tracing::warn!(target: "scuffed_stat_tracker", error = %e, "queue title: stored title not read, the game is read on its next capture");
                return;
            }
        };
        if existing.is_some() {
            feed.mark_read_for(session_id);
            tracing::info!(target: "scuffed_stat_tracker", session = session_id, "queue title: the game has one already, not read again");
            return;
        }
        let started = match store.session_started(session_id).await {
            Ok(Some(started)) => started,
            Ok(None) => {
                tracing::info!(target: "scuffed_stat_tracker", session = session_id, "queue title: no game start stored, not read");
                return;
            }
            Err(e) => {
                tracing::warn!(target: "scuffed_stat_tracker", error = %e, "queue title: game start not read, the game is read on its next capture");
                return;
            }
        };
        feed.mark_read_for(session_id);
        let read = if feed.fits(started) {
            feed.read_now().await
        } else {
            match feed.fed_at() {
                None => {
                    tracing::info!(target: "scuffed_stat_tracker", session = session_id, "queue title: no pre-game frames held for this game");
                }
                Some(fed_at) => {
                    tracing::info!(target: "scuffed_stat_tracker", session = session_id, %fed_at, %started, "queue title: the frames held are not this game's (game start does not fit), not read");
                }
            }
            Field::none()
        };
        let game = ReadGame {
            session: session_id,
            started,
            now: Utc::now(),
        };
        let (title, hint) = apply_read(&read, &mut feed.pending_hint, game);
        match &hint {
            HintOutcome::NoHint => {}
            HintOutcome::Confirmed(h) => {
                tracing::info!(target: "scuffed_stat_tracker", hint = %h, "queue title: next mode hint confirmed")
            }
            HintOutcome::Dropped(h) => {
                tracing::info!(target: "scuffed_stat_tracker", hint = %h, "queue title: next mode hint dropped, the read disagrees")
            }
            HintOutcome::Expired(h) => {
                tracing::info!(target: "scuffed_stat_tracker", hint = %h, "queue title: next mode hint expired, the read was weak or the hint too old")
            }
            HintOutcome::Pending(h) => {
                tracing::info!(target: "scuffed_stat_tracker", hint = %h, "queue title: next mode hint still pending, not this game's")
            }
        }
        if let Some(warning) = title.warning() {
            tracing::info!(target: "scuffed_stat_tracker", session = session_id, "{warning}");
        } else {
            tracing::info!(target: "scuffed_stat_tracker", session = session_id, queue = title.value.as_deref().unwrap_or(""), confidence = title.confidence, "queue title read");
        }
        if let Err(e) = store.attach_queue_title(session_id, &title).await {
            // The frames are used up; the game's next capture at least
            // stores the warning, so it does not look like the reader off.
            feed.forget_read_for(session_id);
            tracing::warn!(target: "scuffed_stat_tracker", error = %e, "queue title: not stored, the game is read again on its next capture");
        }
    }
}
