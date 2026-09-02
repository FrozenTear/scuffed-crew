use std::path::Path;
use std::time::SystemTime;

use stat_tracker::storage::{self, Snapshot};

use crate::cli::FixtureKind;
use crate::fixtures;
use crate::model::Game;

/// Read the daemon snapshot via the existing lib API. Missing / unreadable → empty.
pub fn load_snapshot(data_dir: &Path) -> Snapshot {
    storage::read_snapshot(data_dir).unwrap_or(Snapshot {
        matches: Vec::new(),
        sessions: Vec::new(),
    })
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
    if kind == FixtureKind::Sample {
        std::fs::write(
            data_dir.join("seasons.json"),
            fixtures::sample_seasons_json(),
        )?;
    } else {
        let _ = std::fs::remove_file(data_dir.join("seasons.json"));
    }
    Ok(load_snapshot(data_dir))
}

pub fn games_from_snapshot(snap: &Snapshot) -> Vec<Game> {
    // Snapshot is already newest-first / latest-per-game; keep first row per session.
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for m in &snap.matches {
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
}
