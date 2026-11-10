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

/// The commands waiting in `commands/`, oldest first. A read-only look:
/// unlike [`crate::storage::read_commands`] it never removes a file it
/// can't parse, which is the tracker's call.
pub fn queued_commands(data_dir: &Path) -> Vec<crate::storage::StoreCommand> {
    let Ok(entries) = std::fs::read_dir(data_dir.join("commands")) else {
        return Vec::new();
    };
    let mut files: Vec<std::path::PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect();
    files.sort();
    files
        .into_iter()
        .filter_map(|p| std::fs::read(p).ok())
        .filter_map(|b| serde_json::from_slice::<crate::storage::StoreCommand>(&b).ok())
        .collect()
}

/// Session ids with a `DeleteSession` waiting in `commands/`.
pub fn queued_deletes(data_dir: &Path) -> Vec<String> {
    queued_commands(data_dir)
        .into_iter()
        .filter_map(|c| match c {
            crate::storage::StoreCommand::DeleteSession { session_id } => Some(session_id),
            _ => None,
        })
        .collect()
}

/// Show the queued map and hero picks and upload retries on `snap`'s rows
/// as the tracker will apply them, so a reload before it does (or with it
/// stopped) does not bring back Needs a look or Upload rejected while the
/// command still waits. Only the map and hero of an edit are shown here;
/// the tracker applies the rest. `cmds` is oldest first, so a later pick
/// wins.
pub fn overlay_queued(snap: &mut Snapshot, cmds: &[crate::storage::StoreCommand]) {
    use crate::storage::StoreCommand;
    for cmd in cmds {
        match cmd {
            StoreCommand::EditMatch { session_id, edit } if !session_id.is_empty() => {
                for m in snap
                    .matches
                    .iter_mut()
                    .filter(|m| &m.session_id == session_id)
                {
                    if let Some(map) = &edit.map_name {
                        m.corrected_map_name = Some(map.clone());
                        mark_edited(&mut m.edited_fields, "map_name");
                        crate::suspect::clear_edited_suspects(&mut m.suspect_fields, "map_name");
                    }
                    if let Some(hero) = &edit.hero {
                        pick_hero(m, hero, edit.role.is_some());
                    }
                }
            }
            StoreCommand::RetryUpload { session_id } if !session_id.is_empty() => {
                for m in snap
                    .matches
                    .iter_mut()
                    .filter(|m| &m.session_id == session_id)
                {
                    m.upload_reject = None;
                }
            }
            StoreCommand::RetryRejectedUploads => {
                for m in &mut snap.matches {
                    m.upload_reject = None;
                }
            }
            _ => {}
        }
    }
}

fn mark_edited(fields: &mut Vec<String>, name: &str) {
    if !fields.iter().any(|f| f == name) {
        fields.push(name.to_string());
    }
}

/// A hero pick as the store applies it: the hero, and the role it implies
/// when the edit set none and it differs.
fn pick_hero(m: &mut crate::model::PersonalMatch, hero: &str, role_set: bool) {
    m.corrected_hero = Some(hero.to_string());
    mark_edited(&mut m.edited_fields, "hero");
    crate::suspect::clear_edited_suspects(&mut m.suspect_fields, "hero");
    let guessed = crate::parse::guess_role_public(hero);
    if !role_set && !crate::parse::hero_is_unknown_label(hero) && guessed != m.display_role() {
        m.corrected_role = Some(guessed);
        mark_edited(&mut m.edited_fields, "role");
    }
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
