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
//! current game. When the game is first recorded the frames are read, off
//! the poll loop's thread, and [`apply_read`] decides what the game record
//! gets:
//!
//! * a confident read (accepted by the reader, [`confident`]) sets the
//!   queue;
//! * a weak, failed or missing read keeps the queue the record has and sets
//!   the warning ([`stat_tracker::model::QueueTitle::warning`]).
//!
//! Nothing here can stop, delay past one read or drop a game: every
//! failure is a warning on the game and a log line.
//!
//! # Next mode hint
//!
//! The rank reader reads the mode banner word after a game; it names the
//! queue of the game that comes next, so it waits here as the pending
//! [`QueueTitleFeed::pending_hint`], set through
//! [`super::pregame_dump::PregameDump::set_next_mode_hint`]. The next
//! game's read settles it: a confident read that agrees confirms it (it is
//! stored on that game), a confident read that disagrees drops it, and a
//! weak read leaves it pending. "Agrees" is [`hint_agrees`].

use std::path::Path;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use image::{DynamicImage, RgbImage};
use stat_tracker::model::QueueTitle;
use stat_tracker::shadow::queue_title::{Field, MIN_SCORE, POLICY, Reader, Screen};

use super::pregame_dump::PregameOp;

/// One frame read, as the reader does it. A function so a test can inject
/// reads.
pub(crate) type ReadFn = Arc<dyn Fn(&RgbImage, Screen) -> Field<String> + Send + Sync>;

/// What a read did to the pending next mode hint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HintOutcome {
    /// No hint was pending.
    NoHint,
    /// A confident read agreed: the hint is stored on the game.
    Confirmed(String),
    /// A confident read disagreed: the hint is gone.
    Dropped(String),
    /// The read was weak: the hint waits for the next read.
    Pending(String),
}

pub(crate) struct QueueTitleFeed {
    read: ReadFn,
    folder: Option<String>,
    /// The current game's frames, by [`POLICY`] slot.
    frames: Vec<(Screen, &'static str, Arc<DynamicImage>)>,
    /// When the last frame was held, for [`super::bans_picks::frames_fit`].
    fed_at: Option<DateTime<Utc>>,
    /// The game read last, so a later capture of it is not read again.
    read_for: Option<String>,
    /// The next mode hint waiting for the next game's read.
    pub(crate) pending_hint: Option<String>,
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
            frames: Vec::new(),
            fed_at: None,
            read_for: None,
            pending_hint: None,
        }
    }

    /// One batch the saver sends. A shot of a new game folder drops the old
    /// game's frames.
    pub(crate) fn observe(&mut self, ops: &[PregameOp<Arc<DynamicImage>>]) {
        for op in ops {
            let PregameOp::Save(shot) = op else {
                continue;
            };
            if self.folder.as_deref() != Some(shot.folder.as_str()) {
                self.folder = Some(shot.folder.clone());
                self.frames.clear();
                self.fed_at = None;
            }
            let Some((screen, which)) = policy_slot(shot.name) else {
                continue;
            };
            self.frames.retain(|(s, w, _)| (*s, *w) != (screen, which));
            self.frames.push((screen, which, Arc::clone(&shot.frame)));
            self.fed_at = Some(Utc::now());
        }
    }

    pub(crate) fn was_read_for(&self, session_id: &str) -> bool {
        self.read_for.as_deref() == Some(session_id)
    }

    pub(crate) fn mark_read_for(&mut self, session_id: &str) {
        self.read_for = Some(session_id.to_string());
    }

    /// The held frames belong to a game first recorded at `started`.
    pub(crate) fn fits(&self, started: DateTime<Utc>) -> bool {
        super::bans_picks::frames_fit(self.fed_at, started)
    }

    /// Read the held frames in [`POLICY`] order, off the poll loop's
    /// thread; the first [`confident`] value wins, and with none the read
    /// has no value. The frames are dropped either way. A
    /// reader that panics, or no frame at all, is a read with no value.
    pub(crate) async fn read_now(&mut self) -> Field<String> {
        let frames = std::mem::take(&mut self.frames);
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
        let f = read(&frame.to_rgb8(), screen);
        if confident(&f).is_some() {
            return f;
        }
    }
    Field::none()
}

/// The value of a read the reader accepted: some text at confidence
/// [`MIN_SCORE`] or more (the reader only returns a value then; an injected
/// or future read below it counts as weak too).
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
pub(crate) fn queue_key(text: &str) -> String {
    let upper = text.to_uppercase();
    let mut words: Vec<&str> = upper
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    if words.last() == Some(&"QUEUE") {
        words.pop();
    }
    words.join(" ")
}

/// A next mode hint agrees with a queue title when both name the same
/// queue ([`queue_key`]) and that queue is not empty. Anything else
/// disagrees, so a hint the reader cannot place (say a mode word with no
/// queue in it) is dropped by the next confident read.
pub(crate) fn hint_agrees(hint: &str, title: &str) -> bool {
    let key = queue_key(hint);
    !key.is_empty() && key == queue_key(title)
}

/// What the game record gets from one read, over what it had (`existing`),
/// and what becomes of the pending hint (taken when settled).
pub(crate) fn apply_read(
    existing: Option<QueueTitle>,
    read: &Field<String>,
    pending: &mut Option<String>,
) -> (QueueTitle, HintOutcome) {
    let mut title = existing.unwrap_or_default();
    let Some(value) = confident(read) else {
        title.unsure = true;
        let hint = pending
            .clone()
            .map_or(HintOutcome::NoHint, HintOutcome::Pending);
        return (title, hint);
    };
    title.value = Some(value.to_string());
    title.confidence = read.confidence.clamp(0.0, 1.0);
    title.unsure = false;
    let hint = match pending.take() {
        None => HintOutcome::NoHint,
        Some(h) if hint_agrees(&h, value) => {
            title.mode_hint = Some(h.clone());
            HintOutcome::Confirmed(h)
        }
        Some(h) => HintOutcome::Dropped(h),
    };
    (title, hint)
}

#[cfg(test)]
#[path = "tests_queue_title.rs"]
mod tests;
