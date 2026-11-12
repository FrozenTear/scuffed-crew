//! The wiring for Scuffed Vision's rank update reader (`rank-v1`,
//! [`stat_tracker::shadow::rank`]): which post-game frames it reads, how the
//! reads of one rank screen are combined, and what the game record gets.
//!
//! Off by default: [`RankFeed::from_env`] is `None` unless
//! [`stat_tracker::shadow::rank::ENV`] is on and the pack loads. With no
//! feed nothing here runs: no rank screen is read, saved or uploaded, and
//! no game changes.
//!
//! With it on, every poll frame the end detector reads as the rank screen
//! (its result word, `OutcomeSource::RankScreen`) is read once, inside the
//! tick's analysis, by [`stat_tracker::shadow::rank::game::read_for_game`]:
//! 2560x1440 frames only, never the placement screen. The reads are held
//! for the open game while the screen lasts ([`RankFeed::observe`]). One
//! analyzed tick that misses the screen does not end it: it is held as a
//! break between reads. The screen is gone after [`RANK_SETTLE_MISSES`]
//! analyzed ticks in a row miss it, when another game's screen starts or
//! the open game changes or closes, or when the game process stops
//! ([`close`]). Then [`settle`] keeps each field only when the newest read
//! that has it agrees with the read right before it, so a value caught
//! while it still counts up, or one the screen has since replaced, is not
//! kept. The result is stored on that game once
//! (`LocalStore::attach_rank_update`), which queues it for upload when the
//! upload changes. The mode banner word goes on to the queue title wiring
//! as the pending next mode hint
//! ([`super::pregame_dump::PregameDump::set_next_mode_hint`]).
//!
//! Nothing here can stop, fail or drop a game, or change any other field:
//! a reader that panics or reads nothing is no rank, a store error is
//! tried again on later ticks, on at most [`RANK_STORE_TRIES`] ticks in
//! all, and then dropped with a log line, and the stats sync runs on its
//! own task as before.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{DateTime, Utc};
use image::RgbImage;
use stat_tracker::detect::match_end::OutcomeSource;
use stat_tracker::model::{RankNumber, RankTags, RankText, RankUpdate};
use stat_tracker::shadow::rank::game::load_for_game;
use stat_tracker::shadow::rank::{FRAME_H, FRAME_W, RankSetting};

use super::pregame_dump::PregameDump;

/// One frame read, as the reader does it. A function so a test can inject
/// reads.
pub(crate) type RankReadFn = Arc<dyn Fn(&RgbImage) -> Option<RankUpdate> + Send + Sync>;

/// The most reads held for one rank screen (the newest are kept).
pub(crate) const RANK_HELD_READS: usize = 16;

/// How many analyzed ticks in a row must miss the rank screen before its
/// held reads settle. One miss (a misread word, a header change during a
/// rank-up) is a break between reads, not the end of the screen.
pub(crate) const RANK_SETTLE_MISSES: u8 = 2;

/// On how many ticks in all a settled update the store refused is tried:
/// the tick it settled on and the ticks after it.
pub(crate) const RANK_STORE_TRIES: u8 = 3;

/// A caught reader panic is logged once per run, not once per frame.
static PANIC_LOGGED: AtomicBool = AtomicBool::new(false);

type StoreResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// What the wiring needs from the store ([`crate::storage::LocalStore`]); a
/// trait so a test can make the write fail.
pub(crate) trait RankStore {
    async fn attach_rank_update(&self, session_id: &str, rank: &RankUpdate) -> StoreResult<bool>;
}

impl RankStore for crate::storage::LocalStore {
    async fn attach_rank_update(&self, session_id: &str, rank: &RankUpdate) -> StoreResult<bool> {
        crate::storage::LocalStore::attach_rank_update(self, session_id, rank).await
    }
}

/// The reads of the rank screen on screen now.
struct Held {
    session: String,
    /// When the newest read was taken.
    at: DateTime<Utc>,
    /// One per rank screen tick, oldest first; `None` for a frame read as
    /// the rank screen that gave no read, and for a break (a tick that
    /// missed the screen between two that showed it).
    reads: VecDeque<Option<RankUpdate>>,
    /// Analyzed ticks in a row that missed the screen since the newest read.
    missed: u8,
}

impl Held {
    fn push(&mut self, read: Option<RankUpdate>) {
        while self.reads.len() >= RANK_HELD_READS {
            self.reads.pop_front();
        }
        self.reads.push_back(read);
    }
}

/// A settled rank update for its game.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Settled {
    pub(crate) session: String,
    pub(crate) rank: RankUpdate,
    /// When the rank screen was last read.
    pub(crate) at: DateTime<Utc>,
}

pub(crate) struct RankFeed {
    read: RankReadFn,
    held: Option<Held>,
    /// A settled update the store did not take, and the tries left.
    pub(crate) unsaved: Option<(Settled, u8)>,
}

impl RankFeed {
    /// The live feed: `None`, nothing read, unless [`RankSetting::from_env`]
    /// is on and the pack loads.
    pub(crate) fn from_env(data_dir: &Path) -> Option<Self> {
        Self::for_setting(RankSetting::from_env(), data_dir)
    }

    pub(crate) fn for_setting(setting: RankSetting, data_dir: &Path) -> Option<Self> {
        if !setting.is_on() {
            return None;
        }
        match load_for_game(data_dir) {
            Ok(read) => {
                tracing::info!(target: "scuffed_stat_tracker", "rank update: reader on");
                let size_logged = AtomicBool::new(false);
                Some(Self::with_read(Arc::new(move |img| {
                    let (width, height) = img.dimensions();
                    if (width, height) != (FRAME_W, FRAME_H)
                        && !size_logged.swap(true, Ordering::Relaxed)
                    {
                        tracing::info!(target: "scuffed_stat_tracker", width, height, "rank update: only 2560x1440 frames are read, this frame size gives no rank (logged once per run)");
                    }
                    read(img)
                })))
            }
            Err(e) => {
                tracing::warn!(target: "scuffed_stat_tracker", "rank update reader on but not loaded: {e}");
                None
            }
        }
    }

    pub(crate) fn with_read(read: RankReadFn) -> Self {
        Self {
            read,
            held: None,
            unsaved: None,
        }
    }

    /// The read function, for the poll's analysis thread.
    pub(crate) fn reader(&self) -> RankReadFn {
        Arc::clone(&self.read)
    }

    /// One analyzed poll tick. `seen` is the tick's rank screen read
    /// ([`read_rank_screen`]): `None` when the frame was not the rank
    /// screen. `game` is the game the rank screen belongs to (the open game,
    /// when it was active recently). A read is held for `game`. A tick that
    /// misses the screen is a break between reads; the held reads settle
    /// ([`settle`]) after [`RANK_SETTLE_MISSES`] such ticks in a row, or
    /// at once when `game` is `None` or another game. The result is returned
    /// when it holds any field.
    pub(crate) fn observe(
        &mut self,
        seen: Option<Option<RankUpdate>>,
        game: Option<&str>,
        now: DateTime<Utc>,
    ) -> Option<Settled> {
        let Some(game) = game else {
            return self.finish();
        };
        let done = match &self.held {
            Some(h) if h.session != game => self.finish(),
            _ => None,
        };
        let Some(read) = seen else {
            if let Some(h) = self.held.as_mut() {
                h.missed = h.missed.saturating_add(1);
                if h.missed >= RANK_SETTLE_MISSES {
                    return self.finish();
                }
            }
            return done;
        };
        let held = self.held.get_or_insert_with(|| Held {
            session: game.to_string(),
            at: now,
            reads: VecDeque::new(),
            missed: 0,
        });
        if held.missed > 0 {
            // the screen came back after a missed tick: a break, so the
            // reads on either side of it never count as a pair
            held.push(None);
            held.missed = 0;
        }
        held.push(read);
        held.at = now;
        done
    }

    /// Settle the held reads now, whatever the misses: the game process
    /// stopped.
    pub(crate) fn close(&mut self) -> Option<Settled> {
        self.finish()
    }

    /// Settle the held reads, if any.
    fn finish(&mut self) -> Option<Settled> {
        let held = self.held.take()?;
        let reads: Vec<Option<RankUpdate>> = held.reads.into();
        let rank = settle(&reads);
        if rank.is_empty() {
            let read = reads.iter().filter(|r| r.is_some()).count();
            if read == 0 {
                tracing::info!(target: "scuffed_stat_tracker", session = %held.session, entries = reads.len(), "rank update: the reader read none of the rank screen frames (a frame size other than 2560x1440, not a rank update screen, or unreadable), no rank for the game");
            } else {
                tracing::info!(target: "scuffed_stat_tracker", session = %held.session, entries = reads.len(), read, "rank update: no field read the same on its newest two reads in a row, no rank for the game");
            }
            return None;
        }
        Some(Settled {
            session: held.session,
            rank,
            at: held.at,
        })
    }
}

impl Drop for RankFeed {
    /// Shutdown (the loop returns): held reads and an unsaved update are
    /// dropped, and the log says so.
    fn drop(&mut self) {
        if let Some(h) = &self.held {
            tracing::info!(target: "scuffed_stat_tracker", session = %h.session, frames = h.reads.len(), "rank update: shutting down with a rank screen held, dropped");
        }
        if let Some((s, _)) = &self.unsaved {
            tracing::info!(target: "scuffed_stat_tracker", session = %s.session, "rank update: shutting down with an update the store has not taken, dropped");
        }
    }
}

/// The rank screen read of one poll frame: `None` when the reader is off
/// (`read` is `None`) or the end detector did not read the frame as the
/// rank screen; else the frame's read, where a reader that panics or reads
/// nothing is `Some(None)`. A caught panic is logged once per run.
pub(crate) fn read_rank_screen(
    read: Option<&RankReadFn>,
    source: Option<OutcomeSource>,
    img: &RgbImage,
) -> Option<Option<RankUpdate>> {
    let read = read.filter(|_| source == Some(OutcomeSource::RankScreen))?;
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| read(img))) {
        Ok(r) => Some(r),
        Err(payload) => {
            if !PANIC_LOGGED.swap(true, Ordering::Relaxed) {
                let why = payload
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| payload.downcast_ref::<String>().cloned())
                    .unwrap_or_default();
                tracing::warn!(target: "scuffed_stat_tracker", panic = %why, "rank update: the reader panicked on a frame, read as no read (logged once per run)");
            }
            Some(None)
        }
    }
}

/// The held reads (oldest first) as one update. Per field: the newest read
/// that has the field is kept only when the read right before it has the
/// same value, at the lower of the two confidences; otherwise the field is
/// `None`. So a single read, a value still counting up, and a value the
/// screen has replaced on its newest read (`[A, A, B]`) keep nothing.
///
/// A read with no value for the field (a frame with no read, a read that
/// lacks the field, or a break for a missed tick) is not a value that
/// disagrees: after the newest pair it changes nothing, so a screen that
/// fades to nothing keeps its last agreed pair (`[A, A, no read]` keeps
/// `A`). Before the newest read that has the field it breaks the pair
/// (`[A, no read, A]` keeps nothing). Each field settles on its own, so a
/// field that stops reading while others go on keeps its older pair and
/// the record can mix moments the screen never showed together.
pub(crate) fn settle(reads: &[Option<RankUpdate>]) -> RankUpdate {
    RankUpdate {
        role: agreed(reads, |r| &r.role),
        role_rank: agreed(reads, |r| &r.role_rank),
        division: agreed(reads, |r| &r.division),
        progress_pct: agreed(reads, |r| &r.progress_pct),
        progress_change: agreed(reads, |r| &r.progress_change),
        change_tags: agreed(reads, |r| &r.change_tags),
        higher_than_pct: agreed(reads, |r| &r.higher_than_pct),
        hero_sr: agreed(reads, |r| &r.hero_sr),
        hero_sr_change: agreed(reads, |r| &r.hero_sr_change),
        next_mode_hint: agreed(reads, |r| &r.next_mode_hint),
    }
}

/// A rank field: whether two reads hold the same value, and the field at
/// the lower of two confidences.
trait Agree: Clone {
    fn same(&self, other: &Self) -> bool;
    fn lower(&self, other: &Self) -> Self;
}

macro_rules! agree {
    ($t:ty) => {
        impl Agree for $t {
            fn same(&self, other: &Self) -> bool {
                self.value == other.value
            }
            fn lower(&self, other: &Self) -> Self {
                Self {
                    confidence: self.confidence.min(other.confidence),
                    ..self.clone()
                }
            }
        }
    };
}

agree!(RankText);
agree!(RankNumber);
agree!(RankTags);

fn agreed<X: Agree>(
    reads: &[Option<RankUpdate>],
    get: impl Fn(&RankUpdate) -> &Option<X>,
) -> Option<X> {
    let field = |i: usize| reads.get(i)?.as_ref().and_then(|r| get(r).as_ref());
    let newest = (0..reads.len()).rev().find(|&i| field(i).is_some())?;
    let b = field(newest)?;
    let a = field(newest.checked_sub(1)?)?;
    a.same(b).then(|| b.lower(a))
}

/// The game a rank screen read now belongs to: the open game, when it was
/// active within the idle limit (the same rule as the report kit's outcome
/// crop). The rank screen follows the accolade screen, whose confirmed word
/// is activity, so this is the game that just ended.
pub(crate) fn rank_game(game: Option<&crate::ActiveGame>) -> Option<&str> {
    game.filter(|g| g.last_activity.elapsed() <= super::debug_dumps::RANK_OUTCOME_MAX_IDLE)
        .map(|g| g.session_id.as_str())
}

/// One analyzed poll tick of the wiring: hold the rank screen read or
/// settle the held ones ([`RankFeed::observe`]), store a settled update on
/// its game and hand its mode banner word to the queue title wiring, and
/// try an update the store refused again. `true` when an update was stored
/// on its game (the live snapshot is then out of date).
pub(crate) async fn tick(
    feed: &mut RankFeed,
    seen: Option<Option<RankUpdate>>,
    game: Option<&str>,
    store: &impl RankStore,
    dump: Option<&mut PregameDump>,
) -> bool {
    tick_at(feed, seen, game, store, dump, Utc::now()).await
}

/// [`tick`] with the time of this tick's read.
pub(crate) async fn tick_at(
    feed: &mut RankFeed,
    seen: Option<Option<RankUpdate>>,
    game: Option<&str>,
    store: &impl RankStore,
    dump: Option<&mut PregameDump>,
    now: DateTime<Utc>,
) -> bool {
    let settled = feed.observe(seen, game, now);
    apply(feed, settled, store, dump).await
}

/// The game process stopped: settle a rank screen still held now, and
/// store it ([`tick`]).
pub(crate) async fn close(
    feed: &mut RankFeed,
    store: &impl RankStore,
    dump: Option<&mut PregameDump>,
) -> bool {
    let settled = feed.close();
    apply(feed, settled, store, dump).await
}

async fn apply(
    feed: &mut RankFeed,
    settled: Option<Settled>,
    store: &impl RankStore,
    dump: Option<&mut PregameDump>,
) -> bool {
    let mut stored = false;
    // an update still waiting for the store goes first, so a newer one
    // cannot push it out unless both fail
    if let Some((s, tries)) = feed.unsaved.take() {
        stored |= store_rank(feed, store, s, tries).await;
    }
    if let Some(s) = settled {
        if let Some(dump) = dump {
            // A newer rank screen means a game came after the source of any
            // pending hint: a screen with no banner clears that hint, as it
            // no longer names the next game's queue. A second settle of the
            // hint's own game leaves it (`set_next_mode_hint` checks).
            let hint = s.rank.next_mode_hint.as_ref().map(|h| h.value.as_str());
            dump.set_next_mode_hint(hint, &s.session, s.at);
        }
        stored |= store_rank(feed, store, s, RANK_STORE_TRIES).await;
    }
    stored
}

async fn store_rank(feed: &mut RankFeed, store: &impl RankStore, s: Settled, tries: u8) -> bool {
    match store.attach_rank_update(&s.session, &s.rank).await {
        Ok(true) => {
            tracing::info!(target: "scuffed_stat_tracker", session = %s.session, "rank update stored");
            true
        }
        Ok(false) => {
            tracing::info!(target: "scuffed_stat_tracker", session = %s.session, "rank update not stored: the game is not recorded or has one already");
            false
        }
        Err(e) if tries > 1 => {
            tracing::warn!(target: "scuffed_stat_tracker", error = %e, "rank update not stored, tried again on the next tick");
            let session = s.session.clone();
            if let Some((old, _)) = feed.unsaved.replace((s, tries - 1)) {
                tracing::warn!(target: "scuffed_stat_tracker", session = %old.session, newer = %session, "rank update not stored, dropped: a newer one waits for the store");
            }
            false
        }
        Err(e) => {
            tracing::warn!(target: "scuffed_stat_tracker", error = %e, session = %s.session, "rank update not stored, dropped");
            false
        }
    }
}

#[cfg(test)]
#[path = "tests_rank.rs"]
mod tests;
