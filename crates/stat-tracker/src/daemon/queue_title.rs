//! The wiring for Scuffed Vision's queue title reader (`rank-v1`,
//! [`stat_tracker::shadow::queue_title`]): which pre-game frames it reads,
//! when, and what the game record gets from it.
//!
//! Off by default: [`QueueTitleFeed::from_env`] is `None` unless
//! [`stat_tracker::shadow::queue_title::ENV`] is on and the pack loads, and
//! with no feed nothing here runs and no game changes.
//!
//! With it on, the feed sees every batch the pre-game saver sends (the
//! same batches the bans and picks feed sees) and holds the frames of the
//! reader's [`POLICY`] (hero ban last and first, hero select last) for the
//! current game folder. When the open game is first recorded the frames are
//! read once, off the poll loop's thread, and [`apply_read`] decides what
//! the game record gets:
//!
//! * a confident read (accepted by the reader, [`confident`]) sets the
//!   queue;
//! * a weak, failed or missing read sets the warning
//!   ([`stat_tracker::model::QueueTitle::warning`]).
//!
//! What is stored is stored once (`LocalStore::attach_queue_title`): a game
//! that has a queue title is not read again, so a restart mid-game cannot
//! turn a confident title into a warning. The feed closes the current
//! game folder when the open game is decided, read or not
//! ([`QueueTitleFeed::read_now`], [`QueueTitleFeed::drop_frames`]): a frame
//! of the same folder held after that (a late `heroselect_last`, say in
//! quick play, where a Tab as the hero select closes decides the game
//! before its only policy frame is saved) is ignored, so it cannot give the
//! next game, which may open no folder of its own, this game's queue. No
//! time keeps the folder open: a board carried from the last session is
//! written at its old time, so a game's first record can be older than its
//! own folder. When the next game's start screen came so late that its
//! folder opened before this decision, that game gets the warning, never
//! another game's queue.
//!
//! The held frames are tied to the open game when they were held
//! ([`QueueTitleFeed::held_for`]). A game that is never read (no Tab, or
//! first stored as a best guess on close or at shutdown) leaves its frames
//! behind; the next game's read finds them tied to another game that was
//! never read and drops them unread, so they cannot give it that game's
//! queue. Frames held in a new folder while the open game was one decided
//! already are the next game's (its start screen opened it after they were
//! held) and are read as before; the decided game's own folder is closed,
//! so none of its frames is held after its decision.
//!
//! A read whose store write fails is kept ([`QueueTitleFeed::unsaved`]) and
//! written on the game's next capture, so a store error does not turn a
//! confident read into the warning.
//!
//! Nothing here can stop, delay past one read or drop a game: every
//! failure is a warning on the game and a log line.
//!
//! # Next mode hint
//!
//! The rank reader reads the mode banner word on a game's rank screen; it
//! names the queue of the game that comes next, so it waits here as the
//! pending [`PendingHint`], set through
//! [`super::pregame_dump::PregameDump::set_next_mode_hint`] with the session
//! it was read on and the time the rank screen showed. It belongs to the
//! next game only ([`settle_hint`]):
//!
//! * the first read of a different game that started after the hint was
//!   read settles it, whatever the strength of that read: a confident read
//!   that agrees ([`hint_agrees`]) confirms it (it is stored on that game),
//!   a confident read that disagrees drops it, and a weak read expires it;
//! * it is never settled on the game it was read on, nor on a game that
//!   started before it was read: those leave it waiting;
//! * once pre-game frames are held for a game other than its source, it is
//!   that game's ([`PendingHint::target`]): a read of any other game
//!   expires it, so a game that is never read does not pass it on. Later
//!   frames of the same folder move it to the game they are held for (the
//!   tick that opens the next game holds its first frame under the game it
//!   retires); a new folder does not;
//! * it expires once it is older than the game window the frames use
//!   ([`super::bans_picks::frames_fit`]), so it never lands on a much later
//!   game.

use std::path::Path;
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use image::{DynamicImage, RgbImage};
use stat_tracker::model::QueueTitle;
use stat_tracker::shadow::queue_title::{Field, MIN_SCORE, POLICY, Reader, Screen};

use super::bans_picks::{GAME_WINDOW_MINUTES, frames_fit};
use super::pregame_dump::PregameOp;

/// One frame read, as the reader does it. A function so a test can inject
/// reads.
pub(crate) type ReadFn = Arc<dyn Fn(&RgbImage, Screen) -> Field<String> + Send + Sync>;

/// The next mode hint waiting for the next game's read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingHint {
    /// The hint the rank wiring handed on ([`super::rank::queue_hint`]: the
    /// mode banner word without its `COMPETITIVE` and team size words),
    /// trimmed and not empty.
    pub(crate) text: String,
    /// The session whose rank screen showed it: the hint names the game
    /// after this one, so this one never settles it.
    pub(crate) source: String,
    /// When the rank screen it was read on showed (the frame's time, not
    /// the time it was handed over).
    pub(crate) at: DateTime<Utc>,
    /// The game other than `source` that pre-game frames were held for
    /// after the hint was set, in the first folder that held such a frame:
    /// the next game. The last such frame of that folder sets it; frames of
    /// a later folder do not. `None` until then.
    pub(crate) target: Option<String>,
    /// The folder `target` was set in.
    pub(crate) target_folder: Option<String>,
}

/// What a read did to the pending next mode hint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HintOutcome {
    /// No hint was pending.
    NoHint,
    /// A confident read agreed: the hint is stored on the game.
    Confirmed(String),
    /// A confident read disagreed: the hint is gone.
    Dropped(String),
    /// The read was weak, the hint was older than the game window, or it
    /// was another game's (its [`PendingHint::target`], never read): the
    /// hint is gone, stored nowhere.
    Expired(String),
    /// Not this game's hint (its source game, or a game that started before
    /// it was read): it waits for the next game.
    Pending(String),
}

/// The game a read is for.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ReadGame<'a> {
    pub(crate) session: &'a str,
    /// The game's first record.
    pub(crate) started: DateTime<Utc>,
    /// When the read is applied.
    pub(crate) now: DateTime<Utc>,
}

pub(crate) struct QueueTitleFeed {
    read: ReadFn,
    folder: Option<String>,
    /// This folder was closed when the open game was decided, read or not:
    /// later frames of it are not held.
    done: bool,
    /// The current game's frames, by [`POLICY`] slot.
    frames: Vec<(Screen, &'static str, Arc<DynamicImage>)>,
    /// When the last frame was held, for [`frames_fit`].
    fed_at: Option<DateTime<Utc>>,
    /// The open game when the last frame was held, `None` with no open
    /// game or nothing held.
    held_for: Option<String>,
    /// The game read last, so a later capture of it is not read again.
    read_for: Option<String>,
    /// A game's read that the store did not take: written on that game's
    /// next capture instead of a read.
    pub(crate) unsaved: Option<(String, QueueTitle)>,
    /// The next mode hint waiting for the next game's read.
    pub(crate) pending_hint: Option<PendingHint>,
}

impl QueueTitleFeed {
    /// The live feed: `None`, nothing read, unless the switch is on and
    /// the pack loads ([`Reader::from_env`]).
    pub(crate) fn from_env(data_dir: &Path) -> Option<Self> {
        let reader = Reader::from_env(data_dir)?;
        tracing::info!(target: "scuffed_stat_tracker", "queue title: reader on");
        Some(Self::with_read(Arc::new(move |img, screen| {
            reader.read_field(img, screen)
        })))
    }

    pub(crate) fn with_read(read: ReadFn) -> Self {
        Self {
            read,
            folder: None,
            done: false,
            frames: Vec::new(),
            fed_at: None,
            held_for: None,
            read_for: None,
            unsaved: None,
            pending_hint: None,
        }
    }

    /// One batch the saver sends while `open` is the open game. A shot of a
    /// new game folder drops the old game's frames and the closed mark; a
    /// frame of a closed folder is not held. A frame held ties the frames
    /// to `open` ([`Self::held_for`]) and, when `open` is not the pending
    /// hint's source, the hint to `open` ([`PendingHint::target`]) unless
    /// the hint was tied in an earlier folder.
    pub(crate) fn observe(&mut self, ops: &[PregameOp<Arc<DynamicImage>>], open: Option<&str>) {
        for op in ops {
            let PregameOp::Save(shot) = op else {
                continue;
            };
            if self.folder.as_deref() != Some(shot.folder.as_str()) {
                self.folder = Some(shot.folder.clone());
                self.frames.clear();
                self.fed_at = None;
                self.held_for = None;
                self.done = false;
            }
            let Some((screen, which)) = policy_slot(shot.name) else {
                continue;
            };
            if self.done {
                tracing::info!(target: "scuffed_stat_tracker", name = shot.name, game = %shot.folder, reason = "closed when the open game was decided", "queue title: frame not held");
                continue;
            }
            self.frames.retain(|(s, w, _)| (*s, *w) != (screen, which));
            self.frames.push((screen, which, Arc::clone(&shot.frame)));
            self.fed_at = Some(Utc::now());
            self.held_for = open.map(str::to_string);
            if let (Some(hint), Some(open)) = (self.pending_hint.as_mut(), open)
                && hint.source != open
                && (hint.target.is_none() || hint.target_folder == self.folder)
            {
                hint.target = Some(open.to_string());
                hint.target_folder = self.folder.clone();
            }
        }
    }

    /// The open game when the last frame was held: the game the frames are
    /// for, unless it had been decided already (then its start screen came
    /// late and they are the next game's).
    pub(crate) fn held_for(&self) -> Option<&str> {
        self.held_for.as_deref()
    }

    /// Drop the held frames unread and close the folder, so nothing held
    /// later in it is read either. Called when the open game is decided
    /// with no read, whenever the folder opened.
    pub(crate) fn drop_frames(&mut self) {
        self.frames.clear();
        self.fed_at = None;
        self.held_for = None;
        self.done = true;
    }

    pub(crate) fn was_read_for(&self, session_id: &str) -> bool {
        self.read_for.as_deref() == Some(session_id)
    }

    pub(crate) fn mark_read_for(&mut self, session_id: &str) {
        self.read_for = Some(session_id.to_string());
    }

    /// Undo [`Self::mark_read_for`] for `session_id` (its result was not
    /// stored), so its next capture is read again.
    pub(crate) fn forget_read_for(&mut self, session_id: &str) {
        if self.was_read_for(session_id) {
            self.read_for = None;
        }
    }

    /// When the last frame was held; `None` with nothing held.
    pub(crate) fn fed_at(&self) -> Option<DateTime<Utc>> {
        self.fed_at
    }

    /// The held frames belong to a game first recorded at `started`.
    pub(crate) fn fits(&self, started: DateTime<Utc>) -> bool {
        frames_fit(self.fed_at, started)
    }

    /// Read the held frames in [`POLICY`] order, off the poll loop's
    /// thread; the first [`confident`] value wins, and with none the read
    /// has no value. The frames are dropped and the folder is closed
    /// either way, so nothing held later in it is read for another game.
    /// A reader that panics, or no frame at all, is a read with no value.
    pub(crate) async fn read_now(&mut self) -> Field<String> {
        let frames = std::mem::take(&mut self.frames);
        self.done = true;
        self.fed_at = None;
        self.held_for = None;
        let read = Arc::clone(&self.read);
        tokio::task::spawn_blocking(move || {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| read_frames(&read, &frames)))
                .unwrap_or_else(|_| Field::none())
        })
        .await
        .unwrap_or_else(|_| Field::none())
    }
}

/// The [`POLICY`] slot a pre-game frame name (`heroban_last.png`) fills.
fn policy_slot(name: &str) -> Option<(Screen, &'static str)> {
    let (screen, which) = name.strip_suffix(".png")?.split_once('_')?;
    let screen = Screen::parse(screen)?;
    POLICY
        .iter()
        .copied()
        .find(|(s, w)| *s == screen && *w == which)
}

fn read_frames(
    read: &ReadFn,
    frames: &[(Screen, &'static str, Arc<DynamicImage>)],
) -> Field<String> {
    for (screen, which) in POLICY {
        let Some((_, _, frame)) = frames.iter().find(|(s, w, _)| (*s, *w) == (screen, which))
        else {
            continue;
        };
        // Borrowed on RGB frames: no full-frame copy for a crop of 2% of it.
        let f = read(&stat_tracker::frame::rgb_view(frame), screen);
        if confident(&f).is_some() {
            return f;
        }
    }
    Field::none()
}

/// The value of a read the reader accepted: some text at confidence
/// [`MIN_SCORE`] or more (the reader only returns a value then; an injected
/// or future read below it counts as weak too). NaN is weak.
pub(crate) fn confident(read: &Field<String>) -> Option<&str> {
    read.value
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty() && read.confidence >= MIN_SCORE)
}

/// The queue a title or hint names: upper case, every run of characters
/// that are not letters or digits as one space, and a last word `QUEUE`
/// dropped. `OPEN QUEUE`, `Open Queue`, `open_queue` and `OPEN` are all
/// `OPEN`; `ROLE QUEUE` is `ROLE`; `QUEUE` alone is empty.
///
/// This is the one key both readers agree on, and the contract for the
/// rank reader's hint spelling: a hint must name the queue the way the
/// queue title pack spells the title (`OPEN QUEUE`, `ROLE QUEUE`, with or
/// without the last `QUEUE`). A word in front of it is part of the key, so
/// `COMPETITIVE: ROLE QUEUE` (`COMPETITIVE ROLE`) does not agree with a
/// `ROLE QUEUE` title (`ROLE`); the rank wiring maps its own labels to
/// this spelling before it sets a hint ([`super::rank::queue_hint`] drops
/// the `COMPETITIVE:` of the banner and a `5V5` or `6V6` after it). A
/// spelling that does not match is dropped, never confirmed: the safe way
/// to fail.
/// The key itself is [`stat_tracker::model::queue_key`], shared with the
/// upload's open queue check.
pub(crate) fn queue_key(text: &str) -> String {
    stat_tracker::model::queue_key(text)
}

/// A next mode hint agrees with a queue title when both name the same
/// queue ([`queue_key`]) and that queue is not empty. Anything else
/// disagrees, so a hint the reader cannot place (say a mode word with no
/// queue in it) is dropped by the next confident read.
pub(crate) fn hint_agrees(hint: &str, title: &str) -> bool {
    let key = queue_key(hint);
    !key.is_empty() && key == queue_key(title)
}

/// What becomes of the pending hint on a read of `game` whose confident
/// value is `value` (`None` for a weak read). Taken when it is settled or
/// expires; left when it is not this game's.
pub(crate) fn settle_hint(
    pending: &mut Option<PendingHint>,
    value: Option<&str>,
    game: ReadGame<'_>,
) -> HintOutcome {
    let Some(hint) = pending.as_ref() else {
        return HintOutcome::NoHint;
    };
    let text = hint.text.clone();
    if game.now - hint.at > Duration::minutes(GAME_WINDOW_MINUTES) {
        *pending = None;
        return HintOutcome::Expired(text);
    }
    if hint.source == game.session {
        return HintOutcome::Pending(text);
    }
    if !frames_fit(Some(hint.at), game.started) {
        if game.started < hint.at {
            // A game that was open before the hint was read.
            return HintOutcome::Pending(text);
        }
        // Started more than the window after the hint. The age check above
        // already covers this unless the game's start is later than `now`
        // (a clock that moved back): kept so the rule does not lean on that.
        *pending = None;
        return HintOutcome::Expired(text);
    }
    if hint.target.as_deref().is_some_and(|t| t != game.session) {
        // The next game had frames held and was never read: its hint is
        // not this later game's.
        *pending = None;
        return HintOutcome::Expired(text);
    }
    *pending = None;
    match value {
        None => HintOutcome::Expired(text),
        Some(v) if hint_agrees(&text, v) => HintOutcome::Confirmed(text),
        Some(_) => HintOutcome::Dropped(text),
    }
}

/// What the game record gets from its one read, and what becomes of the
/// pending hint ([`settle_hint`]).
pub(crate) fn apply_read(
    read: &Field<String>,
    pending: &mut Option<PendingHint>,
    game: ReadGame<'_>,
) -> (QueueTitle, HintOutcome) {
    let value = confident(read);
    let hint = settle_hint(pending, value, game);
    let Some(value) = value else {
        let title = QueueTitle {
            unsure: true,
            ..QueueTitle::default()
        };
        return (title, hint);
    };
    let title = QueueTitle {
        value: Some(value.to_string()),
        confidence: read.confidence.clamp(0.0, 1.0),
        unsure: false,
        mode_hint: match &hint {
            HintOutcome::Confirmed(h) => Some(h.clone()),
            _ => None,
        },
    };
    (title, hint)
}

#[cfg(test)]
#[path = "tests_queue_title.rs"]
mod tests;
