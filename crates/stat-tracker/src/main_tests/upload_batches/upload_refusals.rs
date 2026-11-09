//! Daemon tests: upload_refusals. Rows the site refuses are set aside
//! across chunks and passes without starving good games. A child of
//! `upload_batches.rs`, kept apart so both files stay under the size limit.

use super::*;

fn refused_row(message: &str) -> sync::SyncUploadError {
    sync::SyncUploadError {
        message: message.into(),
        status: Some(400),
        ..sync::SyncUploadError::plain("")
    }
}

/// A chunk whose only rows are refused, after an earlier chunk of the same
/// pass was accepted: the pass already proved the server takes uploads, so
/// the refused row is set aside with the server text instead of staying at
/// the head of the queue on its own.
#[tokio::test]
async fn refused_rows_in_a_later_chunk_are_quarantined_when_the_pass_uploaded_rows() {
    let n = 201i64;
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    insert_synced_games(&store, n).await;
    // Oldest game: the last one sent, alone in the second chunk of 200.
    let bad = "s0";
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let calls_pass = std::sync::Arc::clone(&calls);
    let attempt = try_sync_with(&store, dir.path(), None, move |matches, _| {
        calls_pass.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let bad_here = matches.iter().any(|row| row.session_id == bad);
        async move {
            if bad_here {
                Err(refused_row(
                    "matches[0]: played_at must be on or after 2022-10-04T00:00:00Z",
                ))
            } else {
                Ok(upload_ok())
            }
        }
    })
    .await;
    assert!(matches!(attempt, sync::SyncAttempt::Uploaded));
    let used = calls.load(std::sync::atomic::Ordering::SeqCst);
    assert!(
        used <= sync::isolation_request_budget(200) + sync::isolation_request_budget(1),
        "requests stay inside the per-chunk budgets, used {used}"
    );
    let rows = store.get_all_matches().await.unwrap();
    let bad_row = rows.iter().find(|row| row.session_id == bad).unwrap();
    assert!(!bad_row.synced, "a refused game is kept, never dropped");
    assert_eq!(
        bad_row.upload_rejection(),
        Some("Upload rejected by server: played_at must be on or after 2022-10-04T00:00:00Z")
    );
    assert!(
        rows.iter()
            .filter(|row| row.session_id != bad)
            .all(|row| row.synced && row.upload_rejection().is_none())
    );
    assert!(store.get_unsynced().await.unwrap().is_empty());
}

/// The site lets one token send 10 uploads in a burst, then answers 429.
/// Two refused games at the head of a 200-game queue use up that burst
/// before any good game is reached. Later passes must still get the good
/// games up and set the two refused ones aside, never drop them.
#[tokio::test]
async fn refused_rows_at_the_head_of_the_queue_do_not_starve_later_games() {
    const BURST: usize = 10;
    let n = 200i64;
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    insert_synced_games(&store, n).await;
    // Newest first: these two are the first rows of the first chunk.
    let bad = ["s199", "s198"];
    let mut caps = sync::BatchCaps::default();
    let mut refused = sync::RefusedRows::default();
    let mut passes = 0;
    let mut total_calls = 0usize;
    while passes < 6 && !store.get_unsynced().await.unwrap().is_empty() {
        passes += 1;
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let calls_pass = std::sync::Arc::clone(&calls);
        let pass = try_sync_with_state(
            &store,
            dir.path(),
            None,
            &mut caps,
            &mut refused,
            move |matches, _| {
                let call = calls_pass.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                let bad_here = matches
                    .iter()
                    .any(|row| bad.contains(&row.session_id.as_str()));
                async move {
                    if call > BURST {
                        return Err(sync::SyncUploadError {
                            message: "rate limited".into(),
                            status: Some(429),
                            retry_after: Some(std::time::Duration::from_secs(6)),
                            ..sync::SyncUploadError::plain("")
                        });
                    }
                    if bad_here {
                        Err(refused_row("matches[0]: hero is not allowed"))
                    } else {
                        Ok(upload_ok())
                    }
                }
            },
        )
        .await;
        if passes == 1 {
            assert!(
                !matches!(pass, sync::SyncAttempt::Uploaded),
                "the first pass reaches no good game inside the burst"
            );
        }
        let used = calls.load(std::sync::atomic::Ordering::SeqCst);
        assert!(
            used <= BURST + 1,
            "one pass stops at the first 429, used {used}"
        );
        total_calls += used;
    }
    assert!(
        store.get_unsynced().await.unwrap().is_empty(),
        "every game was uploaded or set aside within {passes} passes ({total_calls} requests)"
    );
    let rows = store.get_all_matches().await.unwrap();
    assert_eq!(rows.len(), n as usize, "no game is dropped");
    for row in &rows {
        if bad.contains(&row.session_id.as_str()) {
            assert!(!row.synced);
            assert_eq!(
                row.upload_rejection(),
                Some("Upload rejected by server: hero is not allowed")
            );
        } else {
            assert!(row.synced && row.upload_rejection().is_none());
        }
    }
}

/// Fail safe across passes: when the server refuses every row (here an
/// unknown recognizer), sending refused rows last must never turn into a
/// quarantine. Every game stays queued, unflagged, and each pass stays
/// inside the request budget.
#[tokio::test]
async fn server_wide_refusal_over_many_passes_quarantines_nothing() {
    let n = 6i64;
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    insert_synced_games(&store, n).await;
    let mut caps = sync::BatchCaps::default();
    let mut refused = sync::RefusedRows::default();
    for _ in 0..5 {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let calls_pass = std::sync::Arc::clone(&calls);
        let attempt = try_sync_with_state(
            &store,
            dir.path(),
            None,
            &mut caps,
            &mut refused,
            move |_matches, _| {
                calls_pass.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                async move { Err(refused_row("matches[0]: recognizer is not allowed")) }
            },
        )
        .await;
        assert!(matches!(
            attempt,
            sync::SyncAttempt::ServerError { retry_after: None }
        ));
        assert!(
            calls.load(std::sync::atomic::Ordering::SeqCst)
                <= sync::isolation_request_budget(n as usize)
        );
        let rows = store.get_all_matches().await.unwrap();
        assert!(
            rows.iter()
                .all(|row| !row.synced && row.upload_rejection().is_none())
        );
        assert_eq!(store.get_unsynced().await.unwrap().len(), n as usize);
    }
}
