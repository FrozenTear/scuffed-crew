use std::path::Path;
use std::time::SystemTime;

use stat_tracker::storage::{self, Snapshot};

use crate::cli::FixtureKind;
use crate::fixtures;
use crate::model::Game;

/// Read the daemon snapshot via the existing lib API. Missing / unreadable → empty.
/// Deleted games are left out, so every tab and total skips them: a delete
/// still queued in `commands/` (the tracker may be stopped, and the GUI
/// never applies commands itself) counts as a pending delete, and no row
/// with a pending delete is kept.
pub fn load_snapshot(data_dir: &Path) -> Snapshot {
    let mut snap = storage::read_snapshot(data_dir).unwrap_or_default();
    for (_, cmd) in storage::read_commands(data_dir) {
        if let storage::StoreCommand::DeleteSession { session_id } = cmd
            && !snap.pending_deletes.contains(&session_id)
        {
            snap.pending_deletes.push(session_id);
        }
    }
    drop_deleted(&mut snap);
    snap
}

/// Drop the match and session rows of games with a pending delete.
pub fn drop_deleted(snap: &mut Snapshot) {
    if snap.pending_deletes.is_empty() {
        return;
    }
    let gone: std::collections::HashSet<String> = snap.pending_deletes.iter().cloned().collect();
    snap.matches
        .retain(|m| m.session_id.is_empty() || !gone.contains(&m.session_id));
    snap.sessions.retain(|s| !gone.contains(&s.session_id));
}

pub fn snapshot_mtime(data_dir: &Path) -> Option<SystemTime> {
    std::fs::metadata(data_dir.join("live_snapshot.json"))
        .and_then(|m| m.modified())
        .ok()
}

/// Materialize a fixture through `live_snapshot.json` and `read_snapshot`.
pub fn install_fixture(data_dir: &Path, kind: FixtureKind) -> anyhow::Result<Snapshot> {
    std::fs::create_dir_all(data_dir)?;
    let snap = fixtures::snapshot(kind);
    let bytes = serde_json::to_vec_pretty(&snap)?;
    std::fs::write(data_dir.join("live_snapshot.json"), bytes)?;
    if kind.has_games() {
        std::fs::write(
            data_dir.join("seasons.json"),
            fixtures::sample_seasons_json(),
        )?;
    } else {
        let _ = std::fs::remove_file(data_dir.join("seasons.json"));
    }
    if kind == FixtureKind::Games {
        crate::fixtures_games::write_kits(data_dir)?;
    }
    Ok(load_snapshot(data_dir))
}

pub fn games_from_snapshot(snap: &Snapshot) -> Vec<Game> {
    // Snapshot is already newest-first / latest-per-game; keep first row per session.
    // Deathmatch stays in the local store and is never part of the totals or
    // the history list. The mode or the map (Château Guillard) is enough.
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for m in &snap.matches {
        let map = m.display_map_name();
        let mode = stat_tracker::parse::uploaded_game_mode(map, &m.game_mode);
        if !stat_tracker::parse::stats_row_is_tracked(map, &m.game_mode)
            || !stat_tracker::parse::stats_row_is_tracked(map, &mode)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::FixtureKind;

    #[test]
    fn sample_fixture_roundtrips_through_read_snapshot() {
        let dir = std::env::temp_dir().join(format!(
            "sst-ui-snap-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let snap = install_fixture(&dir, FixtureKind::Sample).expect("install");
        assert_eq!(snap.matches.len(), 5);
        assert_eq!(snap.sessions.len(), 2);
        let games = games_from_snapshot(&snap);
        assert_eq!(games.len(), 5);
        assert_eq!(games[0].hero, "Junker Queen");
        assert_eq!(games[0].map_name, "King's Row");
        assert!(
            games[1].edited,
            "sample Ashe row carries an elims correction"
        );
        assert!(
            games[2].show_timeline(),
            "sample Ana row has two hero segments"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A delete made while the tracker is stopped stays queued; a GUI
    /// restart reads the old snapshot, and the game must not come back.
    #[test]
    fn a_game_deleted_while_the_tracker_is_stopped_stays_gone_on_reload() {
        let dir = std::env::temp_dir().join(format!(
            "sst-ui-del-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let snap = install_fixture(&dir, FixtureKind::Sample).expect("install");
        let sid = snap.matches[0].session_id.clone();
        let before = games_from_snapshot(&snap).len();
        crate::commands::delete_session(&dir, &sid).expect("queue");

        let reloaded = load_snapshot(&dir);
        let games = games_from_snapshot(&reloaded);
        assert_eq!(games.len(), before - 1);
        assert!(games.iter().all(|g| g.session_id != sid), "row gone");
        assert!(reloaded.sessions.iter().all(|s| s.session_id != sid));
        assert!(
            reloaded.pending_deletes.contains(&sid),
            "pending for the site"
        );
        assert!(
            crate::commands::queued(&dir).iter().any(|c| matches!(
                c,
                storage::StoreCommand::DeleteSession { session_id } if *session_id == sid
            )),
            "still queued for the tracker to apply and upload"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A snapshot that still lists a game with a tombstone leaves it out.
    #[test]
    fn a_row_with_a_pending_tombstone_is_left_out() {
        let mut snap = fixtures::snapshot(FixtureKind::Sample);
        let sid = snap.matches[0].session_id.clone();
        snap.pending_deletes = vec![sid.clone()];
        drop_deleted(&mut snap);
        assert!(snap.matches.iter().all(|m| m.session_id != sid));
        assert!(snap.sessions.iter().all(|s| s.session_id != sid));
    }

    #[test]
    fn empty_fixture_is_empty_overview_data() {
        let dir = std::env::temp_dir().join(format!(
            "sst-ui-empty-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let snap = install_fixture(&dir, FixtureKind::Empty).expect("install");
        assert!(snap.matches.is_empty());
        assert!(games_from_snapshot(&snap).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejected_upload_keeps_the_server_message() {
        let mut snap = fixtures::snapshot(FixtureKind::Sample);
        snap.matches[0].upload_reject = Some("matches[0]: hero is not allowed".into());
        let games = games_from_snapshot(&snap);
        assert_eq!(
            games[0].upload_reject.as_deref(),
            Some("matches[0]: hero is not allowed")
        );
        assert_eq!(crate::model::UPLOAD_REJECTED_LABEL, "Upload rejected");
    }

    #[test]
    fn deathmatch_rows_are_left_out_of_history_and_totals() {
        let mut snap = fixtures::snapshot(FixtureKind::Sample);
        let tracked = games_from_snapshot(&snap).len();
        let mut dm = snap.matches[0].clone();
        dm.session_id = "dm-guillard".into();
        dm.map_name = "Château Guillard".into();
        dm.game_mode = "Deathmatch".into();
        dm.outcome = "victory".into();
        snap.matches.insert(0, dm);
        let games = games_from_snapshot(&snap);
        assert_eq!(games.len(), tracked);
        assert!(games.iter().all(|game| game.session_id != "dm-guillard"));
        let totals = crate::aggregate::aggregate(&games, None, None);
        assert_eq!(totals.record.games, tracked);
        assert!(
            totals
                .maps
                .iter()
                .all(|map| map.map_name != "Château Guillard")
        );

        let mut corrected = snap.matches[0].clone();
        corrected.session_id = "edited-to-dm".into();
        corrected.map_name = "Busan".into();
        corrected.game_mode = "Control".into();
        corrected.corrected_map_name = Some("Château Guillard".into());
        snap.matches.insert(0, corrected);
        let games = games_from_snapshot(&snap);
        assert!(games.iter().all(|game| game.session_id != "edited-to-dm"));
        assert_eq!(games.len(), tracked);
    }

    #[test]
    fn practice_range_is_left_out_of_history() {
        let mut snap = fixtures::snapshot(FixtureKind::Sample);
        let tracked = games_from_snapshot(&snap).len();
        let mut practice = snap.matches[0].clone();
        practice.session_id = "practice-range".into();
        practice.map_name = "Practice Range".into();
        practice.game_mode = "Practice".into();
        practice.outcome = "victory".into();
        snap.matches.insert(0, practice);
        let games = games_from_snapshot(&snap);
        assert_eq!(games.len(), tracked);
        assert!(games.iter().all(|game| game.session_id != "practice-range"));

        let mut assault = snap.matches[0].clone();
        assault.session_id = "hanamura".into();
        assault.map_name = "Hanamura".into();
        assault.game_mode = "Assault".into();
        snap.matches.insert(0, assault);
        let games = games_from_snapshot(&snap);
        assert_eq!(games.len(), tracked);
        assert!(games.iter().all(|game| game.session_id != "hanamura"));
    }
}
