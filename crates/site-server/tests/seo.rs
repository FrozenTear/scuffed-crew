//! Robots, sitemap, and static-cache headers.
//!
//! These hit the router (not the handlers alone) so a regression that lets the
//! SPA catch-all swallow `/robots.txt` or `/sitemap.xml` fails the test.

use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use tower::ServiceExt;

use scuffed_auth::SessionConfig;
use scuffed_db::migrations::run_migrations;
use scuffed_db::{Database, OrgRole, TournamentFormat, TournamentStatus};
use scuffed_site_server::create_router_with_dist;
use scuffed_site_server::state::{AppState, OAuthConfig};

const SHELL: &str = "SPA-SHELL-MARKER";
const PUBLIC_BASE: &str = "https://crew.example.test";

struct TempTree {
    root: PathBuf,
}

impl TempTree {
    fn new(label: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("scuffed-seo-{label}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("dist/assets")).unwrap();
        std::fs::create_dir_all(root.join("uploads")).unwrap();
        std::fs::write(
            root.join("dist/index.html"),
            format!("<!DOCTYPE html><html><body>{SHELL}</body></html>"),
        )
        .unwrap();
        std::fs::write(root.join("dist/assets/favicon.svg"), "<svg></svg>").unwrap();
        std::fs::write(root.join("dist/assets/plain.js"), "console.log('plain');").unwrap();
        std::fs::write(root.join("dist/assets/plain.css"), "body{color:red}").unwrap();
        std::fs::write(root.join("dist/assets/app.wasm"), "unhashed-wasm").unwrap();
        std::fs::write(
            root.join("dist/assets/app-dxhabc12345.js"),
            "console.log('hashed');",
        )
        .unwrap();
        std::fs::write(root.join("dist/assets/app-dxhabc12345.wasm"), "hashed-wasm").unwrap();
        std::fs::write(root.join("dist/assets/tailwind-dxhdeadbeef.css"), "body{}").unwrap();
        std::fs::write(root.join("uploads/note.txt"), "upload-bytes").unwrap();
        Self { root }
    }

    fn dist(&self) -> PathBuf {
        self.root.join("dist")
    }

    fn uploads(&self) -> PathBuf {
        self.root.join("uploads")
    }
}

impl Drop for TempTree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

async fn test_state(upload_dir: PathBuf) -> AppState {
    let db = Database::connect_memory()
        .await
        .expect("in-memory DB connect");
    run_migrations(&db.client).await.expect("migrations");
    AppState {
        db: Arc::new(db),
        session_config: SessionConfig::default(),
        oauth_config: OAuthConfig {
            discord_client_id: String::new(),
            discord_client_secret: String::new(),
            google_client_id: String::new(),
            google_client_secret: String::new(),
            redirect_base_url: PUBLIC_BASE.into(),
            allowed_origins: vec![PUBLIC_BASE.into()],
        },
        upload_dir,
        notifier: None,
        nostr_challenge_key: [0u8; 32],
        consumed_challenges: scuffed_site_server::challenge_store::ConsumedChallengeStore::new(),
        nostr_rate_limiter: scuffed_site_server::nostr_rate_limit::NostrRateLimiter::new(),
        login_lockout: scuffed_site_server::login_lockout::LoginLockout::new(),
        crypto: None,
        relay_url: None,
        dm_events: None,
        nip05_domain: None,
        nip05_republish_enabled: false,
    }
}

async fn get(app: axum::Router, uri: &str) -> (StatusCode, axum::http::HeaderMap, String) {
    call(app, Method::GET, uri).await
}

async fn call(
    app: axum::Router,
    method: Method,
    uri: &str,
) -> (StatusCode, axum::http::HeaderMap, String) {
    let response = app
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body = String::from_utf8_lossy(&bytes).into_owned();
    (status, headers, body)
}

fn content_type(headers: &axum::http::HeaderMap) -> &str {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
}

fn cache_control(headers: &axum::http::HeaderMap) -> Option<&str> {
    headers
        .get(header::CACHE_CONTROL)
        .and_then(|v| v.to_str().ok())
}

#[tokio::test]
async fn robots_and_sitemap_are_not_the_spa_shell() {
    let tree = TempTree::new("crawlers");
    let state = test_state(tree.uploads()).await;
    let db = state.db.clone();

    let draft = db
        .create_article(
            "draft-hidden-slug",
            "Hidden draft",
            "not public",
            None,
            None,
            "author-1",
        )
        .await
        .unwrap();
    let published = db
        .create_article(
            "published-visible-slug",
            "Visible note",
            "public body",
            None,
            None,
            "author-1",
        )
        .await
        .unwrap();
    db.publish_article(&published.id).await.unwrap();

    let hidden_cup = db
        .create_tournament(
            "Hidden Cup",
            None,
            TournamentFormat::SingleElim,
            None,
            1,
            None,
            false,
            false,
            None,
            None,
            None,
            None,
            None,
            "officer-1",
        )
        .await
        .unwrap();
    let open_cup = db
        .create_tournament(
            "Open Cup",
            None,
            TournamentFormat::SingleElim,
            None,
            1,
            None,
            false,
            true,
            None,
            None,
            None,
            None,
            None,
            "officer-1",
        )
        .await
        .unwrap();
    db.update_tournament_status(&open_cup.id, TournamentStatus::Registration)
        .await
        .unwrap();

    let active = db
        .create_member("user-active", "Roster Active", OrgRole::Member)
        .await
        .unwrap();
    let inactive = db
        .create_member("user-inactive", "Roster Inactive", OrgRole::Member)
        .await
        .unwrap();
    db.update_member(
        &inactive.id,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Some(false),
        None,
        None,
        None,
    )
    .await
    .unwrap();

    let app = create_router_with_dist(state, tree.dist());

    let (status, headers, body) = get(app.clone(), "/robots.txt").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        content_type(&headers).starts_with("text/plain"),
        "content-type {}",
        content_type(&headers)
    );
    assert!(!body.contains(SHELL));
    assert!(!body.contains("<html"));
    assert!(body.contains("Disallow: /admin\n"));
    assert!(body.contains("Disallow: /api/\n"));
    assert!(body.contains("Disallow: /login\n"));
    assert!(body.contains(&format!("Sitemap: {PUBLIC_BASE}/sitemap.xml\n")));
    assert!(!body.contains("Disallow: /blog"));
    assert!(!body.contains("Disallow: /members"));
    assert!(!body.contains("Disallow: /teams"));
    assert!(!body.contains("scuffedcrew"));

    let (status, headers, body) = get(app, "/sitemap.xml").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        content_type(&headers).starts_with("application/xml"),
        "content-type {}",
        content_type(&headers)
    );
    assert_eq!(
        cache_control(&headers),
        Some("public, max-age=3600"),
        "sitemap cache"
    );
    assert!(!body.contains(SHELL));
    assert!(!body.contains("<html"));
    assert!(body.contains(&format!("{PUBLIC_BASE}/")));
    assert!(body.contains(&format!("{PUBLIC_BASE}/blog</loc>")));
    assert!(body.contains(&format!("{PUBLIC_BASE}/patch-notes</loc>")));
    assert!(body.contains(&format!("{PUBLIC_BASE}/members</loc>")));
    assert!(body.contains(&format!("{PUBLIC_BASE}/blog/published-visible-slug</loc>")));
    assert!(
        body.contains("<lastmod>"),
        "published article should carry lastmod"
    );
    assert!(
        !body.contains("draft-hidden-slug"),
        "draft article leaked into sitemap:\n{body}"
    );
    assert!(
        !body.contains(&format!("/tournaments/{}", hidden_cup.id)),
        "draft tournament leaked"
    );
    assert!(body.contains(&format!("{PUBLIC_BASE}/tournaments/{}", open_cup.id)));
    assert!(body.contains(&format!("{PUBLIC_BASE}/members/{}", active.id)));
    assert!(
        !body.contains(&format!("/members/{}", inactive.id)),
        "inactive member leaked"
    );
    let _ = draft;
}

#[tokio::test]
async fn sitemap_strategy_routes_follow_strategies_enabled() {
    let tree = TempTree::new("strategy-flag");
    let state = test_state(tree.uploads()).await;
    let db = state.db.clone();
    let dist = tree.dist();

    // Default matches the API gate: missing settings are created with the flag on.
    let app = create_router_with_dist(state.clone(), dist.clone());
    let (status, _, body) = get(app, "/sitemap.xml").await;
    assert_eq!(status, StatusCode::OK);
    assert_strategy_locs(&body, true);

    db.get_settings().await.unwrap();
    db.client
        .query("UPDATE site_settings SET strategies_enabled = false")
        .await
        .unwrap();
    assert!(
        !db.get_settings().await.unwrap().strategies_enabled,
        "sitemap must read the same flag the strategy API gate uses"
    );

    let app = create_router_with_dist(state, dist);
    let (status, _, body) = get(app, "/sitemap.xml").await;
    assert_eq!(status, StatusCode::OK);
    assert_strategy_locs(&body, false);
}

fn assert_strategy_locs(body: &str, enabled: bool) {
    for path in ["/strategy", "/strategy/heroes", "/strategy/meta"] {
        let needle = format!("{PUBLIC_BASE}{path}</loc>");
        if enabled {
            assert!(body.contains(&needle), "missing {needle}\n{body}");
        } else {
            assert!(!body.contains(&needle), "unexpected {needle}\n{body}");
        }
    }
    assert!(
        !body.contains("/strategy/patch-notes"),
        "duplicate of /patch-notes must never be listed:\n{body}"
    );
    assert!(body.contains(&format!("{PUBLIC_BASE}/patch-notes</loc>")));
}

#[tokio::test]
async fn static_cache_headers_follow_asset_class() {
    let tree = TempTree::new("cache");
    let state = test_state(tree.uploads()).await;
    let app = create_router_with_dist(state, tree.dist());

    let (status, headers, body) = get(app.clone(), "/index.html").await;
    assert_eq!(status, StatusCode::OK);
    assert!(content_type(&headers).starts_with("text/html"));
    assert!(body.contains(SHELL));
    assert_eq!(cache_control(&headers), Some("no-cache"));

    let (status, headers, body) = get(app.clone(), "/blog/anything").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains(SHELL));
    assert_eq!(cache_control(&headers), Some("no-cache"));

    let (status, headers, body) = get(app.clone(), "/assets/missing-dxhabc12345.js").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains(SHELL),
        "missing hashed file must fall back to the shell"
    );
    assert_eq!(
        cache_control(&headers),
        Some("no-cache"),
        "html fallback must not be immutable"
    );

    let (status, headers, body) = get(app.clone(), "/assets/app-dxhabc12345.js").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("hashed"));
    assert_eq!(
        cache_control(&headers),
        Some("public, max-age=31536000, immutable")
    );

    let (status, headers, _) = get(app.clone(), "/assets/app-dxhabc12345.wasm").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        cache_control(&headers),
        Some("public, max-age=31536000, immutable")
    );

    let (status, headers, _) = get(app.clone(), "/assets/tailwind-dxhdeadbeef.css").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        cache_control(&headers),
        Some("public, max-age=31536000, immutable")
    );

    let (status, headers, _) = get(app.clone(), "/assets/favicon.svg").await;
    assert_eq!(status, StatusCode::OK);
    let favicon_cache = cache_control(&headers).unwrap();
    assert_eq!(favicon_cache, "public, max-age=86400");
    assert!(!favicon_cache.contains("immutable"));

    let (status, headers, _) = get(app.clone(), "/assets/plain.js").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(cache_control(&headers), Some("no-cache"));

    let (status, headers, _) = get(app.clone(), "/assets/plain.css").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(cache_control(&headers), Some("no-cache"));

    let (status, headers, _) = get(app.clone(), "/assets/app.wasm").await;
    assert_eq!(status, StatusCode::OK);
    let wasm_cache = cache_control(&headers).unwrap();
    assert_eq!(wasm_cache, "no-cache");
    assert!(!wasm_cache.contains("immutable"));

    let (status, headers, _) = get(app.clone(), "/api/health").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        cache_control(&headers).is_none(),
        "api cache header must stay untouched, got {:?}",
        cache_control(&headers)
    );

    let (status, headers, body) = get(app, "/uploads/note.txt").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("upload-bytes"));
    assert!(
        cache_control(&headers).is_none(),
        "uploads cache header must stay untouched, got {:?}",
        cache_control(&headers)
    );
}

#[tokio::test]
async fn unmatched_api_paths_are_json_404_and_client_routes_stay_the_shell() {
    let tree = TempTree::new("api-404");
    let state = test_state(tree.uploads()).await;
    // A route merged after `create_router` must still win over the API 404.
    // `scuffed-server` registers strategy and chat this way.
    let app = create_router_with_dist(state, tree.dist()).route(
        "/api/merged-probe",
        axum::routing::get(|| async { "merged-ok" }),
    );

    for method in [
        Method::GET,
        Method::POST,
        Method::PUT,
        Method::PATCH,
        Method::DELETE,
    ] {
        let (status, headers, body) = call(app.clone(), method.clone(), "/api/nope").await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method}");
        assert_json_not_found(&headers, &body, method.as_str());
    }

    let (status, headers, body) = call(app.clone(), Method::HEAD, "/api/nope").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(content_type(&headers), "application/json");
    assert_eq!(cache_control(&headers), Some("no-store"));
    assert!(
        body.is_empty(),
        "HEAD must not include a body, got {body:?}"
    );

    let (status, headers, body) = call(app.clone(), Method::GET, "/api/stats/me/roles").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_json_not_found(&headers, &body, "stats roles");

    let (status, headers, body) = call(app.clone(), Method::GET, "/api").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_json_not_found(&headers, &body, "exact /api");

    let (status, headers, body) = call(app.clone(), Method::POST, "/api/nope?x=1").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_json_not_found(&headers, &body, "query string");

    let (status, _, body) = call(app.clone(), Method::GET, "/api/health").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "known GET must not become the API 404"
    );
    assert!(!body.contains(SHELL));

    let (status, _, body) = call(app.clone(), Method::POST, "/api/health").await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert!(!body.contains(SHELL));
    assert!(!body.contains("\"error\""));

    let (status, _, body) = call(app.clone(), Method::GET, "/api/merged-probe").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "merged-ok");

    for path in [
        "/",
        "/wiki/some-page",
        "/strategy/x",
        "/apiary",
        "/api-docs",
    ] {
        let (status, headers, body) = get(app.clone(), path).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert!(
            content_type(&headers).starts_with("text/html"),
            "{path} content-type {}",
            content_type(&headers)
        );
        assert!(body.contains(SHELL), "{path} must be the SPA shell");
        assert_eq!(cache_control(&headers), Some("no-cache"), "{path}");
    }

    let (status, headers, body) = get(app, "/assets/favicon.svg").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("<svg"));
    assert_eq!(cache_control(&headers), Some("public, max-age=86400"));
}

fn assert_json_not_found(headers: &axum::http::HeaderMap, body: &str, label: &str) {
    assert_eq!(content_type(headers), "application/json", "{label}");
    assert_eq!(cache_control(headers), Some("no-store"), "{label}");
    let value: serde_json::Value = serde_json::from_str(body).unwrap_or_else(|err| {
        panic!("{label}: body is not JSON ({err}): {body}");
    });
    assert_eq!(value, serde_json::json!({"error": "Not found"}), "{label}");
}
