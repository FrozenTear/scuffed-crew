//! The game list the pages show, from a `live_snapshot.json` (or the same
//! rows read from the store): one row per game. Moved from the GUI crate
//! unchanged.

use std::path::Path;

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

/// Session ids with a `DeleteSession` waiting in `commands/`. A read-only
/// look: unlike [`crate::storage::read_commands`] it never removes a file it
/// can't parse, which is the tracker's call.
pub fn queued_deletes(data_dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(data_dir.join("commands")) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .filter_map(|p| std::fs::read(p).ok())
        .filter_map(|b| serde_json::from_slice::<crate::storage::StoreCommand>(&b).ok())
        .filter_map(|c| match c {
            crate::storage::StoreCommand::DeleteSession { session_id } => Some(session_id),
            _ => None,
        })
        .collect()
}

/// Drop the match and session rows of these games.
pub fn drop_sessions(snap: &mut Snapshot, gone: &[String]) {
    if gone.is_empty() {
        return;
    }
    snap.matches
        .retain(|m| m.session_id.is_empty() || !gone.contains(&m.session_id));
    snap.sessions.retain(|s| !gone.contains(&s.session_id));
}
