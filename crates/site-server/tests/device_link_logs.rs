//! Tracing capture for device-link sign-in.
//!
//! This is its own test binary on purpose. `tracing` caches each callsite's
//! interest for the whole process. The other device-link tests emit the same
//! log lines with no subscriber, and the first registration wins. If that
//! happens first, "device link token handed over" stays disabled and this
//! check cannot see it. Nothing else in this process emits those lines.

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
use scuffed_site_server::create_router;
use scuffed_site_server::state::{AppState, OAuthConfig};

const SESSION: &str = "link-session-token";
const SITE_ORIGIN: &str = "http://localhost:3000";

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

fn req(method: Method, uri: &str, body: Option<Value>, bearer: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-forwarded-for", "127.0.0.1")
        .header(header::ORIGIN, SITE_ORIGIN)
        .extension(axum::extract::ConnectInfo(SocketAddr::from((
            [127, 0, 0, 1],
            40000,
        ))));
    if let Some(token) = bearer {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let payload = body
        .map(|value| serde_json::to_vec(&value).unwrap())
        .unwrap_or_default();
    builder.body(Body::from(payload)).unwrap()
}

async fn send(app: &axum::Router, request: Request<Body>) -> (StatusCode, String) {
    let response = app.clone().oneshot(request).await.expect("response");
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

fn json_of(body: &str) -> Value {
    serde_json::from_str(body).unwrap_or(Value::Null)
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
        .with_writer(Capture(buf.clone()))
        .with_env_filter(tracing_subscriber::EnvFilter::new(
            "scuffed_site_server=trace",
        ))
        .without_time()
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);

    let (status, body) = send(
        &app,
        req(
            Method::POST,
            "/api/link/start",
            Some(json!({"device_label": "Living Room PC", "app_version": "0.4.2"})),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let started = json_of(&body);
    let user_code = started["user_code"].as_str().unwrap().to_string();
    let device_code = started["device_code"].as_str().unwrap().to_string();
    let canonical_user_code: String = user_code.chars().filter(|ch| *ch != '-').collect();

    let _ = send(
        &app,
        req(
            Method::POST,
            &format!("/api/link/lookup?user_code={user_code}&device_code={device_code}"),
            Some(json!({"user_code": user_code})),
            Some(SESSION),
        ),
    )
    .await;
    let (status, _) = send(
        &app,
        req(
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
        req(
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
        logs.contains("device link approved"),
        "approve was not traced: {logs}"
    );
    assert!(
        logs.contains("device link token handed over"),
        "handover was not traced: {logs}"
    );
    assert!(!logs.contains(&user_code), "{logs}");
    assert!(!logs.contains(&canonical_user_code), "{logs}");
    assert!(!logs.contains(&device_code), "{logs}");
    assert!(!logs.contains(&token), "{logs}");
}
