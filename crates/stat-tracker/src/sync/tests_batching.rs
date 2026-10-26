//! Tests of upload batching: `isolate_rejected` (bisecting a refused batch)
//! and the learned batch caps.

use super::tests::client;
use super::*;

fn payload_reject(status: u16) -> SyncUploadError {
    SyncUploadError {
        message: "matches[0]: hero is not allowed".into(),
        status: Some(status),
        ..SyncUploadError::plain("")
    }
}

#[tokio::test]
async fn one_bad_row_uploads_the_rest_of_the_batch() {
    for status in [400_u16, 422] {
        let n = 16usize;
        let bad = 5usize;
        let uploaded = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let rows: Vec<usize> = (0..n).collect();
        let isolation = isolate_rejected(&rows, |batch| {
            let uploaded = std::sync::Arc::clone(&uploaded);
            let bad_here = batch.contains(&bad);
            let ids = batch.to_vec();
            async move {
                if bad_here {
                    Err(payload_reject(status))
                } else {
                    uploaded.lock().unwrap().extend(ids);
                    Ok(scuffed_types::api::StatsUploadResponse {
                        inserted: 1,
                        skipped: 0,
                        deleted: 0,
                    })
                }
            }
        })
        .await;
        let mut got = uploaded.lock().unwrap().clone();
        got.sort_unstable();
        got.dedup();
        let expected: Vec<usize> = (0..n).filter(|index| *index != bad).collect();
        assert_eq!(got, expected, "status {status} should upload N-1 rows");
        assert_eq!(isolation.rejected.len(), 1);
        assert_eq!(isolation.rejected[0].index, bad);
        assert_eq!(
            isolation.rejected[0].message,
            "matches[0]: hero is not allowed"
        );
        assert!(isolation.deferred.is_empty());
        assert!(isolation.stopped.is_none());
        assert!(
            isolation.requests <= isolation_request_budget(n),
            "status {status} used {} requests, budget {}",
            isolation.requests,
            isolation_request_budget(n)
        );
        assert!(isolation.requests > 1, "the batch must be split");
    }
}

#[tokio::test]
async fn all_bad_batch_defers_every_row_and_backs_off() {
    let n = 7usize;
    let calls = std::sync::atomic::AtomicUsize::new(0);
    let rows: Vec<usize> = (0..n).collect();
    let isolation = isolate_rejected(&rows, |_batch| {
        calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        async { Err(payload_reject(422)) }
    })
    .await;
    let budget = isolation_request_budget(n);
    assert!(
        isolation.requests <= budget,
        "used {} requests, budget {budget}",
        isolation.requests
    );
    assert_eq!(
        isolation.requests,
        calls.load(std::sync::atomic::Ordering::SeqCst)
    );
    assert!(isolation.requests > 1, "the batch is split");
    assert!(isolation.accepted.is_empty());
    assert!(
        isolation.rejected.is_empty(),
        "a pass that accepts nothing quarantines nothing"
    );
    assert_eq!(isolation.deferred, (0..n).collect::<Vec<_>>());
    let stopped = isolation.stopped.expect("server-wide refusal");
    assert!(stopped.message.starts_with(SERVER_REFUSING_UPLOADS));
    assert!(stopped.message.contains("hero is not allowed"));
    assert!(matches!(
        stopped.attempt(),
        SyncAttempt::ServerError { retry_after: None }
    ));
}

#[tokio::test]
async fn mixed_batch_quarantines_only_the_refused_rows() {
    let n = 6usize;
    let bad = [1usize, 4];
    let rows: Vec<usize> = (0..n).collect();
    let isolation = isolate_rejected(&rows, |batch| {
        let bad_here = batch.iter().any(|index| bad.contains(index));
        let inserted = batch.len() as u32;
        async move {
            if bad_here {
                Err(payload_reject(422))
            } else {
                Ok(scuffed_types::api::StatsUploadResponse {
                    inserted,
                    skipped: 0,
                    deleted: 0,
                })
            }
        }
    })
    .await;
    let mut rejected: Vec<usize> = isolation.rejected.iter().map(|row| row.index).collect();
    rejected.sort_unstable();
    assert_eq!(rejected, vec![1, 4]);
    let mut accepted = isolation.accepted.clone();
    accepted.sort_unstable();
    assert_eq!(accepted, vec![0, 2, 3, 5]);
    assert!(isolation.deferred.is_empty());
    assert!(isolation.stopped.is_none());
    assert!(isolation.requests <= isolation_request_budget(n));
}

#[tokio::test]
async fn whole_request_400_defers_when_every_row_fails() {
    for status in [400_u16, 422] {
        let n = 4usize;
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let rows: Vec<u32> = (0..n as u32).collect();
        let isolation = isolate_rejected(&rows, |_batch| {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async move {
                Err(SyncUploadError {
                    message: "schema mismatch".into(),
                    status: Some(status),
                    code: Some("invalid_body".into()),
                    ..SyncUploadError::plain("")
                })
            }
        })
        .await;
        let used = calls.load(std::sync::atomic::Ordering::SeqCst);
        assert!(
            used > 2,
            "status {status} blames one row and keeps walking the rest"
        );
        assert_eq!(isolation.requests, used);
        assert!(isolation.requests <= isolation_request_budget(n));
        assert!(isolation.accepted.is_empty());
        assert!(
            isolation.rejected.is_empty(),
            "an all-bad pass quarantines nothing"
        );
        assert_eq!(isolation.deferred, (0..n).collect::<Vec<_>>());
        let stopped = isolation.stopped.expect("server-wide refusal");
        assert!(stopped.message.starts_with(SERVER_REFUSING_UPLOADS));
        assert!(matches!(
            stopped.attempt(),
            SyncAttempt::ServerError { retry_after: None }
        ));
    }
}

#[tokio::test]
async fn nonspecific_400_blames_only_the_failed_row() {
    let rows: Vec<usize> = (0..5).collect();
    let bad = 1usize;
    let isolation = isolate_rejected(&rows, |batch| {
        let ids = batch.to_vec();
        async move {
            if ids.len() > 1 || ids[0] == bad {
                Err(SyncUploadError {
                    message: "schema mismatch".into(),
                    status: Some(400),
                    code: Some("invalid_body".into()),
                    ..SyncUploadError::plain("")
                })
            } else {
                Ok(scuffed_types::api::StatsUploadResponse {
                    inserted: 1,
                    skipped: 0,
                    deleted: 0,
                })
            }
        }
    })
    .await;
    assert_eq!(isolation.accepted, vec![0, 2, 3, 4]);
    assert_eq!(isolation.rejected.len(), 1);
    assert_eq!(isolation.rejected[0].index, bad);
    assert_eq!(isolation.rejected[0].message, "schema mismatch");
    assert!(isolation.deferred.is_empty());
    assert!(isolation.stopped.is_none());
    assert!(isolation.requests <= isolation_request_budget(5));
}

#[tokio::test]
async fn nonspecific_400_still_uploads_a_row_that_succeeds_alone() {
    let rows: Vec<usize> = (0..4).collect();
    let isolation = isolate_rejected(&rows, |batch| {
        let ids = batch.to_vec();
        async move {
            if ids.len() > 1 {
                Err(SyncUploadError {
                    message: "schema mismatch".into(),
                    status: Some(400),
                    ..SyncUploadError::plain("")
                })
            } else if ids[0] == 2 {
                Err(payload_reject(422))
            } else {
                Ok(scuffed_types::api::StatsUploadResponse {
                    inserted: 1,
                    skipped: 0,
                    deleted: 0,
                })
            }
        }
    })
    .await;
    assert_eq!(isolation.accepted, vec![0, 1, 3]);
    assert_eq!(isolation.rejected.len(), 1);
    assert_eq!(isolation.rejected[0].index, 2);
    assert!(isolation.deferred.is_empty());
    assert!(isolation.stopped.is_none());
    assert!(isolation.requests <= isolation_request_budget(4));
}

#[tokio::test]
async fn rate_limit_mid_split_keeps_retry_after_and_does_not_quarantine() {
    let rows: Vec<usize> = (0..4).collect();
    let retry = std::time::Duration::from_secs(12);
    let isolation = isolate_rejected(&rows, |batch| {
        let len = batch.len();
        let wait = retry;
        async move {
            if len == 4 {
                Err(payload_reject(400))
            } else {
                Err(SyncUploadError {
                    message: "slow down".into(),
                    status: Some(429),
                    retry_after: Some(wait),
                    code: Some("rate_limited".into()),
                    ..SyncUploadError::plain("")
                })
            }
        }
    })
    .await;
    assert_eq!(isolation.requests, 2);
    assert!(isolation.accepted.is_empty());
    assert!(isolation.rejected.is_empty());
    assert_eq!(isolation.deferred, vec![0, 1, 2, 3]);
    let stopped = isolation.stopped.expect("429 stops the pass");
    assert_eq!(stopped.status, Some(429));
    assert_eq!(stopped.retry_after, Some(retry));
    assert!(matches!(
        stopped.attempt(),
        SyncAttempt::RateLimited {
            retry_after: Some(wait)
        } if wait == retry
    ));
}

#[tokio::test]
async fn row_specific_split_stays_inside_the_request_budget() {
    let n = 32usize;
    let calls = std::sync::atomic::AtomicUsize::new(0);
    let rows: Vec<u32> = (0..n as u32).collect();
    let isolation = isolate_rejected(&rows, |_| {
        calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        async { Err(payload_reject(400)) }
    })
    .await;
    let budget = isolation_request_budget(n);
    assert!(isolation.requests <= budget);
    assert_eq!(
        isolation.requests,
        calls.load(std::sync::atomic::Ordering::SeqCst)
    );
    assert!(isolation.requests > 1, "the batch is split");
    assert!(isolation.accepted.is_empty());
    assert!(
        isolation.rejected.is_empty(),
        "a pass that accepts nothing quarantines nothing"
    );
    assert_eq!(isolation.deferred, (0..n).collect::<Vec<_>>());
    let stopped = isolation.stopped.expect("server-wide refusal");
    assert!(stopped.message.starts_with(SERVER_REFUSING_UPLOADS));
}

#[tokio::test]
async fn one_bad_row_in_eight_uploads_seven() {
    let n = 8usize;
    let bad = 3usize;
    let uploaded = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let rows: Vec<usize> = (0..n).collect();
    let isolation = isolate_rejected(&rows, |batch| {
        let uploaded = std::sync::Arc::clone(&uploaded);
        let bad_here = batch.contains(&bad);
        let ids = batch.to_vec();
        async move {
            if bad_here {
                Err(payload_reject(400))
            } else {
                let inserted = ids.len() as u32;
                uploaded.lock().unwrap().extend(ids);
                Ok(scuffed_types::api::StatsUploadResponse {
                    inserted,
                    skipped: 0,
                    deleted: 0,
                })
            }
        }
    })
    .await;
    let mut got = uploaded.lock().unwrap().clone();
    got.sort_unstable();
    got.dedup();
    assert_eq!(got.len(), n - 1);
    assert!(!got.contains(&bad));
    assert_eq!(isolation.rejected.len(), 1);
    assert_eq!(isolation.rejected[0].index, bad);
    assert!(isolation.stopped.is_none());
    assert_eq!(isolation.accepted.len(), n - 1);
}

#[tokio::test]
async fn bad_rows_in_both_halves_still_upload_the_rest() {
    let n = 8usize;
    let bad = [0usize, 4];
    let uploaded = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let rows: Vec<usize> = (0..n).collect();
    let isolation = isolate_rejected(&rows, |batch| {
        let uploaded = std::sync::Arc::clone(&uploaded);
        let bad_here = batch.iter().any(|index| bad.contains(index));
        let ids = batch.to_vec();
        async move {
            if bad_here {
                Err(payload_reject(400))
            } else {
                uploaded.lock().unwrap().extend(ids);
                Ok(scuffed_types::api::StatsUploadResponse {
                    inserted: 1,
                    skipped: 0,
                    deleted: 0,
                })
            }
        }
    })
    .await;
    let mut got = uploaded.lock().unwrap().clone();
    got.sort_unstable();
    got.dedup();
    assert_eq!(got, vec![1, 2, 3, 5, 6, 7]);
    assert_eq!(isolation.rejected.len(), 2);
    assert!(isolation.stopped.is_none());
}

#[tokio::test]
async fn server_error_mid_split_keeps_the_accepted_half_and_retry_after() {
    // Left half of 8 succeeds. The right half splits, then a 503 lands
    // on its left child while the sibling is still queued. Further
    // requests must not run: if `halt` stayed false, 6..8 would be sent.
    let rows: Vec<usize> = (0..8).collect();
    let calls = std::sync::atomic::AtomicUsize::new(0);
    let retry = std::time::Duration::from_secs(30);
    let isolation = isolate_rejected(&rows, |batch| {
        calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let first = batch[0];
        let len = batch.len();
        let wait = retry;
        async move {
            if len == 8 {
                Err(payload_reject(400))
            } else if first == 0 {
                Ok(scuffed_types::api::StatsUploadResponse {
                    inserted: len as u32,
                    skipped: 0,
                    deleted: 0,
                })
            } else if len == 4 {
                Err(payload_reject(422))
            } else if first == 4 && len == 2 {
                Err(SyncUploadError {
                    message: "unavailable".into(),
                    status: Some(503),
                    retry_after: Some(wait),
                    ..SyncUploadError::plain("")
                })
            } else {
                panic!("request after 503 must not be sent, got {first}..+{len}");
            }
        }
    })
    .await;
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 4);
    assert_eq!(isolation.requests, 4);
    assert_eq!(isolation.accepted, vec![0, 1, 2, 3]);
    assert!(isolation.rejected.is_empty());
    assert_eq!(isolation.deferred, vec![4, 5, 6, 7]);
    let stopped = isolation.stopped.expect("503 stops the pass");
    assert_eq!(stopped.status, Some(503));
    assert_eq!(stopped.retry_after, Some(retry));
    assert!(matches!(
        stopped.attempt(),
        SyncAttempt::ServerError {
            retry_after: Some(wait)
        } if wait == retry
    ));
}

#[tokio::test]
async fn batch_sizes_and_several_bad_rows() {
    // N=1 refused cannot be told from a server-wide refusal.
    let alone = isolate_case(1, &[0]).await;
    assert!(alone.accepted.is_empty());
    assert!(alone.rejected.is_empty());
    assert_eq!(alone.deferred, vec![0]);
    assert!(
        alone
            .stopped
            .expect("n=1")
            .message
            .starts_with(SERVER_REFUSING_UPLOADS)
    );
    assert_eq!(alone.requests, 1);

    let one_ok = isolate_case(1, &[]).await;
    assert_eq!(one_ok.accepted, vec![0]);
    assert!(one_ok.rejected.is_empty());
    assert!(one_ok.stopped.is_none());
    assert_eq!(one_ok.requests, 1);

    // N=2, one bad row.
    let pair = isolate_case(2, &[1]).await;
    assert_eq!(pair.accepted, vec![0]);
    assert_eq!(pair.rejected.len(), 1);
    assert_eq!(pair.rejected[0].index, 1);
    assert!(pair.stopped.is_none());

    // N=2, both refused. Nothing is quarantined.
    let both = isolate_case(2, &[0, 1]).await;
    assert!(both.accepted.is_empty());
    assert!(both.rejected.is_empty());
    assert_eq!(both.deferred, vec![0, 1]);
    assert!(both.stopped.is_some());
    assert!(both.requests <= isolation_request_budget(2));

    // Odd length, one bad row in the longer half.
    let odd = isolate_case(5, &[2]).await;
    assert_eq!(odd.rejected.len(), 1);
    assert_eq!(odd.rejected[0].index, 2);
    assert_eq!(odd.accepted.len(), 4);
    assert!(odd.stopped.is_none());
    assert!(!odd.accepted.contains(&2));

    // Several bad rows.
    let many = isolate_case(7, &[1, 4, 6]).await;
    let mut rejected: Vec<usize> = many.rejected.iter().map(|row| row.index).collect();
    rejected.sort_unstable();
    assert_eq!(rejected, vec![1, 4, 6]);
    assert_eq!(many.accepted.len(), 4);
    assert!(many.stopped.is_none());
    for index in [1, 4, 6] {
        assert!(!many.accepted.contains(&index));
    }
}

async fn isolate_case(n: usize, bad: &[usize]) -> UploadIsolation {
    let rows: Vec<usize> = (0..n).collect();
    isolate_rejected(&rows, |batch| {
        let bad_here = batch.iter().any(|index| bad.contains(index));
        let inserted = batch.len() as u32;
        async move {
            if bad_here {
                Err(payload_reject(400))
            } else {
                Ok(scuffed_types::api::StatsUploadResponse {
                    inserted,
                    skipped: 0,
                    deleted: 0,
                })
            }
        }
    })
    .await
}

#[tokio::test]
async fn server_and_auth_errors_do_not_split_the_batch() {
    let cases = [
        None,
        Some(500_u16),
        Some(503),
        Some(401),
        Some(403),
        Some(429),
        Some(409),
    ];
    for status in cases {
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let rows = ["a", "b", "c", "d"];
        let isolation = isolate_rejected(&rows, |_| {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let code = status;
            async move {
                Err(SyncUploadError {
                    message: "down".into(),
                    status: code,
                    ..SyncUploadError::plain("")
                })
            }
        })
        .await;
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "{status:?} must not split"
        );
        assert!(isolation.accepted.is_empty());
        assert!(isolation.rejected.is_empty());
        assert_eq!(isolation.deferred, vec![0, 1, 2, 3]);
        let attempt = isolation.stopped.expect("stopped").attempt();
        match status {
            Some(401) | Some(403) => {
                assert!(matches!(attempt, SyncAttempt::AuthRejected));
            }
            Some(429) => {
                assert!(matches!(attempt, SyncAttempt::RateLimited { .. }));
            }
            _ => {
                assert!(matches!(attempt, SyncAttempt::ServerError { .. }));
            }
        }
    }
}

#[tokio::test]
async fn stats_delete_pending_409_does_not_split_or_quarantine() {
    let calls = std::sync::atomic::AtomicUsize::new(0);
    let rows = ["a", "b", "c", "d"];
    let isolation = isolate_rejected(&rows, |_| {
        calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        async move {
            Err(SyncUploadError {
                message: "stats delete pending".into(),
                status: Some(409),
                code: Some("stats_delete_pending".into()),
                ..SyncUploadError::plain("")
            })
        }
    })
    .await;
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(isolation.accepted.is_empty());
    assert!(isolation.rejected.is_empty());
    assert_eq!(isolation.deferred, vec![0, 1, 2, 3]);
    let err = isolation.stopped.expect("stopped");
    assert!(err.is_stats_delete_pending());
    assert!(matches!(err.attempt(), SyncAttempt::StatsDeletePending));
}

#[tokio::test]
async fn unknown_409_does_not_split_and_stays_a_server_error() {
    let calls = std::sync::atomic::AtomicUsize::new(0);
    let rows = ["a", "b", "c", "d"];
    let isolation = isolate_rejected(&rows, |_| {
        calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        async move {
            Err(SyncUploadError {
                message: "conflict".into(),
                status: Some(409),
                code: Some("not_a_known_conflict".into()),
                ..SyncUploadError::plain("")
            })
        }
    })
    .await;
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(isolation.accepted.is_empty());
    assert!(isolation.rejected.is_empty());
    assert_eq!(isolation.deferred, vec![0, 1, 2, 3]);
    let err = isolation.stopped.expect("stopped");
    assert!(!err.is_stats_delete_pending());
    assert!(matches!(
        err.attempt(),
        SyncAttempt::ServerError { retry_after: None }
    ));
}

#[test]
fn batch_limits_fall_to_the_server_values_and_stay_at_least_one() {
    let mut matches = SYNC_UPLOAD_MAX_MATCHES;
    let mut deletes = SYNC_UPLOAD_MAX_DELETES;
    assert!(lower_batch_limits(
        &mut matches,
        &mut deletes,
        Some(50),
        Some(100)
    ));
    assert_eq!(matches, 50);
    assert_eq!(deletes, 100);
    assert!(
        !lower_batch_limits(&mut matches, &mut deletes, Some(50), Some(100)),
        "the same caps are not progress"
    );
    assert!(lower_batch_limits(
        &mut matches,
        &mut deletes,
        Some(0),
        None
    ));
    assert_eq!(matches, 1, "a zero cap is bounded to 1");
    assert_eq!(deletes, 100, "a missing field keeps the current cap");
    assert!(!lower_batch_limits(
        &mut matches,
        &mut deletes,
        Some(500),
        Some(500)
    ));
    assert_eq!(matches, 1);
    assert_eq!(deletes, 100, "a higher server value does not raise the cap");
}

#[test]
fn learned_batch_caps_only_fall() {
    let c = client("https://crew.example");
    assert_eq!(c.batch_caps(), BatchCaps::default());
    c.remember_batch_caps(BatchCaps {
        matches: 50,
        deletes: 100,
    });
    assert_eq!(
        c.batch_caps(),
        BatchCaps {
            matches: 50,
            deletes: 100,
        }
    );
    c.remember_batch_caps(BatchCaps {
        matches: 200,
        deletes: 80,
    });
    assert_eq!(
        c.batch_caps(),
        BatchCaps {
            matches: 50,
            deletes: 80,
        },
        "a later pass cannot raise a learned cap"
    );
    assert_eq!(c.clone().batch_caps().matches, 50);
}

#[test]
fn replace_credentials_resets_learned_caps_when_the_server_url_changes() {
    let c = client("https://crew.example");
    c.remember_batch_caps(BatchCaps {
        matches: 50,
        deletes: 100,
    });
    c.replace_credentials(SyncConfig {
        server_url: "https://crew.example".into(),
        token: "other-token".into(),
    });
    assert_eq!(
        c.batch_caps(),
        BatchCaps {
            matches: 50,
            deletes: 100,
        },
        "a new token on the same server keeps the learned caps"
    );
    c.replace_credentials(SyncConfig {
        server_url: "https://other.example".into(),
        token: "other-token".into(),
    });
    assert_eq!(c.batch_caps(), BatchCaps::default());
    assert_eq!(c.batch_caps().matches, SYNC_UPLOAD_MAX_MATCHES);
    assert_eq!(c.batch_caps().deletes, SYNC_UPLOAD_MAX_DELETES);
}
