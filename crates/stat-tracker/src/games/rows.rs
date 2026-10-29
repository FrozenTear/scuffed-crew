//! The game list the pages show, from a `live_snapshot.json` (or the same
//! rows read from the store): one row per game. Moved from the GUI crate
//! unchanged.

use super::model::Game;
use crate::storage::Snapshot;

pub fn games_from_snapshot(snap: &Snapshot) -> Vec<Game> {
    // Snapshot is already newest-first / latest-per-game; keep first row per session.
    // Deathmatch stays in the local store and is never part of the totals or
    // the history list. The mode or the map (Château Guillard) is enough.
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for m in &snap.matches {
        let map = m.display_map_name();
        let mode = crate::parse::uploaded_game_mode(map, &m.game_mode);
        if !crate::parse::stats_row_is_tracked(map, &m.game_mode)
            || !crate::parse::stats_row_is_tracked(map, &mode)
        {
            continue;
        }
        if m.session_id.is_empty() {
            out.push(Game::from_match(m));
            continue;
        }
        if seen.insert(m.session_id.clone()) {
            out.push(Game::from_match(m));
        }
    }
    out
}
