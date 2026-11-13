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

/// Show the queued edits and upload retries on `snap`'s rows as the
/// tracker will apply them, so a reload before it does (or with it
/// stopped) does not bring back Needs a look or Upload rejected while the
/// command still waits. Edits go through [`crate::storage::apply_match_edit`]
/// and clear `synced` and `upload_reject` as `edit_match` does. `cmds` is
/// oldest first, so a later pick wins.
pub fn overlay_queued(snap: &mut Snapshot, cmds: &[crate::storage::StoreCommand]) {
    use crate::storage::StoreCommand;
    let now = surrealdb_types::Datetime::from(chrono::Utc::now());
    for cmd in cmds {
        match cmd {
            StoreCommand::EditMatch { session_id, edit } if !session_id.is_empty() => {
                for m in snap
                    .matches
                    .iter_mut()
                    .filter(|m| &m.session_id == session_id)
                {
                    if crate::storage::apply_match_edit(m, edit, now) {
                        m.synced = false;
                        m.upload_reject = None;
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

/// Drop the match and session rows of these games.
pub fn drop_sessions(snap: &mut Snapshot, gone: &[String]) {
    if gone.is_empty() {
        return;
    }
    snap.matches
        .retain(|m| m.session_id.is_empty() || !gone.contains(&m.session_id));
    snap.sessions.retain(|s| !gone.contains(&s.session_id));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PersonalMatch;
    use crate::storage::{MatchEdit, StoreCommand};

    fn pm(session_id: &str) -> PersonalMatch {
        PersonalMatch {
            id: None,
            hero: "Mercy".into(),
            map_name: "Ilios".into(),
            game_mode: "Competitive".into(),
            role: "Support".into(),
            outcome: "win".into(),
            elims: 1,
            deaths: 1,
            assists: 1,
            damage: 1,
            healing: 1,
            mitigation: 0,
            played_at: surrealdb_types::Datetime::from(
                chrono::DateTime::parse_from_rfc3339("2026-10-18T20:00:00Z")
                    .unwrap()
                    .with_timezone(&chrono::Utc),
            ),
            synced: true,
            sync_rev: 0,
            upload_reject: Some("server said no".into()),
            session_id: session_id.into(),
            corrected_hero: None,
            corrected_role: None,
            corrected_map_name: None,
            corrected_outcome: None,
            corrected_elims: None,
            corrected_deaths: None,
            corrected_assists: None,
            corrected_damage: None,
            corrected_healing: None,
            corrected_mitigation: None,
            edited_fields: Vec::new(),
            edited_at: None,
            heroes_played: Vec::new(),
            segment_resolutions: Vec::new(),
            recognizer: String::new(),
            suspect_fields: Vec::new(),
            name_recognizer: None,
            bans_picks: None,
            bans_picks_status: None,
            queue_title: None,
            rank_update: None,
            match_times: None,
        }
    }

    /// A queued hero plus role edit shows the role from the edit, not the
    /// guessed one, and drops synced and a refused upload as the store does.
    #[test]
    fn overlay_queued_applies_the_edit_role_and_clears_sync() {
        let mut snap = Snapshot {
            matches: vec![pm("s1")],
            ..Snapshot::default()
        };
        overlay_queued(
            &mut snap,
            &[StoreCommand::EditMatch {
                session_id: "s1".into(),
                edit: MatchEdit {
                    hero: Some("Ana".into()),
                    role: Some("Tank".into()),
                    ..MatchEdit::default()
                },
            }],
        );
        let m = &snap.matches[0];
        assert_eq!(m.display_hero(), "Ana");
        assert_eq!(m.display_role(), "Tank");
        assert!(!m.synced);
        assert_eq!(m.upload_reject, None);
    }
}
