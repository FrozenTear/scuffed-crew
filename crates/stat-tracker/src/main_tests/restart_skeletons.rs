//! Daemon tests: builders for `active_game.json` skeletons and the restart
//! paths they feed. See `mod.rs`.

use super::*;

pub(crate) fn persisted_with_hold(
    session_id: &str,
    outcome: detect::MatchOutcome,
    map: Option<&str>,
    imported: bool,
    elims: u32,
    last_activity: chrono::DateTime<Utc>,
) -> PersistedGame {
    PersistedGame {
        rejoin_carry: None,
        replaced_text_fallback_map: None,
        best_guess: None,
        session_id: session_id.into(),
        outcome,
        map: map.map(str::to_string),
        map_source: None,
        map_candidates: Vec::new(),
        session_created: false,
        opened_at: last_activity - chrono::Duration::minutes(20),
        last_activity,
        outcome_recorded_at: None,
        gate: None,
        last_stats_at: None,
        hero_auth: HeroAuthState::default(),
        result_outcome: None,
        result_confirmed: false,
        result_seen_at: None,
        pending_boundary: false,
        awaiting_first_board: false,
        reset_streak: 0,
        reset_baseline: None,
        baseline_row: None,
        deferred: Some(fresh_counters(elims)),
        deferred_hero: Some("Tracer".into()),
        deferred_at: Some(last_activity),
        deferred_imported: imported,
        progressed_boards: 0,
        baseline_at: None,
        text_fallback_locked: false,
        match_times: Default::default(),
    }
}

/// A game on disk with a stored board, last captured `ago` before now.
pub(crate) fn restart_skeleton(map: Option<&str>, ago: chrono::Duration) -> PersistedGame {
    let mut p = persisted_with_hold(
        "before-crash",
        detect::MatchOutcome::Unknown,
        map,
        false,
        0,
        Utc::now() - ago,
    );
    p.session_created = true;
    p.deferred = None;
    p.deferred_hero = None;
    p.deferred_at = None;
    p
}

/// Restart from `p`, then ask whether the first Tab opens a new game.
pub(crate) fn first_tab_after_restart_starts_new_game(p: &PersistedGame) -> bool {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(active_game_path(dir.path()), serde_json::to_vec(p).unwrap()).unwrap();
    let st = startup_session(dir.path());
    should_start_fresh_session(st.active_game.as_ref(), Instant::now())
}

pub(crate) async fn begin_from_persisted(
    dir: &std::path::Path,
    store: &storage::LocalStore,
    game: PersistedGame,
) -> SessionState {
    std::fs::write(active_game_path(dir), serde_json::to_vec(&game).unwrap()).unwrap();
    let mut sync_task = None;
    let backoff = std::sync::Arc::new(std::sync::Mutex::new(sync::SyncBackoff::default()));
    begin_daemon_session(dir, &mut sync_task, &backoff, store, None).await
}

#[tokio::test]
async fn a_backwards_clock_archives_an_imported_hold_with_its_persisted_map() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let when = Utc::now() + chrono::Duration::hours(3);
    begin_from_persisted(
        dir.path(),
        &store,
        persisted_with_hold(
            "clock-back",
            detect::MatchOutcome::Defeat,
            Some("Nepal"),
            true,
            8,
            when,
        ),
    )
    .await;
    let held = store
        .get_all_matches()
        .await
        .unwrap()
        .into_iter()
        .find(|row| row.elims == 8)
        .expect("a backwards clock archives the hold");
    assert_eq!(held.session_id, "clock-back");
    assert_eq!(held.map_name, "Nepal");
    assert_eq!(held.outcome, "defeat");
    assert!(!active_game_path(dir.path()).exists());
}

/// A skeleton written before the game's start and end were kept still
/// resumes, with neither: the game records and its length falls back.
#[test]
fn a_skeleton_from_before_match_times_resumes_with_none() {
    let mut p = restart_skeleton(Some("Busan"), chrono::Duration::seconds(30));
    p.session_id = "older-skeleton".into();
    let mut json = serde_json::to_value(&p).unwrap();
    json.as_object_mut().unwrap().remove("match_times");
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        active_game_path(dir.path()),
        serde_json::to_vec(&json).unwrap(),
    )
    .unwrap();
    let g = startup_session(dir.path())
        .active_game
        .expect("the older skeleton resumes");
    assert_eq!(g.session_id, "older-skeleton");
    assert_eq!(g.match_times, stat_tracker::model::MatchTimes::default());
}
