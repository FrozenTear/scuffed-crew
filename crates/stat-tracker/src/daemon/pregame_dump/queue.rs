//! The saver's tie to the queue title reader ([`super::super::queue_title`]):
//! the pending next mode hint, and the read when a game is first recorded.
//! The feed comes in through [`PregameDump::with_readers`].

use super::*;
use crate::daemon::queue_title::{HintOutcome, PendingHint, QueueTitleFeed, ReadGame, apply_read};
use chrono::{DateTime, Utc};
use stat_tracker::model::QueueTitle;
use stat_tracker::shadow::queue_title::Field;

type StoreResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// What the queue title wiring needs from the store
/// ([`crate::storage::LocalStore`]); a trait so a test can make a write
/// fail.
pub(crate) trait TitleStore {
    async fn session_queue_title(&self, session_id: &str) -> StoreResult<Option<QueueTitle>>;
    async fn session_started(&self, session_id: &str) -> StoreResult<Option<DateTime<Utc>>>;
    async fn attach_queue_title(&self, session_id: &str, title: &QueueTitle) -> StoreResult<bool>;
}

impl TitleStore for crate::storage::LocalStore {
    async fn session_queue_title(&self, session_id: &str) -> StoreResult<Option<QueueTitle>> {
        crate::storage::LocalStore::session_queue_title(self, session_id).await
    }

    async fn session_started(&self, session_id: &str) -> StoreResult<Option<DateTime<Utc>>> {
        crate::storage::LocalStore::session_started(self, session_id).await
    }

    async fn attach_queue_title(&self, session_id: &str, title: &QueueTitle) -> StoreResult<bool> {
        crate::storage::LocalStore::attach_queue_title(self, session_id, title).await
    }
}

impl PregameDump {
    /// Set the pending next mode hint: the mode banner word read on the
    /// rank screen of `source_session`, which names the queue of the game
    /// after it. `shown_at` is when that rank screen showed: the time of
    /// the frame it was read on, not of this call; the age and fit checks
    /// use it. The call must come before the next game's read: the next
    /// game is read once, usually on its first Tab (a store error can put
    /// it off to a later one), and that read settles the hint
    /// ([`crate::daemon::queue_title::settle_hint`]). A later call is not
    /// the next game's: it goes to the game after, where a read that agrees
    /// confirms it, one that disagrees drops it, and a weak read or none
    /// expires it. `source_session` itself never settles it. A blank hint
    /// clears it, a new one replaces the one pending (a log line says so).
    /// Nothing happens with the reader off. The rank reader's wiring calls
    /// this; until it lands only tests do.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn set_next_mode_hint(
        &mut self,
        hint: Option<&str>,
        source_session: &str,
        shown_at: DateTime<Utc>,
    ) {
        let Some(feed) = self.queue.as_mut() else {
            return;
        };
        let hint = hint
            .map(str::trim)
            .filter(|h| !h.is_empty())
            .map(|text| PendingHint {
                text: text.to_string(),
                source: source_session.to_string(),
                at: shown_at,
                target: None,
                target_folder: None,
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
    /// the open game. A store error before the read returns with the game
    /// not marked read, so its next capture tries again; a store error
    /// writing the result keeps the result, and the game's next capture
    /// writes it instead of reading again. Frames that are not this game's
    /// (they do not fit its start, or they were held for another game that
    /// was never read) or a reader that fails give the game the warning.
    /// Never a failed capture.
    pub(crate) async fn attach_queue_title(
        &mut self,
        store: &impl TitleStore,
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
        // A result the store did not take is this game's, or no game's: the
        // game it was read for is no longer the one recorded.
        let unsaved = feed.unsaved.take().filter(|(s, _)| s == session_id);
        if existing.is_some() {
            feed.mark_read_for(session_id);
            tracing::info!(target: "scuffed_stat_tracker", session = session_id, "queue title: the game has one already, not read again");
            return;
        }
        if let Some((_, title)) = unsaved {
            feed.mark_read_for(session_id);
            tracing::info!(target: "scuffed_stat_tracker", session = session_id, "queue title: storing the read the store did not take");
            store_title(feed, store, session_id, title).await;
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
        let unread = match held_for_unread_game(feed, store, session_id).await {
            Ok(unread) => unread,
            Err(e) => {
                tracing::warn!(target: "scuffed_stat_tracker", error = %e, "queue title: the held frames' game not looked up, the game is read on its next capture");
                return;
            }
        };
        feed.mark_read_for(session_id);
        let read = if let Some(other) = unread {
            feed.drop_frames();
            tracing::info!(target: "scuffed_stat_tracker", session = session_id, held_for = %other, "queue title: the frames held are another game's that was never read, dropped unread");
            Field::none()
        } else if feed.fits(started) {
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
            // The game is decided: close its folder as a read does, so a
            // frame of it saved later is not read for the next game.
            feed.drop_frames();
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
                tracing::info!(target: "scuffed_stat_tracker", hint = %h, "queue title: next mode hint expired, the read was weak, the hint too old or another game's")
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
        store_title(feed, store, session_id, title).await;
    }
}

/// The game the held frames were held for, when that is not `session_id`
/// and it was never read: neither the game read last nor one with a stored
/// queue title (a game read before a restart). Its frames are not
/// `session_id`'s. `None` when they may be: held for this game, with no
/// open game, or in a new folder while a game decided already was open
/// (the start screen came late; the decided game's own folder was closed
/// at its decision, read or not, so none of its frames is held after it).
/// A store error is returned: the caller leaves the game unmarked, so its
/// next capture tries again, like the other store errors before the read.
async fn held_for_unread_game(
    feed: &QueueTitleFeed,
    store: &impl TitleStore,
    session_id: &str,
) -> StoreResult<Option<String>> {
    let Some(other) = feed.held_for().filter(|other| *other != session_id) else {
        return Ok(None);
    };
    if feed.fed_at().is_none() || feed.was_read_for(other) {
        return Ok(None);
    }
    Ok(match store.session_queue_title(other).await? {
        Some(_) => None,
        None => Some(other.to_string()),
    })
}

/// Write `title` on `session_id`. When the store does not take it, keep it
/// on the feed and unmark the game, so its next capture writes it instead
/// of reading again (the frames are used up and a settled hint is taken).
async fn store_title(
    feed: &mut QueueTitleFeed,
    store: &impl TitleStore,
    session_id: &str,
    title: QueueTitle,
) {
    if let Err(e) = store.attach_queue_title(session_id, &title).await {
        feed.forget_read_for(session_id);
        feed.unsaved = Some((session_id.to_string(), title));
        tracing::warn!(target: "scuffed_stat_tracker", error = %e, "queue title: not stored, kept and stored on the game's next capture");
    }
}
