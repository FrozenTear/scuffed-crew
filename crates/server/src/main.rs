use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::http::HeaderValue;
use axum::routing::get;
use scuffed_auth::SessionConfig;
use scuffed_db::migrations::run_migrations;
use scuffed_db::Database;
use scuffed_site_server::{
    create_router,
    notifications::MatrixNotifier,
    state::{AppState, OAuthConfig},
    uploads,
};
use tower_http::compression::CompressionLayer;
use tracing_subscriber::EnvFilter;

mod collab;
mod routes;

async fn security_headers(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();

    headers.insert(
        axum::http::header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        axum::http::header::X_FRAME_OPTIONS,
        HeaderValue::from_static("DENY"),
    );
    // Disable the legacy XSS filter — modern browsers ignore it and it can introduce
    // vulnerabilities in older ones.
    headers.insert(
        axum::http::HeaderName::from_static("x-xss-protection"),
        HeaderValue::from_static("0"),
    );
    headers.insert(
        axum::http::HeaderName::from_static("referrer-policy"),
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    headers.insert(
        axum::http::HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
    );

    // Only send HSTS in production — prevents breaking local dev over HTTP.
    if std::env::var("PRODUCTION").is_ok() {
        headers.insert(
            axum::http::header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=63072000; includeSubDomains; preload"),
        );
    }

    response
}

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
        if std::env::var("SURREALDB_URL").is_err() {
            panic!("SURREALDB_MIGRATE_ONLY=1 requires SURREALDB_URL (remote DB)");
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
    let is_dev = std::env::var("SURREALDB_URL").is_err();
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

    // Emergency local admin password reset (production only). Unset BOOTSTRAP_ADMIN_RESET after use.
    if !is_dev
        && std::env::var("BOOTSTRAP_ADMIN_RESET").ok().as_deref() == Some("1")
        && let Ok(new_password) = std::env::var("BOOTSTRAP_ADMIN_PASSWORD")
        && !new_password.is_empty()
    {
        let username = std::env::var("BOOTSTRAP_ADMIN_USERNAME")
            .unwrap_or_else(|_| "admin".to_string());
        match scuffed_auth::password::hash_password(&new_password) {
            Ok(hash) => match db.get_local_user_by_username(&username).await {
                Ok(Some((user, _))) => {
                    if let Err(e) = db.set_local_password_hash(&user.id, &hash).await {
                        tracing::error!("BOOTSTRAP_ADMIN_RESET failed to update hash: {e}");
                    } else {
                        tracing::warn!(
                            "BOOTSTRAP_ADMIN_RESET applied for local user '{username}' — remove BOOTSTRAP_ADMIN_RESET from env"
                        );
                    }
                }
                Ok(None) => tracing::error!(
                    "BOOTSTRAP_ADMIN_RESET: no local user '{username}' — create via first-boot setup first"
                ),
                Err(e) => tracing::error!("BOOTSTRAP_ADMIN_RESET lookup failed: {e}"),
            },
            Err(e) => tracing::error!("BOOTSTRAP_ADMIN_RESET hash failed: {e}"),
        }
    }

    let db = Arc::new(db);

    let upload_dir =
        PathBuf::from(std::env::var("UPLOAD_DIR").unwrap_or_else(|_| "data/uploads".to_string()));
    uploads::ensure_upload_dir(&upload_dir)
        .await
        .expect("Failed to create upload directory");

    let notifier = MatrixNotifier::from_env();
    if notifier.is_none() {
        tracing::info!("Matrix notifications not configured — running without");
    }

    // Nostr challenge signing key: from env or deterministic dev fallback
    let nostr_challenge_key: [u8; 32] = match std::env::var("NOSTR_CHALLENGE_SECRET") {
        Ok(secret) if !secret.is_empty() => *blake3::hash(secret.as_bytes()).as_bytes(),
        _ => {
            if is_dev {
                tracing::warn!(
                    "Using deterministic dev key for Nostr challenges — NOT for production"
                );
            }
            *blake3::hash(b"scuffed-crew-dev-nostr-challenge-key").as_bytes()
        }
    };

    // Single shared CryptoService from Database (no second from_env() load).
    let crypto = db.crypto.clone();
    if crypto.is_none() {
        tracing::info!("ENCRYPTION_KEY not set — Nostr key encryption disabled");
    }

    let relay_url = std::env::var("NOSTR_RELAY_URL").ok();
    if let Some(ref url) = relay_url {
        tracing::info!("Nostr relay configured: {url}");
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
        notifier,
        nostr_challenge_key,
        crypto,
        relay_url,
        dm_events,
    };

    // Create the collaboration room manager
    let rooms = Arc::new(collab::RoomManager::new());
    let ws_state = routes::ws::WsState {
        app: state.clone(),
        rooms,
    };

    // Spawn hourly session cleanup task
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

    // Build the unified router: existing org routes + strategy routes + chat + WebSocket,
    // then apply production middleware to the combined router.
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
            "/api/chat/decrypt",
            axum::routing::post(routes::chat::decrypt_message).with_state(state),
        )
        .route(
            "/api/strategy/ws",
            get(routes::ws::websocket_handler).with_state(ws_state),
        )
        .layer(DefaultBodyLimit::max(10 * 1024 * 1024))
        .layer(CompressionLayer::new())
        .layer(axum::middleware::from_fn(security_headers));

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
