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
//! unknown", and says what is still known ([`unknown_words`]).
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
/// An open queue title, but no team size the upload could use (none in
/// the title and none from the board, or the two disagree).
pub const OPEN_SIZE_UNKNOWN: &str = "Open queue, size unknown";
/// A role queue title, but no team size the upload could use.
pub const ROLE_SIZE_UNKNOWN: &str = "Role queue, size unknown";
/// An unknown queue with nothing more to say: a title that names neither
/// queue (or both), or no title and no board size.
pub const UNKNOWN: &str = "Queue unknown";
/// No queue title and a 5-per-team board.
pub const UNKNOWN_5V5: &str = "5V5 queue unknown";
/// No queue title and a 6-per-team board: 6v6, but role or open was not
/// read (for example an `Open` role line alone).
pub const UNKNOWN_6V6: &str = "6V6 queue unknown";

/// Every label, for tests that measure or compare them all.
#[cfg(test)]
pub const ALL: [&str; 9] = [
    ROLE_5V5,
    OPEN_5V5,
    ROLE_6V6,
    OPEN_6V6,
    OPEN_SIZE_UNKNOWN,
    ROLE_SIZE_UNKNOWN,
    UNKNOWN,
    UNKNOWN_5V5,
    UNKNOWN_6V6,
];

/// The words for a known queue, or `None` for `unknown`.
pub fn known_words(queue: RankQueue) -> Option<&'static str> {
    match queue {
        RankQueue::Role5v5 => Some(ROLE_5V5),
        RankQueue::Open5v5 => Some(OPEN_5V5),
        RankQueue::Role6v6 => Some(ROLE_6V6),
        RankQueue::Open6v6 => Some(OPEN_6V6),
        RankQueue::Unknown => None,
    }
}

/// The words for a queue the upload sends as `unknown`, from what it was
/// decided on. `title` is the confident queue title the upload sends as
/// `rank.queue_title` (`None` when there is none): it names open or role
/// queue with the same word reader the upload uses, and only the size is
/// missing. With no title, the board's team size is all that is known
/// ("6V6 queue unknown"). A title beside a board size is not given the
/// board's size: with that pair the upload would have known the queue,
/// unless the title's own size word contradicts the board.
pub fn unknown_words(title: Option<&str>, team_size: Option<u8>) -> &'static str {
    use stat_tracker::rank_queue::{title_is_open, title_is_role};
    match (title, team_size) {
        (Some(t), _) if title_is_open(t) => OPEN_SIZE_UNKNOWN,
        (Some(t), _) if title_is_role(t) => ROLE_SIZE_UNKNOWN,
        (Some(_), _) => UNKNOWN,
        (None, Some(5)) => UNKNOWN_5V5,
        (None, Some(6)) => UNKNOWN_6V6,
        (None, _) => UNKNOWN,
    }
}

/// The game's queue in words, or `None` when the upload sends no `rank`
/// for it (see the module docs).
pub fn of_game(g: &Game) -> Option<&'static str> {
    let slot = g.rank_slot?;
    Some(known_words(slot.queue).unwrap_or_else(|| {
        let title = g.queue_title.as_ref().and_then(|t| t.confident_value());
        unknown_words(title, g.team_size)
    }))
}

/// The upload sends a known queue (not `unknown`) for this game.
pub fn is_known(g: &Game) -> bool {
    g.rank_slot.is_some_and(|s| s.queue != RankQueue::Unknown)
}

#[cfg(test)]
#[path = "games_queue_tests.rs"]
pub(crate) mod tests;
