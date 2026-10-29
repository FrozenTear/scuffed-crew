use std::path::Path;
use std::time::SystemTime;

use stat_tracker::storage::{self, Snapshot};

use crate::cli::FixtureKind;
use crate::fixtures;
use crate::model::Game;

/// Read the daemon snapshot via the existing lib API. Missing / unreadable → empty.
/// Games the player deleted are left out, so every tab and total skips
/// them: a `DeleteSession` still queued in `commands/` (the tracker may be
/// stopped, and the GUI never applies commands itself). The tracker
/// removes the rows when it applies the delete. A pending tombstone alone
/// hides nothing (the store keeps some for games it keeps); it only feeds
/// the "next sync" line, as does a queued delete.
pub fn load_snapshot(data_dir: &Path) -> Snapshot {
    let mut snap = storage::read_snapshot(data_dir).unwrap_or_default();
    let queued = queued_deletes(data_dir);
    for sid in &queued {
        if !snap.pending_deletes.contains(sid) {
            snap.pending_deletes.push(sid.clone());
        }
    }
    drop_sessions(&mut snap, &queued);
    snap
}

/// Session ids with a `DeleteSession` waiting in `commands/`. A read-only
/// look: unlike [`storage::read_commands`] it never removes a file it
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
        .filter_map(|b| serde_json::from_slice::<storage::StoreCommand>(&b).ok())
        .filter_map(|c| match c {
            storage::StoreCommand::DeleteSession { session_id } => Some(session_id),
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

pub fn snapshot_mtime(data_dir: &Path) -> Option<SystemTime> {
    std::fs::metadata(data_dir.join("live_snapshot.json"))
        .and_then(|m| m.modified())
        .ok()
}

/// Materialize a fixture through `live_snapshot.json` and `read_snapshot`.
/// Queued commands are cleared only in the kind's default fixture dir: a
/// `--data-dir` the user passed may be a live tree, and its queued edits,
/// results and deletes must stay.
pub fn install_fixture(data_dir: &Path, kind: FixtureKind) -> anyhow::Result<Snapshot> {
    install_fixture_in(data_dir, kind, clears_commands(data_dir, kind))
}

/// Whether a fixture run in `data_dir` clears `commands/`: only in the
/// kind's default fixture dir.
fn clears_commands(data_dir: &Path, kind: FixtureKind) -> bool {
    data_dir == crate::cli::default_fixture_dir(kind)
}

/// [`install_fixture`], with `clear_commands` deciding whether queued
/// commands go. Each kind reuses its default dir, so clearing it there
/// brings back a game deleted in an earlier run.
fn install_fixture_in(
    data_dir: &Path,
    kind: FixtureKind,
    clear_commands: bool,
) -> anyhow::Result<Snapshot> {
    std::fs::create_dir_all(data_dir)?;
    if clear_commands {
        let _ = std::fs::remove_dir_all(data_dir.join("commands"));
    }
    let snap = fixtures::snapshot(kind);
    let bytes = serde_json::to_vec_pretty(&snap)?;
    std::fs::write(data_dir.join("live_snapshot.json"), bytes)?;
    if let Some(json) = fixtures::seasons_json(kind) {
        std::fs::write(data_dir.join("seasons.json"), json)?;
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

    /// A pending tombstone alone is not a delete: the store keeps them
    /// for games it keeps, so the game still shows.
    #[test]
    fn a_kept_game_with_a_pending_tombstone_still_shows() {
        let dir = tempfile::tempdir().unwrap();
        let mut snap = install_fixture(dir.path(), FixtureKind::Sample).expect("install");
        let sid = snap.matches[0].session_id.clone();
        snap.pending_deletes = vec![sid.clone()];
        std::fs::write(
            dir.path().join("live_snapshot.json"),
            serde_json::to_vec(&snap).unwrap(),
        )
        .unwrap();
        let reloaded = load_snapshot(dir.path());
        assert!(
            games_from_snapshot(&reloaded)
                .iter()
                .any(|g| g.session_id == sid)
        );
        assert!(reloaded.sessions.iter().any(|s| s.session_id == sid));
        assert_eq!(reloaded.pending_deletes, vec![sid]);
    }

    /// Reading the queue to hide deletes leaves every file in place, even
    /// one it can't parse: removing those is the tracker's call.
    #[test]
    fn the_reload_only_reads_the_queue() {
        let dir = tempfile::tempdir().unwrap();
        let cmds = dir.path().join("commands");
        std::fs::create_dir_all(&cmds).unwrap();
        std::fs::write(cmds.join("cmd_1_1.json"), b"not json").unwrap();
        crate::commands::delete_session(dir.path(), "sess-x").unwrap();
        assert_eq!(queued_deletes(dir.path()), vec!["sess-x".to_string()]);
        let _ = load_snapshot(dir.path());
        assert!(cmds.join("cmd_1_1.json").exists());
        assert_eq!(std::fs::read_dir(&cmds).unwrap().count(), 2);
    }

    /// A run in the default fixture dir starts clean: a delete queued in an
    /// earlier run of the same kind doesn't hide the game.
    #[test]
    fn a_fixture_run_in_the_default_dir_starts_with_no_queued_deletes() {
        let dir = tempfile::tempdir().unwrap();
        let snap = install_fixture_in(dir.path(), FixtureKind::Sample, true).expect("install");
        let sid = snap.matches[0].session_id.clone();
        crate::commands::delete_session(dir.path(), &sid).unwrap();
        let again = install_fixture_in(dir.path(), FixtureKind::Sample, true).expect("install");
        assert!(
            games_from_snapshot(&again)
                .iter()
                .any(|g| g.session_id == sid)
        );
        assert!(queued_deletes(dir.path()).is_empty());
    }

    /// A `--data-dir` the user passed may be a live tree: a fixture run
    /// there keeps every queued command, and the deleted game stays gone.
    #[test]
    fn a_fixture_run_in_a_given_data_dir_keeps_queued_commands() {
        let dir = tempfile::tempdir().unwrap();
        let snap = install_fixture(dir.path(), FixtureKind::Sample).expect("install");
        let sid = snap.matches[0].session_id.clone();
        crate::commands::delete_session(dir.path(), &sid).unwrap();
        let cmds = dir.path().join("commands");
        std::fs::write(cmds.join("cmd_1_1.json"), b"not json").unwrap();
        let again = install_fixture(dir.path(), FixtureKind::Sample).expect("install");
        assert!(
            games_from_snapshot(&again)
                .iter()
                .all(|g| g.session_id != sid)
        );
        assert_eq!(queued_deletes(dir.path()), vec![sid]);
        assert_eq!(std::fs::read_dir(&cmds).unwrap().count(), 2);
    }

    /// Only the kind's own default dir clears `commands/`; a `--data-dir`
    /// the user passed never does.
    #[test]
    fn only_the_default_fixture_dir_clears_commands() {
        let dir = tempfile::tempdir().unwrap();
        let games = crate::cli::default_fixture_dir(FixtureKind::Games);
        assert_eq!(
            crate::cli::Cli::parse_args(["--fixture", "games"]).data_dir,
            games
        );
        assert!(clears_commands(&games, FixtureKind::Games));
        assert!(!clears_commands(&games, FixtureKind::Sample));
        let given = crate::cli::Cli::parse_args([
            "--fixture",
            "games",
            "--data-dir",
            dir.path().to_str().unwrap(),
        ]);
        assert!(!clears_commands(&given.data_dir, FixtureKind::Games));
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

    /// The tracker's full store, as the old export wrote it: every row.
    async fn full_snapshot(store: &storage::LocalStore) -> Snapshot {
        Snapshot {
            matches: store.get_all_matches().await.unwrap(),
            sessions: store.get_all_sessions().await.unwrap(),
            pending_deletes: store.get_pending_tombstones().await.unwrap(),
        }
    }

    /// The daemon's compact `live_snapshot.json` (newest tracked row per
    /// game) shows exactly the games, in the same order, as the old full
    /// export did. Run on every fixture plus extra capture rows per game,
    /// a game whose newest row is untracked, and a deleted game.
    #[tokio::test]
    async fn the_compact_snapshot_shows_the_same_games_as_the_full_one() {
        for kind in [FixtureKind::Sample, FixtureKind::Games, FixtureKind::Full] {
            let dir = tempfile::tempdir().unwrap();
            let store = storage::LocalStore::open(dir.path()).await.unwrap();
            let fixture = fixtures::snapshot(kind);
            for (i, m) in fixture.matches.iter().enumerate() {
                let mut m = m.clone();
                store.insert_match(m.clone()).await.unwrap();
                // Two older captures of the same game with other numbers.
                if !m.session_id.is_empty() && i % 3 == 0 {
                    let newest: chrono::DateTime<chrono::Utc> = m.played_at.into();
                    for back in [60, 120] {
                        m.played_at = (newest - chrono::TimeDelta::seconds(back)).into();
                        m.elims += 1;
                        store.insert_match(m.clone()).await.unwrap();
                    }
                }
            }
            for s in &fixture.sessions {
                store.create_session(s).await.unwrap();
            }
            if let Some(first) = fixture.matches.iter().find(|m| !m.session_id.is_empty()) {
                // The newest row corrected to a deathmatch map: the GUI shows
                // the game's newest tracked row instead.
                let mut dm = first.clone();
                let at: chrono::DateTime<chrono::Utc> = first.played_at.into();
                dm.played_at = (at + chrono::TimeDelta::seconds(30)).into();
                dm.corrected_map_name = Some("Château Guillard".into());
                store.insert_match(dm).await.unwrap();
            }
            if let Some(last) = fixture
                .matches
                .iter()
                .rev()
                .find(|m| !m.session_id.is_empty())
            {
                store.delete_session(&last.session_id).await.unwrap();
            }

            let full = full_snapshot(&store).await;
            store.export_snapshot(dir.path()).await.unwrap();
            let compact = load_snapshot(dir.path());
            assert!(compact.matches.len() <= full.matches.len(), "{kind:?}");
            assert_eq!(
                format!("{:?}", games_from_snapshot(&compact)),
                format!("{:?}", games_from_snapshot(&full)),
                "{kind:?}"
            );
            assert_eq!(compact.pending_deletes, full.pending_deletes, "{kind:?}");
            assert_eq!(compact.sessions.len(), full.sessions.len(), "{kind:?}");
        }
    }

    /// The same parity on a copy of a real store. Ignored: needs
    /// `MEMFIX_PARITY_STORE` = a data dir holding `stats.surrealkv`
    /// (copied to a temp dir first; the given dir is never opened).
    #[tokio::test]
    #[ignore]
    async fn the_compact_snapshot_matches_on_a_real_store_copy() {
        let Some(src) = std::env::var_os("MEMFIX_PARITY_STORE") else {
            panic!("set MEMFIX_PARITY_STORE");
        };
        let dir = tempfile::tempdir().unwrap();
        let status = std::process::Command::new("cp")
            .arg("-a")
            .arg(std::path::Path::new(&src).join("stats.surrealkv"))
            .arg(dir.path())
            .status()
            .unwrap();
        assert!(status.success());
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        let full = full_snapshot(&store).await;
        let t = std::time::Instant::now();
        store.export_snapshot(dir.path()).await.unwrap();
        let ms = t.elapsed().as_millis();
        let bytes = std::fs::metadata(dir.path().join("live_snapshot.json"))
            .unwrap()
            .len();
        let compact = load_snapshot(dir.path());
        let (old, new) = (games_from_snapshot(&full), games_from_snapshot(&compact));
        println!(
            "rows {} -> {}, sessions {}, games {} -> {}, compact file {bytes} B, write {ms} ms",
            full.matches.len(),
            compact.matches.len(),
            compact.sessions.len(),
            old.len(),
            new.len()
        );
        assert_eq!(format!("{new:?}"), format!("{old:?}"));
    }
}
