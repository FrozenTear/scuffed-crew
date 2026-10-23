//! Tests of `SyncBackoff` and `Retry-After` parsing.

use super::*;

#[test]
fn backoff_grows_caps_and_resets_on_success() {
    let d1 = sync_backoff_delay(1);
    let d2 = sync_backoff_delay(2);
    let d3 = sync_backoff_delay(3);
    assert_eq!(d1, SYNC_BACKOFF_BASE);
    assert!(d2 > d1 && d3 > d2, "delay must grow: {d1:?} {d2:?} {d3:?}");
    assert_eq!(d2, SYNC_BACKOFF_BASE * 2);
    assert_eq!(sync_backoff_delay(0), std::time::Duration::ZERO);
    assert_eq!(sync_backoff_delay(20), SYNC_BACKOFF_CAP);
    assert!(sync_backoff_delay(5) <= SYNC_BACKOFF_CAP);
    assert_eq!(sync_backoff_delay(6), SYNC_BACKOFF_CAP);

    let mut backoff = SyncBackoff::default();
    let t0 = std::time::Instant::now();
    assert!(backoff.should_attempt(t0));
    backoff.record_failure(t0);
    assert_eq!(backoff.failures(), 1);
    assert!(!backoff.should_attempt(t0));
    assert_eq!(backoff.retry_after(t0), d1);
    assert!(backoff.should_attempt(t0 + d1));
    assert!(!backoff.should_attempt(t0 + d1 - std::time::Duration::from_millis(1)));

    backoff.record_failure(t0);
    assert_eq!(backoff.failures(), 2);
    assert_eq!(backoff.retry_after(t0), d2);
    assert!(d2 > d1);

    backoff.observe(SyncAttempt::NoServerCall, t0);
    assert_eq!(
        backoff.failures(),
        2,
        "a local/empty attempt must not move the clock"
    );

    backoff.record_success();
    assert_eq!(backoff.failures(), 0);
    assert!(backoff.should_attempt(t0));
    assert_eq!(backoff.retry_after(t0), std::time::Duration::ZERO);

    backoff.observe(SyncAttempt::ServerError { retry_after: None }, t0);
    assert_eq!(backoff.retry_after(t0), SYNC_BACKOFF_BASE);
    backoff.observe(SyncAttempt::Uploaded, t0);
    assert!(backoff.should_attempt(t0), "success resets the backoff");
    assert_eq!(backoff.failures(), 0);
}

#[test]
fn retry_after_parses_seconds_and_http_date() {
    let now = chrono::DateTime::parse_from_rfc3339("2026-09-24T08:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    assert_eq!(
        parse_retry_after("120", now),
        Some(std::time::Duration::from_secs(120))
    );
    assert_eq!(
        parse_retry_after("  45  ", now),
        Some(std::time::Duration::from_secs(45))
    );
    assert_eq!(
        parse_retry_after("Thu, 24 Sep 2026 08:02:00 GMT", now),
        Some(std::time::Duration::from_secs(120))
    );
    assert_eq!(
        parse_retry_after("Thu, 24 Sep 2026 07:00:00 GMT", now),
        Some(std::time::Duration::ZERO),
        "a Retry-After date in the past means retry now"
    );
    assert!(parse_retry_after("not-a-date", now).is_none());
    assert_eq!(
        parse_retry_after("999999999999999", now),
        Some(std::time::Duration::from_secs(999_999_999_999_999))
    );
    assert_eq!(
        parse_retry_after(&u64::MAX.to_string(), now),
        Some(std::time::Duration::MAX),
        "a delay-seconds that overflows Duration must saturate, not panic"
    );
}

#[test]
fn retry_after_429_waits_and_is_not_a_hard_failure() {
    let mut backoff = SyncBackoff::default();
    let t0 = std::time::Instant::now();
    backoff.record_failure(t0);
    assert_eq!(backoff.failures(), 1);

    backoff.observe(
        SyncAttempt::RateLimited {
            retry_after: Some(std::time::Duration::from_secs(120)),
        },
        t0,
    );
    assert_eq!(
        backoff.failures(),
        1,
        "429 must not advance the failure streak"
    );
    assert_eq!(backoff.retry_after(t0), std::time::Duration::from_secs(120));
    assert!(!backoff.should_attempt(t0 + std::time::Duration::from_secs(119)));
    assert!(backoff.should_attempt(t0 + std::time::Duration::from_secs(120)));

    backoff.observe(
        SyncAttempt::RateLimited {
            retry_after: Some(std::time::Duration::from_secs(24 * 60 * 60)),
        },
        t0,
    );
    assert_eq!(backoff.failures(), 1);
    assert_eq!(
        backoff.retry_after(t0),
        SYNC_BACKOFF_CAP,
        "Retry-After is capped at the backoff maximum"
    );

    let mut fresh = SyncBackoff::default();
    fresh.observe(SyncAttempt::RateLimited { retry_after: None }, t0);
    assert_eq!(fresh.failures(), 0);
    assert_eq!(fresh.retry_after(t0), SYNC_BACKOFF_BASE);
}

#[test]
fn retry_after_503_is_a_floor_on_the_exponential_delay() {
    let mut backoff = SyncBackoff::default();
    let t0 = std::time::Instant::now();
    backoff.observe(
        SyncAttempt::ServerError {
            retry_after: Some(std::time::Duration::from_secs(120)),
        },
        t0,
    );
    assert_eq!(backoff.failures(), 1, "503 still counts as a server error");
    assert_eq!(backoff.retry_after(t0), std::time::Duration::from_secs(120));

    backoff.observe(
        SyncAttempt::ServerError {
            retry_after: Some(std::time::Duration::from_secs(24 * 60 * 60)),
        },
        t0,
    );
    assert_eq!(backoff.failures(), 2);
    assert_eq!(backoff.retry_after(t0), SYNC_BACKOFF_CAP);
}
