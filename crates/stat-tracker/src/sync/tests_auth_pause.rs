//! Tests of the `sync_auth.json` pause and the status text the GUI shows.

use super::*;

#[test]
fn auth_pause_matches_only_the_rejected_url_and_token() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_auth_pause(dir.path(), "https://crew.example", "old-token").unwrap();
    assert!(auth_pause_matches(
        dir.path(),
        "https://crew.example",
        "old-token"
    ));
    assert!(
        !auth_pause_matches(dir.path(), "https://crew.example", "new-token"),
        "a new token must clear the pause"
    );
    assert!(
        !auth_pause_matches(dir.path(), "https://other.example", "old-token"),
        "a new URL must clear the pause"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.path().join(AUTH_PAUSE_FILE))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
    clear_auth_pause(dir.path());
    assert!(!auth_pause_matches(
        dir.path(),
        "https://crew.example",
        "old-token"
    ));
}

#[test]
fn clearing_auth_rejection_allows_the_next_attempt_immediately() {
    let t0 = std::time::Instant::now();
    let mut backoff = SyncBackoff::default();
    backoff.observe(SyncAttempt::AuthRejected, t0);
    assert!(!backoff.should_attempt(t0));
    backoff.clear_auth_rejected();
    assert!(!backoff.auth_rejected());
    assert!(backoff.should_attempt(t0));
    assert_eq!(backoff.failures(), 0);
}

#[test]
fn expired_pause_uses_the_relink_status() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_auth_pause_reason(
        dir.path(),
        "https://crew.example",
        "old-token",
        AuthPauseReason::Expired,
    )
    .unwrap();
    assert_eq!(
        auth_pause_status(dir.path(), "https://crew.example", "old-token").as_deref(),
        Some(SYNC_TOKEN_EXPIRED_STATUS)
    );
    assert!(auth_pause_matches(
        dir.path(),
        "https://crew.example",
        "old-token"
    ));
    assert_eq!(
        SYNC_TOKEN_EXPIRED_STATUS,
        "Sync token expired, relink in Settings"
    );
    write_auth_pause(dir.path(), "https://crew.example", "old-token").unwrap();
    assert_eq!(
        auth_pause_status(dir.path(), "https://crew.example", "old-token").as_deref(),
        Some(SYNC_TOKEN_REJECTED_STATUS)
    );
}

#[test]
fn delete_pending_pause_uses_the_account_page_status() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_auth_pause_reason(
        dir.path(),
        "https://crew.example",
        "old-token",
        AuthPauseReason::DeletePending,
    )
    .unwrap();
    assert_eq!(
        auth_pause_reason(dir.path(), "https://crew.example", "old-token"),
        Some(AuthPauseReason::DeletePending)
    );
    assert_eq!(
        auth_pause_status(dir.path(), "https://crew.example", "old-token").as_deref(),
        Some(SYNC_STATS_DELETE_PENDING_STATUS)
    );
    assert_eq!(
        auth_pause_reason(dir.path(), "https://other.example", "old-token"),
        Some(AuthPauseReason::DeletePending),
        "a URL change does not clear a stats-delete pause"
    );
    assert_eq!(
        auth_pause_reason(dir.path(), "https://crew.example", "new-token"),
        None,
        "a new token clears a stats-delete pause"
    );
    assert_eq!(
        SYNC_STATS_DELETE_PENDING_STATUS,
        "Your site stats are scheduled for deletion. To resume syncing, undo it on the website account page, then make a new sync token there and paste it into the app."
    );
}

#[test]
fn delete_pending_status_uses_the_local_date_only() {
    let raw = "2026-10-19T23:30:00Z";
    let date = format_local_date(raw).expect("rfc3339");
    let local = chrono::DateTime::parse_from_rfc3339(raw)
        .unwrap()
        .with_timezone(&chrono::Local);
    assert_eq!(date, local.format("%B %-d, %Y").to_string());
    assert!(!date.contains(':'), "date only, got {date}");
    assert!(!date.contains('/'), "full month name, got {date}");
    let status = delete_pending_status(Some(raw));
    assert_eq!(
        status,
        format!(
            "Your site stats are scheduled for deletion on {date}. To resume syncing, undo it on the website account page, then make a new sync token there and paste it into the app."
        )
    );
    assert_eq!(
        delete_pending_status(None),
        SYNC_STATS_DELETE_PENDING_STATUS
    );
    assert_eq!(
        delete_pending_status(Some("not-a-date")),
        SYNC_STATS_DELETE_PENDING_STATUS
    );
    let dir = tempfile::tempdir().expect("tempdir");
    write_delete_pending_pause(dir.path(), "https://crew.example", "old-token", Some(raw)).unwrap();
    assert_eq!(
        auth_pause_status(dir.path(), "https://crew.example", "old-token").as_deref(),
        Some(status.as_str())
    );
    let parsed = parse_upload_error_body(
        r#"{"code":"stats_delete_pending","stats_delete_scheduled_at":"2026-10-19T23:30:00Z"}"#,
    );
    assert_eq!(parsed.code.as_deref(), Some("stats_delete_pending"));
    assert_eq!(
        parsed.stats_delete_scheduled_at.as_deref(),
        Some("2026-10-19T23:30:00Z")
    );
}
