//! HTTP coverage for recognizer pack downloads.
//!
//! Fixture bytes are synthetic text. Nothing here is game art or an image.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

use scuffed_auth::SessionConfig;
use scuffed_auth::crypto::hash_session_token;
use scuffed_db::migrations::run_migrations;
use scuffed_db::{Database, OrgRole};
use scuffed_site_server::create_router;
use scuffed_site_server::extractors::TOKEN_CHECK_UNAUTHORIZED;
use scuffed_site_server::state::{AppState, OAuthConfig};

const VALID: &str = "pack-token-valid";
const REVOKED: &str = "pack-token-revoked";
const EXPIRED: &str = "pack-token-expired";
const SECRET: &[u8] = b"SECRET-BYTES-NOT-A-PACK\n";

struct TempTree {
    root: PathBuf,
}

impl TempTree {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "scuffed-http-packs-{label}-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    fn path(&self) -> &Path {
        &self.root
    }
}

impl Drop for TempTree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn write_packs(dir: &Path, files: &[(&str, &str, &[u8])]) {
    std::fs::create_dir_all(dir).unwrap();
    let mut manifest = Vec::new();
    for (name, version, bytes) in files {
        std::fs::write(dir.join(name), bytes).unwrap();
        manifest.push(json!({
            "name": name,
            "version": version,
            "sha256": sha256_hex(bytes),
            "size": bytes.len(),
        }));
    }
    std::fs::write(
        dir.join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
}

async fn test_state(packs_dir: Option<PathBuf>) -> AppState {
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
        packs_dir,
        notifier: None,
        nostr_challenge_key: *blake3::hash(b"pack-challenge-key").as_bytes(),
        consumed_challenges: scuffed_site_server::challenge_store::ConsumedChallengeStore::new(),
        nostr_rate_limiter: scuffed_site_server::nostr_rate_limit::NostrRateLimiter::new(),
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
    let member = state
        .db
        .create_member("packmember", "Tracker", OrgRole::Member)
        .await
        .expect("member");
    state
        .db
        .create_daemon_token(&member.id, VALID, "valid")
        .await
        .expect("valid token");
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
    let token_hash = hash_session_token(EXPIRED);
    state
        .db
        .client
        .query(
            "CREATE daemon_token SET \
                member_id = $mid, \
                token_hash = $tok, \
                label = 'expired', \
                is_active = true, \
                created_at = time::now(), \
                expires_at = time::now() - 1d",
        )
        .bind(("mid", member.id.clone()))
        .bind(("tok", token_hash))
        .await
        .expect("insert expired")
        .check()
        .expect("expired token");
    member.id
}

async fn call(
    app: &axum::Router,
    uri: &str,
    peer: [u8; 4],
    bearer: Option<&str>,
) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let mut builder = Request::builder().method("GET").uri(uri);
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
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, headers, bytes.to_vec())
}

fn header_values(headers: &axum::http::HeaderMap, name: &header::HeaderName) -> Vec<String> {
    headers
        .get_all(name)
        .iter()
        .map(|value| value.to_str().unwrap_or("").to_string())
        .collect()
}

fn assert_rate_limit(headers: &axum::http::HeaderMap, body: &[u8]) {
    assert_eq!(
        header_values(headers, &header::CACHE_CONTROL),
        ["no-store".to_string()]
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
}

fn packs_disabled() -> Vec<u8> {
    serde_json::to_vec(&json!({"error": "packs_disabled"})).unwrap()
}

#[tokio::test]
async fn missing_bad_and_revoked_tokens_share_one_401() {
    let tree = TempTree::new("auth");
    let packs = tree.path().join("packs");
    write_packs(&packs, &[("hero-icons.bin", "1", b"not-an-image\n")]);
    let state = test_state(Some(packs)).await;
    seed_member(&state).await;
    let app = create_router(state);
    let unauthorized = serde_json::to_vec(&json!({"error": TOKEN_CHECK_UNAUTHORIZED})).unwrap();
    let peer = [203, 0, 113, 40];

    let cases = [None, Some("not-a-real-token"), Some(REVOKED), Some(EXPIRED)];
    for bearer in cases {
        for uri in ["/api/tracker/packs", "/api/tracker/packs/hero-icons.bin"] {
            let (status, headers, body) = call(&app, uri, peer, bearer).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{uri} bearer={bearer:?}");
            assert_eq!(
                header_values(&headers, &header::CACHE_CONTROL),
                ["no-store".to_string()],
                "{uri} 401 cache"
            );
            assert_eq!(body, unauthorized, "{uri} bearer={bearer:?}");
        }
    }

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/tracker/packs")
                .header(header::COOKIE, "session=not-a-daemon-token")
                .extension(axum::extract::ConnectInfo(SocketAddr::from((
                    [203, 0, 113, 47],
                    44000,
                ))))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(bytes.as_ref(), unauthorized);
}

#[tokio::test]
async fn disabled_directory_returns_503() {
    let peer = [203, 0, 113, 44];
    let disabled = packs_disabled();

    let unset = create_router(test_state(None).await);
    for uri in ["/api/tracker/packs", "/api/tracker/packs/hero-icons.bin"] {
        let (status, headers, body) = call(&unset, uri, peer, None).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{uri}");
        assert_eq!(body, disabled, "{uri}");
        assert_eq!(
            header_values(&headers, &header::CACHE_CONTROL),
            ["private, no-store".to_string()]
        );
        assert_eq!(
            header_values(&headers, &header::CONTENT_TYPE),
            ["application/json".to_string()]
        );
    }

    let state = test_state(Some(PathBuf::from("/tmp/scuffed-packs-missing"))).await;
    let member = seed_member(&state).await;
    let missing = create_router(state);
    let (status, _, body) = call(&missing, "/api/tracker/packs", peer, Some(VALID)).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body, disabled);
    let _ = member;

    let tree = TempTree::new("file-not-dir");
    let file_path = tree.path().join("not-a-dir");
    std::fs::write(&file_path, b"nope").unwrap();
    let file_app = create_router(test_state(Some(file_path)).await);
    let (status, _, body) = call(&file_app, "/api/tracker/packs", peer, None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body, disabled);
}

#[tokio::test]
async fn list_and_download_sha_matches_and_does_not_write() {
    let tree = TempTree::new("ok");
    let packs = tree.path().join("packs");
    let hero = b"not-an-image\nhero-pack-v1\n";
    let notes = b"second-pack-fixture\n";
    write_packs(
        &packs,
        &[("hero-icons.bin", "1", hero), ("notes.txt", "2", notes)],
    );
    let state = test_state(Some(packs)).await;
    let member_id = seed_member(&state).await;
    let before_audit = state.db.count_table("audit_log").await.unwrap();
    let before_tokens = state.db.count_table("daemon_token").await.unwrap();
    let before_matches = state.db.count_table("personal_match").await.unwrap();
    let before_used = state.db.list_daemon_tokens(&member_id).await.unwrap();
    assert!(before_used.iter().all(|token| token.last_used_at.is_none()));

    let app = create_router(state.clone());
    let peer = [203, 0, 113, 42];
    let (status, headers, body) = call(&app, "/api/tracker/packs", peer, Some(VALID)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        header_values(&headers, &header::CACHE_CONTROL),
        ["private, no-store".to_string()]
    );
    let list: Vec<Value> = serde_json::from_slice(&body).unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0]["name"], "hero-icons.bin");
    assert_eq!(list[0]["version"], "1");
    assert_eq!(list[0]["sha256"], sha256_hex(hero));
    assert_eq!(list[0]["size"], hero.len());
    assert_eq!(list[0].as_object().unwrap().len(), 4);
    assert_eq!(list[1]["name"], "notes.txt");
    assert_eq!(list[1]["sha256"], sha256_hex(notes));

    let (status, headers, body) =
        call(&app, "/api/tracker/packs/hero-icons.bin", peer, Some(VALID)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, hero);
    assert_eq!(sha256_hex(&body), sha256_hex(hero));
    assert_eq!(
        header_values(&headers, &header::CACHE_CONTROL),
        ["private, no-store".to_string()]
    );
    assert_eq!(
        header_values(&headers, &header::CONTENT_TYPE),
        ["application/octet-stream".to_string()]
    );
    assert_eq!(
        header_values(&headers, &header::CONTENT_DISPOSITION),
        ["attachment; filename=\"hero-icons.bin\"".to_string()]
    );
    assert_eq!(
        header_values(&headers, &header::ETAG),
        [format!("\"{}\"", sha256_hex(hero))]
    );
    assert!(
        headers.get(header::X_CONTENT_TYPE_OPTIONS).is_none(),
        "nosniff comes from the scuffed-server security layer, not this route"
    );
    assert!(!body.windows(8).any(|window| window == b"\x89PNG\r\n\x1a\n"));

    let (status, _, notes_body) =
        call(&app, "/api/tracker/packs/notes.txt", peer, Some(VALID)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(notes_body, notes);
    assert_eq!(sha256_hex(&notes_body), list[1]["sha256"].as_str().unwrap());

    assert_eq!(
        state.db.count_table("audit_log").await.unwrap(),
        before_audit
    );
    assert_eq!(
        state.db.count_table("daemon_token").await.unwrap(),
        before_tokens
    );
    assert_eq!(
        state.db.count_table("personal_match").await.unwrap(),
        before_matches
    );
    let after_used = state.db.list_daemon_tokens(&member_id).await.unwrap();
    assert!(
        after_used.iter().all(|token| token.last_used_at.is_none()),
        "pack auth must not stamp last_used_at"
    );
}

#[tokio::test]
async fn unknown_name_is_404() {
    let tree = TempTree::new("missing-name");
    let packs = tree.path().join("packs");
    write_packs(&packs, &[("hero-icons.bin", "1", b"not-an-image\n")]);
    let state = test_state(Some(packs)).await;
    seed_member(&state).await;
    let app = create_router(state);
    let peer = [203, 0, 113, 45];
    let expected = serde_json::to_vec(&json!({"error": "pack_not_found"})).unwrap();
    for uri in [
        "/api/tracker/packs/missing.bin",
        "/api/tracker/packs/Hero-Icons.bin",
    ] {
        let (status, _, body) = call(&app, uri, peer, Some(VALID)).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
        assert_eq!(body, expected, "{uri}");
    }
}

#[tokio::test]
async fn traversal_is_rejected() {
    let tree = TempTree::new("traversal");
    let packs = tree.path().join("packs");
    let outside = tree.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    let secret_path = outside.join("secret.bin");
    std::fs::write(&secret_path, SECRET).unwrap();
    write_packs(&packs, &[("hero-icons.bin", "1", b"not-an-image\n")]);
    std::os::unix::fs::symlink(&secret_path, packs.join("linked.bin")).unwrap();

    let state = test_state(Some(packs.clone())).await;
    seed_member(&state).await;
    let app = create_router(state);
    let peer = [203, 0, 113, 43];
    let not_found = serde_json::to_vec(&json!({"error": "pack_not_found"})).unwrap();

    for uri in [
        "/api/tracker/packs/..",
        "/api/tracker/packs/%2e%2e",
        "/api/tracker/packs/linked.bin",
        "/api/tracker/packs/hero-icons.bin/../../secret.bin",
        "/api/tracker/packs/..%2f..%2fsecret.bin",
    ] {
        let (status, _, body) = call(&app, uri, peer, Some(VALID)).await;
        assert_ne!(status, StatusCode::OK, "{uri} must not succeed");
        assert!(
            status == StatusCode::NOT_FOUND || status == StatusCode::BAD_REQUEST,
            "{uri} status {status}"
        );
        assert!(
            !body.windows(SECRET.len()).any(|window| window == SECRET),
            "{uri} leaked the outside file"
        );
    }
    let (status, _, body) = call(&app, "/api/tracker/packs/linked.bin", peer, Some(VALID)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body, not_found);

    let hostile = tree.path().join("hostile");
    std::fs::create_dir_all(&hostile).unwrap();
    std::fs::write(
        hostile.join("manifest.json"),
        serde_json::to_vec(&json!([{
            "name": "../secret.bin",
            "version": "1",
            "sha256": sha256_hex(SECRET),
            "size": SECRET.len(),
        }]))
        .unwrap(),
    )
    .unwrap();
    let hostile_app = create_router(test_state(Some(hostile)).await);
    let (status, _, body) = call(
        &hostile_app,
        "/api/tracker/packs/hero-icons.bin",
        peer,
        Some(VALID),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body, packs_disabled());
    assert!(!body.windows(SECRET.len()).any(|window| window == SECRET));

    let linked = tree.path().join("linked-manifest");
    std::fs::create_dir_all(&linked).unwrap();
    std::os::unix::fs::symlink(&secret_path, linked.join("linked.bin")).unwrap();
    std::fs::write(
        linked.join("manifest.json"),
        serde_json::to_vec(&json!([{
            "name": "linked.bin",
            "version": "1",
            "sha256": sha256_hex(SECRET),
            "size": SECRET.len(),
        }]))
        .unwrap(),
    )
    .unwrap();
    let linked_state = test_state(Some(linked)).await;
    seed_member(&linked_state).await;
    let linked_app = create_router(linked_state);
    let (status, _, body) = call(
        &linked_app,
        "/api/tracker/packs/linked.bin",
        [203, 0, 113, 46],
        Some(VALID),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body, not_found);
    assert!(!body.windows(SECRET.len()).any(|window| window == SECRET));
}

#[tokio::test]
async fn rate_limit_json_429() {
    let tree = TempTree::new("rate");
    let packs = tree.path().join("packs");
    write_packs(&packs, &[("hero-icons.bin", "1", b"not-an-image\n")]);
    let state = test_state(Some(packs)).await;
    let app = create_router(state);
    let peer = [203, 0, 113, 41];

    for i in 0..8 {
        let (status, _, _) = call(&app, "/api/tracker/packs", peer, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "burst {i}");
    }
    let (status, headers, body) = call(&app, "/api/tracker/packs/hero-icons.bin", peer, None).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_rate_limit(&headers, &body);

    let (token_status, _, _) = call(&app, "/api/stats/token-check", peer, None).await;
    assert_eq!(
        token_status,
        StatusCode::UNAUTHORIZED,
        "the pack bucket must not consume the token-check bucket"
    );
}
