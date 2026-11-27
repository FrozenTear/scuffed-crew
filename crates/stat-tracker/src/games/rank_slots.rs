//! Local rank kept per queue and role (#654): a game's rank read is only
//! ever grouped with ranks of the same [`RankSlot`], so a 6v6 role queue
//! rank never stands in for, or replaces, a 5v5 role queue or open queue
//! rank, and a game whose queue could not be read never joins a known
//! queue.
//!
//! Each game keeps its own rank (`PersonalMatch::rank_update`, written once
//! per game); nothing here changes the store. The slot is
//! [`Game::rank_slot`], the same queue and role the upload sends.

use super::model::Game;
use crate::rank_queue::RankSlot;

/// The game has a rank screen read the upload keeps
/// ([`RankSlot::rank_read`]) and a slot to file it under. The stored
/// `rank_update` is not looked at: a field the upload drops is no rank here.
fn ranked_slot(g: &Game) -> Option<RankSlot> {
    g.rank_slot.filter(|s| s.rank_read)
}

/// The newest ranked game of each slot (queue and role), newest first. A
/// game with no rank screen read the upload keeps, or no slot (not Competitive evidence, a
/// mode that is not Competitive, or the readers off), is left out. Two
/// games of one slot played at the same time keep the one listed first.
pub fn latest_rank_per_slot(games: &[Game]) -> Vec<&Game> {
    let mut latest: Vec<(RankSlot, &Game)> = Vec::new();
    for g in games {
        let Some(slot) = ranked_slot(g) else {
            continue;
        };
        match latest.iter_mut().find(|(s, _)| s.same_slot(&slot)) {
            Some((_, kept)) if g.played_at > kept.played_at => *kept = g,
            Some(_) => {}
            None => latest.push((slot, g)),
        }
    }
    let mut out: Vec<&Game> = latest.into_iter().map(|(_, g)| g).collect();
    out.sort_by_key(|g| std::cmp::Reverse(g.played_at));
    out
}

#[cfg(test)]
#[path = "tests_rank_slots.rs"]
mod tests;
