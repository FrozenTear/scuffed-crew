//! The queue a game was played in, as the Games view names it (#654, T14).
//!
//! The queue is the one the upload sends as `rank.queue` for the same game:
//! [`Game::rank_slot`], which the tracker builds from the upload's own
//! `rank` object (`stat_tracker::sync::rank_slot`). Nothing here decides a
//! queue; it only names one. So the chip can never say another queue than
//! the site gets.
//!
//! Words: a known queue is spelled in the game's capitals, as the queue
//! title has been since the Rank column landed (#890): "5V5 ROLE QUEUE",
//! "6V6 OPEN QUEUE". The size is always named, so a 5v5 role queue game and
//! a 6v6 role queue game never read alike. An unknown queue is the
//! tracker's own words in sentence case, like the Rank cell's "Role
//! unknown", with the board's team size first when a Tab read gave one
//! ("6V6 queue unknown").
//!
//! No queue at all (`rank_slot` is `None`) draws nothing: the readers are
//! off (the default), the game is not Competitive (its title names QUICK,
//! UNRANKED, ARCADE, STADIUM or CUSTOM), or nothing about it was
//! Competitive evidence. The upload sends no `rank` for those games, and
//! the Rank cell already shows "-".

use crate::model::Game;
use scuffed_types::api::RankQueue;

/// Role queue, 5 per team.
pub const ROLE_5V5: &str = "5V5 ROLE QUEUE";
/// Open queue, 5 per team.
pub const OPEN_5V5: &str = "5V5 OPEN QUEUE";
/// Role queue, 6 per team (2-2-2, Season 6).
pub const ROLE_6V6: &str = "6V6 ROLE QUEUE";
/// Open queue, 6 per team.
pub const OPEN_6V6: &str = "6V6 OPEN QUEUE";
/// A Competitive game whose queue could not be read, with no board size.
pub const UNKNOWN: &str = "Queue unknown";
/// An unknown queue on a 5-per-team board.
pub const UNKNOWN_5V5: &str = "5V5 queue unknown";
/// An unknown queue on a 6-per-team board: 6v6, but role or open was not
/// read (for example an `Open` role line with no queue title).
pub const UNKNOWN_6V6: &str = "6V6 queue unknown";

/// Every label, for tests that measure or compare them all.
#[cfg(test)]
pub const ALL: [&str; 7] = [
    ROLE_5V5,
    OPEN_5V5,
    ROLE_6V6,
    OPEN_6V6,
    UNKNOWN,
    UNKNOWN_5V5,
    UNKNOWN_6V6,
];

/// The words for `queue`. `team_size` is used only to qualify an unknown
/// queue; a known queue already carries its size.
pub fn label(queue: RankQueue, team_size: Option<u8>) -> &'static str {
    match (queue, team_size) {
        (RankQueue::Role5v5, _) => ROLE_5V5,
        (RankQueue::Open5v5, _) => OPEN_5V5,
        (RankQueue::Role6v6, _) => ROLE_6V6,
        (RankQueue::Open6v6, _) => OPEN_6V6,
        (RankQueue::Unknown, Some(5)) => UNKNOWN_5V5,
        (RankQueue::Unknown, Some(6)) => UNKNOWN_6V6,
        (RankQueue::Unknown, _) => UNKNOWN,
    }
}

/// The game's queue in words, or `None` when the upload sends no `rank`
/// for it (see the module docs).
pub fn of_game(g: &Game) -> Option<&'static str> {
    g.rank_slot.map(|slot| label(slot.queue, g.team_size))
}

/// The upload sends a known queue (not `unknown`) for this game.
pub fn is_known(g: &Game) -> bool {
    g.rank_slot.is_some_and(|s| s.queue != RankQueue::Unknown)
}

#[cfg(test)]
#[path = "games_queue_tests.rs"]
pub(crate) mod tests;
