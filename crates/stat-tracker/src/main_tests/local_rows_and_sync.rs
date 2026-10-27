//! Daemon tests: local_rows_and_sync. See `mod.rs`.

use super::*;

#[tokio::test]
async fn blank_map_and_unknown_hero_stay_local_until_picked() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let mut missed_map = test_match("held-map", "victory");
    missed_map.map_name.clear();
    missed_map.game_mode.clear();
    store.insert_match(missed_map).await.unwrap();
    let mut unknown_hero = test_match("held-hero", "victory");
    unknown_hero.hero = "Unknown".into();
    store.insert_match(unknown_hero).await.unwrap();

    let calls = std::sync::Arc::new(std::sync::Mutex::new(0u32));
    let calls_bg = std::sync::Arc::clone(&calls);
    let attempt = try_sync_with(&store, dir.path(), None, move |_matches, _| {
        *calls_bg.lock().unwrap() += 1;
        async { Ok(upload_ok()) }
    })
    .await;
    assert!(matches!(attempt, sync::SyncAttempt::NoServerCall));
    assert_eq!(*calls.lock().unwrap(), 0);
    assert!(
        store
            .get_all_matches()
            .await
            .unwrap()
            .iter()
            .all(|row| !row.synced)
    );

    store
        .edit_match(
            "held-map",
            &storage::MatchEdit {
                map_name: Some("King's Row".into()),
                ..storage::MatchEdit::default()
            },
        )
        .await
        .unwrap();
    store
        .edit_match(
            "held-hero",
            &storage::MatchEdit {
                hero: Some("Ana".into()),
                ..storage::MatchEdit::default()
            },
        )
        .await
        .unwrap();

    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen_bg = std::sync::Arc::clone(&seen);
    let attempt = try_sync_with(&store, dir.path(), None, move |matches, _| {
        let seen_bg = std::sync::Arc::clone(&seen_bg);
        async move {
            seen_bg.lock().unwrap().extend(matches);
            Ok(upload_ok())
        }
    })
    .await;
    assert!(matches!(attempt, sync::SyncAttempt::Uploaded));
    let got = seen.lock().unwrap().clone();
    let body = serde_json::to_string(&sync::upload_request(&got, &[])).unwrap();
    assert!(!body.contains("\"map_name\":\"\""));
    assert!(!body.contains("\"game_mode\":\"\""));
    assert!(!body.contains("Unknown"));
    let parsed: scuffed_types::api::StatsUploadRequest = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed.matches.len(), 2);
    assert!(
        parsed
            .matches
            .iter()
            .any(|m| m.map_name == "King's Row" && m.game_mode == "Hybrid" && m.hero == "Ana")
    );
    assert!(
        parsed
            .matches
            .iter()
            .any(|m| m.map_name == "Busan" && m.game_mode == "Control" && m.hero == "Ana")
    );
    assert!(
        store
            .get_all_matches()
            .await
            .unwrap()
            .iter()
            .all(|row| row.synced)
    );
}

#[tokio::test]
async fn held_game_stays_unsynced_while_a_ready_game_syncs() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    store
        .insert_match(test_match("ready-game", "victory"))
        .await
        .unwrap();
    let mut held = test_match("held-game", "victory");
    held.map_name.clear();
    held.game_mode.clear();
    store.insert_match(held).await.unwrap();

    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen_bg = std::sync::Arc::clone(&seen);
    let attempt = try_sync_with(&store, dir.path(), None, move |matches, _| {
        let seen_bg = std::sync::Arc::clone(&seen_bg);
        async move {
            seen_bg.lock().unwrap().extend(matches);
            Ok(upload_ok())
        }
    })
    .await;
    assert!(matches!(attempt, sync::SyncAttempt::Uploaded));
    let got = seen.lock().unwrap().clone();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].session_id, "ready-game");
    let body = serde_json::to_string(&sync::upload_request(&got, &[])).unwrap();
    assert!(!body.contains("\"map_name\":\"\""));

    let rows = store.get_all_matches().await.unwrap();
    let ready = rows
        .iter()
        .find(|row| row.session_id == "ready-game")
        .unwrap();
    let held = rows
        .iter()
        .find(|row| row.session_id == "held-game")
        .unwrap();
    assert!(ready.synced, "the ready game is marked synced");
    assert!(!held.synced, "the held game stays unsynced");
    assert!(held.map_name.is_empty());
}

/// Shadow mode is log only: the same capture stored and uploaded with the
/// flag off and on gives byte-identical stored rows and upload body. The
/// hand-off below is the one the accept path runs after `stage_capture`.
#[tokio::test]
async fn shadow_flag_on_and_off_store_and_upload_identical_rows() {
    async fn run(flag_on: bool) -> (String, String, bool) {
        let dir = tempfile::tempdir().unwrap();
        let cfg = config::Config {
            data_dir: dir.path().to_path_buf(),
            shadow_recognizer: flag_on,
            reader: config::ReaderSetting::OcrV1,
            ..config::Config::default()
        };
        let worker = shadow::worker::ShadowWorker::start_if_enabled(&cfg);
        assert_eq!(worker.is_some(), flag_on);
        let scoreboard = Arc::new(image::DynamicImage::new_rgb8(1664, 1007));
        let rows: Vec<ocr::RowOcrResult> = (0..10)
            .map(|i| ocr::RowOcrResult {
                name: None,
                name_alternates: Vec::new(),
                stats: ["12", "3", "4", "5,480", "950", "2,347"]
                    .iter()
                    .map(|v| ocr::CellOcrResult {
                        value: v.to_string(),
                        confidence: 80 + i,
                        suspect: false,
                    })
                    .collect(),
                mean_confidence: 80 + i,
            })
            .collect();
        let rows_before = format!("{rows:?}");
        let input = shadow::worker::ShadowInput::when(worker.is_some(), &scoreboard, 5, 1440);
        if let (Some(w), Some(input)) = (&worker, &input) {
            assert!(w.try_submit(shadow::worker::ShadowJob::new(
                input,
                &rows,
                Some(0),
                "sess-shadow",
                Utc::now(),
            )));
        }
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        let mut row = test_match("sess-shadow", "victory");
        row.played_at = SurrealDatetime::from(
            chrono::DateTime::<Utc>::from_timestamp(1_760_000_000, 0).unwrap(),
        );
        store.insert_match(row).await.unwrap();
        let log = dir.path().join("shadow").join("digits.jsonl");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while flag_on && !log.exists() && std::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        drop(worker);
        assert_eq!(
            format!("{rows:?}"),
            rows_before,
            "the hand-off never edits rows"
        );
        let stored = store.get_unsynced().await.unwrap();
        assert_eq!(stored.len(), 1);
        let stored_json = serde_json::to_string(&stored).unwrap();
        let body = serde_json::to_string(&sync::upload_request(&stored, &[])).unwrap();
        (stored_json, body, log.exists())
    }
    let (stored_off, body_off, log_off) = run(false).await;
    let (stored_on, body_on, log_on) = run(true).await;
    assert_eq!(
        stored_off, stored_on,
        "stored rows must not depend on shadow mode"
    );
    assert_eq!(
        body_off, body_on,
        "upload body must not depend on shadow mode"
    );
    assert!(!log_off, "flag off writes no shadow log");
    assert!(log_on, "flag on wrote its log line");
}

/// The accept path hands the shadow worker shared borrows only, and the
/// row commit never sees shadow state.
#[test]
fn shadow_hook_only_borrows_and_stays_out_of_the_commit() {
    // The accept path is `handle_capture`, in src/daemon/tab/grab.rs since
    // 0.6 S10: found by name, wherever it lives.
    let src = fn_source("handle_capture");
    let hook = src
        .find("worker.try_submit(shadow::worker::ShadowJob::new(")
        .expect("shadow hand-off in the accept path");
    let call = &src[hook..hook + 300];
    assert!(call.contains("&rows,"), "{call}");
    assert!(call.contains("&staged.target_session,"), "{call}");
    // In src/daemon/tab/commit.rs since 0.6 S4: found by name, wherever it lives.
    let body = fn_source("commit_capture_rows");
    assert!(
        body.contains("insert_match("),
        "the slice is the commit itself"
    );
    assert!(
        !body.contains("shadow"),
        "commit_capture_rows must not read shadow state"
    );
}

#[tokio::test]
async fn local_write_during_inflight_upload_stays_unsynced_until_next_sync() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    store
        .insert_match(test_match("s1", "victory"))
        .await
        .unwrap();

    let entered = std::sync::Arc::new(tokio::sync::Notify::new());
    let release = std::sync::Arc::new(tokio::sync::Notify::new());
    let uploaded: std::sync::Arc<std::sync::Mutex<Vec<String>>> =
        std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));

    let store_bg = store.clone();
    let dir_bg = dir.path().to_path_buf();
    let entered_bg = entered.clone();
    let release_bg = release.clone();
    let uploaded_bg = uploaded.clone();
    let sync = tokio::spawn(async move {
        try_sync_with(&store_bg, &dir_bg, None, move |matches, _tombstones| {
            let entered_bg = entered_bg.clone();
            let release_bg = release_bg.clone();
            let uploaded_bg = uploaded_bg.clone();
            async move {
                uploaded_bg
                    .lock()
                    .unwrap()
                    .push(matches[0].display_outcome().to_string());
                entered_bg.notify_one();
                release_bg.notified().await;
                Ok(upload_ok())
            }
        })
        .await;
    });

    entered.notified().await;
    store.set_session_outcome("s1", "defeat").await.unwrap();
    release.notify_one();
    sync.await.unwrap();

    let mid = store.get_all_matches().await.unwrap();
    assert_eq!(mid[0].outcome, "defeat");
    assert!(
        !mid[0].synced,
        "in-flight mark must not commit the revised row"
    );

    let uploaded_bg = uploaded.clone();
    try_sync_with(&store, dir.path(), None, move |matches, _| {
        let uploaded_bg = uploaded_bg.clone();
        async move {
            uploaded_bg
                .lock()
                .unwrap()
                .push(matches[0].display_outcome().to_string());
            Ok(upload_ok())
        }
    })
    .await;

    assert_eq!(
        uploaded.lock().unwrap().as_slice(),
        ["victory", "defeat"],
        "the revised payload is what the follow-up sync uploads"
    );
    let done = store.get_all_matches().await.unwrap();
    assert!(done[0].synced);
    assert_eq!(done[0].outcome, "defeat");
}

#[tokio::test]
async fn shutdown_joins_inflight_sync_before_marking() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    store
        .insert_match(test_match("s1", "victory"))
        .await
        .unwrap();

    SYNC_DRAIN_WAITING.store(0, std::sync::atomic::Ordering::SeqCst);
    let release = std::sync::Arc::new(tokio::sync::Notify::new());
    let uploads = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let in_upload = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let overlapped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

    let inflight = tokio::spawn({
        let store = store.clone();
        let dir = dir.path().to_path_buf();
        let release = release.clone();
        let uploads = uploads.clone();
        let in_upload = in_upload.clone();
        let overlapped = overlapped.clone();
        async move {
            try_sync_with(&store, &dir, None, move |_matches, _tombstones| {
                let release = release.clone();
                let uploads = uploads.clone();
                let in_upload = in_upload.clone();
                let overlapped = overlapped.clone();
                async move {
                    if in_upload.swap(true, std::sync::atomic::Ordering::SeqCst) {
                        overlapped.store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                    uploads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    release.notified().await;
                    in_upload.store(false, std::sync::atomic::Ordering::SeqCst);
                    Ok(upload_ok())
                }
            })
            .await;
        }
    });

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while uploads.load(std::sync::atomic::Ordering::SeqCst) == 0 {
        if std::time::Instant::now() > deadline {
            panic!("in-flight sync never entered upload");
        }
        tokio::task::yield_now().await;
    }

    let shutdown = tokio::spawn({
        let store = store.clone();
        let dir = dir.path().to_path_buf();
        let uploads = uploads.clone();
        let in_upload = in_upload.clone();
        let overlapped = overlapped.clone();
        async move {
            drain_sync_then(Some(inflight), || {
                let store = store.clone();
                let dir = dir.clone();
                let uploads = uploads.clone();
                let in_upload = in_upload.clone();
                let overlapped = overlapped.clone();
                async move {
                    try_sync_with(&store, &dir, None, move |_matches, _tombstones| {
                        let uploads = uploads.clone();
                        let in_upload = in_upload.clone();
                        let overlapped = overlapped.clone();
                        async move {
                            if in_upload.swap(true, std::sync::atomic::Ordering::SeqCst) {
                                overlapped.store(true, std::sync::atomic::Ordering::SeqCst);
                            }
                            uploads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                            in_upload.store(false, std::sync::atomic::Ordering::SeqCst);
                            Ok(upload_ok())
                        }
                    })
                    .await;
                }
            })
            .await;
        }
    });

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while SYNC_DRAIN_WAITING.load(std::sync::atomic::Ordering::SeqCst) == 0 {
        if shutdown.is_finished() {
            panic!("shutdown finished before joining the in-flight sync");
        }
        if std::time::Instant::now() > deadline {
            panic!("shutdown never blocked in drain_sync_then");
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(
        uploads.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "final sync uploaded while the in-flight sync was still in flight"
    );
    assert!(!overlapped.load(std::sync::atomic::Ordering::SeqCst));

    release.notify_one();
    shutdown.await.unwrap();

    assert!(!overlapped.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(
        uploads.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "shutdown sync must not upload rows the in-flight sync already marked"
    );
    let rows = store.get_all_matches().await.unwrap();
    assert!(rows.iter().all(|m| m.synced));
}

#[test]
fn unsafe_sync_url_yields_no_client() {
    assert!(open_sync_client(None).is_none());
    let clear = config::SyncConfig {
        server_url: "http://example.com".into(),
        token: "secret".into(),
    };
    assert!(
        open_sync_client(Some(&clear)).is_none(),
        "daemon must not build a client that could send the token"
    );
    let https = config::SyncConfig {
        server_url: "https://crew.example".into(),
        token: "secret".into(),
    };
    assert!(open_sync_client(Some(&https)).is_some());
    for url in ["http://localhost:3030", "http://127.0.0.1", "http://[::1]"] {
        let cfg = config::SyncConfig {
            server_url: url.into(),
            token: "secret".into(),
        };
        assert!(
            open_sync_client(Some(&cfg)).is_some(),
            "{url} is the local-dev exception"
        );
    }
}

#[test]
fn periodic_sync_respects_single_flight_and_backoff() {
    assert!(
        !periodic_sync_due(4, false, true),
        "not yet every N captures"
    );
    assert!(periodic_sync_due(5, false, true));
    assert!(
        !periodic_sync_due(5, true, true),
        "single-flight: an in-flight upload blocks the next spawn"
    );
    assert!(
        !periodic_sync_due(10, false, false),
        "backoff window blocks periodic sync"
    );
    assert!(
        periodic_sync_due(10, false, true),
        "a reset backoff allows the next periodic sync"
    );
}

#[test]
fn quiet_close_waits_for_grace_and_does_not_invent_an_outcome() {
    let now = test_now();
    let close = finished_game_close_after(30);
    assert_eq!(
        close, POST_MATCH_GRACE,
        "a shorter setting still waits out grace"
    );
    let mut finished = game(detect::MatchOutcome::Defeat, Some(0), now);
    finished.last_activity = now;
    finished.outcome_recorded_at = Some(now);
    assert!(
        quiet_close_reason(&finished, now + POST_MATCH_GRACE, close).is_none(),
        "grace is still open at exactly 75s"
    );
    assert_eq!(
        quiet_close_reason(
            &finished,
            now + POST_MATCH_GRACE + Duration::from_secs(1),
            close
        ),
        Some("finished game closed after quiet period")
    );
    let close = finished_game_close_after(config::FINISHED_GAME_CLOSE_DEFAULT_SECS);
    assert!(quiet_close_reason(&finished, now + Duration::from_secs(179), close).is_none());
    assert_eq!(
        quiet_close_reason(&finished, now + Duration::from_secs(180), close),
        Some("finished game closed after quiet period")
    );
    let mut hinted = game(detect::MatchOutcome::Unknown, None, now);
    hinted.last_activity = now;
    hinted.result_mark = Some(ResultMark {
        outcome: detect::MatchOutcome::Defeat,
        confirmed: false,
        seen_at: now,
    });
    assert!(
        quiet_close_reason(&hinted, now + Duration::from_secs(180), close).is_none(),
        "a hint is not a recorded result"
    );
    assert!(
        quiet_close_reason(&hinted, now + UNFINISHED_SESSION_IDLE, close).is_none(),
        "the timer does not close an unfinished game"
    );
    assert!(
        quiet_close_reason(
            &hinted,
            now + UNFINISHED_SESSION_IDLE + Duration::from_secs(1),
            close
        )
        .is_none(),
        "a long match stays open until a result or a later Tab"
    );
    assert!(
        quiet_close_reason(
            &hinted,
            now + UNFINISHED_SESSION_MAX - Duration::from_secs(1),
            close
        )
        .is_none(),
        "just short of the long bound the unfinished game stays open"
    );
    assert_eq!(
        quiet_close_reason(&hinted, now + UNFINISHED_SESSION_MAX, close),
        Some(UNFINISHED_MAX_REASON),
        "after several hours an unfinished game closes"
    );
    assert_eq!(
        session_close_log("finished game closed after quiet period"),
        "finished game closed after quiet period"
    );
    assert_eq!(
        session_close_log("superseded by Tab-opened game"),
        "closed session at new-game boundary"
    );
}
