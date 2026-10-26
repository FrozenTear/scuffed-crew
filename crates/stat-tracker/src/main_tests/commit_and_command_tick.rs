//! Daemon tests: commit_and_command_tick. See `mod.rs`.

use super::*;

#[tokio::test]
async fn commit_capture_rows_writes_the_current_row_then_the_carried_board() {
    // Bounded by name: the items that used to sit between this fn and
    // `handle_capture` start moving out of main.rs in 0.6 S3.
    let body = fn_source("commit_capture_rows");
    let insert = body.find("insert_match(").expect("insert");
    let carried = body
        .find("write_carried_after_current(")
        .expect("carried write");
    assert!(
        insert < carried,
        "the carried board is written after the insert"
    );
    assert!(body.contains("should_write_carried(true,"));

    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let (mut staged, mut parsed) =
        staged_row("carry-order", "Busan", Some(boundary::MapSource::TopBar));
    assert_eq!(
        prepare_capture_row(
            &mut parsed,
            &mut staged,
            Some("Busan"),
            Some("BUSAN"),
            "",
            true,
        ),
        PreparedRow::Store
    );
    let current = Utc::now();
    staged.carried = Some(fresh_counters(2));
    staged.carried_hero = Some("Tracer".into());
    staged.carried_at = current - chrono::Duration::seconds(40);
    parsed.played_at = SurrealDatetime::from(current);
    parsed.elims = 14;
    commit_capture_rows(
        &store,
        dir.path(),
        &staged,
        &parsed,
        SurrealDatetime::from(current),
    )
    .await
    .unwrap();
    let rows = store.get_session_snapshots("carry-order").await.unwrap();
    assert!(
        rows.iter()
            .any(|row| row.elims == 14 && row.game_mode == "Control")
    );
    assert!(
        rows.iter()
            .any(|row| row.elims == 2 && row.hero == "Tracer")
    );
    store.export_snapshot(dir.path()).await.unwrap();
    let snap = storage::read_snapshot(dir.path()).unwrap();
    let session = snap
        .sessions
        .iter()
        .find(|session| session.session_id == "carry-order")
        .unwrap();
    let stamp: chrono::DateTime<Utc> = session.last_capture_at.into();
    assert!(
        stamp + chrono::Duration::seconds(2) >= current,
        "a carried board must not rewind last_capture_at: {stamp} vs {current}"
    );
}

#[tokio::test]
async fn append_capture_keeps_the_newer_session_stamp() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let current = Utc::now();
    store
        .create_session(&storage::MatchSession {
            session_id: "stamp".into(),
            hero: "Ana".into(),
            map_name: "Busan".into(),
            role: "Support".into(),
            started_at: SurrealDatetime::from(current),
            last_capture_at: SurrealDatetime::from(current),
            capture_count: 1,
            final_outcome: "victory".into(),
        })
        .await
        .unwrap();
    store
        .append_capture(
            "stamp",
            SurrealDatetime::from(current - chrono::Duration::minutes(5)),
            "victory",
        )
        .await
        .unwrap();
    store.export_snapshot(dir.path()).await.unwrap();
    let snap = storage::read_snapshot(dir.path()).unwrap();
    let stamp: chrono::DateTime<Utc> = snap.sessions[0].last_capture_at.into();
    assert!(stamp + chrono::Duration::seconds(2) >= current);
}

#[tokio::test]
async fn set_session_map_updates_game_mode() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let mut row = test_match("mode-fix", "victory");
    row.map_name = "Dorado".into();
    row.game_mode = "Escort".into();
    store.insert_match(row).await.unwrap();
    store.set_session_map("mode-fix", "Busan").await.unwrap();
    let rows = store.get_session_snapshots("mode-fix").await.unwrap();
    assert_eq!(rows[0].map_name, "Busan");
    assert_eq!(rows[0].game_mode, "Control");
    assert_eq!(
        parse::uploaded_game_mode(rows[0].display_map_name(), &rows[0].game_mode),
        "Control"
    );
}

#[tokio::test]
async fn end_screen_split_log_uses_the_open_session_at_the_call_site() {
    LAST_END_SCREEN_SPLIT.with(|slot| slot.set(None));
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let now = test_now();
    let mut g = game(detect::MatchOutcome::Unknown, None, now);
    g.pending_boundary = true;
    g.map = Some("Busan".into());
    g.map_source = Some(boundary::MapSource::TopBar);
    let mut st = session(Some(g), now);
    apply_poll_decision(
        &mut st,
        &store,
        dir.path(),
        boundary::PollDecision::Open(boundary::OpenNew {
            reason: boundary::CloseReason::EndScreenMap,
            seal_outcome: Some(detect::MatchOutcome::Victory),
            new_outcome: detect::MatchOutcome::Unknown,
            new_map: Some("Ilios".into()),
            candidates: Vec::new(),
            new_result: None,
        }),
        now,
        false,
    )
    .await;
    let logged = LAST_END_SCREEN_SPLIT.with(|slot| slot.take()).unwrap();
    assert!(logged.contains("armed=true"), "{logged}");
    assert!(logged.contains("accolade_map=Ilios"), "{logged}");
}

#[tokio::test]
async fn set_outcome_on_the_command_tick_uploads_the_open_session() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    store
        .insert_match(unfinished_row("open-cmd"))
        .await
        .unwrap();
    let now = test_now();
    let mut g = game(detect::MatchOutcome::Unknown, None, now);
    g.session_id = "open-cmd".into();
    g.map = Some("Busan".into());
    let mut st = session(Some(g), now);
    storage::queue_command(
        dir.path(),
        &storage::StoreCommand::SetOutcome {
            session_id: "open-cmd".into(),
            outcome: "victory".into(),
        },
    )
    .unwrap();
    let (port, rx) = spawn_upload_sink();
    let client = loopback_client(port);
    let backoff = std::sync::Arc::new(std::sync::Mutex::new(sync::SyncBackoff::default()));
    let mut sync_task = None;
    apply_queued_commands(
        &mut st,
        &store,
        dir.path(),
        &mut sync_task,
        &backoff,
        Some(&client),
    )
    .await;
    assert!(st.active_game.as_ref().unwrap().finished());
    let handle = sync_task.expect("SetOutcome schedules an upload");
    handle.await.unwrap();
    let body = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    assert!(body.contains("open-cmd"), "{body}");
    assert!(body.contains("victory"), "{body}");
    assert!(body.contains("\"edited\":true"), "{body}");
}

/// O3: a GUI delete sends its tombstone on the same command tick. It
/// used to wait for the next Tab sync or a daemon restart (owner's
/// journal: delete applied 15:47, tombstone sent 18:27), so the game
/// stayed on the site for hours after it vanished from the app.
#[tokio::test]
async fn a_delete_command_sends_its_tombstone_on_the_same_tick() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let mut row = test_match("delete-me", "victory");
    row.map_name = "Busan".into();
    row.game_mode = "Control".into();
    store.insert_match(row).await.unwrap();
    let claims = storage::SyncClaim::capture(&store.get_unsynced().await.unwrap());
    store.mark_synced(&claims).await.unwrap();
    let now = test_now();
    let mut st = session(None, now);
    storage::queue_command(
        dir.path(),
        &storage::StoreCommand::DeleteSession {
            session_id: "delete-me".into(),
        },
    )
    .unwrap();
    let (port, rx) = spawn_upload_sink();
    let client = loopback_client(port);
    let backoff = std::sync::Arc::new(std::sync::Mutex::new(sync::SyncBackoff::default()));
    let mut sync_task = None;
    apply_queued_commands(
        &mut st,
        &store,
        dir.path(),
        &mut sync_task,
        &backoff,
        Some(&client),
    )
    .await;
    let handle = sync_task.expect("a delete schedules an upload");
    handle.await.unwrap();
    let body = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    assert!(
        body.contains("\"deleted_sessions\":[\"delete-me\"]"),
        "{body}"
    );
    assert!(
        store.get_pending_tombstones().await.unwrap().is_empty(),
        "the acknowledged tombstone is cleared"
    );
}

/// DATA-4: a game uploaded as Control and later re-read as a Deathmatch
/// map must be deleted on the site. Before the fix the new snapshot was
/// only marked synced locally and the Control copy stayed online.
#[tokio::test]
async fn a_game_reclassified_as_untracked_sends_a_server_delete() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    store.insert_match(tracked_row("reclass", 0)).await.unwrap();
    let first = try_sync_with(&store, dir.path(), None, |matches, tombstones| async move {
        assert_eq!(matches.len(), 1);
        assert!(tombstones.is_empty());
        Ok(upload_ok())
    })
    .await;
    assert!(matches!(first, sync::SyncAttempt::Uploaded));

    store
        .insert_match(deathmatch_row("reclass", 60))
        .await
        .unwrap();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(None));
    let seen_upload = std::sync::Arc::clone(&seen);
    try_sync_with(&store, dir.path(), None, move |matches, tombstones| {
        let seen_upload = std::sync::Arc::clone(&seen_upload);
        async move {
            *seen_upload.lock().unwrap() = Some((matches.len(), tombstones));
            Ok(upload_ok())
        }
    })
    .await;
    assert_eq!(
        seen.lock().unwrap().clone(),
        Some((0, vec!["reclass".to_string()])),
        "the reclassified game is deleted on the site and not re-uploaded"
    );
    assert!(store.get_unsynced().await.unwrap().is_empty());
    assert!(store.get_pending_tombstones().await.unwrap().is_empty());
}

/// N1: a DATA-4 tombstone whose upload failed, then a tracked snapshot
/// of the same game (the map corrected back). The next request must carry
/// the game and not its stale tombstone, or the server stores the game
/// and deletes it in the same request and nothing re-sends it.
#[tokio::test]
async fn a_reupload_drops_the_stale_tombstone_of_its_session() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    store.insert_match(tracked_row("flip", 0)).await.unwrap();
    try_sync_with(
        &store,
        dir.path(),
        None,
        |_m, _t| async move { Ok(upload_ok()) },
    )
    .await;
    store
        .insert_match(deathmatch_row("flip", 60))
        .await
        .unwrap();
    let failed = try_sync_with(&store, dir.path(), None, |_m, tombstones| async move {
        assert_eq!(tombstones, vec!["flip".to_string()]);
        Err(sync::SyncUploadError {
            status: Some(503),
            ..sync::SyncUploadError::plain("site down")
        })
    })
    .await;
    assert!(!matches!(failed, sync::SyncAttempt::Uploaded));
    assert_eq!(
        store.get_pending_tombstones().await.unwrap(),
        vec!["flip".to_string()]
    );

    store.insert_match(tracked_row("flip", 120)).await.unwrap();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(None));
    let seen_upload = std::sync::Arc::clone(&seen);
    try_sync_with(&store, dir.path(), None, move |matches, tombstones| {
        let seen_upload = std::sync::Arc::clone(&seen_upload);
        async move {
            let ids: Vec<String> = matches.into_iter().map(|m| m.session_id).collect();
            *seen_upload.lock().unwrap() = Some((ids, tombstones));
            Ok(upload_ok())
        }
    })
    .await;
    assert_eq!(
        seen.lock().unwrap().clone(),
        Some((vec!["flip".to_string()], Vec::<String>::new())),
        "the game goes up without its stale tombstone"
    );
    assert!(store.get_pending_tombstones().await.unwrap().is_empty());
}

#[test]
fn split_superseded_tombstones_keeps_unrelated_deletes() {
    let rows = vec![tracked_row("up", 0), tracked_row("", 0)];
    let (superseded, rest) = split_superseded_tombstones(
        vec!["gone".into(), "up".into(), String::new(), "other".into()],
        &rows,
    );
    assert_eq!(superseded, vec!["up".to_string()]);
    assert_eq!(
        rest,
        vec!["gone".to_string(), String::new(), "other".to_string()]
    );
}

/// A game that was never a tracked read sends no delete, so a
/// Deathmatch evening does not post a request on every sync.
#[tokio::test]
async fn a_plain_deathmatch_game_sends_no_server_delete() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    store.insert_match(deathmatch_row("dm", 0)).await.unwrap();
    store.insert_match(deathmatch_row("dm", 30)).await.unwrap();
    let called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let called_upload = std::sync::Arc::clone(&called);
    let attempt = try_sync_with(&store, dir.path(), None, move |_matches, _tombstones| {
        let called_upload = std::sync::Arc::clone(&called_upload);
        async move {
            called_upload.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(upload_ok())
        }
    })
    .await;
    assert!(
        !called.load(std::sync::atomic::Ordering::SeqCst),
        "a deathmatch-only game must not call upload"
    );
    assert!(matches!(attempt, sync::SyncAttempt::NoServerCall));
    assert!(store.get_pending_tombstones().await.unwrap().is_empty());
    assert!(store.get_unsynced().await.unwrap().is_empty());
}

#[test]
fn split_local_only_lets_the_newest_row_decide_the_session() {
    let rows = vec![
        // Session a: Control, then re-read as Deathmatch -> retired.
        tracked_row("a", 0),
        deathmatch_row("a", 10),
        // Session b: a stray Deathmatch read, then Control -> uploaded.
        deathmatch_row("b", 0),
        tracked_row("b", 10),
        // Legacy rows with no session id are judged one by one.
        deathmatch_row("", 0),
        tracked_row("", 5),
    ];
    let split = split_local_only(rows);
    assert_eq!(split.retire, vec!["a".to_string()]);
    let local: Vec<(&str, &str)> = split
        .local_only
        .iter()
        .map(|r| (r.session_id.as_str(), r.map_name.as_str()))
        .collect();
    assert_eq!(
        local,
        vec![
            ("a", "Busan"),
            ("a", "Château Guillard"),
            ("b", "Château Guillard"),
            ("", "Château Guillard"),
        ]
    );
    let send: Vec<(&str, &str)> = split
        .send
        .iter()
        .map(|r| (r.session_id.as_str(), r.map_name.as_str()))
        .collect();
    assert_eq!(send, vec![("b", "Busan"), ("", "Busan")]);
}

#[test]
fn session_may_be_on_site_counts_raw_and_edited_reads() {
    assert!(!session_may_be_on_site(&[deathmatch_row("s", 0)]));
    assert!(session_may_be_on_site(&[
        deathmatch_row("s", 0),
        tracked_row("s", 1)
    ]));
    // A GUI edit to a Deathmatch map: the upload was the raw Control read.
    let mut edited = tracked_row("s", 0);
    edited.corrected_map_name = Some("Château Guillard".into());
    assert!(session_may_be_on_site(&[edited]));
}

#[test]
fn idle_sync_due_needs_the_interval_an_open_backoff_and_pending_work() {
    let due = IDLE_SYNC_EVERY;
    let early = IDLE_SYNC_EVERY - std::time::Duration::from_secs(1);
    assert!(idle_sync_due(due, false, true, true));
    assert!(!idle_sync_due(early, false, true, true), "too soon");
    assert!(!idle_sync_due(due, true, true, true), "never a second sync");
    assert!(!idle_sync_due(due, false, false, true), "backoff closed");
    assert!(!idle_sync_due(due, false, true, false), "nothing to send");
}

/// DATA-2: a game left unsent (site down at the last game close) goes
/// out on the idle tick without a Tab, a GUI command, or a restart.
#[tokio::test]
async fn idle_tick_retries_an_unsent_game() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    store
        .insert_match(tracked_row("left-over", 0))
        .await
        .unwrap();
    let (port, rx) = spawn_upload_sink();
    let client = loopback_client(port);
    let backoff = std::sync::Arc::new(std::sync::Mutex::new(sync::SyncBackoff::default()));
    let mut sync_task = None;
    let now = Instant::now();

    // Inside the interval: nothing starts.
    let mut checked = now;
    maybe_spawn_idle_sync(
        &mut sync_task,
        &backoff,
        &store,
        Some(&client),
        dir.path(),
        &mut checked,
        now,
    )
    .await;
    assert!(sync_task.is_none());

    let later = now + IDLE_SYNC_EVERY;
    maybe_spawn_idle_sync(
        &mut sync_task,
        &backoff,
        &store,
        Some(&client),
        dir.path(),
        &mut checked,
        later,
    )
    .await;
    assert_eq!(checked, later);
    sync_task
        .take()
        .expect("the idle tick schedules the upload")
        .await
        .unwrap();
    let body = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    assert!(body.contains("left-over"), "{body}");
    assert!(store.get_unsynced().await.unwrap().is_empty());

    // Nothing left: the next idle check starts no task.
    maybe_spawn_idle_sync(
        &mut sync_task,
        &backoff,
        &store,
        Some(&client),
        dir.path(),
        &mut checked,
        later + IDLE_SYNC_EVERY,
    )
    .await;
    assert!(sync_task.is_none());
}

#[tokio::test]
async fn schedule_store_sync_joins_the_previous_task() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = std::sync::Arc::clone(&done);
    let previous = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
    });
    let backoff = std::sync::Arc::new(std::sync::Mutex::new(sync::SyncBackoff::default()));
    let mut sync_task = Some(previous);
    schedule_store_sync(&mut sync_task, &backoff, &store, None, dir.path(), None);
    tokio::time::sleep(std::time::Duration::from_millis(40)).await;
    assert!(
        !sync_task.as_ref().unwrap().is_finished(),
        "the new upload must still be joining the previous task"
    );
    sync_task.take().unwrap().await.unwrap();
    assert!(done.load(std::sync::atomic::Ordering::SeqCst));
}

#[tokio::test]
async fn scheduled_sync_feeds_the_backoff_clock() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let mut row = test_match("busan-fail", "victory");
    row.map_name = "Busan".into();
    row.game_mode = "Control".into();
    store.insert_match(row).await.unwrap();
    let client = loopback_client(9);
    let backoff = std::sync::Arc::new(std::sync::Mutex::new(sync::SyncBackoff::default()));
    let mut sync_task = None;
    schedule_store_sync(
        &mut sync_task,
        &backoff,
        &store,
        Some(&client),
        dir.path(),
        None,
    );
    sync_task.take().unwrap().await.unwrap();
    assert!(backoff.lock().unwrap().failures() >= 1);
}

#[tokio::test]
async fn command_tick_settles_only_the_closed_session() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    store
        .insert_match(unfinished_row("closed-one"))
        .await
        .unwrap();
    store
        .insert_match(unfinished_row("still-open"))
        .await
        .unwrap();
    let now = test_now();
    let mut st = session(None, now);
    st.settle_closed_unknown = Some("closed-one".into());
    let client = loopback_client(9);
    let backoff = std::sync::Arc::new(std::sync::Mutex::new(sync::SyncBackoff::default()));
    let mut sync_task = None;
    schedule_settled_unknown(
        &mut st,
        &mut sync_task,
        &backoff,
        &store,
        Some(&client),
        dir.path(),
    );
    assert!(st.settle_closed_unknown.is_none());
    sync_task.take().unwrap().await.unwrap();
    let rows = store.get_all_matches().await.unwrap();
    assert!(
        rows.iter()
            .any(|row| row.session_id == "closed-one" && row.synced)
    );
    assert!(
        rows.iter()
            .any(|row| row.session_id == "still-open" && !row.synced)
    );
}

#[tokio::test]
async fn an_unfinished_game_closes_after_six_hours() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    store
        .insert_match(unfinished_row("long-busan"))
        .await
        .unwrap();
    let now = test_now();
    let mut g = game(detect::MatchOutcome::Unknown, None, now);
    g.session_id = "long-busan".into();
    g.map = Some("Busan".into());
    g.last_activity = now - UNFINISHED_SESSION_MAX;
    let mut st = session(Some(g), now);
    let closed = close_quiet_session_and_sync(
        &mut st,
        &store,
        dir.path(),
        now,
        finished_game_close_after(config::FINISHED_GAME_CLOSE_DEFAULT_SECS),
        || async {},
    )
    .await;
    assert!(closed);
    assert!(st.active_game.is_none());
    assert_eq!(st.settle_closed_unknown.as_deref(), Some("long-busan"));
}

/// Replays 2026-10-17: Oasis ends in Victory, a Tab 81s later lands on
/// the rank screen and its capture is rejected, then nothing else
/// happens. Oasis must upload at that Tab, and the empty game the Tab
/// opened must close after 20 minutes idle with nothing uploaded for it.
#[tokio::test]
async fn last_night_replay_uploads_oasis_at_the_late_tab_and_closes_the_empty_game() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let mut row = test_match("oasis", "victory");
    row.map_name = "Oasis".into();
    row.game_mode = "Control".into();
    row.hero = "Wrecking Ball".into();
    row.role = "Tank".into();
    store.insert_match(row).await.unwrap();
    let tab_at = test_now();
    let mut g = game(detect::MatchOutcome::Victory, Some(81), tab_at);
    g.session_id = "oasis".into();
    g.map = Some("Oasis".into());
    g.last_activity = tab_at - Duration::from_secs(81);
    let mut st = session(Some(g), tab_at);
    persist_active_game(dir.path(), st.active_game.as_ref());

    let uploaded = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let syncs = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let upload_into = |seen: std::sync::Arc<std::sync::Mutex<Vec<String>>>| {
        move |matches: Vec<storage::PersonalMatch>, _| {
            seen.lock()
                .unwrap()
                .extend(matches.into_iter().map(|row| row.session_id));
            async { Ok(upload_ok()) }
        }
    };

    // The 81s Tab: past the 75s grace, so it closes Oasis and opens a game.
    let opened = {
        let (seen, syncs) = (
            std::sync::Arc::clone(&uploaded),
            std::sync::Arc::clone(&syncs),
        );
        let store = &store;
        let path = dir.path();
        open_fresh_for_tab_and_sync(&mut st, store, path, tab_at, move || async move {
            syncs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            try_sync_with(store, path, None, upload_into(seen)).await;
        })
        .await
    };
    assert!(opened, "a Tab 81s after the result opens the next game");
    assert_eq!(syncs.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(*uploaded.lock().unwrap(), vec!["oasis".to_string()]);
    let oasis = store.get_all_matches().await.unwrap();
    assert!(
        oasis
            .iter()
            .all(|row| row.session_id != "oasis" || row.synced),
        "Oasis is uploaded at the Tab, not hours later"
    );

    // The capture of that Tab is rejected: not a scoreboard.
    let empty = st.active_game.as_ref().unwrap().session_id.clone();
    assert_ne!(empty, "oasis");
    let reject_at = tab_at + Duration::from_secs(1);
    let recorded = apply_capture_report(
        &mut st,
        &store,
        dir.path(),
        &empty,
        Ok(rejected_report(&empty)),
        reject_at,
    )
    .await;
    assert!(!recorded);
    let g = st.active_game.as_ref().expect("the empty game is open");
    assert!(!g.session_created);
    assert!(is_empty_tab_game(g));

    // The quiet timer: still open at 19 minutes, closed at 20.
    uploaded.lock().unwrap().clear();
    let quiet = finished_game_close_after(config::FINISHED_GAME_CLOSE_DEFAULT_SECS);
    let early = tab_at + Duration::from_secs(19 * 60);
    assert!(
        !close_quiet_session_and_sync(&mut st, &store, dir.path(), early, quiet, || async {}).await
    );
    assert_eq!(
        st.active_game.as_ref().map(|g| g.session_id.as_str()),
        Some(empty.as_str())
    );
    let idle_at = reject_at + UNFINISHED_SESSION_IDLE;
    let closed = {
        let seen = std::sync::Arc::clone(&uploaded);
        let store = &store;
        let path = dir.path();
        close_quiet_session_and_sync(&mut st, store, path, idle_at, quiet, move || async move {
            try_sync_with(store, path, None, upload_into(seen)).await;
        })
        .await
    };
    assert!(closed, "the empty game closes after 20 minutes idle");
    assert!(st.active_game.is_none());
    assert!(!active_game_path(dir.path()).exists());
    assert!(st.settle_closed_unknown.is_none());
    assert!(
        uploaded.lock().unwrap().iter().all(|sid| sid != &empty),
        "nothing is uploaded for the empty game"
    );
    let rows = store.get_all_matches().await.unwrap();
    assert!(rows.iter().all(|row| row.session_id != empty));
}

#[test]
fn only_a_tab_that_closes_a_finished_recorded_game_uploads() {
    let now = test_now();
    // Inside the 75s grace the Tab still belongs to the finished game.
    let inside = game(detect::MatchOutcome::Victory, Some(60), now);
    assert!(!tab_closes_a_recorded_game(Some(&inside), now));
    // Past grace: closes a finished game with rows, so upload it.
    let past = game(detect::MatchOutcome::Victory, Some(81), now);
    assert!(tab_closes_a_recorded_game(Some(&past), now));
    // A finished game that never stored a row has nothing to send.
    let mut no_rows = game(detect::MatchOutcome::Defeat, Some(81), now);
    no_rows.session_created = false;
    assert!(!tab_closes_a_recorded_game(Some(&no_rows), now));
    // An unfinished game is held locally, not sent.
    let mut unfinished = game(detect::MatchOutcome::Unknown, None, now);
    unfinished.last_activity = now - UNFINISHED_SESSION_IDLE - Duration::from_secs(1);
    assert!(!tab_closes_a_recorded_game(Some(&unfinished), now));
    assert!(!tab_closes_a_recorded_game(None, now));
}

#[test]
fn only_an_empty_tab_game_closes_after_twenty_minutes() {
    let now = test_now();
    let quiet = finished_game_close_after(config::FINISHED_GAME_CLOSE_DEFAULT_SECS);
    let idle = |mut g: ActiveGame| {
        g.last_activity = now - UNFINISHED_SESSION_IDLE;
        g
    };
    let empty = || {
        let mut g = game(detect::MatchOutcome::Unknown, None, now);
        g.session_created = false;
        g
    };
    assert_eq!(
        quiet_close_reason(&idle(empty()), now, quiet),
        Some(EMPTY_IDLE_REASON)
    );
    let mut recent = empty();
    recent.last_activity = now - UNFINISHED_SESSION_IDLE + Duration::from_secs(1);
    assert_eq!(quiet_close_reason(&recent, now, quiet), None);
    // A game with stored rows waits for its result as before.
    let stored = game(detect::MatchOutcome::Unknown, None, now);
    assert_eq!(quiet_close_reason(&idle(stored), now, quiet), None);
    // A start-screen game still waiting for its first board stays open.
    let mut start = empty();
    start.awaiting_first_board = true;
    assert_eq!(quiet_close_reason(&idle(start), now, quiet), None);
    // A held board or a result hint is not empty.
    let mut held = empty();
    held.deferred = Some(fresh_counters(2));
    assert_eq!(quiet_close_reason(&idle(held), now, quiet), None);
    let mut hinted = empty();
    hinted.result_mark = Some(ResultMark {
        outcome: detect::MatchOutcome::Victory,
        confirmed: false,
        seen_at: now - UNFINISHED_SESSION_IDLE,
    });
    assert_eq!(quiet_close_reason(&idle(hinted), now, quiet), None);
}
