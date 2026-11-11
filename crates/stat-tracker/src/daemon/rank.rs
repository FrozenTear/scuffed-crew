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
//! for the open game while the screen lasts ([`RankFeed::observe`]). When
//! the screen is gone (a tick that does not read it, another game, or the
//! game closing), [`settle`] keeps each field only where two reads in a row
//! agree, so a value caught while it still counts up is not kept, and the
//! result is stored on that game once (`LocalStore::attach_rank_update`),
//! which queues it for upload. The mode banner word goes on to the queue
//! title wiring as the pending next mode hint
//! ([`super::pregame_dump::PregameDump::set_next_mode_hint`]).
//!
//! Nothing here can stop, fail or drop a game, or change any other field:
//! a reader that panics or reads nothing is no rank, a store error is
//! retried on the next ticks ([`RANK_STORE_TRIES`]) and then dropped with a
//! log line, and the stats sync runs on its own task as before.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use image::RgbImage;
use stat_tracker::detect::match_end::OutcomeSource;
use stat_tracker::model::{RankNumber, RankTags, RankText, RankUpdate};
use stat_tracker::shadow::rank::{RankSetting, Reader, game::read_for_game};

use super::pregame_dump::PregameDump;

/// One frame read, as the reader does it. A function so a test can inject
/// reads.
pub(crate) type RankReadFn = Arc<dyn Fn(&RgbImage) -> Option<RankUpdate> + Send + Sync>;

/// The most reads held for one rank screen (the newest are kept).
pub(crate) const RANK_HELD_READS: usize = 16;

/// How many ticks a rank update the store refused is tried again.
pub(crate) const RANK_STORE_TRIES: u8 = 3;

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
    /// the rank screen that gave no read.
    reads: VecDeque<Option<RankUpdate>>,
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
        match Reader::load(data_dir) {
            Ok(reader) => {
                tracing::info!(target: "scuffed_stat_tracker", "rank update: reader on");
                Some(Self::with_read(Arc::new(move |img| {
                    read_for_game(&reader, img)
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

    /// One poll tick. `seen` is the tick's rank screen read
    /// ([`read_rank_screen`]): `None` when the frame was not the rank
    /// screen. `game` is the game the rank screen belongs to (the open game,
    /// when it was active recently). A read is held for `game`; the held
    /// reads settle ([`settle`]) when the screen is gone or another game's
    /// screen starts, and the result is returned when it holds any field.
    pub(crate) fn observe(
        &mut self,
        seen: Option<Option<RankUpdate>>,
        game: Option<&str>,
        now: DateTime<Utc>,
    ) -> Option<Settled> {
        let (Some(read), Some(game)) = (seen, game) else {
            return self.finish();
        };
        let done = match &self.held {
            Some(h) if h.session == game => None,
            _ => self.finish(),
        };
        let held = self.held.get_or_insert_with(|| Held {
            session: game.to_string(),
            at: now,
            reads: VecDeque::new(),
        });
        while held.reads.len() >= RANK_HELD_READS {
            held.reads.pop_front();
        }
        held.reads.push_back(read);
        held.at = now;
        done
    }

    /// Settle the held reads, if any.
    fn finish(&mut self) -> Option<Settled> {
        let held = self.held.take()?;
        let reads: Vec<Option<RankUpdate>> = held.reads.into();
        let rank = settle(&reads);
        if rank.is_empty() {
            tracing::info!(target: "scuffed_stat_tracker", session = %held.session, frames = reads.len(), "rank update: nothing read twice in a row, no rank for the game");
            return None;
        }
        Some(Settled {
            session: held.session,
            rank,
            at: held.at,
        })
    }
}

/// The rank screen read of one poll frame: `None` when the reader is off
/// (`read` is `None`) or the end detector did not read the frame as the
/// rank screen; else the frame's read, where a reader that panics or reads
/// nothing is `Some(None)`.
pub(crate) fn read_rank_screen(
    read: Option<&RankReadFn>,
    source: Option<OutcomeSource>,
    img: &RgbImage,
) -> Option<Option<RankUpdate>> {
    let read = read.filter(|_| source == Some(OutcomeSource::RankScreen))?;
    Some(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| read(img)))
            .ok()
            .flatten(),
    )
}

/// The held reads (oldest first) as one update: each field from the newest
/// two reads in a row that both have it with the same value, at the lower
/// of their two confidences. A field no two reads in a row agree on is
/// `None`, so a single read, or a value still counting up, keeps nothing.
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
    reads.windows(2).rev().find_map(|w| {
        let a = get(w[0].as_ref()?).as_ref()?;
        let b = get(w[1].as_ref()?).as_ref()?;
        a.same(b).then(|| b.lower(a))
    })
}

/// One poll tick of the wiring: hold or settle the rank screen reads
/// ([`RankFeed::observe`]), store a settled update on its game and hand
/// its mode banner word to the queue title wiring, or try an update the
/// store refused again.
pub(crate) async fn tick(
    feed: &mut RankFeed,
    seen: Option<Option<RankUpdate>>,
    game: Option<&str>,
    store: &impl RankStore,
    dump: Option<&mut PregameDump>,
) {
    let settled = feed.observe(seen, game, Utc::now());
    if let Some(s) = &settled
        && let Some(dump) = dump
    {
        let hint = s.rank.next_mode_hint.as_ref().map(|h| h.value.as_str());
        if hint.is_some() {
            dump.set_next_mode_hint(hint, &s.session, s.at);
        }
    }
    let next = match settled {
        Some(s) => Some((s, RANK_STORE_TRIES)),
        None => feed.unsaved.take(),
    };
    if let Some((s, tries)) = next {
        store_rank(feed, store, s, tries).await;
    }
}

async fn store_rank(feed: &mut RankFeed, store: &impl RankStore, s: Settled, tries: u8) {
    match store.attach_rank_update(&s.session, &s.rank).await {
        Ok(true) => {
            tracing::info!(target: "scuffed_stat_tracker", session = %s.session, "rank update stored");
        }
        Ok(false) => {
            tracing::info!(target: "scuffed_stat_tracker", session = %s.session, "rank update not stored: the game is not recorded or has one already");
        }
        Err(e) if tries > 1 => {
            tracing::warn!(target: "scuffed_stat_tracker", error = %e, "rank update not stored, tried again on the next tick");
            feed.unsaved = Some((s, tries - 1));
        }
        Err(e) => {
            tracing::warn!(target: "scuffed_stat_tracker", error = %e, session = %s.session, "rank update not stored, dropped");
        }
    }
}

#[cfg(test)]
#[path = "tests_rank.rs"]
mod tests;
