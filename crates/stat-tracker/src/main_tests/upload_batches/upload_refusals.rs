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

/// Fail safe across passes: when the server refuses every row (in real
/// life a PC clock more than 24 hours fast, or a site bug; here a fake
/// message), sending refused rows last must never turn into a quarantine. Every game stays queued, unflagged, and each pass stays
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

type AcceptCounts = std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, usize>>>;

/// Run sync passes the way the daemon does, with one `BatchCaps` and one
/// `RefusedRows` for the whole run. The site answers `budget(pass)` requests
/// in a pass (pass 1 first) and then 429. A batch with a row in `bad` gets a
/// 400 that names a row. Returns the passes used and how often each game
/// was accepted.
async fn run_passes_with_budget(
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    bad: &'static [&'static str],
    budget: fn(usize) -> usize,
    max_passes: usize,
) -> (usize, std::collections::HashMap<String, usize>) {
    let accepted: AcceptCounts = Default::default();
    let mut caps = sync::BatchCaps::default();
    let mut refused = sync::RefusedRows::default();
    let mut passes = 0;
    while passes < max_passes && !store.get_unsynced().await.unwrap().is_empty() {
        passes += 1;
        let allowed = budget(passes);
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let calls_pass = std::sync::Arc::clone(&calls);
        let accepted_pass = std::sync::Arc::clone(&accepted);
        try_sync_with_state(
            store,
            data_dir,
            None,
            &mut caps,
            &mut refused,
            move |matches, _| {
                let call = calls_pass.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                let bad_here = matches
                    .iter()
                    .any(|row| bad.contains(&row.session_id.as_str()));
                let accepted = std::sync::Arc::clone(&accepted_pass);
                async move {
                    if call > allowed {
                        return Err(sync::SyncUploadError {
                            message: "rate limited".into(),
                            status: Some(429),
                            retry_after: Some(std::time::Duration::from_secs(6)),
                            ..sync::SyncUploadError::plain("")
                        });
                    }
                    if bad_here {
                        return Err(refused_row("matches[0]: hero is not allowed"));
                    }
                    let mut seen = accepted.lock().unwrap();
                    for row in &matches {
                        *seen.entry(row.session_id.clone()).or_default() += 1;
                    }
                    Ok(upload_ok())
                }
            },
        )
        .await;
        let used = calls.load(std::sync::atomic::Ordering::SeqCst);
        assert!(
            used <= allowed + 1,
            "pass {passes} stops at the first 429, used {used}"
        );
    }
    let accepted = accepted.lock().unwrap().clone();
    (passes, accepted)
}

/// Every refused game is set aside with the site's reason, every other game
/// was accepted exactly once, and no game is lost.
async fn assert_refused_set_aside_and_the_rest_uploaded_once(
    store: &storage::LocalStore,
    n: i64,
    bad: &[&str],
    passes: usize,
    accepted: &std::collections::HashMap<String, usize>,
) {
    assert!(
        store.get_unsynced().await.unwrap().is_empty(),
        "every game was uploaded or set aside within {passes} passes"
    );
    let rows = store.get_all_matches().await.unwrap();
    assert_eq!(rows.len(), n as usize, "no game is dropped");
    for row in &rows {
        let id = row.session_id.as_str();
        if bad.contains(&id) {
            assert!(!row.synced, "{id}");
            assert_eq!(
                row.upload_rejection(),
                Some("Upload rejected by server: hero is not allowed"),
                "{id} is set aside with the site's reason"
            );
            assert_eq!(accepted.get(id), None, "{id} was never accepted");
        } else {
            assert!(row.synced && row.upload_rejection().is_none(), "{id}");
            assert_eq!(accepted.get(id), Some(&1), "{id} was sent once");
        }
    }
}

/// 217 games, the two newest refused, 10 requests per pass. Pass 1 holds the
/// two refused games. Pass 2 uploads every good game and hits the 429 right
/// after the last one, so the two refused games are all that is left. The
/// pass after must still set them aside, not retry them on every pass.
#[tokio::test]
async fn refused_games_left_alone_after_a_burst_are_set_aside_next_pass() {
    const BAD: &[&str] = &["s216", "s215"];
    let n = 217i64;
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    insert_synced_games(&store, n).await;
    let (passes, accepted) = run_passes_with_budget(&store, dir.path(), BAD, |_| 10, 8).await;
    assert_refused_set_aside_and_the_rest_uploaded_once(&store, n, BAD, passes, &accepted).await;
    assert_eq!(passes, 3, "held, then the good games, then set aside");
}

/// The site refills one request every 6 s, so a pass after a 429 often gets
/// only a few requests: 10 on the first pass, then 3. The good games go up a
/// few per pass, each pass ends on a 429, and the two refused games are set
/// aside once they are the only ones left.
#[tokio::test]
async fn refused_games_are_set_aside_when_later_passes_get_three_requests() {
    const BAD: &[&str] = &["s199", "s198"];
    let n = 200i64;
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    insert_synced_games(&store, n).await;
    let (passes, accepted) = run_passes_with_budget(
        &store,
        dir.path(),
        BAD,
        |pass| if pass == 1 { 10 } else { 3 },
        40,
    )
    .await;
    assert_refused_set_aside_and_the_rest_uploaded_once(&store, n, BAD, passes, &accepted).await;
}

/// 250 games and 501 pending deletes, the last delete id one the site
/// refuses. Chunk 1 (200 games, deletes 1 to 500) goes up. Chunk 2 carries
/// only the bad delete, and every request with it is refused as a whole.
/// That refusal is not about any game, so no game is set aside, in this pass
/// or the next, and the bad delete stays queued.
#[tokio::test]
async fn a_bad_delete_on_a_later_chunk_sets_no_game_aside() {
    let n = 250i64;
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    insert_synced_games(&store, n).await;
    let deletes = sync::SYNC_UPLOAD_MAX_DELETES + 1;
    for i in 0..deletes {
        store.delete_session(&format!("t{i}")).await.unwrap();
    }
    let pending = store.get_pending_tombstones().await.unwrap();
    assert_eq!(pending.len(), deletes);
    // Whichever id lands in the second delete window is the bad one.
    let window_two = std::sync::Arc::new(std::sync::Mutex::new(None::<String>));
    let mut caps = sync::BatchCaps::default();
    let mut refused = sync::RefusedRows::default();
    for pass in 1..=2 {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let calls_pass = std::sync::Arc::clone(&calls);
        let window_pass = std::sync::Arc::clone(&window_two);
        let attempt = try_sync_with_state(
            &store,
            dir.path(),
            None,
            &mut caps,
            &mut refused,
            move |_matches, tombstones| {
                calls_pass.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let mut window = window_pass.lock().unwrap();
                if window.is_none() && tombstones.len() == 1 {
                    *window = tombstones.first().cloned();
                }
                let bad_here = window.as_ref().is_some_and(|bad| tombstones.contains(bad));
                async move {
                    if bad_here {
                        Err(refused_row(
                            "deleted_sessions[0]: session id must be at most 64 characters",
                        ))
                    } else {
                        Ok(upload_ok())
                    }
                }
            },
        )
        .await;
        assert!(
            matches!(attempt, sync::SyncAttempt::ServerError { .. }),
            "pass {pass}: {attempt:?}"
        );
        let used = calls.load(std::sync::atomic::Ordering::SeqCst);
        assert!(
            used <= 1 + sync::isolation_request_budget(50),
            "pass {pass} used {used}"
        );
        let rows = store.get_all_matches().await.unwrap();
        assert_eq!(rows.len(), n as usize);
        assert!(
            rows.iter().all(|row| row.upload_rejection().is_none()),
            "pass {pass}: no game is set aside for a delete error"
        );
        assert_eq!(rows.iter().filter(|row| row.synced).count(), 200);
        assert_eq!(store.get_unsynced().await.unwrap().len(), 50);
        assert!(
            !refused.accepted_then_cut(),
            "a whole-request refusal is no proof"
        );
    }
    let bad = window_two.lock().unwrap().clone().expect("a second window");
    assert_eq!(
        store.get_pending_tombstones().await.unwrap(),
        vec![bad],
        "the refused delete stays queued"
    );
}

/// Rows an earlier pass held go last, the order inside each part is kept,
/// a row with no key stays in the first part, and keys no longer queued are
/// forgotten.
#[test]
fn refused_rows_last_keeps_order_and_drops_keys_no_longer_queued() {
    let rows = vec![
        test_match("a", "victory"),
        test_match("b", "victory"),
        test_match("", "victory"),
        test_match("c", "victory"),
        test_match("d", "victory"),
    ];
    let mut refused = sync::RefusedRows::default();
    for key in ["d", "b", "gone"] {
        refused.insert(key.into());
    }
    refused.set_accepted_then_cut(true);
    let ordered = refused_rows_last(rows, &mut refused);
    let ids: Vec<&str> = ordered.iter().map(|row| row.session_id.as_str()).collect();
    assert_eq!(ids, ["a", "", "c", "b", "d"]);
    assert!(refused.contains("b") && refused.contains("d"));
    assert!(
        !refused.contains("gone"),
        "a key no longer queued is dropped"
    );
    assert!(refused.accepted_then_cut(), "reordering keeps the proof");
}

/// A loopback site for `try_sync`: answers `budget` requests, then 429. A
/// body naming `bad` gets a 400 that names a row; anything else is stored.
fn spawn_refusing_site(
    bad: &'static str,
    budget: std::sync::Arc<std::sync::atomic::AtomicUsize>,
) -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for sock in listener.incoming() {
            let Ok(mut sock) = sock else {
                return;
            };
            sock.set_read_timeout(Some(std::time::Duration::from_secs(3)))
                .ok();
            let mut buf = Vec::new();
            let mut tmp = [0u8; 8192];
            loop {
                match std::io::Read::read(&mut sock, &mut tmp) {
                    Ok(0) => break,
                    Ok(n) => {
                        buf.extend_from_slice(&tmp[..n]);
                        if http_request_ready(&buf) {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let request = String::from_utf8_lossy(&buf).into_owned();
            let left = budget.load(std::sync::atomic::Ordering::SeqCst);
            let (status, extra, body) = if left == 0 {
                (
                    "429 Too Many Requests",
                    "Retry-After: 6\r\n",
                    r#"{"error":"rate_limited"}"#.to_string(),
                )
            } else {
                budget.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                if request.contains(&format!("\"{bad}\"")) {
                    (
                        "400 Bad Request",
                        "",
                        r#"{"error":"matches[0]: hero is not allowed"}"#.to_string(),
                    )
                } else {
                    (
                        "200 OK",
                        "",
                        r#"{"inserted":1,"skipped":0,"deleted":0}"#.to_string(),
                    )
                }
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = std::io::Write::write_all(&mut sock, response.as_bytes());
        }
    });
    port
}

/// The production path: `try_sync` keeps what a pass learned on the
/// `SyncClient` for the next one. The site answers 2 requests per pass.
/// Pass 1 holds the refused newest game. Pass 2 sends the good game first
/// and hits the 429 right after it. Pass 3 sets the refused game aside.
/// Without the client keeping the set, every pass repeats pass 1 and the
/// good game never uploads.
#[tokio::test]
async fn try_sync_keeps_refused_rows_on_the_client_between_passes() {
    let budget = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let port = spawn_refusing_site("s1", std::sync::Arc::clone(&budget));
    let client = sync::SyncClient::new(config::SyncConfig {
        server_url: format!("http://127.0.0.1:{port}"),
        token: "tok".into(),
    });
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    insert_synced_games(&store, 2).await;

    budget.store(2, std::sync::atomic::Ordering::SeqCst);
    let first = try_sync(&store, &client, dir.path(), None).await;
    assert!(
        matches!(first, sync::SyncAttempt::ServerError { .. }),
        "{first:?}"
    );
    assert!(client.refused_rows().contains("s1"), "the client keeps it");
    assert!(!client.refused_rows().accepted_then_cut());

    budget.store(2, std::sync::atomic::Ordering::SeqCst);
    let second = try_sync(&store, &client, dir.path(), None).await;
    assert!(
        matches!(second, sync::SyncAttempt::RateLimited { .. }),
        "{second:?}"
    );
    assert!(client.refused_rows().accepted_then_cut());
    let rows = store.get_all_matches().await.unwrap();
    let good = rows.iter().find(|row| row.session_id == "s0").unwrap();
    assert!(good.synced, "the good game went up on pass 2");

    budget.store(2, std::sync::atomic::Ordering::SeqCst);
    let third = try_sync(&store, &client, dir.path(), None).await;
    assert!(matches!(third, sync::SyncAttempt::Uploaded), "{third:?}");
    let rows = store.get_all_matches().await.unwrap();
    let refused = rows.iter().find(|row| row.session_id == "s1").unwrap();
    assert!(!refused.synced);
    assert_eq!(
        refused.upload_rejection(),
        Some("Upload rejected by server: hero is not allowed")
    );
    assert!(store.get_unsynced().await.unwrap().is_empty());
    assert!(!client.refused_rows().contains("s1"));
    assert!(!client.refused_rows().accepted_then_cut());
}
