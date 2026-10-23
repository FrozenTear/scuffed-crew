//! Daemon tests: shutdown_and_restart. See `mod.rs`.

use super::*;

#[tokio::test]
async fn shutdown_keeps_an_unfinished_game_so_a_restart_resumes_it() {
    assert_eq!(
        stat_tracker::replace::shutdown_time_left(),
        stat_tracker::replace::SHUTDOWN_DEADLINE,
        "a shutdown with no signal keeps the full deadline"
    );
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    store
        .insert_match(unfinished_row("quit-after-match"))
        .await
        .unwrap();
    let now = test_now();
    let mut g = game(detect::MatchOutcome::Unknown, None, now);
    g.session_id = "quit-after-match".into();
    g.map = Some("Busan".into());
    let mut st = session(Some(g), now);
    persist_active_game(dir.path(), st.active_game.as_ref());
    shutdown_from_signal(
        &mut st,
        &mut None,
        &mut tokio::sync::mpsc::unbounded_channel().1,
        &mut None,
        None,
        &std::sync::Mutex::new(sync::SyncBackoff::default()),
        &store,
        dir.path(),
    )
    .await;
    assert_eq!(
        st.active_game.as_ref().map(|g| g.session_id.as_str()),
        Some("quit-after-match"),
        "a clean shutdown keeps the unfinished skeleton"
    );
    assert!(active_game_path(dir.path()).exists());
    assert!(st.settle_closed_unknown.is_none());
    let restored = startup_session(dir.path());
    assert_eq!(
        restored.active_game.as_ref().map(|g| g.session_id.as_str()),
        Some("quit-after-match"),
        "a GUI restart resumes the same session"
    );
    let rows = store.get_all_matches().await.unwrap();
    assert!(
        rows.iter()
            .any(|row| row.session_id == "quit-after-match" && !row.synced),
        "the resumed row stays unsynced so a late result can attach"
    );
}

/// MAIN-2: the report of the capture shutdown waited for is applied
/// before the game is saved. It sat in the channel the run loop no
/// longer reads, and its map, gate and hero authority were lost.
#[tokio::test]
async fn shutdown_applies_the_report_of_the_capture_it_drained() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let now = test_now();
    let mut g = game(detect::MatchOutcome::Unknown, None, now);
    g.session_id = "drained".into();
    let mut st = session(Some(g), now);
    let mut report = rejected_report("drained");
    report.recorded = true;
    report.map = Some("Busan".into());
    report.map_source = Some(boundary::MapSource::TopBar);
    let board = Counters {
        elims: 7,
        assists: 2,
        deaths: 3,
        damage: 2400,
        healing: 0,
        mitigation: 900,
    };
    report.gate_state = Some(GateState {
        accepted: board,
        last_raw: board,
        ..GateState::default()
    });
    report.hero_auth.accepted_hero = Some("Ana".into());
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let task = tokio::spawn(async move {
        tokio::task::yield_now().await;
        let _ = tx.send(("drained".to_string(), Ok(report)));
    });
    let stopped = shutdown_from_signal(
        &mut st,
        &mut Some(task),
        &mut rx,
        &mut None,
        None,
        &std::sync::Mutex::new(sync::SyncBackoff::default()),
        &store,
        dir.path(),
    )
    .await;
    assert_eq!(stopped, None);
    let restored = startup_session(dir.path());
    let g = restored.active_game.expect("the unfinished game is kept");
    assert_eq!(g.session_id, "drained");
    assert_eq!(g.map.as_deref(), Some("Busan"), "the report's map is saved");
    assert_eq!(g.gate.map(|gate| gate.accepted.elims), Some(7));
    assert_eq!(g.hero_auth.accepted_hero.as_deref(), Some("Ana"));
}

/// A capture that does not finish is abandoned before the deadline, and
/// the game is still saved.
#[tokio::test]
async fn a_stuck_capture_at_shutdown_does_not_cost_the_save() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let now = test_now();
    let mut g = game(detect::MatchOutcome::Unknown, None, now);
    g.session_id = "stuck-ocr".into();
    g.map = Some("Busan".into());
    let mut st = session(Some(g), now);
    let stuck = tokio::spawn(std::future::pending::<()>());
    let stopped = shutdown_from_signal(
        &mut st,
        &mut Some(stuck),
        &mut tokio::sync::mpsc::unbounded_channel().1,
        &mut None,
        None,
        &std::sync::Mutex::new(sync::SyncBackoff::default()),
        &store,
        dir.path(),
    )
    .await;
    assert_eq!(stopped, None, "every stage after the drain finished");
    let restored = startup_session(dir.path());
    assert_eq!(
        restored.active_game.map(|g| g.session_id),
        Some("stuck-ocr".to_string()),
        "the game was saved without the capture"
    );
}

/// MAIN-3: the game is saved before the portal close and the upload,
/// and the deadline log names the step that was running.
#[test]
fn shutdown_saves_the_game_before_the_portal_and_the_upload() {
    let pos = |stage| {
        SHUTDOWN_STAGES
            .iter()
            .position(|s| *s == stage)
            .expect("every stage runs")
    };
    assert_eq!(SHUTDOWN_STAGES[0], ShutdownStage::CaptureDrain);
    assert_eq!(SHUTDOWN_STAGES[1], ShutdownStage::SessionSave);
    assert!(pos(ShutdownStage::SessionSave) < pos(ShutdownStage::ScreencastClose));
    assert!(pos(ShutdownStage::SessionSave) < pos(ShutdownStage::FinalUpload));
    assert!(pos(ShutdownStage::FinalUpload) < pos(ShutdownStage::SnapshotExport));
    assert!(SCREENCAST_CLOSE_TIMEOUT <= Duration::from_secs(1));
    let line = shutdown_overrun_at(pos(ShutdownStage::FinalUpload));
    assert!(
        line.ends_with("still running: final upload (not reached: snapshot export)"),
        "{line}"
    );
    assert!(!line.contains("screencast"), "{line}");
}

/// MAIN-3: an upload that does not finish does not cost the save, and
/// the deadline is reported against the upload.
#[tokio::test]
async fn a_hung_upload_at_shutdown_still_saves_the_game_and_is_named() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    store
        .insert_match(unfinished_row("slow-network"))
        .await
        .unwrap();
    let now = test_now();
    let mut g = game(detect::MatchOutcome::Unknown, None, now);
    g.session_id = "slow-network".into();
    g.map = Some("Busan".into());
    let mut st = session(Some(g), now);
    // An upload already in flight that never returns.
    let mut sync_task = Some(tokio::spawn(std::future::pending::<()>()));
    let stopped = shutdown_from_signal(
        &mut st,
        &mut None,
        &mut tokio::sync::mpsc::unbounded_channel().1,
        &mut sync_task,
        None,
        &std::sync::Mutex::new(sync::SyncBackoff::default()),
        &store,
        dir.path(),
    )
    .await;
    assert_eq!(stopped, Some(ShutdownStage::FinalUpload));
    let restored = startup_session(dir.path());
    assert_eq!(
        restored.active_game.map(|g| g.session_id),
        Some("slow-network".to_string()),
        "the game was saved before the upload"
    );
}

#[tokio::test]
async fn a_new_tab_after_idle_closes_an_unfinished_game_and_holds_it() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    store
        .insert_match(unfinished_row("left-open"))
        .await
        .unwrap();
    let now = test_now();
    let mut g = game(detect::MatchOutcome::Unknown, None, now);
    g.session_id = "left-open".into();
    g.map = Some("Busan".into());
    g.last_activity = now - UNFINISHED_SESSION_IDLE - Duration::from_secs(1);
    let mut st = session(Some(g), now);
    persist_active_game(dir.path(), st.active_game.as_ref());
    let opened = open_fresh_if_tab_starts_one(&mut st, &store, dir.path(), now).await;
    assert!(opened, "the idle bound opens the next session");
    assert_ne!(
        st.active_game.as_ref().map(|g| g.session_id.as_str()),
        Some("left-open")
    );
    assert_eq!(st.settle_closed_unknown.as_deref(), Some("left-open"));
    assert_unknown_is_held_locally_on_close(&store, dir.path(), "left-open").await;
}

#[tokio::test]
async fn restart_holds_a_stale_unfinished_game_and_keeps_a_fresh_one() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    store
        .insert_match(unfinished_row("stale-unfinished"))
        .await
        .unwrap();
    let when = Utc::now() - chrono::Duration::hours(9);
    let stale = PersistedGame {
        rejoin_carry: None,
        replaced_text_fallback_map: None,
        best_guess: None,
        session_id: "stale-unfinished".into(),
        outcome: detect::MatchOutcome::Unknown,
        map: Some("Busan".into()),
        map_source: None,
        map_candidates: Vec::new(),
        session_created: true,
        opened_at: when - chrono::Duration::minutes(20),
        last_activity: when,
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
        deferred: None,
        deferred_hero: None,
        deferred_at: None,
        deferred_imported: false,
        progressed_boards: 0,
        baseline_at: None,
        text_fallback_locked: false,
    };
    std::fs::write(
        active_game_path(dir.path()),
        serde_json::to_vec(&stale).unwrap(),
    )
    .unwrap();
    let dropped = retire_stale_skeleton(dir.path());
    assert_eq!(
        dropped.as_ref().map(|drop| drop.session_id.as_str()),
        Some("stale-unfinished")
    );
    assert!(dropped.as_ref().is_some_and(|drop| drop.archive.is_none()));
    assert!(!active_game_path(dir.path()).exists());
    let restored = startup_session(dir.path());
    assert!(restored.active_game.is_none());
    assert_unknown_is_held_locally_on_close(&store, dir.path(), "stale-unfinished").await;

    let fresh_dir = tempfile::tempdir().unwrap();
    let fresh_store = storage::LocalStore::open(fresh_dir.path()).await.unwrap();
    fresh_store
        .insert_match(unfinished_row("still-this-match"))
        .await
        .unwrap();
    let now = Utc::now();
    let fresh = PersistedGame {
        replaced_text_fallback_map: None,
        session_id: "still-this-match".into(),
        last_activity: now,
        opened_at: now - chrono::Duration::minutes(5),
        ..stale
    };
    std::fs::write(
        active_game_path(fresh_dir.path()),
        serde_json::to_vec(&fresh).unwrap(),
    )
    .unwrap();
    assert!(
        retire_stale_skeleton(fresh_dir.path()).is_none(),
        "a restart inside the idle bound resumes the match"
    );
    let resumed = startup_session(fresh_dir.path());
    assert_eq!(
        resumed.active_game.as_ref().map(|g| g.session_id.as_str()),
        Some("still-this-match")
    );
    let seen = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen_upload = std::sync::Arc::clone(&seen);
    try_sync_with(&fresh_store, fresh_dir.path(), None, move |matches, _| {
        let seen_upload = std::sync::Arc::clone(&seen_upload);
        async move {
            seen_upload.store(matches.len(), std::sync::atomic::Ordering::SeqCst);
            Ok(upload_ok())
        }
    })
    .await;
    assert_eq!(seen.load(std::sync::atomic::Ordering::SeqCst), 0);
    let rows = fresh_store.get_all_matches().await.unwrap();
    assert!(
        rows.iter()
            .any(|row| row.session_id == "still-this-match" && !row.synced),
        "a resumed match stays unsynced so a late result can attach"
    );
}

#[tokio::test]
async fn set_outcome_on_the_open_session_uploads_on_the_next_sync() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    store
        .insert_match(unfinished_row("open-set"))
        .await
        .unwrap();
    let now = test_now();
    let mut g = game(detect::MatchOutcome::Unknown, None, now);
    g.session_id = "open-set".into();
    g.map = Some("Busan".into());
    let mut st = session(Some(g), now);
    assert!(st.settle_closed_unknown.is_none());
    if let Some(active) = st.active_game.as_mut() {
        active.record_outcome(detect::MatchOutcome::Victory);
    }
    store
        .apply_command(&storage::StoreCommand::SetOutcome {
            session_id: "open-set".into(),
            outcome: "victory".into(),
        })
        .await
        .unwrap();
    assert_eq!(
        st.active_game
            .as_ref()
            .map(|active| active.session_id.as_str()),
        Some("open-set"),
        "SetOutcome does not close the open session"
    );
    assert!(st.settle_closed_unknown.is_none());
    assert!(st.active_game.as_ref().unwrap().finished());
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen_upload = std::sync::Arc::clone(&seen);
    let attempt = try_sync_with(&store, dir.path(), None, move |matches, _| {
        let seen_upload = std::sync::Arc::clone(&seen_upload);
        async move {
            *seen_upload.lock().unwrap() = matches
                .into_iter()
                .map(|row| {
                    let outcome = row.display_outcome().to_string();
                    let edited = row.is_edited();
                    (row.session_id, outcome, edited)
                })
                .collect();
            Ok(upload_ok())
        }
    })
    .await;
    assert!(
        matches!(attempt, sync::SyncAttempt::Uploaded),
        "the next sync uploads the open session"
    );
    assert_eq!(
        *seen.lock().unwrap(),
        vec![("open-set".to_string(), "victory".to_string(), true)]
    );
}

#[tokio::test]
async fn restart_syncs_a_stale_finished_skeleton_without_losing_rows() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    store
        .insert_match(test_match("colosseo-defeat", "defeat"))
        .await
        .unwrap();
    let when = Utc::now() - chrono::Duration::hours(9);
    let stale = PersistedGame {
        rejoin_carry: None,
        replaced_text_fallback_map: None,
        best_guess: None,
        session_id: "colosseo-defeat".into(),
        outcome: detect::MatchOutcome::Defeat,
        map: Some("Colosseo".into()),
        map_source: None,
        map_candidates: Vec::new(),
        session_created: true,
        opened_at: when - chrono::Duration::minutes(20),
        last_activity: when,
        outcome_recorded_at: Some(when),
        gate: None,
        last_stats_at: None,
        hero_auth: HeroAuthState::default(),
        result_outcome: Some(detect::MatchOutcome::Defeat),
        result_confirmed: true,
        result_seen_at: Some(when),
        pending_boundary: false,
        awaiting_first_board: false,
        reset_streak: 0,
        reset_baseline: None,
        baseline_row: None,
        deferred: None,
        deferred_hero: None,
        deferred_at: None,
        deferred_imported: false,
        progressed_boards: 0,
        baseline_at: None,
        text_fallback_locked: false,
    };
    std::fs::write(
        active_game_path(dir.path()),
        serde_json::to_vec(&stale).unwrap(),
    )
    .unwrap();
    let uploads = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let sync_with = |uploads: std::sync::Arc<std::sync::atomic::AtomicUsize>,
                     store: storage::LocalStore,
                     dir: std::path::PathBuf| {
        move |dropped: Option<String>| {
            let uploads = std::sync::Arc::clone(&uploads);
            let store = store.clone();
            let dir = dir.clone();
            async move {
                let _ = try_sync_with(&store, &dir, dropped, move |matches, _tombstones| {
                    let uploads = std::sync::Arc::clone(&uploads);
                    async move {
                        assert_eq!(matches.len(), 1);
                        assert_eq!(matches[0].session_id, "colosseo-defeat");
                        assert_eq!(matches[0].outcome, "defeat");
                        uploads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        Ok(upload_ok())
                    }
                })
                .await;
            }
        }
    };
    let restored = recover_or_sync_active_game(
        dir.path(),
        &store,
        sync_with(
            std::sync::Arc::clone(&uploads),
            store.clone(),
            dir.path().to_path_buf(),
        ),
    )
    .await;
    assert!(
        restored.active_game.is_none() && restored.last_game_open.is_none(),
        "a 9-hour-old skeleton is not the open game"
    );
    assert_eq!(uploads.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(!active_game_path(dir.path()).exists());
    let rows = store.get_all_matches().await.unwrap();
    assert_eq!(
        rows.len(),
        1,
        "dropping the skeleton must not drop the rows"
    );
    assert_eq!(rows[0].session_id, "colosseo-defeat");
    assert_eq!(rows[0].outcome, "defeat");
    assert_eq!(rows[0].elims, 10);
    assert!(rows[0].synced);

    let again = recover_or_sync_active_game(
        dir.path(),
        &store,
        sync_with(
            std::sync::Arc::clone(&uploads),
            store.clone(),
            dir.path().to_path_buf(),
        ),
    )
    .await;
    assert!(again.active_game.is_none());
    assert_eq!(
        uploads.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "a second start must not upload the same game again"
    );
    assert_eq!(store.get_all_matches().await.unwrap().len(), 1);
}

#[tokio::test]
async fn quiet_close_persists_so_a_restart_does_not_retire_twice() {
    let mut night = Night::new().await;
    let id = finish_colosseo(&mut night).await;
    let activity = night.game().last_activity;
    night.idle_through(activity + close_after()).await;
    assert!(night.st.active_game.is_none());
    assert!(!active_game_path(night.dir.path()).exists());
    assert_eq!(night.upload_count(), 1);
    let uploads = night.upload_count();
    let recovered = night.restart_from_disk().await;
    assert!(
        recovered.is_none(),
        "the cleared skeleton must not come back as the open game"
    );
    assert!(recover_active_game(night.dir.path()).is_none());
    assert_eq!(
        night.upload_count(),
        uploads,
        "restart must not upload again"
    );
    assert!(
        !night.quiet_tick().await,
        "a second quiet tick must not close the session again"
    );
    assert_eq!(night.upload_count(), uploads);
    let rows = night.store.get_session_snapshots(&id).await.unwrap();
    assert!(!rows.is_empty());
    assert!(rows.iter().all(|row| row.outcome == "defeat" && row.synced));
}

#[tokio::test]
async fn shutdown_uploads_the_finished_board_not_a_fresh_hold() {
    assert_eq!(
        stat_tracker::replace::shutdown_time_left(),
        stat_tracker::replace::SHUTDOWN_DEADLINE,
        "a shutdown with no signal keeps the full deadline"
    );
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let mut row = test_match("busan-win", "victory");
    row.map_name = "Busan".into();
    row.game_mode = "Control".into();
    row.elims = 14;
    store.insert_match(row).await.unwrap();
    let now = test_now();
    let mut g = game(detect::MatchOutcome::Victory, Some(10), now);
    g.session_id = "busan-win".into();
    g.map = Some("Busan".into());
    g.deferred = Some(fresh_counters(2));
    g.deferred_hero = Some("Tracer".into());
    g.deferred_imported = false;
    let mut st = session(Some(g), now);
    shutdown_from_signal(
        &mut st,
        &mut None,
        &mut tokio::sync::mpsc::unbounded_channel().1,
        &mut None,
        None,
        &std::sync::Mutex::new(sync::SyncBackoff::default()),
        &store,
        dir.path(),
    )
    .await;
    let held = st
        .active_game
        .as_ref()
        .expect("fresh hold opens the next session");
    assert_ne!(held.session_id, "busan-win");
    assert!(held.deferred_imported);
    assert_eq!(held.deferred.map(|c| c.elims), Some(2));
    let busan = store.get_session_snapshots("busan-win").await.unwrap();
    assert_eq!(busan.len(), 1);
    assert_eq!(busan[0].elims, 14);
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen_upload = std::sync::Arc::clone(&seen);
    try_sync_with(&store, dir.path(), None, move |matches, _| {
        let seen_upload = std::sync::Arc::clone(&seen_upload);
        async move {
            *seen_upload.lock().unwrap() = matches
                .into_iter()
                .map(|row| (row.session_id, row.outcome, row.elims, row.map_name))
                .collect();
            Ok(upload_ok())
        }
    })
    .await;
    assert_eq!(
        *seen.lock().unwrap(),
        vec![(
            "busan-win".to_string(),
            "victory".to_string(),
            14,
            "Busan".to_string()
        )]
    );
    let resumed = startup_session(dir.path());
    assert_eq!(
        resumed.active_game.as_ref().map(|g| g.session_id.as_str()),
        Some(held.session_id.as_str()),
        "a GUI restart resumes the next session, not a rewritten Busan row"
    );
    let called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let called_upload = std::sync::Arc::clone(&called);
    try_sync_with(&store, dir.path(), None, move |matches, _| {
        let called_upload = std::sync::Arc::clone(&called_upload);
        async move {
            assert!(
                matches.iter().all(|row| row.elims != 2),
                "restart must not upload the held board as Busan: {matches:?}"
            );
            called_upload.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(upload_ok())
        }
    })
    .await;
    assert!(
        !called.load(std::sync::atomic::Ordering::SeqCst),
        "the finished row was already uploaded"
    );
    let all = store.get_all_matches().await.unwrap();
    assert!(all.iter().all(|row| row.elims != 2));
}

/// The exit-75 self-restart calls `shutdown_from_signal` without a
/// signal. That path gets the full deadline, and the Tab board held on
/// the session is written before the process is replaced.
#[tokio::test]
async fn an_update_restart_gets_the_full_shutdown_deadline_and_keeps_the_tab_row() {
    assert!(
        !stat_tracker::replace::shutdown_requested(),
        "a self-restart has no signal"
    );
    assert_eq!(
        stat_tracker::replace::shutdown_time_left(),
        stat_tracker::replace::SHUTDOWN_DEADLINE
    );
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let mut row = test_match("tab-on-restart", "victory");
    row.map_name = "Busan".into();
    row.game_mode = "Control".into();
    row.hero = "Ana".into();
    row.elims = 9;
    store.insert_match(row).await.unwrap();
    let now = test_now();
    let mut g = game(detect::MatchOutcome::Victory, Some(10), now);
    g.session_id = "tab-on-restart".into();
    g.map = Some("Busan".into());
    g.session_created = false;
    g.deferred = Some(fresh_counters(4));
    g.deferred_hero = None;
    g.deferred_imported = true;
    let mut st = session(Some(g), now);
    shutdown_from_signal(
        &mut st,
        &mut None,
        &mut tokio::sync::mpsc::unbounded_channel().1,
        &mut None,
        None,
        &std::sync::Mutex::new(sync::SyncBackoff::default()),
        &store,
        dir.path(),
    )
    .await;
    let rows = store.get_session_snapshots("tab-on-restart").await.unwrap();
    assert!(
        rows.iter().any(|row| row.elims == 4 && row.hero == "Ana"),
        "the update restart must persist the Tab row inside the deadline: {rows:?}"
    );
    assert_eq!(
        stat_tracker::replace::shutdown_time_left(),
        stat_tracker::replace::SHUTDOWN_DEADLINE,
        "the restart does not start the signal clock"
    );
}

#[tokio::test]
async fn shutdown_writes_an_imported_hold_and_does_not_warn() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let mut row = test_match("carried-in", "victory");
    row.map_name = "Busan".into();
    row.game_mode = "Control".into();
    row.hero = "Ana".into();
    row.elims = 9;
    store.insert_match(row).await.unwrap();
    let now = test_now();
    let mut g = game(detect::MatchOutcome::Victory, Some(10), now);
    g.session_id = "carried-in".into();
    g.map = Some("Busan".into());
    g.session_created = false;
    g.deferred = Some(fresh_counters(3));
    g.deferred_hero = None;
    g.deferred_imported = true;
    let mut st = session(Some(g), now);
    close_session_on_shutdown(&mut st, &store, dir.path()).await;
    let rows = store.get_session_snapshots("carried-in").await.unwrap();
    assert!(
        rows.iter().any(|row| row.elims == 3 && row.hero == "Ana"),
        "an imported hold is written, and the hero falls back to the newest row: {rows:?}"
    );
    let note = dir.path().join("debug").join("unrecorded_games.jsonl");
    if let Ok(text) = std::fs::read_to_string(&note) {
        assert!(
            !text.contains("carried-in"),
            "writing the board must not warn that the game had no captures: {text}"
        );
    }
}

#[tokio::test]
async fn a_failed_shutdown_write_keeps_an_imported_hold_for_the_next_start() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let mut row = test_match("held-session-fail", "victory");
    row.map_name = "Busan".into();
    row.game_mode = "Control".into();
    row.hero = "Ana".into();
    row.elims = 9;
    store.insert_match(row).await.unwrap();
    let now = test_now();
    let mut g = game(detect::MatchOutcome::Victory, Some(10), now);
    g.session_id = "held-session-fail".into();
    g.map = Some("Busan".into());
    g.deferred = Some(fresh_counters(4));
    g.deferred_hero = Some("Tracer".into());
    g.deferred_at = Some(Utc::now());
    g.deferred_imported = true;
    let mut st = session(Some(g), now);
    *FAIL_HELD_BOARD_SESSION.lock().unwrap() = Some("held-session-fail".into());
    close_session_on_shutdown(&mut st, &store, dir.path()).await;
    let held = st.active_game.as_ref().expect("the hold stays open");
    assert!(held.deferred_imported);
    assert_eq!(held.deferred.map(|c| c.elims), Some(4));
    let rows = store
        .get_session_snapshots("held-session-fail")
        .await
        .unwrap();
    assert!(
        rows.iter().all(|row| row.elims != 4),
        "a failed write must not drop the hold by storing it: {rows:?}"
    );
    assert!(active_game_path(dir.path()).exists());
    let mut sync_task = None;
    let backoff = std::sync::Arc::new(std::sync::Mutex::new(sync::SyncBackoff::default()));
    let restarted = begin_daemon_session(dir.path(), &mut sync_task, &backoff, &store, None).await;
    let rows = store
        .get_session_snapshots("held-session-fail")
        .await
        .unwrap();
    assert!(
        rows.iter().all(|row| row.elims != 4),
        "a resume within 20 minutes must leave the hold pending: {rows:?}"
    );
    let held = restarted
        .active_game
        .as_ref()
        .expect("the session is still open");
    assert!(held.deferred_imported);
    assert_eq!(held.deferred.map(|c| c.elims), Some(4));
}

#[tokio::test]
async fn a_stale_restart_keeps_a_held_board_as_an_unknown_session() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let mut decided = test_match("already-decided", "victory");
    decided.map_name = "Busan".into();
    decided.game_mode = "Control".into();
    decided.elims = 14;
    store.insert_match(decided).await.unwrap();
    let when = Utc::now() - chrono::Duration::hours(9);
    let stale = PersistedGame {
        rejoin_carry: None,
        replaced_text_fallback_map: None,
        best_guess: None,
        session_id: "next-game".into(),
        outcome: detect::MatchOutcome::Unknown,
        map: None,
        map_source: None,
        map_candidates: Vec::new(),
        session_created: false,
        opened_at: when - chrono::Duration::minutes(20),
        last_activity: when,
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
        deferred: Some(fresh_counters(2)),
        deferred_hero: Some("Tracer".into()),
        deferred_at: Some(when),
        deferred_imported: true,
        progressed_boards: 0,
        baseline_at: None,
        text_fallback_locked: false,
    };
    std::fs::write(
        active_game_path(dir.path()),
        serde_json::to_vec(&stale).unwrap(),
    )
    .unwrap();
    let mut sync_task = None;
    let backoff = std::sync::Arc::new(std::sync::Mutex::new(sync::SyncBackoff::default()));
    let st = begin_daemon_session(dir.path(), &mut sync_task, &backoff, &store, None).await;
    assert!(st.active_game.is_none());
    assert!(!active_game_path(dir.path()).exists());
    let rows = store.get_all_matches().await.unwrap();
    let held = rows
        .iter()
        .find(|row| row.elims == 2)
        .expect("the hold is in local history");
    assert_eq!(held.outcome, "unknown");
    assert_eq!(held.session_id, "next-game");
    assert_eq!(held.hero, "Tracer");
    assert!(held.map_name.is_empty());
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen_upload = std::sync::Arc::clone(&seen);
    try_sync_with(
        &store,
        dir.path(),
        Some("next-game".into()),
        move |matches, _| {
            let seen_upload = std::sync::Arc::clone(&seen_upload);
            async move {
                *seen_upload.lock().unwrap() = matches
                    .into_iter()
                    .map(|row| (row.session_id, row.elims, row.map_name, row.outcome))
                    .collect();
                Ok(upload_ok())
            }
        },
    )
    .await;
    assert_eq!(
        *seen.lock().unwrap(),
        vec![(
            "already-decided".to_string(),
            14,
            "Busan".to_string(),
            "victory".to_string()
        )]
    );
    let kept = store
        .get_all_matches()
        .await
        .unwrap()
        .into_iter()
        .find(|row| row.elims == 2)
        .unwrap();
    assert!(kept.synced, "the unknown hold is kept locally and not sent");
}

/// QA: same map, last capture recent, no result seen: the restart
/// resumes the game, so the next Tab stays on it.
#[test]
fn a_restart_resumes_the_same_recent_game_with_no_result() {
    let p = restart_skeleton(Some("Busan"), chrono::Duration::seconds(40));
    assert_eq!(restart_resume_refusal(&p, None, Utc::now()), None);
    assert!(!first_tab_after_restart_starts_new_game(&p));
}

/// QA: a long gap since the last capture starts a new game, even on the
/// same map, well inside the old 20 minute idle bound.
#[test]
fn a_restart_after_a_long_gap_starts_a_new_game() {
    let gap = chrono::Duration::from_std(RESUME_AFTER_RESTART_MAX_GAP).unwrap()
        + chrono::Duration::seconds(30);
    assert!(gap.to_std().unwrap() < UNFINISHED_SESSION_IDLE);
    let p = restart_skeleton(Some("Busan"), gap);
    assert_eq!(
        restart_resume_refusal(&p, None, Utc::now()),
        Some("previous run stopped too long ago")
    );
    assert!(first_tab_after_restart_starts_new_game(&p));
}

/// A game with a board but no map cannot be matched, so it is not
/// resumed. A clock that moved back is not resumed either.
#[test]
fn a_restart_without_a_map_or_with_a_future_capture_starts_a_new_game() {
    let p = restart_skeleton(None, chrono::Duration::seconds(40));
    assert!(restart_resume_refusal(&p, None, Utc::now()).is_some());
    assert!(first_tab_after_restart_starts_new_game(&p));
    let p = restart_skeleton(Some("Busan"), chrono::Duration::seconds(-120));
    assert!(restart_resume_refusal(&p, None, Utc::now()).is_some());
    assert!(first_tab_after_restart_starts_new_game(&p));
}

/// QA: a result screen was seen before the crash. The restart keeps the
/// game so its result is not lost, and the first Tab past the
/// post-match grace starts a new game. Inside the grace the post-match
/// scoreboard still belongs to it.
#[test]
fn a_restart_after_a_result_screen_starts_a_new_game() {
    let mut p = restart_skeleton(Some("Busan"), chrono::Duration::seconds(30));
    p.result_outcome = Some(detect::MatchOutcome::Victory);
    p.result_seen_at = Some(Utc::now() - chrono::Duration::seconds(100));
    assert!(first_tab_after_restart_starts_new_game(&p));

    p.result_seen_at = Some(Utc::now() - chrono::Duration::seconds(10));
    assert!(
        !first_tab_after_restart_starts_new_game(&p),
        "the post-match scoreboard inside the grace stays on the game"
    );

    let mut p = restart_skeleton(Some("Busan"), chrono::Duration::seconds(100));
    p.outcome = detect::MatchOutcome::Defeat;
    p.outcome_recorded_at = Some(Utc::now() - chrono::Duration::seconds(100));
    assert!(first_tab_after_restart_starts_new_game(&p));
}

/// QA: two real games in a row on the same map stay separate across a
/// crash. Game 1 on Busan saw its result, the tracker restarted, and
/// game 2 on Busan opens its own session; game 2's restart then resumes
/// game 2, not game 1.
#[test]
fn two_games_in_a_row_on_the_same_map_stay_separate_across_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let mut game1 = restart_skeleton(Some("Busan"), chrono::Duration::seconds(150));
    game1.session_id = "game-1".into();
    game1.result_outcome = Some(detect::MatchOutcome::Victory);
    game1.result_seen_at = Some(Utc::now() - chrono::Duration::seconds(150));
    std::fs::write(
        active_game_path(dir.path()),
        serde_json::to_vec(&game1).unwrap(),
    )
    .unwrap();
    let mut st = startup_session(dir.path());
    assert_eq!(
        st.active_game.as_ref().map(|g| g.session_id.as_str()),
        Some("game-1")
    );
    let now = Instant::now();
    assert!(should_start_fresh_session(st.active_game.as_ref(), now));
    let mut game2 = ActiveGame::open_at(
        "game-2".into(),
        detect::MatchOutcome::Unknown,
        Vec::new(),
        now,
    );
    game2.map = Some("Busan".into());
    game2.session_created = true;
    st.active_game = Some(game2);
    assert!(!should_start_fresh_session(st.active_game.as_ref(), now));

    persist_active_game(dir.path(), st.active_game.as_ref());
    let st = startup_session(dir.path());
    assert_eq!(
        st.active_game.as_ref().map(|g| g.session_id.as_str()),
        Some("game-2"),
        "a restart during game 2 resumes game 2"
    );
    assert!(!should_start_fresh_session(
        st.active_game.as_ref(),
        Instant::now()
    ));
}

/// QA (a), 2026-10-15 Rialto: a crash, a rejoin with the scoreboard back
/// at zero, then a few more elims and the Victory. All of it stays one
/// game, the stored stats never go down, and the result lands on it.
#[tokio::test]
async fn a_rejoin_with_reset_stats_on_the_same_map_continues_the_recovered_game() {
    let mut night = Night::new().await;
    let id = rialto_before_crash(&mut night).await;
    night.crash_and_restart().await;
    assert_eq!(night.id(), id, "the restart resumes the Rialto game");

    night.advance(Duration::from_secs(70));
    night
        .tab_once(
            night_counters(0, 0, 0, 0, 0, 0),
            "Ana",
            Some(0),
            Some("Rialto"),
            NIGHT_CLEAN,
        )
        .await;
    assert_eq!(
        night.id(),
        id,
        "the zero board after the rejoin does not split"
    );
    night.advance(Duration::from_secs(50));
    night
        .tab_once(
            night_counters(2, 1, 0, 300, 900, 0),
            "Ana",
            Some(0),
            Some("Rialto"),
            NIGHT_CLEAN,
        )
        .await;
    assert_eq!(
        night.id(),
        id,
        "the next board after the rejoin does not split"
    );
    night.advance(Duration::from_secs(120));
    night
        .tab_once(
            night_counters(5, 3, 1, 1200, 2500, 0),
            "Ana",
            Some(0),
            Some("Rialto"),
            NIGHT_CLEAN,
        )
        .await;
    assert_eq!(night.id(), id);

    night
        .confirmed_without_map(detect::MatchOutcome::Victory)
        .await;
    assert_eq!(night.game().outcome, detect::MatchOutcome::Victory);
    assert_eq!(night.id(), id);

    assert_eq!(
        night.elims(&id).await,
        vec![4, 9, 9, 11, 14],
        "pre-crash plus post-rejoin"
    );
    let rows = night.store.get_all_matches().await.unwrap();
    assert!(
        rows.iter().all(|row| row.session_id == id),
        "one game: {rows:?}"
    );
    let last = night.store.get_session_snapshots(&id).await.unwrap();
    let last = last.last().unwrap();
    assert_eq!((last.deaths, last.damage, last.healing), (6, 4200, 9500));
    assert!(
        night
            .store
            .get_session_snapshots(&id)
            .await
            .unwrap()
            .iter()
            .all(|r| r.outcome == "victory"),
        "the Victory is on the recovered game"
    );
}

/// QA on #293: a crash, a rejoin that continues the game, then a second
/// restart in the same game (an update, the OOM killer or a clean stop).
/// The carry comes back from `active_game.json`, no new rejoin window
/// opens, and the next board adds onto the stored total once, not twice.
#[tokio::test]
async fn a_second_restart_after_a_rejoin_keeps_the_carry_and_does_not_double_stats() {
    let mut night = Night::new().await;
    let id = rialto_before_crash(&mut night).await;
    night.crash_and_restart().await;
    night.advance(Duration::from_secs(70));
    night
        .tab_once(
            night_counters(0, 0, 0, 0, 0, 0),
            "Ana",
            Some(0),
            Some("Rialto"),
            NIGHT_CLEAN,
        )
        .await;
    night.advance(Duration::from_secs(50));
    night
        .tab_once(
            night_counters(2, 1, 0, 300, 900, 0),
            "Ana",
            Some(0),
            Some("Rialto"),
            NIGHT_CLEAN,
        )
        .await;
    assert_eq!(night.id(), id);
    let carry = night.game().rejoin_carry;
    assert!(carry.is_some(), "the rejoin set a carry");

    night.crash_and_restart().await;
    assert_eq!(night.id(), id, "the second restart resumes the game");
    assert_eq!(
        night.game().rejoin_carry,
        carry,
        "the carry survives the second restart"
    );
    assert!(
        night.game().restart_recovered_at.is_none(),
        "no new rejoin window on a game that already has a carry"
    );

    night.advance(Duration::from_secs(40));
    night
        .tab_once(
            night_counters(3, 1, 0, 500, 1200, 0),
            "Ana",
            Some(0),
            Some("Rialto"),
            NIGHT_CLEAN,
        )
        .await;
    assert_eq!(night.id(), id, "the next board stays on the game");
    // The restart resets the harness wall clock, so rows are compared
    // as a set: the values matter here, not their stored order.
    let mut elims = night.elims(&id).await;
    elims.sort_unstable();
    assert_eq!(
        elims,
        vec![4, 9, 9, 11, 12],
        "pre-crash plus post-rejoin, counted once"
    );
    let snaps = night.store.get_session_snapshots(&id).await.unwrap();
    let last = snaps.iter().max_by_key(|row| row.elims).unwrap();
    assert_eq!((last.deaths, last.damage, last.healing), (5, 3500, 8200));
}

/// An `active_game.json` written before `rejoin_carry` existed still
/// loads: the game is resumed with no carry.
#[test]
fn an_old_active_game_json_without_rejoin_carry_still_loads() {
    let p = restart_skeleton(Some("Rialto"), chrono::Duration::seconds(30));
    let mut json = serde_json::to_value(&p).unwrap();
    let obj = json.as_object_mut().unwrap();
    obj.remove("rejoin_carry");
    assert!(!obj.contains_key("rejoin_carry"));
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        active_game_path(dir.path()),
        serde_json::to_vec(&json).unwrap(),
    )
    .unwrap();
    let g = recover_active_game(dir.path()).expect("the old file is resumed");
    assert_eq!(g.session_id, "before-crash");
    assert!(g.rejoin_carry.is_none());
}

/// QA (b): after the crash the next board is on a different map. That
/// is a new game, and nothing is added onto the Rialto game.
#[tokio::test]
async fn a_board_on_a_different_map_after_a_crash_starts_a_new_game() {
    let mut night = Night::new().await;
    let id = rialto_before_crash(&mut night).await;
    night.crash_and_restart().await;
    night.advance(Duration::from_secs(70));
    night
        .tab_once(
            night_counters(0, 0, 0, 0, 0, 0),
            "Ana",
            Some(0),
            Some("Busan"),
            NIGHT_CLEAN,
        )
        .await;
    night.advance(Duration::from_secs(60));
    night
        .tab_once(
            night_counters(1, 1, 0, 200, 500, 0),
            "Ana",
            Some(0),
            Some("Busan"),
            NIGHT_CLEAN,
        )
        .await;
    assert_ne!(night.id(), id, "the Busan boards are a new game");
    assert!(night.game().rejoin_carry.is_none());
    assert_eq!(
        night.elims(&id).await,
        vec![4, 9],
        "nothing added to Rialto"
    );
}

/// QA (c), #289: a result was seen before the crash. The first Tab more
/// than 75 s after that result starts a new game, also on the same map
/// with stats back at zero: the rejoin window does not open for it.
#[tokio::test]
async fn a_restart_after_a_result_still_starts_a_new_game_on_the_first_tab_past_75_s() {
    let mut night = Night::new().await;
    let id = rialto_before_crash(&mut night).await;
    night
        .confirmed_without_map(detect::MatchOutcome::Victory)
        .await;
    night.crash_and_restart().await;
    assert_eq!(
        night.id(),
        id,
        "the result keeps the game for its post-match board"
    );
    assert!(
        night.game().restart_recovered_at.is_none(),
        "no rejoin window after a result"
    );
    night.advance(POST_MATCH_GRACE + Duration::from_secs(5));
    night
        .tab_once(
            night_counters(0, 0, 0, 0, 0, 0),
            "Ana",
            Some(0),
            Some("Rialto"),
            NIGHT_CLEAN,
        )
        .await;
    assert_ne!(night.id(), id, "the next game on Rialto is its own game");
    assert!(night.game().rejoin_carry.is_none());
    assert_eq!(night.elims(&id).await, vec![4, 9]);
}

/// QA (d): past the rejoin window, a stat reset on the same map splits
/// as before (#147).
#[tokio::test]
async fn outside_the_rejoin_window_a_stat_reset_still_splits() {
    let mut night = Night::new().await;
    let id = rialto_before_crash(&mut night).await;
    night.crash_and_restart().await;
    night.advance(REJOIN_CONTINUE_WINDOW + Duration::from_secs(10));
    night
        .tab_once(
            night_counters(0, 0, 0, 0, 0, 0),
            "Ana",
            Some(0),
            Some("Rialto"),
            NIGHT_CLEAN,
        )
        .await;
    night.advance(Duration::from_secs(50));
    night
        .tab_once(
            night_counters(1, 1, 0, 300, 900, 0),
            "Ana",
            Some(0),
            Some("Rialto"),
            NIGHT_CLEAN,
        )
        .await;
    assert_ne!(night.id(), id, "the stat reset splits outside the window");
    assert!(night.game().rejoin_carry.is_none());
    assert_eq!(night.elims(&id).await, vec![4, 9]);
}

/// Without a restart there is no rejoin window: a live stat reset keeps
/// the #147 split.
#[tokio::test]
async fn a_stat_reset_without_a_restart_still_splits() {
    let mut night = Night::new().await;
    let id = rialto_before_crash(&mut night).await;
    night.advance(Duration::from_secs(70));
    night
        .tab_once(
            night_counters(0, 0, 0, 0, 0, 0),
            "Ana",
            Some(0),
            Some("Rialto"),
            NIGHT_CLEAN,
        )
        .await;
    night.advance(Duration::from_secs(50));
    night
        .tab_once(
            night_counters(1, 1, 0, 300, 900, 0),
            "Ana",
            Some(0),
            Some("Rialto"),
            NIGHT_CLEAN,
        )
        .await;
    assert_ne!(night.id(), id);
}

/// The resume window counts from when the previous run stopped (its
/// crash heartbeat), not from its last Tab. An old crash marker or a
/// heartbeat in the future changes nothing.
#[test]
fn the_restart_resume_window_counts_from_the_crash_not_the_last_tab() {
    let now = Utc::now();
    let p = restart_skeleton(Some("Rialto"), chrono::Duration::minutes(8));
    assert!(
        restart_resume_refusal(&p, None, now).is_some(),
        "no marker: last Tab"
    );
    let crash = now - chrono::Duration::seconds(50);
    assert_eq!(
        restart_resume_refusal(&p, Some(crash), now),
        None,
        "crash 50 s ago resumes"
    );
    let old = now - chrono::Duration::hours(3);
    assert!(
        restart_resume_refusal(&p, Some(old), now).is_some(),
        "an old marker changes nothing"
    );
    let future = now + chrono::Duration::minutes(2);
    assert!(
        restart_resume_refusal(&p, Some(future), now).is_some(),
        "a future heartbeat is ignored"
    );
    let late = now - chrono::Duration::minutes(6);
    assert!(
        restart_resume_refusal(&p, Some(late), now).is_some(),
        "crash 6 min ago starts a new game"
    );
    let stale = restart_skeleton(Some("Rialto"), chrono::Duration::minutes(25));
    assert!(
        active_game_from_persisted(stale, Some(crash)).is_none(),
        "the 20 minute idle bound on the last capture still holds"
    );
}

/// Startup opens the rejoin window on a restored unfinished game, and
/// not on one with a result.
#[tokio::test]
async fn startup_opens_the_rejoin_window_only_on_an_unfinished_game() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let p = restart_skeleton(Some("Rialto"), chrono::Duration::seconds(30));
    let st = begin_from_persisted(dir.path(), &store, p).await;
    assert!(
        st.active_game
            .as_ref()
            .unwrap()
            .restart_recovered_at
            .is_some()
    );

    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let mut p = restart_skeleton(Some("Rialto"), chrono::Duration::seconds(30));
    p.result_outcome = Some(detect::MatchOutcome::Victory);
    p.result_seen_at = Some(Utc::now() - chrono::Duration::seconds(10));
    let st = begin_from_persisted(dir.path(), &store, p).await;
    assert!(
        st.active_game
            .as_ref()
            .unwrap()
            .restart_recovered_at
            .is_none()
    );
}

#[tokio::test]
async fn a_stale_fresh_hold_does_not_keep_the_previous_map() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let when = Utc::now() - chrono::Duration::hours(9);
    begin_from_persisted(
        dir.path(),
        &store,
        persisted_with_hold(
            "busan-game",
            detect::MatchOutcome::Victory,
            Some("Busan"),
            false,
            3,
            when,
        ),
    )
    .await;
    let rows = store.get_all_matches().await.unwrap();
    let held = rows
        .iter()
        .find(|row| row.elims == 3)
        .expect("the fresh hold is archived");
    assert_ne!(held.session_id, "busan-game");
    assert!(held.map_name.is_empty(), "fresh hold map: {held:?}");
    assert_eq!(held.outcome, "unknown");
    store
        .apply_command(&storage::StoreCommand::SetOutcome {
            session_id: held.session_id.clone(),
            outcome: "victory".into(),
        })
        .await
        .unwrap();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen_upload = std::sync::Arc::clone(&seen);
    try_sync_with(&store, dir.path(), None, move |matches, _| {
        let seen_upload = std::sync::Arc::clone(&seen_upload);
        async move {
            *seen_upload.lock().unwrap() = matches
                .into_iter()
                .map(|row| (row.session_id, row.elims, row.map_name, row.outcome))
                .collect();
            Ok(upload_ok())
        }
    })
    .await;
    let uploaded = seen.lock().unwrap().clone();
    assert!(
        uploaded.is_empty(),
        "a later GUI result still does not send a blank map: {uploaded:?}"
    );
    let kept = store
        .get_all_matches()
        .await
        .unwrap()
        .into_iter()
        .find(|row| row.elims == 3)
        .unwrap();
    assert!(!kept.synced, "the blank map stays unsynced for a pick");
    assert!(
        kept.map_name.is_empty(),
        "the fresh hold must not become Busan"
    );
    assert_eq!(kept.outcome, "victory");
}

#[tokio::test]
async fn a_stale_imported_hold_keeps_the_session_outcome() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let when = Utc::now() - chrono::Duration::hours(9);
    begin_from_persisted(
        dir.path(),
        &store,
        persisted_with_hold(
            "decided-session",
            detect::MatchOutcome::Victory,
            Some("Busan"),
            true,
            6,
            when,
        ),
    )
    .await;
    let held = store
        .get_all_matches()
        .await
        .unwrap()
        .into_iter()
        .find(|row| row.elims == 6)
        .expect("the imported hold is archived");
    assert_eq!(held.session_id, "decided-session");
    assert_eq!(held.map_name, "Busan");
    assert_eq!(held.outcome, "victory");
}

#[tokio::test]
async fn a_resume_within_twenty_minutes_leaves_an_imported_hold_pending() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let when = Utc::now() - chrono::Duration::minutes(5);
    let st = begin_from_persisted(
        dir.path(),
        &store,
        persisted_with_hold(
            "still-open",
            detect::MatchOutcome::Unknown,
            Some("Busan"),
            true,
            4,
            when,
        ),
    )
    .await;
    let held = st.active_game.as_ref().expect("the session resumes");
    assert!(held.deferred_imported);
    assert_eq!(held.deferred.map(|c| c.elims), Some(4));
    let rows = store.get_all_matches().await.unwrap();
    assert!(
        rows.iter().all(|row| row.elims != 4),
        "resume must not write the hold: {rows:?}"
    );
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
