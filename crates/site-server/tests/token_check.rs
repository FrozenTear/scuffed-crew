//! HTTP coverage for `GET /api/stats/token-check`.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::json;
use tower::ServiceExt;

use scuffed_auth::SessionConfig;
use scuffed_auth::crypto::hash_session_token;
use scuffed_db::migrations::run_migrations;
use scuffed_db::{DaemonToken, Database, OrgRole};
use scuffed_site_server::create_router;
use scuffed_site_server::extractors::TOKEN_CHECK_UNAUTHORIZED;
use scuffed_site_server::state::{AppState, OAuthConfig};

const VALID: &str = "token-check-valid";
const FUTURE: &str = "token-check-future";
const REVOKED: &str = "token-check-revoked";
const EXPIRED: &str = "token-check-expired";
const DISPLAY_NAME: &str = "Lane";

const AUTH_PEER: [u8; 4] = [203, 0, 113, 10];
const RATE_PEER: [u8; 4] = [203, 0, 113, 11];

/// Schema tables. A write to any of them changes this sum.
const TABLES: &[&str] = &[
    "user",
    "session",
    "member",
    "game",
    "team",
    "plays_on",
    "event",
    "application",
    "match_result",
    "announcement",
    "poll",
    "poll_vote",
    "audit_log",
    "moderation_action",
    "site_settings",
    "game_account",
    "event_rsvp",
    "event_attendance",
    "tournament",
    "tournament_participant",
    "tournament_round",
    "tournament_match",
    "strategy",
    "team_channel",
    "group_last_seen",
    "scrim",
    "article",
    "wiki_page",
    "wiki_revision",
    "forum_category",
    "forum_board",
    "forum_thread",
    "forum_reply",
    "personal_match",
    "dm_message",
    "dm_read_marker",
    "daemon_token",
    "member_settings",
    "season",
    "patch_note",
    "bootstrap_lock",
];

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
        notifier: None,
        nostr_challenge_key: *blake3::hash(b"token-check-challenge-key").as_bytes(),
        consumed_challenges: scuffed_site_server::challenge_store::ConsumedChallengeStore::new(),
        nostr_rate_limiter: scuffed_site_server::nostr_rate_limit::NostrRateLimiter::new(),
        login_lockout: scuffed_site_server::login_lockout::LoginLockout::new(),
        crypto: None,
        relay_url: None,
        dm_events: None,
        nip05_domain: None,
        nip05_republish_enabled: false,
        public_settings: scuffed_site_server::state::PublicSettingsCache::new(),
        leaderboard_cache: scuffed_site_server::leaderboard_cache::LeaderboardCache::from_env(),
    }
}

enum Expiry {
    Past,
    Future,
}

fn expires_expr(expiry: Expiry) -> &'static str {
    match expiry {
        Expiry::Past => "time::now() - 1d",
        Expiry::Future => "time::now() + 1d",
    }
}

async fn insert_token(db: &Database, member_id: &str, raw: &str, label: &str, expiry: Expiry) {
    let token_hash = hash_session_token(raw);
    let response = db
        .client
        .query(format!(
            "CREATE daemon_token SET \
                member_id = $mid, \
                token_hash = $tok, \
                label = $label, \
                is_active = true, \
                created_at = time::now(), \
                expires_at = {expires}",
            expires = expires_expr(expiry)
        ))
        .bind(("mid", member_id.to_string()))
        .bind(("tok", token_hash))
        .bind(("label", label.to_string()))
        .await
        .unwrap_or_else(|e| panic!("insert {label}: {e}"));
    response
        .check()
        .unwrap_or_else(|e| panic!("insert {label} failed: {e}"));
}

async fn row_count(db: &Database) -> u64 {
    let mut total = 0u64;
    for table in TABLES {
        let n = db
            .count_table(table)
            .await
            .unwrap_or_else(|e| panic!("count {table}: {e}"));
        total += n;
    }
    total
}

fn last_used(tokens: &[DaemonToken], label: &str) -> bool {
    tokens
        .iter()
        .find(|token| token.label == label)
        .unwrap_or_else(|| panic!("missing token {label}"))
        .last_used_at
        .is_some()
}

async fn get_token_check(
    app: &axum::Router,
    peer: [u8; 4],
    bearer: Option<&str>,
) -> (StatusCode, Vec<u8>) {
    let mut builder = Request::builder()
        .method("GET")
        .uri("/api/stats/token-check");
    if let Some(token) = bearer {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let response = app
        .clone()
        .oneshot(
            builder
                .extension(axum::extract::ConnectInfo(SocketAddr::from((peer, 44000))))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, bytes.to_vec())
}

#[tokio::test]
async fn token_check_returns_display_name_and_identical_401s() {
    let state = test_state().await;
    let member = state
        .db
        .create_member("laneuser", DISPLAY_NAME, OrgRole::Member)
        .await
        .expect("member");

    state
        .db
        .create_daemon_token(&member.id, VALID, "valid")
        .await
        .expect("valid token");
    insert_token(&state.db, &member.id, FUTURE, "future", Expiry::Future).await;

    let revoked = state
        .db
        .create_daemon_token(&member.id, REVOKED, "revoked")
        .await
        .expect("revoked token");
    state
        .db
        .revoke_daemon_token(&revoked.id, &member.id)
        .await
        .expect("revoke");
    insert_token(&state.db, &member.id, EXPIRED, "expired", Expiry::Past).await;

    let before = row_count(&state.db).await;
    let before_tokens = state.db.list_daemon_tokens(&member.id).await.unwrap();
    assert!(!last_used(&before_tokens, "valid"));
    assert!(!last_used(&before_tokens, "future"));
    assert!(!last_used(&before_tokens, "revoked"));
    assert!(!last_used(&before_tokens, "expired"));

    let app = create_router(state.clone());
    let unauthorized = serde_json::to_vec(&json!({"error": TOKEN_CHECK_UNAUTHORIZED})).unwrap();

    let (status, body) = get_token_check(&app, AUTH_PEER, Some(VALID)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        serde_json::to_vec(&json!({"display_name": DISPLAY_NAME})).unwrap(),
        "success body is display_name only"
    );

    let (future_status, future_body) = get_token_check(&app, AUTH_PEER, Some(FUTURE)).await;
    assert_eq!(future_status, StatusCode::OK);
    assert_eq!(future_body, body, "a future expires_at is still valid");

    let cases = [Some("not-a-real-token"), Some(REVOKED), Some(EXPIRED), None];
    for bearer in cases {
        let (status, body) = get_token_check(&app, AUTH_PEER, bearer).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "bearer={bearer:?}");
        assert_eq!(body, unauthorized, "bearer={bearer:?}");
    }

    assert_eq!(
        row_count(&state.db).await,
        before,
        "token-check must not insert or delete rows"
    );
    let after_tokens = state.db.list_daemon_tokens(&member.id).await.unwrap();
    assert!(
        last_used(&after_tokens, "valid"),
        "accepted auth still stamps last_used_at, same as upload"
    );
    assert!(last_used(&after_tokens, "future"));
    assert!(
        !last_used(&after_tokens, "revoked"),
        "revoked auth must not stamp last_used_at"
    );
    assert!(
        !last_used(&after_tokens, "expired"),
        "expired auth must not stamp last_used_at"
    );

    // Token-check governor: burst 8, then 1 every 10s, per client IP. A fresh
    // peer so the auth calls above do not consume this bucket.
    let mut statuses = Vec::new();
    for i in 0..12 {
        let (status, body) = get_token_check(&app, RATE_PEER, Some("guess")).await;
        if i < 8 {
            assert_eq!(status, StatusCode::UNAUTHORIZED);
            assert_eq!(body, unauthorized);
        }
        statuses.push(status);
    }
    assert_eq!(
        statuses[8],
        StatusCode::TOO_MANY_REQUESTS,
        "the 9th guess must hit the token-check governor, saw {statuses:?}"
    );
    assert_eq!(
        row_count(&state.db).await,
        before,
        "rate-limited calls must not insert or delete rows"
    );
}

async fn post_image(app: &axum::Router, peer: [u8; 4]) -> StatusCode {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/upload/image")
                .extension(axum::extract::ConnectInfo(SocketAddr::from((peer, 44000))))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

/// Token-check and image uploads each have a per-IP bucket. Filling one does
/// not 429 the other, even from the same client IP.
#[tokio::test]
async fn token_check_and_image_upload_rate_limits_are_separate() {
    let state = test_state().await;
    let app = create_router(state);

    let token_peer = [203, 0, 113, 20];
    let mut token_statuses = Vec::new();
    for _ in 0..9 {
        let (status, _) = get_token_check(&app, token_peer, None).await;
        token_statuses.push(status);
    }
    assert!(
        token_statuses[..8]
            .iter()
            .all(|status| *status == StatusCode::UNAUTHORIZED),
        "token-check burst should reach the handler, saw {token_statuses:?}"
    );
    assert_eq!(token_statuses[8], StatusCode::TOO_MANY_REQUESTS);
    let image_after = post_image(&app, token_peer).await;
    assert_ne!(
        image_after,
        StatusCode::TOO_MANY_REQUESTS,
        "a used-up token-check bucket must not block image uploads from the same IP"
    );
    assert_eq!(image_after, StatusCode::UNAUTHORIZED);

    let image_peer = [203, 0, 113, 21];
    let mut image_statuses = Vec::new();
    for _ in 0..9 {
        image_statuses.push(post_image(&app, image_peer).await);
    }
    assert!(
        image_statuses[..8]
            .iter()
            .all(|status| *status == StatusCode::UNAUTHORIZED),
        "image upload burst should reach the handler, saw {image_statuses:?}"
    );
    assert_eq!(image_statuses[8], StatusCode::TOO_MANY_REQUESTS);
    let (token_after, _) = get_token_check(&app, image_peer, None).await;
    assert_ne!(
        token_after,
        StatusCode::TOO_MANY_REQUESTS,
        "a used-up image upload bucket must not block token-check from the same IP"
    );
    assert_eq!(token_after, StatusCode::UNAUTHORIZED);
}
