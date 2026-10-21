//! API-9 / SEC-5: anonymous reads of the main API sit behind the shared
//! per-IP public governor, and the expensive ones are cached.
//!
//! Before the fix these GETs were registered on the main router with no
//! limiter, and `/api/nostr/feed` and `/api/nostr/health` opened a relay
//! connection on every anonymous request.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Method, Request, StatusCode, header};
use futures_util::{SinkExt, StreamExt};
use http_body_util::BodyExt;
use tower::ServiceExt;

use scuffed_auth::SessionConfig;
use scuffed_db::Database;
use scuffed_db::migrations::run_migrations;
use scuffed_site_server::create_router;
use scuffed_site_server::state::{AppState, OAuthConfig};

/// Public governor budget from `create_router_with_dist`.
const PUBLIC_BURST: usize = 100;

async fn test_state(relay_url: Option<String>) -> AppState {
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
        auth_redirect_base: None,
        upload_dir: PathBuf::from("/tmp/scuffed-test-uploads"),
        reports_dir: PathBuf::from("/tmp/scuffed-test-reports"),
        reports_enabled: true,
        packs_dir: None,
        notifier: None,
        nostr_challenge_key: [7u8; 32],
        consumed_challenges: scuffed_site_server::challenge_store::ConsumedChallengeStore::new(),
        nostr_rate_limiter: scuffed_site_server::nostr_rate_limit::NostrRateLimiter::new(),
        login_lockout: scuffed_site_server::login_lockout::LoginLockout::new(),
        link_code_attempts: scuffed_site_server::link_attempts::LinkCodeAttempts::new(),
        link_poll: scuffed_site_server::link_poll::LinkPollGate::system(),
        crypto: None,
        relay_url,
        dm_events: None,
        nip05_domain: None,
        nip05_republish_enabled: false,
        public_settings: scuffed_site_server::state::PublicSettingsCache::new(),
        leaderboard_cache: scuffed_site_server::leaderboard_cache::LeaderboardCache::from_env(),
    }
}

async fn call(
    app: &axum::Router,
    method: Method,
    uri: &str,
    peer: [u8; 4],
) -> (StatusCode, axum::http::HeaderMap, String) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .extension(ConnectInfo(SocketAddr::from((peer, 45000))))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        headers,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

/// Every anonymous GET that used to have no limiter. Ids need not exist:
/// the governor answers before the handler runs.
const ANONYMOUS_READS: &[&str] = &[
    "/api/games",
    "/api/games/g1",
    "/api/teams",
    "/api/teams/t1",
    "/api/teams/t1/roster",
    "/api/teams/t1/matches",
    "/api/events",
    "/api/events/e1/rsvps",
    "/api/events/e1/rsvp-summary",
    "/api/announcements",
    "/api/articles",
    "/api/articles/some-slug",
    "/api/tournaments",
    "/api/tournaments/x1",
    "/api/tournaments/x1/bracket",
    "/api/tournaments/x1/participants",
    "/api/tournaments/x1/matches",
    "/api/tournaments/x1/standings",
    "/api/wiki",
    "/api/wiki/topic",
    "/api/wiki/topic/revisions",
    "/api/forum/tree",
    "/api/forum/boards/general",
    "/api/forum/threads",
    "/api/forum/threads/th1",
    "/api/settings",
    "/api/nostr/feed",
    "/api/nostr/health",
    "/sitemap.xml",
];

/// Spend `peer`'s public budget: GET `/api/teams` until the first 429.
/// Returns how many requests succeeded first.
///
/// The bucket refills one token per 100 ms while the loop runs, so the count
/// can exceed the burst slightly; it can never be below it.
async fn drain(app: &axum::Router, peer: [u8; 4]) -> usize {
    for ok in 0..(PUBLIC_BURST * 4) {
        let (status, _, body) = call(app, Method::GET, "/api/teams", peer).await;
        if status == StatusCode::TOO_MANY_REQUESTS {
            return ok;
        }
        assert_eq!(status, StatusCode::OK, "request {ok}: {body}");
    }
    panic!("GET /api/teams was never rate limited");
}

#[tokio::test]
async fn anonymous_reads_share_the_public_per_ip_budget() {
    let app = create_router(test_state(None).await);
    let peer = [203, 0, 113, 77];

    let allowed = drain(&app, peer).await;
    assert!(
        (PUBLIC_BURST..PUBLIC_BURST * 2).contains(&allowed),
        "expected the {PUBLIC_BURST} burst before the first 429, got {allowed}"
    );
    let (status, headers, body) = call(&app, Method::GET, "/api/teams", peer).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert!(headers.get(header::RETRY_AFTER).is_some());
    assert!(
        headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("application/json")),
        "{headers:?}"
    );

    // One bucket for every anonymous read: once `/api/teams` has spent it,
    // each of them is refused. Re-drain first so a token that refilled in
    // the meantime cannot let one through.
    for path in ANONYMOUS_READS {
        drain(&app, peer).await;
        let (status, _, body) = call(&app, Method::GET, path, peer).await;
        assert_eq!(
            status,
            StatusCode::TOO_MANY_REQUESTS,
            "GET {path} is outside the public governor: {body}"
        );
    }

    // Writes on the same paths keep their old behaviour (auth check, no
    // public budget), and a wrong method is still a JSON 405 that does not
    // need a token from the bucket.
    let (status, _, body) = call(&app, Method::POST, "/api/teams", peer).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    let (status, _, body) = call(&app, Method::PUT, "/api/settings", peer).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    let (status, headers, body) = call(&app, Method::DELETE, "/api/teams", peer).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED, "{body}");
    assert_eq!(body, r#"{"error":"Method not allowed"}"#);
    assert_eq!(
        headers.get(header::ALLOW).and_then(|v| v.to_str().ok()),
        Some("GET,HEAD,POST")
    );

    // Another address has its own bucket; liveness and robots stay open.
    let (status, _, _) = call(&app, Method::GET, "/api/teams", [203, 0, 113, 78]).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = call(&app, Method::GET, "/api/health", peer).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = call(&app, Method::GET, "/robots.txt", peer).await;
    assert_eq!(status, StatusCode::OK);
}

/// Accepts TCP connections and counts them. A WebSocket client gets an
/// immediate EOSE for each REQ; a plain HTTP request (the health probe)
/// fails the handshake and is dropped.
async fn fake_relay() -> (String, Arc<AtomicUsize>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let connections = Arc::new(AtomicUsize::new(0));
    let counter = connections.clone();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            counter.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async move {
                let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await else {
                    return;
                };
                while let Some(Ok(msg)) = ws.next().await {
                    let tokio_tungstenite::tungstenite::Message::Text(text) = msg else {
                        continue;
                    };
                    let Ok(serde_json::Value::Array(parts)) =
                        serde_json::from_str::<serde_json::Value>(&text)
                    else {
                        continue;
                    };
                    if parts.first().and_then(|v| v.as_str()) == Some("REQ") {
                        let sub = parts.get(1).cloned().unwrap_or_default();
                        let eose = serde_json::json!(["EOSE", sub]).to_string();
                        if ws
                            .send(tokio_tungstenite::tungstenite::Message::Text(eose.into()))
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                }
            });
        }
    });
    (format!("ws://{addr}"), connections)
}

#[tokio::test]
async fn nostr_feed_reuses_one_relay_query_within_ttl() {
    let (relay_url, connections) = fake_relay().await;
    let app = create_router(test_state(Some(relay_url)).await);

    let (status, _, body) = call(&app, Method::GET, "/api/nostr/feed", [198, 51, 100, 1]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, "[]");
    assert_eq!(connections.load(Ordering::SeqCst), 1);

    // Same query from other callers: served from the cache, no new socket.
    for n in 2..6u8 {
        let (status, _, body) = call(&app, Method::GET, "/api/nostr/feed", [198, 51, 100, n]).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    assert_eq!(connections.load(Ordering::SeqCst), 1);

    // A different query is a different relay result.
    let (status, _, body) = call(
        &app,
        Method::GET,
        "/api/nostr/feed?hashtag=scrims",
        [198, 51, 100, 9],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(connections.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn nostr_health_probes_the_relay_once_within_ttl() {
    let (relay_url, connections) = fake_relay().await;
    let app = create_router(test_state(Some(relay_url.clone())).await);

    for n in 1..5u8 {
        let (status, _, body) =
            call(&app, Method::GET, "/api/nostr/health", [198, 51, 100, n]).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["configured"], true, "{body}");
        assert_eq!(json["relay_url"], relay_url.as_str(), "{body}");
    }
    assert_eq!(
        connections.load(Ordering::SeqCst),
        1,
        "the relay probe must be cached"
    );
}
