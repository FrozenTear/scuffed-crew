use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::routing::get;
use scuffed_auth::SessionConfig;
use scuffed_db::Database;
use scuffed_db::migrations::run_migrations;
use scuffed_site_server::{
    apply_api_method_not_allowed, create_router,
    notifications::Notifier,
    state::{AppState, OAuthConfig, relay_url_from_env},
    uploads,
};
use tower_http::compression::CompressionLayer;
use tower_http::compression::predicate::{And, DefaultPredicate, NotForContentType, Predicate};
use tracing_subscriber::EnvFilter;

mod collab;
mod routes;
mod security;

const DEV_SESSION_TOKEN: &str = "dev-session-token-do-not-use-in-production";

#[tokio::main]
async fn main() {
    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let oauth_config = OAuthConfig::from_env();

    // Init-only: root migrations + ensure EDITOR app user, then exit.
    // Use for separate migrate jobs; set SURREALDB_BOOTSTRAP=0 on long-lived app containers.
    if std::env::var("SURREALDB_MIGRATE_ONLY").ok().as_deref() == Some("1") {
        match scuffed_db::resolve_database_boot_mode_from_env() {
            Ok(scuffed_db::DatabaseBootMode::Remote) => {}
            Ok(scuffed_db::DatabaseBootMode::InMemoryDev) => {
                eprintln!(
                    "error: SURREALDB_MIGRATE_ONLY=1 requires a non-blank SURREALDB_URL (remote DB)"
                );
                std::process::exit(1);
            }
            Err(err) => {
                eprintln!("error: {err}");
                std::process::exit(1);
            }
        }
        Database::bootstrap_from_env()
            .await
            .expect("SURREALDB_MIGRATE_ONLY: bootstrap failed");
        tracing::info!("SURREALDB_MIGRATE_ONLY complete — exiting");
        return;
    }

    // Connect to SurrealDB (remote or in-memory fallback).
    // Prefer SURREALDB_AUTH_MODE=scoped + non-root user in production.
    // Remote scoped: optional root bootstrap (unless SURREALDB_BOOTSTRAP=0), then EDITOR app user.
    // PRODUCTION with an unset or blank SURREALDB_URL refuses to start (no in-memory DB).
    let boot_mode = scuffed_db::database_boot_mode_or_exit();
    let is_dev = boot_mode == scuffed_db::DatabaseBootMode::InMemoryDev;
    let db = if is_dev {
        tracing::info!("No SURREALDB_URL set, using in-memory database");
        let db = Database::connect_memory()
            .await
            .expect("Failed to create in-memory database");
        run_migrations(&db.client)
            .await
            .expect("Failed to run database migrations");
        scuffed_site_server::seed::seed_dev_data(&db, DEV_SESSION_TOKEN)
            .await
            .expect("Failed to seed dev data");
        tracing::info!("Dev data seeded — visit /api/dev/login to set session cookie");
        db
    } else {
        Database::connect_from_env()
            .await
            .expect("Failed to connect to SurrealDB")
    };

    // Emergency local admin password reset (production only).
    // A successful reset is recorded and the same BOOTSTRAP_ADMIN_* values are
    // ignored on later starts. Leftover values after an admin exists are also
    // ignored. Nothing in the environment or secrets file is deleted.
    if !is_dev && let Err(e) = scuffed_site_server::apply_bootstrap_admin_env(&db).await {
        tracing::error!("BOOTSTRAP_ADMIN_*: {e}");
    }

    let db = Arc::new(db);

    let upload_dir =
        PathBuf::from(std::env::var("UPLOAD_DIR").unwrap_or_else(|_| "data/uploads".to_string()));
    uploads::ensure_upload_dir(&upload_dir)
        .await
        .expect("Failed to create upload directory");

    let reports_configured = scuffed_site_server::stat_reports::reports_dir_from_env();
    let (reports_dir, reports_enabled) =
        scuffed_site_server::stat_reports::open_reports_dir(&reports_configured, &upload_dir).await;
    let packs_dir = scuffed_site_server::packs::open_packs_dir(
        scuffed_site_server::packs::packs_dir_from_env().as_deref(),
        &upload_dir,
        &reports_dir,
    );

    let notifier = Notifier::from_env();
    if notifier.is_none() {
        tracing::info!("Notifications not configured (Matrix/Discord) — running without");
    }

    // Nostr challenge signing key: from env, or a deterministic dev-only fallback.
    // Fail closed outside dev: refuse to boot rather than sign challenge tokens with
    // the publicly-known dev MAC key (mirrors the ENCRYPTION_KEY/PRODUCTION policy).
    let nostr_challenge_key: [u8; 32] = match std::env::var("NOSTR_CHALLENGE_SECRET") {
        // Reject whitespace-only secrets too: a `" "` value must fail closed in
        // production rather than boot with a weak, effectively-empty key.
        Ok(secret) if !secret.trim().is_empty() => *blake3::hash(secret.as_bytes()).as_bytes(),
        _ => {
            if !is_dev {
                panic!(
                    "NOSTR_CHALLENGE_SECRET is required when not in dev mode \
                     (Nostr challenge-token MAC key). Set a random 32+ byte value \
                     — install.sh/update.sh generate one — refusing to boot with the \
                     public dev fallback key."
                );
            }
            tracing::warn!("Using deterministic dev key for Nostr challenges — NOT for production");
            *blake3::hash(b"scuffed-crew-dev-nostr-challenge-key").as_bytes()
        }
    };

    // Single shared CryptoService from Database (no second from_env() load).
    let crypto = db.crypto.clone();
    if crypto.is_none() {
        tracing::info!("ENCRYPTION_KEY not set — Nostr key encryption disabled");
    }

    let relay_url = relay_url_from_env();
    if let Some(ref url) = relay_url {
        tracing::info!("Nostr relay configured: {url}");
    } else {
        tracing::info!("NOSTR_RELAY_URL not set (or blank) — Nostr relay features disabled");
    }

    // Start the persistent NIP-44 DM relay subscriber (Phase 5 real-time delivery,
    // [THE-878]). Falls back silently when relay or encryption is not configured —
    // clients keep using `POST /api/nostr/dm/sync` for polling-based delivery.
    let dm_events =
        scuffed_site_server::dm_subscriber::start(db.clone(), crypto.clone(), relay_url.clone());
    if dm_events.is_none() {
        tracing::info!("DM relay subscriber disabled (relay_url or encryption key not configured)");
    }

    let state = AppState {
        db: db.clone(),
        session_config: SessionConfig::default(),
        oauth_config,
        upload_dir,
        reports_dir: reports_dir.clone(),
        reports_enabled,
        packs_dir,
        notifier,
        nostr_challenge_key,
        consumed_challenges: scuffed_site_server::challenge_store::ConsumedChallengeStore::new(),
        nostr_rate_limiter: scuffed_site_server::nostr_rate_limit::NostrRateLimiter::new(),
        login_lockout: scuffed_site_server::login_lockout::LoginLockout::new(),
        link_code_attempts: scuffed_site_server::link_attempts::LinkCodeAttempts::new(),
        link_poll: scuffed_site_server::link_poll::LinkPollGate::system(),
        crypto,
        relay_url,
        dm_events,
        nip05_domain: scuffed_site_server::state::nip05_domain_from_env(),
        nip05_republish_enabled: scuffed_site_server::state::nip05_republish_enabled_from_env(),
        public_settings: scuffed_site_server::state::PublicSettingsCache::new(),
        leaderboard_cache: scuffed_site_server::leaderboard_cache::LeaderboardCache::from_env(),
    };

    // F-API-003: existing teams have no team_channel rows until backfill.
    scuffed_site_server::team_channels::backfill_on_startup(&state).await;

    // Strategy collab admission. Every socket holds a permit until it closes,
    // including sockets that never join a room. Unjoined sockets are closed
    // after the join deadline. See routes::ws.
    let (ws_global, ws_per_ip) = routes::ws::WsAdmission::limits_from_env();
    let rooms = Arc::new(collab::RoomManager::with_global_limit(ws_global));
    let ws_state = routes::ws::WsState {
        app: state.clone(),
        rooms,
        admission: Arc::new(routes::ws::WsAdmission::new(
            ws_global,
            ws_per_ip,
            scuffed_site_server::rate_limit::TrustedProxyIpKeyExtractor::from_env(),
        )),
        join_timeout: routes::ws::WS_JOIN_TIMEOUT,
    };

    // Hourly session cleanup. Device-link codes have their own 60 second timer.
    let cleanup_db = db.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
        loop {
            interval.tick().await;
            if let Err(e) = cleanup_db.cleanup_expired_sessions().await {
                tracing::error!("Session cleanup failed: {e}");
            }
        }
    });
    scuffed_site_server::link_cleanup::spawn_device_link_cleanup(db.clone());

    if reports_enabled {
        scuffed_site_server::stat_reports::spawn_sweeper(db.clone(), reports_dir);
    }

    // Build the unified router: existing org routes + strategy routes + chat + WebSocket,
    // then apply production middleware to the combined router.
    // Site CSP is enforcing unless CSP_ENFORCE=0. See `security`.
    let csp = security::SecurityPolicy::from_env();
    let app = create_router(state.clone())
        .merge(routes::strategy_routes(state.clone()))
        .route(
            "/api/chat/auth-token",
            axum::routing::post(routes::chat::provision_auth_token).with_state(state.clone()),
        )
        .route(
            "/api/chat/send-encrypted",
            axum::routing::post(routes::chat::send_encrypted).with_state(state.clone()),
        )
        .route(
            "/api/chat/send-public",
            axum::routing::post(routes::chat::send_public).with_state(state.clone()),
        )
        .route(
            "/api/chat/decrypt",
            axum::routing::post(routes::chat::decrypt_message).with_state(state),
        )
        .route(
            "/api/strategy/ws",
            get(routes::ws::websocket_handler).with_state(ws_state),
        );
    // After every route, including strategy and chat. create_router already
    // installed this fallback; calling it again covers routes merged above
    // and leaves routers that already have it alone.
    let app = apply_api_method_not_allowed(app)
        .layer(DefaultBodyLimit::max(10 * 1024 * 1024))
        .layer(response_compression_layer())
        .layer(axum::middleware::from_fn(move |req, next| {
            let csp = csp.clone();
            async move { security::apply(req, next, csp).await }
        }));

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3000);
    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("Clan platform server listening on {addr}");

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    // ConnectInfo is required by the auth rate limiter's peer-IP fallback.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await
    .unwrap();
}

/// Response compression for the unified server.
///
/// `GET /api/tracker/packs/{name}` sends `application/octet-stream` for a
/// `.tar` whose contents are already compressed. Compressing that body again
/// changes the sha256 the tracker checks. `application/x-tar` is skipped for
/// the same reason. Other responses keep the default predicate (tiny bodies,
/// images, server-sent events, and gRPC stay uncompressed).
fn response_compression_layer()
-> CompressionLayer<And<And<DefaultPredicate, NotForContentType>, NotForContentType>> {
    CompressionLayer::new().compress_when(
        DefaultPredicate::new()
            .and(NotForContentType::const_new("application/octet-stream"))
            .and(NotForContentType::const_new("application/x-tar")),
    )
}

#[cfg(test)]
mod compression_shell {
    use std::io::Read;
    use std::net::SocketAddr;

    use axum::body::Body;
    use axum::extract::ConnectInfo;
    use axum::http::{Method, Request, StatusCode, header};
    use http_body_util::BodyExt;
    use scuffed_auth::SessionConfig;
    use scuffed_db::OrgRole;
    use scuffed_db::migrations::run_migrations;
    use scuffed_site_server::create_router_with_dist;
    use scuffed_site_server::state::{AppState, OAuthConfig};
    use sha2::{Digest, Sha256};
    use tower::ServiceExt;

    #[tokio::test]
    async fn get_and_head_shell_round_trip_through_compression() {
        let root = std::env::temp_dir().join(format!("scuffed-compress-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("dist")).unwrap();
        std::fs::create_dir_all(root.join("uploads")).unwrap();
        std::fs::write(
            root.join("dist/index.html"),
            "<!DOCTYPE html><html><head><title>The Scuffed Crew</title></head><body>SPA-SHELL-MARKER</body></html>",
        )
        .unwrap();
        let db = scuffed_db::Database::connect_memory()
            .await
            .expect("in-memory DB");
        run_migrations(&db.client).await.expect("migrations");
        let state = AppState {
            db: std::sync::Arc::new(db),
            session_config: SessionConfig::default(),
            oauth_config: OAuthConfig {
                discord_client_id: String::new(),
                discord_client_secret: String::new(),
                google_client_id: String::new(),
                google_client_secret: String::new(),
                redirect_base_url: "https://crew.example.test".into(),
                allowed_origins: vec!["https://crew.example.test".into()],
            },
            upload_dir: root.join("uploads"),
            reports_dir: root.join("reports"),
            reports_enabled: true,
            packs_dir: None,
            notifier: None,
            nostr_challenge_key: [0u8; 32],
            consumed_challenges: scuffed_site_server::challenge_store::ConsumedChallengeStore::new(
            ),
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
        };
        // Same layer `main` puts around the router.
        let app = create_router_with_dist(state, root.join("dist"))
            .layer(super::response_compression_layer());

        let get_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/")
                    .header(header::ACCEPT_ENCODING, "gzip")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get_response.status(), StatusCode::OK);
        assert_eq!(
            get_response
                .headers()
                .get(header::CONTENT_ENCODING)
                .and_then(|value| value.to_str().ok()),
            Some("gzip")
        );
        let compressed = get_response.into_body().collect().await.unwrap().to_bytes();
        let mut decoded = String::new();
        flate2::read::GzDecoder::new(compressed.as_ref())
            .read_to_string(&mut decoded)
            .expect("gzip shell");
        assert!(decoded.contains("sc-settings"), "{decoded}");
        assert!(decoded.contains("SPA-SHELL-MARKER"), "{decoded}");

        let head_response = app
            .oneshot(
                Request::builder()
                    .method(Method::HEAD)
                    .uri("/")
                    .header(header::ACCEPT_ENCODING, "gzip")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(head_response.status(), StatusCode::OK);
        let head_bytes = head_response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes();
        assert_eq!(head_bytes.len(), 0);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn pack_download_skips_compression_and_json_stays_compressed() {
        let root =
            std::env::temp_dir().join(format!("scuffed-pack-compress-{}", uuid::Uuid::new_v4()));
        let packs = root.join("packs");
        std::fs::create_dir_all(root.join("dist")).unwrap();
        std::fs::create_dir_all(root.join("uploads")).unwrap();
        std::fs::create_dir_all(&packs).unwrap();
        std::fs::write(
            root.join("dist/index.html"),
            "<!DOCTYPE html><html><head></head><body>SPA-SHELL-MARKER</body></html>",
        )
        .unwrap();
        // Larger than the compressor's 32 byte floor, and repetitive so a
        // mistaken gzip/br pass would both set Content-Encoding and change
        // the bytes.
        let file_bytes = vec![b'P'; 4096];
        let file_sha = sha256_hex(&file_bytes);
        std::fs::write(packs.join("heroes-v1.tar"), &file_bytes).unwrap();
        let manifest = serde_json::json!([{
            "name": "heroes-v1.tar",
            "version": "1",
            "sha256": file_sha,
            "size": file_bytes.len(),
        }]);
        std::fs::write(
            packs.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();

        let db = scuffed_db::Database::connect_memory()
            .await
            .expect("in-memory DB");
        run_migrations(&db.client).await.expect("migrations");
        let member = db
            .create_member("packmember", "Tracker", OrgRole::Member)
            .await
            .expect("member");
        db.create_daemon_token(&member.id, "pack-token-valid", "valid")
            .await
            .expect("daemon token");
        let state = AppState {
            db: std::sync::Arc::new(db),
            session_config: SessionConfig::default(),
            oauth_config: OAuthConfig {
                discord_client_id: String::new(),
                discord_client_secret: String::new(),
                google_client_id: String::new(),
                google_client_secret: String::new(),
                redirect_base_url: "https://crew.example.test".into(),
                allowed_origins: vec!["https://crew.example.test".into()],
            },
            upload_dir: root.join("uploads"),
            reports_dir: root.join("reports"),
            reports_enabled: true,
            packs_dir: Some(packs),
            notifier: None,
            nostr_challenge_key: [0u8; 32],
            consumed_challenges: scuffed_site_server::challenge_store::ConsumedChallengeStore::new(
            ),
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
        };
        let app = create_router_with_dist(state, root.join("dist"))
            .layer(super::response_compression_layer());
        let peer = SocketAddr::from(([203, 0, 113, 50], 44000));

        let pack = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/tracker/packs/heroes-v1.tar")
                    .header(header::ACCEPT_ENCODING, "gzip, br")
                    .header(header::AUTHORIZATION, "Bearer pack-token-valid")
                    .extension(ConnectInfo(peer))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(pack.status(), StatusCode::OK);
        assert!(
            pack.headers().get(header::CONTENT_ENCODING).is_none(),
            "pack download must not set Content-Encoding, got {:?}",
            pack.headers().get(header::CONTENT_ENCODING)
        );
        assert_eq!(
            pack.headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("application/octet-stream")
        );
        assert_eq!(
            pack.headers()
                .get(header::CONTENT_LENGTH)
                .and_then(|v| v.to_str().ok()),
            Some("4096")
        );
        assert_eq!(
            pack.headers()
                .get(header::CACHE_CONTROL)
                .and_then(|v| v.to_str().ok()),
            Some("private, no-store")
        );
        let etag = format!("\"{file_sha}\"");
        assert_eq!(
            pack.headers()
                .get(header::ETAG)
                .and_then(|v| v.to_str().ok()),
            Some(etag.as_str())
        );
        assert_eq!(
            pack.headers()
                .get(header::CONTENT_DISPOSITION)
                .and_then(|v| v.to_str().ok()),
            Some("attachment; filename=\"heroes-v1.tar\"")
        );
        let pack_body = pack.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(sha256_hex(&pack_body), file_sha);
        assert_eq!(pack_body.as_ref(), file_bytes.as_slice());

        let raw_json = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/auth/providers")
                    .extension(ConnectInfo(peer))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(raw_json.status(), StatusCode::OK);
        assert!(raw_json.headers().get(header::CONTENT_ENCODING).is_none());
        let raw_body = raw_json.into_body().collect().await.unwrap().to_bytes();
        assert!(
            raw_body.len() >= 32,
            "JSON fixture must be large enough for the default compressor, got {}",
            raw_body.len()
        );
        assert!(raw_body.starts_with(b"{"), "providers body is JSON");

        let compressed = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/auth/providers")
                    .header(header::ACCEPT_ENCODING, "gzip, br")
                    .extension(ConnectInfo(peer))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(compressed.status(), StatusCode::OK);
        assert_eq!(
            compressed
                .headers()
                .get(header::CONTENT_ENCODING)
                .and_then(|v| v.to_str().ok()),
            Some("br"),
            "a normal JSON route must still be compressed"
        );
        assert_eq!(
            compressed
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("application/json")
        );
        let compressed_body = compressed.into_body().collect().await.unwrap().to_bytes();
        assert_ne!(compressed_body, raw_body);
        assert!(!compressed_body.starts_with(b"{"));

        let _ = std::fs::remove_dir_all(&root);
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        format!("{:x}", hasher.finalize())
    }
}
