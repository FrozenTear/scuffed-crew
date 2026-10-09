//! HTTP coverage for stat-tracker device-link sign-in.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

use scuffed_auth::SessionConfig;
use scuffed_auth::crypto::hash_session_token;
use scuffed_db::Database;
use scuffed_db::migrations::run_migrations;
use scuffed_db::queries::device_link::{DEVICE_LINK_INTERVAL_SECS, DEVICE_LINK_TTL_SECS};
use scuffed_site_server::create_router;
use scuffed_site_server::link_attempts::LINK_WRONG_CODE_LIMIT;
use scuffed_site_server::routes::link::{LINK_POLL_BURST, LINK_START_BURST, LINK_USER_BURST};
use scuffed_site_server::state::{AppState, OAuthConfig};

const SESSION: &str = "link-session-token";
const INVALID: &str = r#"{"error":"invalid code"}"#;
const SITE_ORIGIN: &str = "http://localhost:3000";
const FOREIGN_ORIGIN: &str = "https://evil.example";

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
        upload_dir: std::path::PathBuf::from("/tmp/scuffed-test-uploads"),
        notifier: None,
        nostr_challenge_key: *blake3::hash(b"device-link-test-key").as_bytes(),
        consumed_challenges: scuffed_site_server::challenge_store::ConsumedChallengeStore::new(),
        nostr_rate_limiter: scuffed_site_server::nostr_rate_limit::NostrRateLimiter::new(),
        login_lockout: scuffed_site_server::login_lockout::LoginLockout::new(),
        link_code_attempts: scuffed_site_server::link_attempts::LinkCodeAttempts::new(),
        crypto: None,
        relay_url: None,
        dm_events: None,
        nip05_domain: None,
        nip05_republish_enabled: false,
        public_settings: scuffed_site_server::state::PublicSettingsCache::new(),
        leaderboard_cache: scuffed_site_server::leaderboard_cache::LeaderboardCache::from_env(),
    }
}

async fn seed_member(db: &Database) {
    let token_hash = hash_session_token(SESSION);
    let pid_hash = hash_session_token("linkuser-pid");
    db.client
        .query(
            "CREATE user:linkuser SET
                provider = 'discord',
                username = 'linkuser',
                avatar_url = NONE,
                provider_id = 'linkuser-pid',
                provider_id_hash = $pidh,
                provider_id_encrypted = NONE,
                created_at = time::now()",
        )
        .bind(("pidh", pid_hash))
        .await
        .expect("seed user");
    db.client
        .query(
            "CREATE member:linkmember SET
                user_id = 'linkuser',
                org_role = 'member',
                display_name = 'Link User',
                bio = NONE,
                avatar_url = NONE,
                timezone = NONE,
                pronouns = NONE,
                availability_status = NONE,
                joined_at = time::now(),
                is_active = true",
        )
        .await
        .expect("seed member");
    db.client
        .query(
            "CREATE session:sess_linkmember SET
                user_id = 'linkuser',
                token = $tok,
                expires_at = time::now() + 365d,
                created_at = time::now()",
        )
        .bind(("tok", token_hash))
        .await
        .expect("seed session");
}

fn req(
    method: Method,
    uri: &str,
    body: Option<Value>,
    bearer: Option<&str>,
    peer: [u8; 4],
    forwarded_for: &str,
    origin: Option<&str>,
) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-forwarded-for", forwarded_for)
        .extension(axum::extract::ConnectInfo(SocketAddr::from((peer, 40000))));
    if let Some(token) = bearer {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    if let Some(origin) = origin {
        builder = builder.header(header::ORIGIN, origin);
    }
    let payload = body
        .map(|value| serde_json::to_vec(&value).unwrap())
        .unwrap_or_default();
    builder.body(Body::from(payload)).unwrap()
}

fn trusted(method: Method, uri: &str, body: Option<Value>, bearer: Option<&str>) -> Request<Body> {
    req(
        method,
        uri,
        body,
        bearer,
        [127, 0, 0, 1],
        "127.0.0.1",
        Some(SITE_ORIGIN),
    )
}

fn from_xff(
    ip: &str,
    method: Method,
    uri: &str,
    body: Option<Value>,
    bearer: Option<&str>,
) -> Request<Body> {
    req(
        method,
        uri,
        body,
        bearer,
        [127, 0, 0, 1],
        ip,
        Some(SITE_ORIGIN),
    )
}

async fn send(app: &axum::Router, request: Request<Body>) -> (StatusCode, String) {
    let (status, _headers, body) = send_full(app, request).await;
    (status, body)
}

async fn send_full(
    app: &axum::Router,
    request: Request<Body>,
) -> (StatusCode, axum::http::HeaderMap, String) {
    let response = app.clone().oneshot(request).await.expect("response");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, headers, String::from_utf8(bytes.to_vec()).unwrap())
}

fn json_of(body: &str) -> Value {
    serde_json::from_str(body).unwrap_or(Value::Null)
}

async fn start_link(app: &axum::Router, label: &str, version: &str) -> Value {
    let (status, body) = send(
        app,
        trusted(
            Method::POST,
            "/api/link/start",
            Some(json!({"device_label": label, "app_version": version})),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    json_of(&body)
}

fn canonical_user_code(display: &str) -> String {
    display.chars().filter(|ch| *ch != '-').collect()
}

#[tokio::test]
async fn happy_path_issues_a_revocable_token_once() {
    let state = test_state().await;
    seed_member(&state.db).await;
    let app = create_router(state.clone());

    let started = start_link(&app, "Living Room PC", "0.4.2").await;
    let user_code = started["user_code"].as_str().unwrap().to_string();
    let device_code = started["device_code"].as_str().unwrap().to_string();
    assert_eq!(user_code.len(), 9);
    assert_eq!(user_code.as_bytes()[4], b'-');
    assert!(
        canonical_user_code(&user_code)
            .bytes()
            .all(|b| b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789".contains(&b)),
        "{user_code}"
    );
    assert_eq!(device_code.len(), 64);
    assert!(device_code.bytes().all(|b| b.is_ascii_hexdigit()));
    assert_eq!(
        started["interval"].as_u64(),
        Some(DEVICE_LINK_INTERVAL_SECS)
    );
    assert_eq!(started["expires_in"].as_u64(), Some(DEVICE_LINK_TTL_SECS));

    let mut stored = state
        .db
        .client
        .query("SELECT * FROM device_link")
        .await
        .unwrap();
    let rows: Vec<serde_json::Value> = stored.take(0).unwrap();
    let blob = rows[0].to_string();
    assert!(!blob.contains(&canonical_user_code(&user_code)));
    assert!(!blob.contains(&user_code));
    assert!(!blob.contains(&device_code));
    assert_eq!(
        rows[0]["user_code_hash"].as_str().unwrap(),
        hash_session_token(&canonical_user_code(&user_code))
    );
    assert_eq!(
        rows[0]["device_code_hash"].as_str().unwrap(),
        hash_session_token(&device_code)
    );

    let (status, body) = send(
        &app,
        trusted(
            Method::POST,
            "/api/link/lookup",
            Some(json!({"user_code": user_code})),
            Some(SESSION),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let lookup = json_of(&body);
    assert_eq!(lookup["device_label"], "Living Room PC");
    assert_eq!(lookup["app_version"], "0.4.2");
    assert!(lookup["created_at"].as_str().unwrap().contains('T'));

    let (status, body) = send(
        &app,
        trusted(
            Method::POST,
            "/api/link/approve",
            Some(json!({"user_code": user_code})),
            Some(SESSION),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, r#"{"ok":true}"#);

    let (status, body) = send(
        &app,
        trusted(
            Method::POST,
            "/api/link/poll",
            Some(json!({"device_code": device_code})),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let polled = json_of(&body);
    assert_eq!(polled["status"], "approved");
    let token = polled["token"].as_str().unwrap().to_string();
    assert_eq!(token.len(), 64);

    let (status, body) = send(
        &app,
        trusted(
            Method::POST,
            "/api/link/poll",
            Some(json!({"device_code": device_code})),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let again = json_of(&body);
    assert_ne!(again["status"], "approved");
    assert!(again.get("token").is_none(), "{body}");
    assert!(!body.contains(&token));

    let mut after = state
        .db
        .client
        .query("SELECT * FROM device_link")
        .await
        .unwrap();
    let rows: Vec<serde_json::Value> = after.take(0).unwrap();
    assert!(!rows[0].to_string().contains(&token));

    let (status, body) = send(
        &app,
        trusted(Method::GET, "/api/stats/tokens", None, Some(SESSION)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let listed = json_of(&body);
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["label"], "Living Room PC");
    assert_eq!(listed[0]["is_active"], true);
    let token_id = listed[0]["id"].as_str().unwrap().to_string();

    let (status, body) = send(
        &app,
        trusted(
            Method::POST,
            "/api/stats/upload",
            Some(json!({"matches": []})),
            Some(&token),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "minted token must pass upload auth: {body}"
    );

    let (status, body) = send(
        &app,
        trusted(Method::GET, "/api/stats/token-check", None, Some(&token)),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "minted token must pass token-check: {body}"
    );
    assert_eq!(json_of(&body)["display_name"], "Link User");

    let (status, _) = send(
        &app,
        trusted(
            Method::DELETE,
            &format!("/api/stats/tokens/{token_id}"),
            None,
            Some(SESSION),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = send(
        &app,
        trusted(
            Method::POST,
            "/api/stats/upload",
            Some(json!({"matches": []})),
            Some(&token),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _) = send(
        &app,
        trusted(Method::GET, "/api/stats/token-check", None, Some(&token)),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn deny_kills_the_code() {
    let state = test_state().await;
    seed_member(&state.db).await;
    let app = create_router(state);
    let started = start_link(&app, "Desk", "1.0.0").await;
    let user_code = started["user_code"].as_str().unwrap();
    let device_code = started["device_code"].as_str().unwrap();

    let (status, body) = send(
        &app,
        trusted(
            Method::POST,
            "/api/link/deny",
            Some(json!({"user_code": user_code})),
            Some(SESSION),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = send(
        &app,
        trusted(
            Method::POST,
            "/api/link/poll",
            Some(json!({"device_code": device_code})),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(json_of(&body)["status"], "denied");
    assert!(json_of(&body).get("token").is_none());

    let (status, body) = send(
        &app,
        trusted(
            Method::POST,
            "/api/link/lookup",
            Some(json!({"user_code": user_code})),
            Some(SESSION),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body, INVALID);
}

#[tokio::test]
async fn slow_down_when_polled_inside_the_interval() {
    let state = test_state().await;
    let app = create_router(state);
    let started = start_link(&app, "Desk", "1.0.0").await;
    let device_code = started["device_code"].as_str().unwrap();

    let (status, body) = send(
        &app,
        trusted(
            Method::POST,
            "/api/link/poll",
            Some(json!({"device_code": device_code})),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, r#"{"status":"pending"}"#);

    let (status, body) = send(
        &app,
        trusted(
            Method::POST,
            "/api/link/poll",
            Some(json!({"device_code": device_code})),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, r#"{"status":"slow_down"}"#);
}

#[tokio::test]
async fn bad_expired_and_used_codes_share_one_error() {
    let state = test_state().await;
    seed_member(&state.db).await;
    let app = create_router(state.clone());

    let mut bodies = Vec::new();
    for (path, ip) in [
        ("/api/link/lookup", "203.0.113.21"),
        ("/api/link/approve", "203.0.113.22"),
        ("/api/link/deny", "203.0.113.23"),
    ] {
        let (status, body) = send(
            &app,
            from_xff(
                ip,
                Method::POST,
                path,
                Some(json!({"user_code": "ZZZZ-ZZZZ"})),
                Some(SESSION),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{path} {body}");
        bodies.push(body);
    }

    let expired = start_link(&app, "Expire Me", "1.0.0").await;
    let expired_user = expired["user_code"].as_str().unwrap().to_string();
    let expired_device = expired["device_code"].as_str().unwrap().to_string();
    state
        .db
        .client
        .query(
            "UPDATE device_link SET expires_at = time::now() - 1s WHERE device_label = 'Expire Me'",
        )
        .await
        .unwrap()
        .check()
        .unwrap();
    let (status, body) = send(
        &app,
        trusted(
            Method::POST,
            "/api/link/poll",
            Some(json!({"device_code": expired_device})),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(json_of(&body)["status"], "expired");

    for (path, ip) in [
        ("/api/link/lookup", "203.0.113.31"),
        ("/api/link/approve", "203.0.113.32"),
        ("/api/link/deny", "203.0.113.33"),
    ] {
        let (status, body) = send(
            &app,
            from_xff(
                ip,
                Method::POST,
                path,
                Some(json!({"user_code": expired_user})),
                Some(SESSION),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{path} {body}");
        bodies.push(body);
    }

    let removed = state.db.cleanup_expired_device_links().await.unwrap();
    assert!(removed >= 1, "expired codes are deleted");
    let removed_again = state.db.cleanup_expired_device_links().await.unwrap();
    assert_eq!(
        removed_again, 0,
        "a second pass finds nothing left to delete"
    );

    let used = start_link(&app, "Used Once", "1.0.0").await;
    let used_user = used["user_code"].as_str().unwrap().to_string();
    let used_device = used["device_code"].as_str().unwrap().to_string();
    let (status, body) = send(
        &app,
        trusted(
            Method::POST,
            "/api/link/approve",
            Some(json!({"user_code": used_user})),
            Some(SESSION),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = send(
        &app,
        trusted(
            Method::POST,
            "/api/link/poll",
            Some(json!({"device_code": used_device})),
            None,
        ),
    )
    .await;
    assert_eq!(json_of(&body)["status"], "approved", "{status} {body}");

    for (path, ip) in [
        ("/api/link/lookup", "203.0.113.41"),
        ("/api/link/approve", "203.0.113.42"),
        ("/api/link/deny", "203.0.113.43"),
    ] {
        let (status, body) = send(
            &app,
            from_xff(
                ip,
                Method::POST,
                path,
                Some(json!({"user_code": used_user})),
                Some(SESSION),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{path} {body}");
        bodies.push(body);
    }

    assert!(bodies.iter().all(|body| body == INVALID), "{bodies:?}");
}

#[tokio::test]
async fn signed_out_approve_is_rejected() {
    let state = test_state().await;
    let app = create_router(state);
    let (status, body) = send(
        &app,
        trusted(
            Method::POST,
            "/api/link/approve",
            Some(json!({"user_code": "ABCD-2345"})),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    let (status, _) = send(
        &app,
        trusted(
            Method::POST,
            "/api/link/lookup",
            Some(json!({"user_code": "ABCD-2345"})),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = send(
        &app,
        trusted(
            Method::POST,
            "/api/link/deny",
            Some(json!({"user_code": "ABCD-2345"})),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn codes_in_the_query_string_are_rejected() {
    let state = test_state().await;
    seed_member(&state.db).await;
    let app = create_router(state.clone());
    let started = start_link(&app, "Desk", "1.0.0").await;
    let user_code = started["user_code"].as_str().unwrap();

    let (status, body) = send(
        &app,
        trusted(
            Method::POST,
            &format!("/api/link/lookup?user_code={user_code}"),
            Some(json!({"user_code": user_code})),
            Some(SESSION),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        body,
        r#"{"error":"codes must be sent in the request body"}"#
    );
    assert!(!body.contains(user_code));

    let (status, body) = send(
        &app,
        trusted(
            Method::GET,
            &format!("/api/link/approve?user_code={user_code}"),
            None,
            Some(SESSION),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(!body.contains(user_code));

    let (status, body) = send(
        &app,
        trusted(
            Method::POST,
            "/api/link/lookup",
            Some(json!({"user_code": user_code})),
            Some(SESSION),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body code still works: {body}");
    assert_eq!(json_of(&body)["device_label"], "Desk");
}

#[tokio::test]
async fn device_label_and_app_version_are_bounded() {
    let state = test_state().await;
    let app = create_router(state);
    let cases = [
        json!({"device_label": "", "app_version": "1.0.0"}),
        json!({"device_label": "a".repeat(65), "app_version": "1.0.0"}),
        json!({"device_label": "bad\nname", "app_version": "1.0.0"}),
        json!({"device_label": "Desk", "app_version": ""}),
        json!({"device_label": "Desk", "app_version": "1.0.0 beta"}),
        json!({"device_label": "Desk", "app_version": "a".repeat(33)}),
    ];
    for (index, body) in cases.into_iter().enumerate() {
        let (status, text) = send(
            &app,
            from_xff(
                &format!("203.0.113.{}", 60 + index),
                Method::POST,
                "/api/link/start",
                Some(body),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");
    }
}

#[tokio::test]
async fn rate_limits_are_per_ip_separate_and_trusted_proxy_aware() {
    let state = test_state().await;
    seed_member(&state.db).await;
    let app = create_router(state);
    let start_body = json!({"device_label": "Desk", "app_version": "1.0.0"});

    let mut first_device = String::new();
    let mut first_user = String::new();
    for i in 0..LINK_START_BURST {
        let (status, body) = send(
            &app,
            from_xff(
                "203.0.113.50",
                Method::POST,
                "/api/link/start",
                Some(start_body.clone()),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "start {i}: {body}");
        if i == 0 {
            first_device = json_of(&body)["device_code"].as_str().unwrap().to_string();
            first_user = json_of(&body)["user_code"].as_str().unwrap().to_string();
        }
    }
    let (status, body) = send(
        &app,
        from_xff(
            "203.0.113.50",
            Method::POST,
            "/api/link/start",
            Some(start_body.clone()),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert!(body.contains("Too Many Requests"), "{body}");

    let (status, body) = send(
        &app,
        from_xff(
            "203.0.113.51",
            Method::POST,
            "/api/link/start",
            Some(start_body.clone()),
            None,
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "other IP has its own start bucket: {body}"
    );

    let (status, body) = send(
        &app,
        from_xff(
            "203.0.113.50",
            Method::POST,
            "/api/link/poll",
            Some(json!({"device_code": first_device})),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "poll bucket is separate: {body}");
    assert_eq!(json_of(&body)["status"], "pending");

    for i in 0..LINK_USER_BURST {
        let (status, body) = send(
            &app,
            from_xff(
                "203.0.113.50",
                Method::POST,
                "/api/link/lookup",
                Some(json!({"user_code": first_user})),
                Some(SESSION),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "lookup {i}: {body}");
    }
    let (status, body) = send(
        &app,
        from_xff(
            "203.0.113.50",
            Method::POST,
            "/api/link/lookup",
            Some(json!({"user_code": first_user})),
            Some(SESSION),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert!(body.contains("Too Many Requests"), "{body}");

    for n in 0..LINK_POLL_BURST {
        let xff = format!("198.51.100.{n}");
        let (status, _) = send(
            &app,
            req(
                Method::POST,
                "/api/link/poll",
                Some(json!({"device_code": "ab".repeat(32)})),
                None,
                [198, 51, 100, 8],
                &xff,
                Some(SITE_ORIGIN),
            ),
        )
        .await;
        assert_ne!(
            status,
            StatusCode::TOO_MANY_REQUESTS,
            "rotating X-Forwarded-For from an untrusted peer must stay on one bucket"
        );
    }
    let (status, body) = send(
        &app,
        req(
            Method::POST,
            "/api/link/poll",
            Some(json!({"device_code": "ab".repeat(32)})),
            None,
            [198, 51, 100, 8],
            "203.0.113.99",
            Some(SITE_ORIGIN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
}

#[tokio::test]
async fn wrong_user_codes_are_limited_per_ip() {
    let state = test_state().await;
    seed_member(&state.db).await;
    let app = create_router(state);
    let started = start_link(&app, "Desk", "1.0.0").await;
    let user_code = started["user_code"].as_str().unwrap().to_string();

    for i in 0..LINK_WRONG_CODE_LIMIT {
        let (status, body) = send(
            &app,
            from_xff(
                "203.0.113.70",
                Method::POST,
                "/api/link/lookup",
                Some(json!({"user_code": "ZZZZ-ZZZZ"})),
                Some(SESSION),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{i} {body}");
        assert_eq!(body, INVALID);
    }
    let (status, body) = send(
        &app,
        from_xff(
            "203.0.113.70",
            Method::POST,
            "/api/link/approve",
            Some(json!({"user_code": user_code})),
            Some(SESSION),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert!(body.contains("too many invalid codes"), "{body}");

    let (status, body) = send(
        &app,
        from_xff(
            "203.0.113.71",
            Method::POST,
            "/api/link/lookup",
            Some(json!({"user_code": user_code})),
            Some(SESSION),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a different IP is not blocked: {body}"
    );
}

struct Capture(Arc<Mutex<Vec<u8>>>);

struct CaptureWriter(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for CaptureWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("log buf").extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
    type Writer = CaptureWriter;

    fn make_writer(&'a self) -> Self::Writer {
        CaptureWriter(self.0.clone())
    }
}

#[tokio::test(flavor = "current_thread")]
async fn logs_do_not_contain_codes_or_the_token() {
    let state = test_state().await;
    seed_member(&state.db).await;
    let app = create_router(state);
    let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(Capture(buf.clone()))
        .with_env_filter(tracing_subscriber::EnvFilter::new(
            "scuffed_site_server=trace",
        ))
        .without_time()
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);

    let started = start_link(&app, "Living Room PC", "0.4.2").await;
    let user_code = started["user_code"].as_str().unwrap().to_string();
    let device_code = started["device_code"].as_str().unwrap().to_string();
    let _ = send(
        &app,
        trusted(
            Method::POST,
            &format!("/api/link/lookup?user_code={user_code}&device_code={device_code}"),
            Some(json!({"user_code": user_code})),
            Some(SESSION),
        ),
    )
    .await;
    let (status, _) = send(
        &app,
        trusted(
            Method::POST,
            "/api/link/approve",
            Some(json!({"user_code": user_code})),
            Some(SESSION),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = send(
        &app,
        trusted(
            Method::POST,
            "/api/link/poll",
            Some(json!({"device_code": device_code})),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let token = json_of(&body)["token"].as_str().unwrap().to_string();

    drop(guard);
    let logs = String::from_utf8(buf.lock().expect("log buf").clone()).unwrap();
    assert!(
        logs.contains("device link started"),
        "subscriber captured nothing useful: {logs}"
    );
    assert!(
        logs.contains("device link token handed over"),
        "handover was not traced: {logs}"
    );
    assert!(!logs.contains(&user_code), "{logs}");
    assert!(!logs.contains(&canonical_user_code(&user_code)), "{logs}");
    assert!(!logs.contains(&device_code), "{logs}");
    assert!(!logs.contains(&token), "{logs}");
}

fn authed_post(uri: &str, body: Value, origin: Option<&str>, cookie: bool) -> Request<Body> {
    let mut request = req(
        Method::POST,
        uri,
        Some(body),
        if cookie { None } else { Some(SESSION) },
        [127, 0, 0, 1],
        "203.0.113.110",
        origin,
    );
    if cookie {
        request.headers_mut().insert(
            header::COOKIE,
            axum::http::HeaderValue::from_str(&format!("sc_session={SESSION}")).unwrap(),
        );
    }
    request
}

#[tokio::test]
async fn approve_and_deny_reject_a_valid_session_without_the_allowed_origin() {
    let state = test_state().await;
    seed_member(&state.db).await;
    let app = create_router(state);
    let approve_me = start_link(&app, "Living Room PC", "0.4.2").await;
    let approve_code = approve_me["user_code"].as_str().unwrap();
    let approve_device = approve_me["device_code"].as_str().unwrap();
    let deny_me = start_link(&app, "Desk", "0.4.2").await;
    let deny_code = deny_me["user_code"].as_str().unwrap();
    let deny_device = deny_me["device_code"].as_str().unwrap();

    for (path, code, cookie, origin) in [
        ("/api/link/approve", approve_code, false, None),
        (
            "/api/link/approve",
            approve_code,
            false,
            Some(FOREIGN_ORIGIN),
        ),
        ("/api/link/approve", approve_code, true, None),
        (
            "/api/link/approve",
            approve_code,
            true,
            Some(FOREIGN_ORIGIN),
        ),
        ("/api/link/deny", deny_code, false, None),
        ("/api/link/deny", deny_code, false, Some(FOREIGN_ORIGIN)),
        ("/api/link/deny", deny_code, true, None),
        ("/api/link/deny", deny_code, true, Some(FOREIGN_ORIGIN)),
    ] {
        let (status, body) = send(
            &app,
            authed_post(path, json!({"user_code": code}), origin, cookie),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "{path} cookie={cookie} {body}"
        );
        assert_eq!(body, r#"{"error":"origin not allowed"}"#);
        assert!(!body.contains(code), "{body}");
    }

    let (status, body) = send(
        &app,
        trusted(Method::GET, "/api/stats/tokens", None, Some(SESSION)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(json_of(&body).as_array().unwrap().len(), 0, "{body}");

    for device in [approve_device, deny_device] {
        let (status, body) = send(
            &app,
            trusted(
                Method::POST,
                "/api/link/poll",
                Some(json!({"device_code": device})),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(json_of(&body)["status"], "pending");
    }

    let (status, body) = send(
        &app,
        trusted(
            Method::POST,
            "/api/link/approve",
            Some(json!({"user_code": approve_code})),
            Some(SESSION),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "allowed origin still approves: {body}"
    );

    let (status, body) = send(
        &app,
        authed_post(
            "/api/link/deny",
            json!({"user_code": deny_code}),
            Some(SITE_ORIGIN),
            true,
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "allowed origin still denies: {body}"
    );
}

#[tokio::test]
async fn lookup_returns_device_label_app_version_and_created_at() {
    let state = test_state().await;
    seed_member(&state.db).await;
    let app = create_router(state);
    let started = start_link(&app, "Living Room PC", "0.4.2").await;
    let user_code = started["user_code"].as_str().unwrap();

    let (status, body) = send(
        &app,
        trusted(
            Method::POST,
            "/api/link/lookup",
            Some(json!({"user_code": user_code})),
            Some(SESSION),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let lookup = json_of(&body);
    assert_eq!(lookup["device_label"], "Living Room PC");
    assert_eq!(lookup["app_version"], "0.4.2");
    let created_at = lookup["created_at"].as_str().unwrap();
    let created = chrono::DateTime::parse_from_rfc3339(created_at).unwrap();
    let age = chrono::Utc::now().signed_duration_since(created.with_timezone(&chrono::Utc));
    assert!(
        age.num_seconds() >= 0 && age.num_seconds() < 60,
        "{created_at}"
    );
}

#[tokio::test]
async fn concurrent_polls_after_approval_hand_the_token_over_once() {
    let state = test_state().await;
    seed_member(&state.db).await;
    let app = create_router(state);
    let started = start_link(&app, "Living Room PC", "0.4.2").await;
    let user_code = started["user_code"].as_str().unwrap().to_string();
    let device_code = started["device_code"].as_str().unwrap().to_string();
    let (status, body) = send(
        &app,
        trusted(
            Method::POST,
            "/api/link/approve",
            Some(json!({"user_code": user_code})),
            Some(SESSION),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let first = trusted(
        Method::POST,
        "/api/link/poll",
        Some(json!({"device_code": device_code})),
        None,
    );
    let second = trusted(
        Method::POST,
        "/api/link/poll",
        Some(json!({"device_code": device_code})),
        None,
    );
    let ((status_a, body_a), (status_b, body_b)) =
        tokio::join!(send(&app, first), send(&app, second));
    assert_eq!(status_a, StatusCode::OK, "{body_a}");
    assert_eq!(status_b, StatusCode::OK, "{body_b}");
    let bodies = [json_of(&body_a), json_of(&body_b)];
    let tokens: Vec<&str> = bodies
        .iter()
        .filter_map(|body| body["token"].as_str())
        .collect();
    assert_eq!(tokens.len(), 1, "{body_a} {body_b}");
    assert_eq!(tokens[0].len(), 64, "{body_a} {body_b}");
    assert_eq!(
        bodies
            .iter()
            .filter(|body| body["status"] == "expired" && body.get("token").is_none())
            .count(),
        1,
        "{body_a} {body_b}"
    );
    assert_eq!(
        bodies
            .iter()
            .filter(|body| body["status"] == "approved")
            .count(),
        1,
        "{body_a} {body_b}"
    );
}

#[tokio::test]
async fn deny_then_poll_is_denied_and_approve_is_the_generic_error() {
    let state = test_state().await;
    seed_member(&state.db).await;
    let app = create_router(state);
    let started = start_link(&app, "Desk", "1.0.0").await;
    let user_code = started["user_code"].as_str().unwrap();
    let device_code = started["device_code"].as_str().unwrap();

    let (status, body) = send(
        &app,
        trusted(
            Method::POST,
            "/api/link/deny",
            Some(json!({"user_code": user_code})),
            Some(SESSION),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = send(
        &app,
        trusted(
            Method::POST,
            "/api/link/poll",
            Some(json!({"device_code": device_code})),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, r#"{"status":"denied"}"#);

    let (status, body) = send(
        &app,
        trusted(
            Method::POST,
            "/api/link/approve",
            Some(json!({"user_code": user_code})),
            Some(SESSION),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body, INVALID);
}

#[tokio::test]
async fn approve_after_ten_minute_expiry_mints_no_token() {
    let state = test_state().await;
    seed_member(&state.db).await;
    let app = create_router(state.clone());
    let started = start_link(&app, "Expire Me", "1.0.0").await;
    assert_eq!(started["expires_in"], DEVICE_LINK_TTL_SECS);
    assert_eq!(DEVICE_LINK_TTL_SECS, 600);
    let user_code = started["user_code"].as_str().unwrap().to_string();
    let device_code = started["device_code"].as_str().unwrap().to_string();
    state
        .db
        .client
        .query(
            "UPDATE device_link SET expires_at = time::now() - 600s WHERE device_label = 'Expire Me'",
        )
        .await
        .unwrap()
        .check()
        .unwrap();

    let (status, body) = send(
        &app,
        trusted(
            Method::POST,
            "/api/link/approve",
            Some(json!({"user_code": user_code})),
            Some(SESSION),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body, INVALID);

    let (status, body) = send(
        &app,
        trusted(Method::GET, "/api/stats/tokens", None, Some(SESSION)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(json_of(&body).as_array().unwrap().len(), 0, "{body}");

    let (status, body) = send(
        &app,
        trusted(
            Method::POST,
            "/api/link/poll",
            Some(json!({"device_code": device_code})),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, r#"{"status":"expired"}"#);
}

#[tokio::test]
async fn poll_slow_down_and_per_ip_limit_sets_retry_after() {
    let state = test_state().await;
    let app = create_router(state);
    let started = start_link(&app, "Desk", "1.0.0").await;
    let device_code = started["device_code"].as_str().unwrap();
    let ip = "203.0.113.90";

    for i in 0..LINK_POLL_BURST {
        let (status, body) = send(
            &app,
            from_xff(
                ip,
                Method::POST,
                "/api/link/poll",
                Some(json!({"device_code": device_code})),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "poll {i}: {body}");
        if i == 0 {
            assert_eq!(body, r#"{"status":"pending"}"#);
        } else {
            assert_eq!(body, r#"{"status":"slow_down"}"#);
        }
    }

    let (status, headers, body) = send_full(
        &app,
        from_xff(
            ip,
            Method::POST,
            "/api/link/poll",
            Some(json!({"device_code": device_code})),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    let retry_after = headers
        .get(header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let wait: u64 = retry_after
        .parse()
        .unwrap_or_else(|_| panic!("Retry-After must be seconds, got {retry_after:?} body {body}"));
    assert!(wait >= 1, "Retry-After={retry_after} body {body}");
}
