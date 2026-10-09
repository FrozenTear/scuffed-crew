//! HTTP coverage for the per-member Nostr limiter 429.

use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::json;
use tower::ServiceExt;

use scuffed_auth::SessionConfig;
use scuffed_db::migrations::run_migrations;
use scuffed_db::{Database, OrgRole};
use scuffed_site_server::create_router;
use scuffed_site_server::nostr_rate_limit::{NostrRateLimiter, RateClass};
use scuffed_site_server::state::{AppState, OAuthConfig};

const TOKEN: &str = "lane-nostr-session";

async fn test_state() -> AppState {
    let db = Database::connect_memory().await.expect("in-memory DB");
    run_migrations(&db.client).await.expect("migrations");
    AppState {
        db: Arc::new(db),
        session_config: SessionConfig::default(),
        oauth_config: OAuthConfig {
            discord_client_id: String::new(),
            discord_client_secret: String::new(),
            google_client_id: String::new(),
            google_client_secret: String::new(),
            redirect_base_url: "http://localhost:3000".into(),
            allowed_origins: vec!["http://localhost:3000".into()],
        },
        upload_dir: PathBuf::from("/tmp/scuffed-test-uploads"),
        reports_dir: PathBuf::from("/tmp/scuffed-test-reports"),
        reports_enabled: true,
        notifier: None,
        nostr_challenge_key: *blake3::hash(b"nostr-member-rate-limit-key").as_bytes(),
        consumed_challenges: scuffed_site_server::challenge_store::ConsumedChallengeStore::new(),
        nostr_rate_limiter: NostrRateLimiter::new(),
        login_lockout: scuffed_site_server::login_lockout::LoginLockout::new(),
        link_code_attempts: scuffed_site_server::link_attempts::LinkCodeAttempts::new(),
        link_poll: scuffed_site_server::link_poll::LinkPollGate::system(),
        crypto: None,
        relay_url: None,
        dm_events: None,
        nip05_domain: None,
        nip05_republish_enabled: false,
        public_settings: scuffed_site_server::state::PublicSettingsCache::new(),
        leaderboard_cache: scuffed_site_server::leaderboard_cache::LeaderboardCache::from_env(),
    }
}

async fn seed_member(state: &AppState) -> String {
    let user = state
        .db
        .create_local_user("lane", "not-used")
        .await
        .expect("user");
    let member = state
        .db
        .create_member(&user.id, "Lane", OrgRole::Member)
        .await
        .expect("member");
    state
        .db
        .create_session(&user.id, TOKEN, 1)
        .await
        .expect("session");
    member.id
}

/// Spend tokens until the bucket denies. Returns the reported wait.
fn drain(limiter: &NostrRateLimiter, member_id: &str, class: RateClass) -> u64 {
    for _ in 0..64 {
        match limiter.check(member_id, class) {
            Ok(()) => {}
            Err(secs) => return secs,
        }
    }
    panic!("bucket did not deny within 64 checks");
}

fn header_values(headers: &axum::http::HeaderMap, name: &header::HeaderName) -> Vec<String> {
    headers
        .get_all(name)
        .iter()
        .map(|value| value.to_str().unwrap_or("").to_string())
        .collect()
}

async fn post_json(
    app: &axum::Router,
    uri: &str,
    body: &str,
) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, headers, bytes.to_vec())
}

fn assert_rate_limited(headers: &axum::http::HeaderMap, body: &[u8]) -> u64 {
    assert_eq!(
        header_values(headers, &header::CACHE_CONTROL),
        ["no-store".to_string()],
        "Cache-Control no-store must appear exactly once"
    );
    assert_eq!(
        header_values(headers, &header::CONTENT_TYPE),
        ["application/json".to_string()]
    );
    let retry = header_values(headers, &header::RETRY_AFTER);
    assert_eq!(
        retry.len(),
        1,
        "Retry-After must appear once, got {retry:?}"
    );
    let secs: u64 = retry[0]
        .parse()
        .unwrap_or_else(|_| panic!("Retry-After must be seconds, got {retry:?}"));
    assert!(secs >= 1, "Retry-After must be at least 1, got {secs}");
    let expected = serde_json::to_vec(&json!({
        "error": "rate_limited",
        "retry_after": secs,
    }))
    .unwrap();
    assert_eq!(body, expected);
    secs
}

#[tokio::test]
async fn nostr_challenge_member_limit_is_json_429() {
    let state = test_state().await;
    let member_id = seed_member(&state).await;
    let app = create_router(state.clone());
    let waited = drain(
        &state.nostr_rate_limiter,
        &member_id,
        RateClass::Interactive,
    );
    assert!(waited >= 1);

    let (status, headers, body) =
        post_json(&app, "/api/nostr/challenge", r#"{"pubkey":"not-a-key"}"#).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    let secs = assert_rate_limited(&headers, &body);
    assert_eq!(secs, 1, "a just-empty interactive bucket waits 1 second");
}

#[tokio::test]
async fn nostr_export_backup_member_limit_reports_key_op_wait() {
    let state = test_state().await;
    let member_id = seed_member(&state).await;
    let app = create_router(state.clone());
    let waited = drain(&state.nostr_rate_limiter, &member_id, RateClass::KeyOp);
    assert_eq!(waited, 60);

    let (status, headers, body) =
        post_json(&app, "/api/nostr/export-backup", r#"{"password":"x"}"#).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    let secs = assert_rate_limited(&headers, &body);
    assert_eq!(secs, 60, "a just-empty key-op bucket waits 60 seconds");
}
