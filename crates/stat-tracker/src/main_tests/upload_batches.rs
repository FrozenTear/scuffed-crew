//! Daemon tests: upload_batches. See `mod.rs`.

use super::*;

#[tokio::test]
async fn deathmatch_rows_stay_local() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let mut guillard = test_match("dm", "victory");
    guillard.map_name = "Château Guillard".into();
    guillard.game_mode = "Deathmatch".into();
    let mut busan = test_match("busan-row", "victory");
    busan.map_name = "Busan".into();
    busan.game_mode = "Control".into();
    let mut corrected = test_match("edited-to-dm", "victory");
    corrected.map_name = "Busan".into();
    corrected.game_mode = "Control".into();
    corrected.corrected_map_name = Some("Château Guillard".into());
    store.insert_match(guillard).await.unwrap();
    store.insert_match(busan).await.unwrap();
    store.insert_match(corrected).await.unwrap();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen_upload = std::sync::Arc::clone(&seen);
    try_sync_with(&store, dir.path(), None, move |matches, _tombstones| {
        let seen_upload = std::sync::Arc::clone(&seen_upload);
        async move {
            *seen_upload.lock().unwrap() = matches.into_iter().map(|row| row.map_name).collect();
            Ok(upload_ok())
        }
    })
    .await;
    assert_eq!(*seen.lock().unwrap(), vec!["Busan".to_string()]);
    let only = {
        let mut row = test_match("dm-only", "victory");
        row.map_name = "Château Guillard".into();
        row.game_mode = "Deathmatch".into();
        row
    };
    store.insert_match(only).await.unwrap();
    let called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let called_upload = std::sync::Arc::clone(&called);
    try_sync_with(&store, dir.path(), None, move |_matches, _tombstones| {
        let called_upload = std::sync::Arc::clone(&called_upload);
        async move {
            called_upload.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(upload_ok())
        }
    })
    .await;
    assert!(
        !called.load(std::sync::atomic::Ordering::SeqCst),
        "a deathmatch-only batch must not call upload"
    );
}

#[tokio::test]
async fn arcade_and_stadium_rows_stay_local() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let local = [
        ("assault", "Hanamura", "Assault"),
        ("elim", "Black Forest", "Elimination"),
        ("ctf", "Ayutthaya", "Capture the Flag"),
        ("race", "Powder Keg Mine", "Payload Race"),
        ("shop", "Workshop Chamber", "Workshop"),
        ("stadium", "Arena Victoriae", "Stadium"),
        ("temple", "Temple of Anubis", "Assault"),
    ];
    for (id, map, mode) in local {
        let mut row = test_match(id, "victory");
        row.map_name = map.into();
        row.game_mode = mode.into();
        store.insert_match(row).await.unwrap();
    }
    let mut busan = test_match("busan-row", "victory");
    busan.map_name = "Busan".into();
    busan.game_mode = "Control".into();
    store.insert_match(busan).await.unwrap();
    let mut mode_only = test_match("mode-assault", "victory");
    mode_only.map_name = "Busan".into();
    mode_only.game_mode = "Assault".into();
    store.insert_match(mode_only).await.unwrap();

    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen_upload = std::sync::Arc::clone(&seen);
    try_sync_with(&store, dir.path(), None, move |matches, _| {
        let seen_upload = std::sync::Arc::clone(&seen_upload);
        async move {
            *seen_upload.lock().unwrap() = matches.into_iter().map(|row| row.session_id).collect();
            Ok(upload_ok())
        }
    })
    .await;
    assert_eq!(*seen.lock().unwrap(), vec!["busan-row".to_string()]);
    let rows = store.get_all_matches().await.unwrap();
    assert!(
        rows.iter().all(|row| row.synced),
        "local-only rows are marked synced and not left pending: {rows:?}"
    );
    assert!(store.get_unsynced().await.unwrap().is_empty());
}

#[tokio::test]
async fn one_bad_row_uploads_the_rest_and_stays_quarantined() {
    let n = 8_i64;
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    for i in 0..n {
        let mut row = test_match(&format!("s{i}"), "victory");
        row.played_at = SurrealDatetime::from(Utc::now() + chrono::Duration::seconds(i));
        store.insert_match(row).await.unwrap();
    }
    let bad = "s3";
    let uploaded = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let uploaded_pass = std::sync::Arc::clone(&uploaded);
    let attempt = try_sync_with(&store, dir.path(), None, move |matches, _tombstones| {
        let uploaded_pass = std::sync::Arc::clone(&uploaded_pass);
        let bad_here = matches.iter().any(|row| row.session_id == bad);
        let ids: Vec<String> = matches.iter().map(|row| row.session_id.clone()).collect();
        async move {
            if bad_here {
                Err(sync::SyncUploadError {
                    message: "matches[0]: hero is not allowed".into(),
                    status: Some(400),
                    ..sync::SyncUploadError::plain("")
                })
            } else {
                uploaded_pass.lock().unwrap().extend(ids);
                Ok(upload_ok())
            }
        }
    })
    .await;
    assert!(matches!(attempt, sync::SyncAttempt::Uploaded));
    let mut got = uploaded.lock().unwrap().clone();
    got.sort();
    got.dedup();
    assert_eq!(got.len(), (n as usize) - 1);
    assert!(!got.iter().any(|id| id == bad));
    let rows = store.get_all_matches().await.unwrap();
    assert!(
        rows.iter()
            .filter(|row| row.session_id != bad)
            .all(|row| row.synced),
        "the other rows upload"
    );
    let bad_row = rows.iter().find(|row| row.session_id == bad).unwrap();
    assert!(!bad_row.synced);
    assert_eq!(
        bad_row.upload_rejection(),
        Some("Upload rejected by server: hero is not allowed")
    );
    // Sync progress only marks the snapshot dirty; with no GUI running in
    // the test it is not written, so write it as a GUI tick would.
    store.export_snapshot(dir.path()).await.unwrap();
    let listed = storage::read_snapshot(dir.path())
        .expect("game list snapshot")
        .matches;
    let listed_bad = listed.iter().find(|row| row.session_id == bad).unwrap();
    assert_eq!(
        listed_bad.upload_rejection(),
        Some("Upload rejected by server: hero is not allowed"),
        "the desktop game list reads this flag and the server error text"
    );
    assert!(
        rows.iter()
            .filter(|row| row.session_id != bad)
            .all(|row| row.synced && row.upload_rejection().is_none()),
        "only the refused row is quarantined"
    );
    assert!(store.get_unsynced().await.unwrap().is_empty());

    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let calls_pass = std::sync::Arc::clone(&calls);
    let again = try_sync_with(&store, dir.path(), None, move |_matches, _| {
        let calls_pass = std::sync::Arc::clone(&calls_pass);
        async move {
            calls_pass.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(upload_ok())
        }
    })
    .await;
    assert!(matches!(again, sync::SyncAttempt::NoServerCall));
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);

    store
        .apply_command(&storage::StoreCommand::RetryUpload {
            session_id: bad.into(),
        })
        .await
        .unwrap();
    let retried = try_sync_with(&store, dir.path(), None, move |matches, _| async move {
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].session_id, bad);
        Ok(upload_ok())
    })
    .await;
    assert!(matches!(retried, sync::SyncAttempt::Uploaded));
    let done = store.get_all_matches().await.unwrap();
    assert!(
        done.iter()
            .find(|row| row.session_id == bad)
            .unwrap()
            .synced
    );
}

#[tokio::test]
async fn server_error_still_retries_the_whole_batch() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    store
        .insert_match(test_match("s1", "victory"))
        .await
        .unwrap();
    store
        .insert_match(test_match("s2", "defeat"))
        .await
        .unwrap();
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let calls_pass = std::sync::Arc::clone(&calls);
    let attempt = try_sync_with(&store, dir.path(), None, move |_matches, _| {
        let calls_pass = std::sync::Arc::clone(&calls_pass);
        async move {
            calls_pass.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err(sync::SyncUploadError {
                message: "down".into(),
                status: Some(500),
                ..sync::SyncUploadError::plain("")
            })
        }
    })
    .await;
    assert!(matches!(
        attempt,
        sync::SyncAttempt::ServerError { retry_after: None }
    ));
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    let rows = store.get_all_matches().await.unwrap();
    assert!(
        rows.iter()
            .all(|row| !row.synced && row.upload_rejection().is_none())
    );
    assert_eq!(store.get_unsynced().await.unwrap().len(), 2);
}

#[tokio::test]
async fn all_bad_batch_quarantines_nothing_and_backs_off() {
    let n = 4_i64;
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    for i in 0..n {
        let mut row = test_match(&format!("s{i}"), "victory");
        row.played_at = SurrealDatetime::from(Utc::now() + chrono::Duration::seconds(i));
        store.insert_match(row).await.unwrap();
    }
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let calls_pass = std::sync::Arc::clone(&calls);
    let attempt = try_sync_with(&store, dir.path(), None, move |_matches, _| {
        let calls_pass = std::sync::Arc::clone(&calls_pass);
        async move {
            calls_pass.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err(sync::SyncUploadError {
                message: "matches[0]: recognizer is not allowed".into(),
                status: Some(422),
                ..sync::SyncUploadError::plain("")
            })
        }
    })
    .await;
    let used = calls.load(std::sync::atomic::Ordering::SeqCst);
    assert!(matches!(
        attempt,
        sync::SyncAttempt::ServerError { retry_after: None }
    ));
    assert!(used > 1, "an all-bad batch is bisected");
    assert!(used <= sync::isolation_request_budget(n as usize));
    let rows = store.get_all_matches().await.unwrap();
    assert_eq!(rows.len(), n as usize);
    assert!(
        rows.iter()
            .all(|row| !row.synced && row.upload_rejection().is_none())
    );
    assert_eq!(store.get_unsynced().await.unwrap().len(), n as usize);
    let mut backoff = sync::SyncBackoff::default();
    let now = std::time::Instant::now();
    backoff.observe(attempt, now);
    assert_eq!(backoff.failures(), 1);
    assert!(!backoff.should_attempt(now));
    assert_eq!(backoff.retry_after(now), sync::SYNC_BACKOFF_BASE);
}

#[tokio::test]
async fn all_refused_chunk_that_hits_429_backs_off_and_honors_retry_after() {
    let n = 8_i64;
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    for i in 0..n {
        let mut row = test_match(&format!("s{i}"), "victory");
        row.played_at = SurrealDatetime::from(Utc::now() + chrono::Duration::seconds(i));
        store.insert_match(row).await.unwrap();
    }
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let calls_pass = std::sync::Arc::clone(&calls);
    let retry_after = std::time::Duration::from_secs(6);
    let attempt = try_sync_with(&store, dir.path(), None, move |_matches, _| {
        let calls_pass = std::sync::Arc::clone(&calls_pass);
        let retry_after = retry_after;
        async move {
            let seen = calls_pass.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if seen < 10 {
                Err(sync::SyncUploadError {
                    message: "matches[0]: recognizer is not allowed".into(),
                    status: Some(422),
                    ..sync::SyncUploadError::plain("")
                })
            } else {
                Err(sync::SyncUploadError {
                    message: "slow down".into(),
                    status: Some(429),
                    retry_after: Some(retry_after),
                    code: Some("rate_limited".into()),
                    ..sync::SyncUploadError::plain("")
                })
            }
        }
    })
    .await;
    let used = calls.load(std::sync::atomic::Ordering::SeqCst);
    assert!(
        used > 10,
        "the 429 lands after row-named refusals, used {used}"
    );
    assert!(matches!(
        attempt,
        sync::SyncAttempt::ServerError {
            retry_after: Some(wait)
        } if wait == retry_after
    ));
    let rows = store.get_all_matches().await.unwrap();
    assert_eq!(rows.len(), n as usize);
    assert!(
        rows.iter()
            .all(|row| !row.synced && row.upload_rejection().is_none())
    );
    assert_eq!(store.get_unsynced().await.unwrap().len(), n as usize);

    let now = std::time::Instant::now();
    let mut backoff = sync::SyncBackoff::default();
    backoff.observe(attempt, now);
    assert_eq!(backoff.failures(), 1);
    assert!(!backoff.should_attempt(now));
    assert_eq!(
        backoff.retry_after(now),
        sync::SYNC_BACKOFF_BASE,
        "a 6s Retry-After stays under the 15s base"
    );

    backoff.observe(attempt, now);
    assert_eq!(backoff.failures(), 2);
    assert_eq!(backoff.retry_after(now), sync::SYNC_BACKOFF_BASE * 2);

    let long_wait = std::time::Duration::from_secs(120);
    let mut floored = sync::SyncBackoff::default();
    floored.observe(
        sync::SyncAttempt::ServerError {
            retry_after: Some(long_wait),
        },
        now,
    );
    assert_eq!(floored.failures(), 1);
    assert_eq!(floored.retry_after(now), long_wait);
    floored.observe(
        sync::SyncAttempt::ServerError {
            retry_after: Some(long_wait),
        },
        now,
    );
    assert_eq!(floored.failures(), 2);
    assert_eq!(
        floored.retry_after(now),
        long_wait,
        "Retry-After is a floor under the doubled delay"
    );
}

#[tokio::test]
async fn mixed_batch_quarantines_only_the_refused_rows() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    for (i, sid) in ["s0", "s1", "s2"].into_iter().enumerate() {
        let mut row = test_match(sid, "victory");
        row.played_at = SurrealDatetime::from(Utc::now() + chrono::Duration::seconds(i as i64));
        store.insert_match(row).await.unwrap();
    }
    let attempt = try_sync_with(&store, dir.path(), None, move |matches, _| async move {
        if matches.iter().any(|row| row.session_id == "s1") {
            Err(sync::SyncUploadError {
                message: "matches[0]: hero is not allowed".into(),
                status: Some(422),
                ..sync::SyncUploadError::plain("")
            })
        } else {
            Ok(upload_ok())
        }
    })
    .await;
    assert!(matches!(attempt, sync::SyncAttempt::Uploaded));
    let rows = store.get_all_matches().await.unwrap();
    let bad = rows.iter().find(|row| row.session_id == "s1").unwrap();
    assert!(!bad.synced);
    assert_eq!(
        bad.upload_rejection(),
        Some("Upload rejected by server: hero is not allowed")
    );
    assert!(
        rows.iter()
            .filter(|row| row.session_id != "s1")
            .all(|row| { row.synced && row.upload_rejection().is_none() })
    );
    let pending = store.get_unsynced().await.unwrap();
    assert!(pending.is_empty());
}

#[tokio::test]
async fn nonspecific_400_all_bad_defers_and_backs_off() {
    let n = 4_i64;
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    for i in 0..n {
        let mut row = test_match(&format!("s{i}"), "victory");
        row.played_at = SurrealDatetime::from(Utc::now() + chrono::Duration::seconds(i));
        store.insert_match(row).await.unwrap();
    }
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let calls_pass = std::sync::Arc::clone(&calls);
    let attempt = try_sync_with(&store, dir.path(), None, move |_matches, _| {
        let calls_pass = std::sync::Arc::clone(&calls_pass);
        async move {
            calls_pass.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err(sync::SyncUploadError {
                message: "schema mismatch".into(),
                status: Some(400),
                code: Some("invalid_body".into()),
                ..sync::SyncUploadError::plain("")
            })
        }
    })
    .await;
    let used = calls.load(std::sync::atomic::Ordering::SeqCst);
    assert!(matches!(
        attempt,
        sync::SyncAttempt::ServerError { retry_after: None }
    ));
    assert!(used > 2, "a failed probe blames one row and continues");
    let rows = store.get_all_matches().await.unwrap();
    assert!(
        rows.iter()
            .all(|row| !row.synced && row.upload_rejection().is_none())
    );
    assert_eq!(store.get_unsynced().await.unwrap().len(), n as usize);
}

#[tokio::test]
async fn nonspecific_400_does_not_quarantine_rows_after_the_bad_one() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    for (i, sid) in ["s0", "s1", "s2", "s3"].into_iter().enumerate() {
        let mut row = test_match(sid, "victory");
        row.played_at = SurrealDatetime::from(Utc::now() + chrono::Duration::seconds(i as i64));
        store.insert_match(row).await.unwrap();
    }
    let attempt = try_sync_with(&store, dir.path(), None, move |matches, _| async move {
        let bad = matches.len() == 1 && matches[0].session_id == "s1";
        if matches.len() > 1 || bad {
            Err(sync::SyncUploadError {
                message: "schema mismatch".into(),
                status: Some(400),
                code: Some("invalid_body".into()),
                ..sync::SyncUploadError::plain("")
            })
        } else {
            Ok(upload_ok())
        }
    })
    .await;
    assert!(matches!(attempt, sync::SyncAttempt::Uploaded));
    let rows = store.get_all_matches().await.unwrap();
    let bad = rows.iter().find(|row| row.session_id == "s1").unwrap();
    assert_eq!(
        bad.upload_rejection(),
        Some("Upload rejected by server: invalid_body: schema mismatch")
    );
    assert!(!bad.synced);
    for sid in ["s0", "s2", "s3"] {
        let row = rows.iter().find(|row| row.session_id == sid).unwrap();
        assert!(row.synced, "{sid} should upload");
        assert!(
            row.upload_rejection().is_none(),
            "{sid} stays out of quarantine"
        );
    }
}

#[tokio::test]
async fn quarantined_row_is_not_resent_until_edited() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    for (i, sid) in ["good", "bad"].into_iter().enumerate() {
        let mut row = test_match(sid, "victory");
        row.played_at = SurrealDatetime::from(Utc::now() + chrono::Duration::seconds(i as i64));
        store.insert_match(row).await.unwrap();
    }
    let attempt = try_sync_with(&store, dir.path(), None, move |matches, _| async move {
        if matches.iter().any(|row| row.session_id == "bad") {
            Err(sync::SyncUploadError {
                message: "matches[0]: hero is not allowed".into(),
                status: Some(400),
                ..sync::SyncUploadError::plain("")
            })
        } else {
            Ok(upload_ok())
        }
    })
    .await;
    assert!(matches!(attempt, sync::SyncAttempt::Uploaded));
    assert!(store.get_unsynced().await.unwrap().is_empty());

    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let calls_pass = std::sync::Arc::clone(&calls);
    let skipped = try_sync_with(&store, dir.path(), None, move |_matches, _| {
        let calls_pass = std::sync::Arc::clone(&calls_pass);
        async move {
            calls_pass.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(upload_ok())
        }
    })
    .await;
    assert!(matches!(skipped, sync::SyncAttempt::NoServerCall));
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);

    store
        .apply_command(&storage::StoreCommand::EditMatch {
            session_id: "bad".into(),
            edit: storage::MatchEdit {
                elims: Some(9),
                ..storage::MatchEdit::default()
            },
        })
        .await
        .unwrap();
    let retried = try_sync_with(&store, dir.path(), None, move |matches, _| async move {
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].session_id, "bad");
        assert_eq!(matches[0].display_elims(), 9);
        Ok(upload_ok())
    })
    .await;
    assert!(matches!(retried, sync::SyncAttempt::Uploaded));
    let done = store.get_all_matches().await.unwrap();
    let bad = done.iter().find(|row| row.session_id == "bad").unwrap();
    assert!(bad.synced);
    assert!(bad.upload_rejection().is_none());
}

#[tokio::test]
async fn pending_deletes_upload_when_no_match_rows_are_queued() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    store.delete_session("gone").await.unwrap();
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let calls_pass = std::sync::Arc::clone(&calls);
    let attempt = try_sync_with(&store, dir.path(), None, move |matches, tombstones| {
        let calls_pass = std::sync::Arc::clone(&calls_pass);
        async move {
            calls_pass.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            assert!(matches.is_empty());
            assert!(tombstones.iter().any(|id| id == "gone"));
            Ok(upload_ok())
        }
    })
    .await;
    assert!(matches!(attempt, sync::SyncAttempt::Uploaded));
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(store.get_pending_tombstones().await.unwrap().is_empty());
}

#[tokio::test]
async fn sync_uploads_matches_in_chunks_of_200() {
    let n = 450i64;
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    insert_synced_games(&store, n).await;
    let sizes = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sizes_pass = std::sync::Arc::clone(&sizes);
    let attempt = try_sync_with(&store, dir.path(), None, move |matches, tombstones| {
        let sizes_pass = std::sync::Arc::clone(&sizes_pass);
        async move {
            assert!(tombstones.is_empty());
            assert!(matches.len() <= sync::SYNC_UPLOAD_MAX_MATCHES);
            sizes_pass.lock().unwrap().push(matches.len());
            Ok(upload_ok())
        }
    })
    .await;
    assert!(matches!(attempt, sync::SyncAttempt::Uploaded));
    assert_eq!(
        *sizes.lock().unwrap(),
        vec![
            sync::SYNC_UPLOAD_MAX_MATCHES,
            sync::SYNC_UPLOAD_MAX_MATCHES,
            n as usize - 2 * sync::SYNC_UPLOAD_MAX_MATCHES,
        ]
    );
    assert!(store.get_unsynced().await.unwrap().is_empty());
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
async fn sync_uploads_tombstones_in_chunks_of_500() {
    let n = 1200i64;
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    for i in 0..n {
        store.delete_session(&format!("t{i}")).await.unwrap();
    }
    let sizes = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sizes_pass = std::sync::Arc::clone(&sizes);
    let attempt = try_sync_with(&store, dir.path(), None, move |matches, tombstones| {
        let sizes_pass = std::sync::Arc::clone(&sizes_pass);
        async move {
            assert!(matches.is_empty());
            assert!(tombstones.len() <= sync::SYNC_UPLOAD_MAX_DELETES);
            sizes_pass.lock().unwrap().push(tombstones.len());
            Ok(upload_ok())
        }
    })
    .await;
    assert!(matches!(attempt, sync::SyncAttempt::Uploaded));
    let cap = sync::SYNC_UPLOAD_MAX_DELETES;
    assert_eq!(*sizes.lock().unwrap(), vec![cap, cap, n as usize - 2 * cap]);
    assert!(store.get_pending_tombstones().await.unwrap().is_empty());
}

#[tokio::test]
async fn sync_413_resends_at_the_server_match_cap_and_is_not_a_failure() {
    let n = 120i64;
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    insert_synced_games(&store, n).await;
    let sizes = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sizes_pass = std::sync::Arc::clone(&sizes);
    let attempt = try_sync_with(&store, dir.path(), None, move |matches, _| {
        let sizes_pass = std::sync::Arc::clone(&sizes_pass);
        let len = matches.len();
        async move {
            sizes_pass.lock().unwrap().push(len);
            if len > 50 {
                Err(batch_too_large(50, sync::SYNC_UPLOAD_MAX_DELETES as u64))
            } else {
                Ok(upload_ok())
            }
        }
    })
    .await;
    assert!(matches!(attempt, sync::SyncAttempt::Uploaded));
    let sizes = sizes.lock().unwrap().clone();
    assert_eq!(
        sizes[0], n as usize,
        "the first request is the uncapped chunk"
    );
    assert!(sizes[1..].iter().all(|len| *len <= 50 && *len > 0));
    assert_eq!(sizes[1..].iter().sum::<usize>(), n as usize);
    assert_eq!(sizes[1..].iter().filter(|len| **len == 50).count(), 2);
    assert!(store.get_unsynced().await.unwrap().is_empty());
    let mut backoff = sync::SyncBackoff::default();
    let now = std::time::Instant::now();
    backoff.record_failure(now);
    backoff.observe(attempt, now);
    assert_eq!(
        backoff.failures(),
        0,
        "a 413 that is resent in the same pass is not a failure streak"
    );
}

#[tokio::test]
async fn learned_caps_apply_to_the_next_pass() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    insert_synced_games(&store, 80).await;
    let mut caps = sync::BatchCaps::default();
    let sizes = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sizes_pass = std::sync::Arc::clone(&sizes);
    let first = try_sync_with_caps(&store, dir.path(), None, &mut caps, move |matches, _| {
        let sizes_pass = std::sync::Arc::clone(&sizes_pass);
        let len = matches.len();
        async move {
            sizes_pass.lock().unwrap().push(len);
            if len > 40 {
                Err(batch_too_large(40, sync::SYNC_UPLOAD_MAX_DELETES as u64))
            } else {
                Ok(upload_ok())
            }
        }
    })
    .await;
    assert!(matches!(first, sync::SyncAttempt::Uploaded));
    assert_eq!(caps.matches, 40);
    assert_eq!(
        sizes.lock().unwrap()[0],
        80,
        "the first pass still probes the old cap"
    );
    assert!(sizes.lock().unwrap()[1..].iter().all(|len| *len <= 40));

    for i in 0..80 {
        let mut row = test_match(&format!("next{i}"), "victory");
        row.played_at = SurrealDatetime::from(Utc::now() + chrono::Duration::seconds(1000 + i));
        store.insert_match(row).await.unwrap();
    }
    let second_sizes = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let second_pass = std::sync::Arc::clone(&second_sizes);
    let second = try_sync_with_caps(&store, dir.path(), None, &mut caps, move |matches, _| {
        let second_pass = std::sync::Arc::clone(&second_pass);
        let len = matches.len();
        async move {
            second_pass.lock().unwrap().push(len);
            if len > 40 {
                panic!("a later pass must not retry the cap the server already refused");
            }
            Ok(upload_ok())
        }
    })
    .await;
    assert!(matches!(second, sync::SyncAttempt::Uploaded));
    assert_eq!(*second_sizes.lock().unwrap(), vec![40, 40]);
    assert_eq!(caps.matches, 40);
    assert!(store.get_unsynced().await.unwrap().is_empty());
}

#[tokio::test]
async fn repeated_or_missing_413_caps_back_off() {
    for missing in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        insert_synced_games(&store, 4).await;
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let calls_pass = std::sync::Arc::clone(&calls);
        let attempt = try_sync_with(&store, dir.path(), None, move |_matches, _| {
            let calls_pass = std::sync::Arc::clone(&calls_pass);
            async move {
                calls_pass.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let mut err = batch_too_large(
                    sync::SYNC_UPLOAD_MAX_MATCHES as u64,
                    sync::SYNC_UPLOAD_MAX_DELETES as u64,
                );
                if missing {
                    err.max_matches = None;
                    err.max_deleted_sessions = None;
                }
                Err(err)
            }
        })
        .await;
        assert!(
            matches!(attempt, sync::SyncAttempt::BatchTooLarge),
            "missing={missing}"
        );
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "a 413 that does not shrink must not loop"
        );
        assert_eq!(store.get_unsynced().await.unwrap().len(), 4);
        let mut backoff = sync::SyncBackoff::default();
        let now = std::time::Instant::now();
        backoff.observe(attempt, now);
        assert_eq!(backoff.failures(), 1, "missing={missing}");
        assert!(!backoff.should_attempt(now));
        assert_eq!(backoff.retry_after(now), sync::SYNC_BACKOFF_BASE);
    }
}

#[tokio::test]
async fn sync_429_stops_later_chunks_and_keeps_the_rest_queued() {
    let n = 450i64;
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    insert_synced_games(&store, n).await;
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let calls_pass = std::sync::Arc::clone(&calls);
    let retry = std::time::Duration::from_secs(120);
    let attempt = try_sync_with(&store, dir.path(), None, move |matches, _| {
        let calls_pass = std::sync::Arc::clone(&calls_pass);
        let n = calls_pass.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let len = matches.len();
        async move {
            assert_eq!(len, sync::SYNC_UPLOAD_MAX_MATCHES);
            if n == 0 {
                Ok(upload_ok())
            } else if n == 1 {
                Err(sync::SyncUploadError {
                    message: "rate_limited".into(),
                    status: Some(429),
                    retry_after: Some(retry),
                    code: Some("rate_limited".into()),
                    ..sync::SyncUploadError::plain("")
                })
            } else {
                panic!("a 429 must stop the remaining chunks");
            }
        }
    })
    .await;
    assert!(matches!(
        attempt,
        sync::SyncAttempt::RateLimited {
            retry_after: Some(wait)
        } if wait == retry
    ));
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    let rows = store.get_all_matches().await.unwrap();
    let synced = rows.iter().filter(|row| row.synced).count();
    assert_eq!(synced, sync::SYNC_UPLOAD_MAX_MATCHES);
    assert_eq!(
        store.get_unsynced().await.unwrap().len(),
        n as usize - sync::SYNC_UPLOAD_MAX_MATCHES
    );
    let mut backoff = sync::SyncBackoff::default();
    let now = std::time::Instant::now();
    backoff.record_failure(now);
    backoff.observe(attempt, now);
    assert_eq!(
        backoff.failures(),
        1,
        "429 must not grow the failure streak"
    );
    assert_eq!(backoff.retry_after(now), retry);
}

#[tokio::test]
async fn token_expired_stops_the_rest_of_the_chunks() {
    let n = 250i64;
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    insert_synced_games(&store, n).await;
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let calls_pass = std::sync::Arc::clone(&calls);
    let attempt = try_sync_with(&store, dir.path(), None, move |_matches, _| {
        let calls_pass = std::sync::Arc::clone(&calls_pass);
        async move {
            let n = calls_pass.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            assert_eq!(n, 0, "token_expired must stop the remaining chunks");
            Err(sync::SyncUploadError {
                message: "token expired".into(),
                status: Some(401),
                code: Some("token_expired".into()),
                ..sync::SyncUploadError::plain("")
            })
        }
    })
    .await;
    assert!(matches!(attempt, sync::SyncAttempt::TokenExpired));
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(
        store
            .get_all_matches()
            .await
            .unwrap()
            .iter()
            .all(|row| !row.synced)
    );
    assert_eq!(store.get_unsynced().await.unwrap().len(), n as usize);
    let mut backoff = sync::SyncBackoff::default();
    let now = std::time::Instant::now();
    backoff.observe(attempt, now);
    assert!(backoff.auth_rejected());
    assert_eq!(backoff.failures(), 0);
}

#[tokio::test]
async fn token_expired_maps_to_the_relink_status() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    store
        .insert_match(test_match("s1", "victory"))
        .await
        .unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let body = br#"{"code":"token_expired"}"#;
    let response = format!(
        "HTTP/1.1 401 no\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let mut bytes = response.into_bytes();
    bytes.extend_from_slice(body);
    let server = std::thread::spawn(move || {
        let (mut sock, _) = listener.accept().expect("accept");
        let mut buf = [0u8; 8192];
        let mut got = Vec::new();
        loop {
            match std::io::Read::read(&mut sock, &mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    got.extend_from_slice(&buf[..n]);
                    if got.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        let _ = std::io::Write::write_all(&mut sock, &bytes);
    });
    let server_url = format!("http://127.0.0.1:{port}");
    let client = sync::SyncClient::new(config::SyncConfig {
        server_url: server_url.clone(),
        token: "tok".into(),
    });
    let attempt = try_sync(&store, &client, dir.path(), None).await;
    server.join().unwrap();
    assert!(matches!(attempt, sync::SyncAttempt::TokenExpired));
    assert_eq!(
        sync::auth_pause_status(dir.path(), &server_url, "tok").as_deref(),
        Some(sync::SYNC_TOKEN_EXPIRED_STATUS)
    );
    assert_eq!(
        sync::SYNC_TOKEN_EXPIRED_STATUS,
        "Sync token expired, relink in Settings"
    );
    assert_eq!(store.get_unsynced().await.unwrap().len(), 1);
}

#[tokio::test]
async fn stats_delete_pending_stops_without_split_or_quarantine_and_sets_the_status() {
    let n = 4i64;
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    insert_synced_games(&store, n).await;
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let calls_pass = std::sync::Arc::clone(&calls);
    let attempt = try_sync_with(&store, dir.path(), None, move |matches, _| {
        let calls_pass = std::sync::Arc::clone(&calls_pass);
        async move {
            let seen = calls_pass.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            assert_eq!(seen, 0, "stats_delete_pending must not split the batch");
            assert_eq!(matches.len(), n as usize);
            Err(sync::SyncUploadError {
                message: "stats delete pending".into(),
                status: Some(409),
                code: Some("stats_delete_pending".into()),
                ..sync::SyncUploadError::plain("")
            })
        }
    })
    .await;
    assert!(matches!(attempt, sync::SyncAttempt::StatsDeletePending));
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    let rows = store.get_all_matches().await.unwrap();
    assert!(rows.iter().all(|row| !row.synced));
    assert!(rows.iter().all(|row| row.upload_rejection().is_none()));
    assert_eq!(store.get_unsynced().await.unwrap().len(), n as usize);
    let mut backoff = sync::SyncBackoff::default();
    let now = std::time::Instant::now();
    backoff.observe(attempt, now);
    assert_eq!(backoff.failures(), 0);
    assert!(!backoff.auth_rejected());
    assert!(backoff.delete_pending());
    assert!(!backoff.should_attempt(now));
    assert_eq!(backoff.retry_after(now), std::time::Duration::ZERO);
    assert!(
        !backoff.should_attempt(now + std::time::Duration::from_secs(24 * 60 * 60)),
        "a stats delete does not retry on its own"
    );
    backoff.clear_auth_rejected();
    assert!(!backoff.delete_pending());
    assert!(backoff.should_attempt(now));

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let body = br#"{"code":"stats_delete_pending"}"#;
    let response = format!(
        "HTTP/1.1 409 no\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let mut bytes = response.into_bytes();
    bytes.extend_from_slice(body);
    let server = std::thread::spawn(move || {
        let (mut sock, _) = listener.accept().expect("accept");
        let mut buf = [0u8; 8192];
        let mut got = Vec::new();
        loop {
            match std::io::Read::read(&mut sock, &mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    got.extend_from_slice(&buf[..n]);
                    if got.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        let _ = std::io::Write::write_all(&mut sock, &bytes);
    });
    let server_url = format!("http://127.0.0.1:{port}");
    let client = sync::SyncClient::new(config::SyncConfig {
        server_url: server_url.clone(),
        token: "tok".into(),
    });
    let status_dir = tempfile::tempdir().unwrap();
    let status_store = storage::LocalStore::open(status_dir.path()).await.unwrap();
    status_store
        .insert_match(test_match("pending", "victory"))
        .await
        .unwrap();
    let live = try_sync(&status_store, &client, status_dir.path(), None).await;
    server.join().unwrap();
    assert!(matches!(live, sync::SyncAttempt::StatsDeletePending));
    assert_eq!(
        sync::auth_pause_status(status_dir.path(), &server_url, "tok").as_deref(),
        Some(sync::SYNC_STATS_DELETE_PENDING_STATUS)
    );
    assert_eq!(
        sync::SYNC_STATS_DELETE_PENDING_STATUS,
        "Your site stats are scheduled for deletion. To resume syncing, undo it on the website account page, then make a new sync token there and paste it into the app."
    );
    let stored = status_store.get_all_matches().await.unwrap();
    assert_eq!(stored.len(), 1);
    assert!(!stored[0].synced);
    assert!(stored[0].upload_rejection().is_none());
    assert_eq!(status_store.get_unsynced().await.unwrap().len(), 1);
}

#[tokio::test]
async fn stats_delete_pending_status_includes_the_scheduled_date() {
    let raw = "2026-10-19T23:30:00Z";
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let body = format!(r#"{{"code":"stats_delete_pending","stats_delete_scheduled_at":"{raw}"}}"#);
    let response = format!(
        "HTTP/1.1 409 no\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let mut bytes = response.into_bytes();
    bytes.extend_from_slice(body.as_bytes());
    let server = std::thread::spawn(move || {
        let (mut sock, _) = listener.accept().expect("accept");
        let mut buf = [0u8; 8192];
        let mut got = Vec::new();
        loop {
            match std::io::Read::read(&mut sock, &mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    got.extend_from_slice(&buf[..n]);
                    if got.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        let _ = std::io::Write::write_all(&mut sock, &bytes);
    });
    let server_url = format!("http://127.0.0.1:{port}");
    let client = sync::SyncClient::new(config::SyncConfig {
        server_url: server_url.clone(),
        token: "tok".into(),
    });
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    store
        .insert_match(test_match("dated", "victory"))
        .await
        .unwrap();
    let live = try_sync(&store, &client, dir.path(), None).await;
    server.join().unwrap();
    assert!(matches!(live, sync::SyncAttempt::StatsDeletePending));
    let date = sync::format_local_date(raw).expect("date");
    let local = chrono::DateTime::parse_from_rfc3339(raw)
        .unwrap()
        .with_timezone(&chrono::Local);
    assert_eq!(date, local.format("%B %-d, %Y").to_string());
    assert!(!date.contains(':'), "date only, got {date}");
    assert!(!date.contains('/'), "full month name, got {date}");
    let status = sync::auth_pause_status(dir.path(), &server_url, "tok").expect("status");
    assert_eq!(status, sync::delete_pending_status(Some(raw)));
    assert!(status.contains(&date));
    assert_eq!(store.get_unsynced().await.unwrap().len(), 1);
}

#[tokio::test]
async fn unknown_409_falls_back_to_the_generic_server_error() {
    let n = 4i64;
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    insert_synced_games(&store, n).await;
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let calls_pass = std::sync::Arc::clone(&calls);
    let attempt = try_sync_with(&store, dir.path(), None, move |matches, _| {
        let calls_pass = std::sync::Arc::clone(&calls_pass);
        async move {
            let seen = calls_pass.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            assert_eq!(seen, 0, "an unknown 409 must not split the batch");
            assert_eq!(matches.len(), n as usize);
            Err(sync::SyncUploadError {
                message: "conflict".into(),
                status: Some(409),
                code: Some("not_a_known_conflict".into()),
                ..sync::SyncUploadError::plain("")
            })
        }
    })
    .await;
    assert!(matches!(
        attempt,
        sync::SyncAttempt::ServerError { retry_after: None }
    ));
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    let rows = store.get_all_matches().await.unwrap();
    assert!(rows.iter().all(|row| !row.synced));
    assert!(rows.iter().all(|row| row.upload_rejection().is_none()));
    assert_eq!(store.get_unsynced().await.unwrap().len(), n as usize);
    let mut backoff = sync::SyncBackoff::default();
    let now = std::time::Instant::now();
    backoff.observe(attempt, now);
    assert_eq!(backoff.failures(), 1);
    assert!(!backoff.auth_rejected());
    assert!(!backoff.delete_pending());
    assert_eq!(backoff.retry_after(now), sync::SYNC_BACKOFF_BASE);
    assert!(backoff.should_attempt(now + sync::SYNC_BACKOFF_BASE));

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let body = br#"{"code":"not_a_known_conflict"}"#;
    let response = format!(
        "HTTP/1.1 409 no\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let mut bytes = response.into_bytes();
    bytes.extend_from_slice(body);
    let server = std::thread::spawn(move || {
        let (mut sock, _) = listener.accept().expect("accept");
        let mut buf = [0u8; 8192];
        let mut got = Vec::new();
        loop {
            match std::io::Read::read(&mut sock, &mut buf) {
                Ok(0) => break,
                Ok(read) => {
                    got.extend_from_slice(&buf[..read]);
                    if got.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        let _ = std::io::Write::write_all(&mut sock, &bytes);
    });
    let server_url = format!("http://127.0.0.1:{port}");
    let client = sync::SyncClient::new(config::SyncConfig {
        server_url: server_url.clone(),
        token: "tok".into(),
    });
    let status_dir = tempfile::tempdir().unwrap();
    let status_store = storage::LocalStore::open(status_dir.path()).await.unwrap();
    status_store
        .insert_match(test_match("conflict", "victory"))
        .await
        .unwrap();
    let live = try_sync(&status_store, &client, status_dir.path(), None).await;
    server.join().unwrap();
    assert!(matches!(
        live,
        sync::SyncAttempt::ServerError { retry_after: None }
    ));
    assert_eq!(
        sync::auth_pause_status(status_dir.path(), &server_url, "tok"),
        None
    );
    assert!(
        status_store
            .get_all_matches()
            .await
            .unwrap()
            .iter()
            .all(|row| row.upload_rejection().is_none() && !row.synced)
    );
}

#[tokio::test]
async fn one_bad_row_in_the_first_chunk_still_uploads_the_next_chunk() {
    let n = 201i64;
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    insert_synced_games(&store, n).await;
    let bad = format!("s{}", n - 1);
    let uploaded = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let uploaded_pass = std::sync::Arc::clone(&uploaded);
    let bad_id = bad.clone();
    let attempt = try_sync_with(&store, dir.path(), None, move |matches, _| {
        let uploaded_pass = std::sync::Arc::clone(&uploaded_pass);
        let bad_here = matches.iter().any(|row| row.session_id == bad_id);
        let ids: Vec<String> = matches.iter().map(|row| row.session_id.clone()).collect();
        async move {
            if bad_here {
                Err(sync::SyncUploadError {
                    message: "matches[0]: hero is not allowed".into(),
                    status: Some(400),
                    ..sync::SyncUploadError::plain("")
                })
            } else {
                uploaded_pass.lock().unwrap().extend(ids);
                Ok(upload_ok())
            }
        }
    })
    .await;
    assert!(matches!(attempt, sync::SyncAttempt::Uploaded));
    let got = uploaded.lock().unwrap().clone();
    assert!(
        got.iter().any(|id| id == "s0"),
        "the next chunk still uploads"
    );
    assert!(!got.iter().any(|id| id == &bad));
    let rows = store.get_all_matches().await.unwrap();
    assert!(
        rows.iter()
            .find(|row| row.session_id == bad)
            .unwrap()
            .upload_rejection()
            .is_some()
    );
    assert!(
        rows.iter()
            .filter(|row| row.session_id != bad)
            .all(|row| row.synced)
    );
    assert!(store.get_unsynced().await.unwrap().is_empty());
}

/// Delete game while the server is down: the game is gone here at once,
/// its tombstone waits, and the next sync that reaches the site sends it.
#[tokio::test]
async fn a_delete_while_offline_sends_its_tombstone_on_the_next_sync() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    store
        .insert_match(test_match("offline", "victory"))
        .await
        .unwrap();
    let claims = storage::SyncClaim::capture(&store.get_unsynced().await.unwrap());
    store.mark_synced(&claims).await.unwrap();
    store.delete_session("offline").await.unwrap();
    assert_eq!(store.match_count().await.unwrap(), 0);

    let down = try_sync_with(&store, dir.path(), None, |_m, tombstones| async move {
        assert_eq!(tombstones, vec!["offline".to_string()]);
        Err(sync::SyncUploadError::plain("connection refused"))
    })
    .await;
    assert!(!matches!(down, sync::SyncAttempt::Uploaded));
    assert_eq!(
        store.get_pending_tombstones().await.unwrap(),
        vec!["offline".to_string()],
        "the tombstone waits for the site"
    );

    let sent = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sent_pass = std::sync::Arc::clone(&sent);
    let back = try_sync_with(&store, dir.path(), None, move |matches, tombstones| {
        let sent_pass = std::sync::Arc::clone(&sent_pass);
        async move {
            assert!(matches.is_empty());
            sent_pass.lock().unwrap().extend(tombstones);
            Ok(upload_ok())
        }
    })
    .await;
    assert!(matches!(back, sync::SyncAttempt::Uploaded));
    assert_eq!(*sent.lock().unwrap(), vec!["offline".to_string()]);
    assert!(store.get_pending_tombstones().await.unwrap().is_empty());
    assert_eq!(store.match_count().await.unwrap(), 0);
}

/// Delete game while that game's upload is in flight: the upload's
/// success does not bring the row back, and the tombstone, queued after
/// the batch was read, goes on the next sync and removes it from the site.
#[tokio::test]
async fn a_delete_during_its_upload_queues_the_tombstone_for_the_next_sync() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    store
        .insert_match(test_match("mid", "victory"))
        .await
        .unwrap();

    let gui = store.clone();
    let first = try_sync_with(&store, dir.path(), None, move |matches, tombstones| {
        let gui = gui.clone();
        async move {
            assert!(matches.iter().any(|m| m.session_id == "mid"));
            assert!(tombstones.is_empty());
            // The GUI's DeleteSession lands while the request is out.
            gui.delete_session("mid").await.unwrap();
            Ok(upload_ok())
        }
    })
    .await;
    assert!(matches!(first, sync::SyncAttempt::Uploaded));
    assert_eq!(
        store.match_count().await.unwrap(),
        0,
        "the acknowledged upload does not resurrect the deleted game"
    );
    assert_eq!(
        store.get_pending_tombstones().await.unwrap(),
        vec!["mid".to_string()]
    );

    let sent = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sent_pass = std::sync::Arc::clone(&sent);
    try_sync_with(&store, dir.path(), None, move |matches, tombstones| {
        let sent_pass = std::sync::Arc::clone(&sent_pass);
        async move {
            assert!(matches.is_empty(), "the deleted game is not uploaded again");
            sent_pass.lock().unwrap().extend(tombstones);
            Ok(upload_ok())
        }
    })
    .await;
    assert_eq!(*sent.lock().unwrap(), vec!["mid".to_string()]);
    assert!(store.get_pending_tombstones().await.unwrap().is_empty());
}
