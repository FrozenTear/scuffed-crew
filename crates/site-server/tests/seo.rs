//! Robots, sitemap, and static-cache headers.
//!
//! These hit the router (not the handlers alone) so a regression that lets the
//! SPA catch-all swallow `/robots.txt` or `/sitemap.xml` fails the test.

use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::Value;
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
            format!(
                "<!DOCTYPE html><html><head><meta charset=\"utf-8\"></head><body>{SHELL}</body></html>"
            ),
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
        reports_dir: PathBuf::from("/tmp/scuffed-test-reports"),
        reports_enabled: true,
        notifier: None,
        nostr_challenge_key: [0u8; 32],
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

async fn get(app: axum::Router, uri: &str) -> (StatusCode, axum::http::HeaderMap, String) {
    exchange(app, Method::GET, uri).await
}

async fn exchange_bytes(
    app: axum::Router,
    method: Method,
    uri: &str,
) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
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
    (status, headers, bytes.to_vec())
}

async fn exchange(
    app: axum::Router,
    method: Method,
    uri: &str,
) -> (StatusCode, axum::http::HeaderMap, String) {
    let (status, headers, bytes) = exchange_bytes(app, method, uri).await;
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
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(
        !body.contains(SHELL),
        "missing hashed file must not fall back to the shell"
    );
    assert_eq!(cache_control(&headers), Some("no-store"));
    assert!(!body.contains("<html"));

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
async fn missing_static_files_404_and_client_routes_stay_shell() {
    let tree = TempTree::new("static-404");
    let state = test_state(tree.uploads()).await;
    let app = create_router_with_dist(state, tree.dist());

    for uri in [
        "/assets/tailwind.css",
        "/assets/missing-favicon.svg",
        "/assets/missing-dxhabc12345.js",
        "/assets/nope.json",
        "/assets/hero.gif",
        "/assets/hero.avif",
        "/outside.wasm",
        "/bundle.mjs",
        "/pic.gif",
        "/photo.avif",
        "/favicon.ico",
    ] {
        let (status, headers, body) = get(app.clone(), uri).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
        assert!(!body.contains(SHELL), "{uri} returned the shell");
        assert!(!body.contains("sc-settings"), "{uri}");
        assert!(
            content_type(&headers).starts_with("text/plain"),
            "{uri} content-type {}",
            content_type(&headers)
        );
        let cache = cache_control(&headers).unwrap_or("");
        assert_eq!(cache, "no-store", "{uri} cache {cache}");
    }

    for uri in [
        "/strategies/foo",
        "/admin/settings",
        "/blog/hello",
        "/wiki/foo.bar",
        "/articles/v1.2-notes",
        "/fonts/missing.woff2",
        "/wiki/config.json",
        "/blog/foo.png",
        "/articles/v1.2.png",
    ] {
        let (status, headers, body) = get(app.clone(), uri).await;
        assert_eq!(status, StatusCode::OK, "{uri}");
        assert!(body.contains(SHELL), "{uri}");
        assert!(body.contains("sc-settings"), "{uri}");
        assert_eq!(cache_control(&headers), Some("no-cache"), "{uri}");
    }

    let (status, headers, body) = get(app.clone(), "/assets/plain.js").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("plain"));
    assert_eq!(cache_control(&headers), Some("no-cache"));

    let (status, headers, body) = get(app.clone(), "/assets/app-dxhabc12345.js").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("hashed"));
    assert_eq!(
        cache_control(&headers),
        Some("public, max-age=31536000, immutable")
    );

    let (status, headers, body) = get(app, "/assets/favicon.svg").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("<svg"));
    assert_eq!(cache_control(&headers), Some("public, max-age=86400"));
}

const SETTINGS_OPEN: &str = "<script id=\"sc-settings\" type=\"application/json\">";

/// JSON text of `#sc-settings`, which must sit immediately before `</head>`.
fn settings_json_before_head(html: &str) -> &str {
    let head = html.find("</head>").expect("</head>");
    let start = html.find(SETTINGS_OPEN).expect("sc-settings open tag");
    assert!(start < head, "settings block must be inside <head>");
    let json_at = start + SETTINGS_OPEN.len();
    let close = html[json_at..head]
        .find("</script>")
        .expect("settings script close");
    assert_eq!(
        &html[json_at + close..head],
        "</script>",
        "settings script must be immediately before </head>"
    );
    &html[json_at..json_at + close]
}

async fn write_settings(
    db: &scuffed_db::Database,
    org_name: Option<&str>,
    site_description: Option<&str>,
) {
    db.update_settings(
        org_name,
        site_description,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .expect("update settings");
}

#[tokio::test]
async fn shell_embeds_anonymous_settings_before_head() {
    let tree = TempTree::new("embed");
    let state = test_state(tree.uploads()).await;
    let app = create_router_with_dist(state, tree.dist());

    let (api_status, _, api_body) = get(app.clone(), "/api/settings").await;
    assert_eq!(api_status, StatusCode::OK);
    let api: serde_json::Value = serde_json::from_str(&api_body).expect("api settings");

    for uri in ["/", "/index.html", "/strategies/foo", "/admin/settings"] {
        let (status, headers, body) = get(app.clone(), uri).await;
        assert_eq!(status, StatusCode::OK, "{uri}");
        assert!(
            content_type(&headers).eq_ignore_ascii_case("text/html; charset=utf-8"),
            "{uri} content-type {}",
            content_type(&headers)
        );
        assert_eq!(cache_control(&headers), Some("no-cache"), "{uri}");
        assert!(body.contains(SHELL), "{uri}");
        let json = settings_json_before_head(&body);
        assert_eq!(
            json, api_body,
            "{uri} must match the anonymous settings body"
        );
        let parsed: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(parsed, api, "{uri}");
    }

    let (_, _, robots) = get(app.clone(), "/robots.txt").await;
    assert!(!robots.contains("sc-settings"));
    let (_, _, sitemap) = get(app.clone(), "/sitemap.xml").await;
    assert!(!sitemap.contains("sc-settings"));
    let (status, _, svg) = get(app.clone(), "/assets/favicon.svg").await;
    assert_eq!(status, StatusCode::OK);
    assert!(svg.contains("<svg"));
    assert!(!svg.contains("sc-settings"));
    let (status, headers, _) = get(app, "/api/health").await;
    assert_eq!(status, StatusCode::OK);
    assert!(cache_control(&headers).is_none());
}

#[tokio::test]
async fn settings_embed_escapes_script_breakout() {
    let tree = TempTree::new("embed-escape");
    let state = test_state(tree.uploads()).await;
    let payload = "</script><script>alert(1)</script>\u{2028}&\u{2029}";
    write_settings(&state.db, None, Some(payload)).await;
    let app = create_router_with_dist(state, tree.dist());

    let (api_status, _, api_body) = get(app.clone(), "/api/settings").await;
    assert_eq!(api_status, StatusCode::OK);
    let api: serde_json::Value = serde_json::from_str(&api_body).unwrap();
    assert_eq!(api["site_description"], payload);

    let (status, _, first) = get(app.clone(), "/").await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, second) = get(app, "/strategies/foo").await;
    assert_eq!(status, StatusCode::OK);
    for body in [&first, &second] {
        let json = settings_json_before_head(body);
        assert!(json.contains("\\u003c/script\\u003e"), "{json}");
        assert!(json.contains("\\u003cscript\\u003e"), "{json}");
        assert!(json.contains("\\u2028"), "{json}");
        assert!(json.contains("\\u2029"), "{json}");
        assert!(json.contains("\\u0026"), "{json}");
        assert!(
            !json.contains('\u{2028}'),
            "cached page reintroduced U+2028"
        );
        assert!(!json.contains('\u{2029}'));
        assert!(
            !json.contains("</script>"),
            "raw script close leaked into the JSON: {json}"
        );
        let parsed: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(parsed, api);
        assert_eq!(parsed["site_description"], payload);
    }
    assert_eq!(
        settings_json_before_head(&first),
        settings_json_before_head(&second),
        "a cache hit must keep the escaped JSON"
    );
}

#[tokio::test]
async fn settings_embed_cache_invalidates_after_update() {
    let tree = TempTree::new("embed-cache");
    let state = test_state(tree.uploads()).await;
    let app = create_router_with_dist(state.clone(), tree.dist());

    let (status, _, body) = get(app.clone(), "/").await;
    assert_eq!(status, StatusCode::OK);
    let first: serde_json::Value = serde_json::from_str(settings_json_before_head(&body)).unwrap();
    assert_eq!(first["org_name"], "My Clan");

    write_settings(&state.db, Some("Stale Clan"), None).await;
    let (_, _, body) = get(app.clone(), "/strategies/foo").await;
    let cached: serde_json::Value = serde_json::from_str(settings_json_before_head(&body)).unwrap();
    assert_eq!(
        cached["org_name"], "My Clan",
        "a DB write that skips invalidation must keep serving the cached embed"
    );

    state.public_settings.invalidate();
    let (_, _, body) = get(app.clone(), "/").await;
    let refreshed: serde_json::Value =
        serde_json::from_str(settings_json_before_head(&body)).unwrap();
    assert_eq!(refreshed["org_name"], "Stale Clan");

    let user = state
        .db
        .create_local_user("embed-admin", "unused-hash")
        .await
        .unwrap();
    state
        .db
        .create_member(&user.id, "Embed Admin", OrgRole::Admin)
        .await
        .unwrap();
    state
        .db
        .create_session(&user.id, "embed-admin-token", 24)
        .await
        .unwrap();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/api/settings")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::COOKIE, "sc_session=embed-admin-token")
                .body(Body::from(r#"{"org_name":"Fresh Clan"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "PUT /api/settings");

    let (_, _, body) = get(app.clone(), "/admin/settings").await;
    let after_put: serde_json::Value =
        serde_json::from_str(settings_json_before_head(&body)).unwrap();
    assert_eq!(after_put["org_name"], "Fresh Clan");

    write_settings(&state.db, Some("After Reject"), None).await;
    let (_, _, body) = get(app.clone(), "/").await;
    let still_fresh: serde_json::Value =
        serde_json::from_str(settings_json_before_head(&body)).unwrap();
    assert_eq!(still_fresh["org_name"], "Fresh Clan");

    let rejected = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/api/settings")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::COOKIE, "sc_session=embed-admin-token")
                .body(Body::from(r#"{"page_bg_color":"not-a-color"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);

    let (_, _, body) = get(app, "/").await;
    let after_reject: serde_json::Value =
        serde_json::from_str(settings_json_before_head(&body)).unwrap();
    assert_eq!(
        after_reject["org_name"], "After Reject",
        "a failed PUT must still drop the fresh cache"
    );
}

#[tokio::test]
async fn head_on_shell_matches_get_headers_and_has_no_body() {
    let tree = TempTree::new("embed-head");
    let state = test_state(tree.uploads()).await;
    let payload = "</script><script>alert(1)</script>";
    write_settings(&state.db, None, Some(payload)).await;
    let app = create_router_with_dist(state, tree.dist());

    let (get_status, get_headers, get_bytes) =
        exchange_bytes(app.clone(), Method::GET, "/wiki/foo.bar").await;
    let (head_status, head_headers, head_bytes) =
        exchange_bytes(app, Method::HEAD, "/wiki/foo.bar").await;

    assert_eq!(get_status, StatusCode::OK);
    assert_eq!(head_status, StatusCode::OK);
    assert_eq!(head_bytes.len(), 0);
    assert!(!get_bytes.is_empty());
    let get_body = String::from_utf8_lossy(&get_bytes);
    assert!(get_body.contains("\\u003c/script\\u003e"));
    assert_eq!(content_type(&get_headers), content_type(&head_headers));
    assert_eq!(cache_control(&get_headers), cache_control(&head_headers));
    assert_eq!(cache_control(&head_headers), Some("no-cache"));
    assert_eq!(
        get_headers.get(header::CONTENT_LENGTH),
        head_headers.get(header::CONTENT_LENGTH)
    );
    let len = get_headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok())
        .expect("content-length");
    assert_eq!(len, get_bytes.len());
}

#[tokio::test]
async fn setup_invalidation_shows_new_org_on_next_shell() {
    let tree = TempTree::new("embed-setup");
    let state = test_state(tree.uploads()).await;
    let app = create_router_with_dist(state, tree.dist());

    let (_, _, body) = get(app.clone(), "/").await;
    let first: serde_json::Value = serde_json::from_str(settings_json_before_head(&body)).unwrap();
    assert_eq!(first["org_name"], "My Clan");

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/auth/setup")
                .header(header::CONTENT_TYPE, "application/json")
                .header("x-forwarded-for", "127.0.0.1")
                .extension(axum::extract::ConnectInfo(std::net::SocketAddr::from((
                    [127, 0, 0, 1],
                    40000,
                ))))
                .body(Body::from(
                    r#"{"username":"firstadmin","password":"a-strong-password","org_name":"Boot Clan"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "POST /api/auth/setup");

    let (_, _, body) = get(app, "/").await;
    let after: serde_json::Value = serde_json::from_str(settings_json_before_head(&body)).unwrap();
    assert_eq!(after["org_name"], "Boot Clan");
}

const BRANDED_SHELL: &str = r#"<!DOCTYPE html><html><head>
<title>The Scuffed Crew</title>
<meta name="description" content="The Scuffed Crew — fallback">
<meta property="og:title" content="The Scuffed Crew" />
<meta content="The Scuffed Crew" property="og:site_name">
<meta property="og:description" content="fallback tagline">
<meta name="viewport" content="width=device-width, initial-scale=1">
</head><body>SPA-SHELL-MARKER</body></html>"#;

#[tokio::test]
async fn shell_rewrites_title_and_meta_and_escapes_clan_name() {
    let tree = TempTree::new("embed-title");
    std::fs::write(tree.dist().join("index.html"), BRANDED_SHELL).unwrap();
    let state = test_state(tree.uploads()).await;
    let org_name = r#"</title><script>alert(1)</script>" onload="alert(1)"#;
    let description = r#"<img src=x onerror=alert(1)> & "quotes""#;
    write_settings(&state.db, Some(org_name), Some(description)).await;
    let app = create_router_with_dist(state, tree.dist());

    let (_, _, body) = get(app, "/").await;
    assert!(body.contains("SPA-SHELL-MARKER"));
    assert!(!body.contains("<script>alert"));
    assert!(!body.contains("onload=\"alert"));
    assert!(
        body.contains("<title>&lt;/title&gt;&lt;script&gt;alert(1)&lt;/script&gt;&quot; onload=")
    );
    assert!(body.contains("property=\"og:title\" content=\"&lt;/title&gt;"));
    assert!(
        body.contains(
            "content=\"&lt;/title&gt;&lt;script&gt;alert(1)&lt;/script&gt;&quot; onload="
        )
    );
    assert!(body.contains("property=\"og:site_name\""));
    assert!(body.contains("&lt;img src=x onerror=alert(1)&gt;"));
    assert!(body.contains("&amp;"));
    assert!(!body.contains("The Scuffed Crew"));
    assert!(!body.contains("fallback tagline"));
    assert!(body.contains("width=device-width"));
    let json = settings_json_before_head(&body);
    let parsed: serde_json::Value = serde_json::from_str(json).unwrap();
    assert_eq!(parsed["org_name"], org_name);
    assert_eq!(parsed["site_description"], description);
}

#[tokio::test]
async fn blank_settings_keep_template_title_and_meta_and_still_embed_json() {
    let tree = TempTree::new("embed-blank");
    std::fs::write(tree.dist().join("index.html"), BRANDED_SHELL).unwrap();
    let state = test_state(tree.uploads()).await;
    write_settings(&state.db, Some(""), Some("")).await;
    let app = create_router_with_dist(state, tree.dist());

    let (_, _, body) = get(app, "/").await;
    assert!(body.contains("<title>The Scuffed Crew</title>"), "{body}");
    assert!(
        body.contains("name=\"description\" content=\"The Scuffed Crew — fallback\""),
        "{body}"
    );
    assert!(
        body.contains("property=\"og:title\" content=\"The Scuffed Crew\""),
        "{body}"
    );
    assert!(
        body.contains("content=\"The Scuffed Crew\" property=\"og:site_name\""),
        "{body}"
    );
    assert!(
        body.contains("property=\"og:description\" content=\"fallback tagline\""),
        "{body}"
    );
    let parsed: serde_json::Value = serde_json::from_str(settings_json_before_head(&body)).unwrap();
    assert_eq!(parsed["org_name"], "");
    assert_eq!(parsed["site_description"], "");
}

#[tokio::test]
async fn officer_strategies_update_reaches_the_shell_and_forbidden_org_name_keeps_the_cache() {
    let tree = TempTree::new("embed-officer");
    let state = test_state(tree.uploads()).await;
    let app = create_router_with_dist(state.clone(), tree.dist());

    let (_, _, body) = get(app.clone(), "/").await;
    let first: serde_json::Value = serde_json::from_str(settings_json_before_head(&body)).unwrap();
    assert_eq!(first["org_name"], "My Clan");
    assert_eq!(first["strategies_enabled"], true);

    let user = state
        .db
        .create_local_user("embed-officer", "unused-hash")
        .await
        .unwrap();
    state
        .db
        .create_member(&user.id, "Embed Officer", OrgRole::Officer)
        .await
        .unwrap();
    state
        .db
        .create_session(&user.id, "embed-officer-token", 24)
        .await
        .unwrap();

    let updated = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/api/settings")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::COOKIE, "sc_session=embed-officer-token")
                .body(Body::from(r#"{"strategies_enabled":false}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(updated.status(), StatusCode::OK);

    let (_, _, body) = get(app.clone(), "/").await;
    let after_officer: serde_json::Value =
        serde_json::from_str(settings_json_before_head(&body)).unwrap();
    assert_eq!(after_officer["strategies_enabled"], false);
    assert_eq!(after_officer["org_name"], "My Clan");

    let forbidden = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/api/settings")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::COOKIE, "sc_session=embed-officer-token")
                .body(Body::from(r#"{"org_name":"Nope Clan"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);

    write_settings(&state.db, Some("Direct Clan"), None).await;
    let (_, _, body) = get(app, "/").await;
    let still_cached: serde_json::Value =
        serde_json::from_str(settings_json_before_head(&body)).unwrap();
    assert_eq!(
        still_cached["org_name"], "My Clan",
        "a 403 must not drop the cache; a direct DB write would show up if it had"
    );
    assert_eq!(still_cached["strategies_enabled"], false);
}

#[tokio::test]
async fn template_read_error_retries_on_the_next_shell() {
    let root = std::env::temp_dir().join(format!("scuffed-seo-retry-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(root.join("dist/index.html")).unwrap();
    std::fs::create_dir_all(root.join("uploads")).unwrap();
    let state = test_state(root.join("uploads")).await;
    let app = create_router_with_dist(state, root.join("dist"));

    let broken = app
        .clone()
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_ne!(broken.status(), StatusCode::OK);
    let broken_body = match broken.into_body().collect().await {
        Ok(collected) => String::from_utf8_lossy(&collected.to_bytes()).into_owned(),
        Err(_) => String::new(),
    };
    assert!(!broken_body.contains("sc-settings"));
    assert!(!broken_body.contains(SHELL));

    std::fs::remove_dir_all(root.join("dist/index.html")).unwrap();
    std::fs::write(
        root.join("dist/index.html"),
        "<!DOCTYPE html><html><head><title>The Scuffed Crew</title></head><body>SPA-SHELL-MARKER</body></html>",
    )
    .unwrap();

    let (status, _, body) = get(app, "/wiki/foo.bar").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains(SHELL));
    assert!(body.contains("sc-settings"));
    assert!(body.contains("<title>My Clan</title>"));
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn missing_index_html_does_not_500() {
    let root = std::env::temp_dir().join(format!("scuffed-seo-noindex-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(root.join("dist/assets")).unwrap();
    std::fs::create_dir_all(root.join("uploads")).unwrap();
    std::fs::write(root.join("dist/assets/plain.js"), "console.log(1);").unwrap();
    let state = test_state(root.join("uploads")).await;
    let app = create_router_with_dist(state, root.join("dist"));

    let (status, _, body) = get(app.clone(), "/").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(!body.contains("sc-settings"));
    assert!(!body.contains(SHELL));

    let (status, headers, body) = get(app, "/assets/plain.js").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("console.log"));
    assert_eq!(cache_control(&headers), Some("no-cache"));
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn unmatched_api_paths_are_json_404_and_client_routes_stay_the_shell() {
    let tree = TempTree::new("api-404");
    let state = test_state(tree.uploads()).await;
    // A route registered after `create_router` must still win over the API 404.
    // `scuffed-server` merges strategy (`.merge`) and routes chat and the
    // websocket (`.route`).
    let app = create_router_with_dist(state, tree.dist())
        .route(
            "/api/route-probe",
            axum::routing::get(|| async { "route-ok" }),
        )
        .merge(axum::Router::new().route(
            "/api/merge-probe",
            axum::routing::get(|| async { "merge-ok" }),
        ));

    let mut get_headers = None;
    let mut get_body = None;
    for method in [
        Method::GET,
        Method::POST,
        Method::PUT,
        Method::PATCH,
        Method::DELETE,
    ] {
        let (status, headers, body) = exchange(app.clone(), method.clone(), "/api/nope").await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method}");
        assert_json_not_found(&headers, &body, method.as_str());
        if method == Method::GET {
            get_headers = Some(headers);
            get_body = Some(body);
        }
    }
    let get_headers = get_headers.expect("the loop must include GET");
    let get_body = get_body.expect("the loop must include GET");

    let (status, headers, body) = exchange(app.clone(), Method::HEAD, "/api/nope").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "HEAD");
    assert_eq!(content_type(&headers), "application/json", "HEAD");
    assert_eq!(cache_control(&headers), Some("no-store"), "HEAD");
    assert!(
        body.is_empty(),
        "HEAD must not include a body, got {body:?}"
    );
    assert_eq!(
        get_headers.get(header::CONTENT_LENGTH),
        headers.get(header::CONTENT_LENGTH),
        "GET and HEAD Content-Length must match"
    );
    let len = get_headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok())
        .expect("content-length");
    assert_eq!(
        len,
        get_body.len(),
        "Content-Length must equal the GET body length"
    );

    // The CORS layer answers every OPTIONS request on an unmatched path before the JSON 404.
    let (status, headers, body) = exchange(app.clone(), Method::OPTIONS, "/api/nope").await;
    assert_eq!(status, StatusCode::OK, "OPTIONS");
    assert!(
        body.is_empty(),
        "OPTIONS must not include a body, got {body:?}"
    );
    assert!(
        headers.get(header::ACCESS_CONTROL_ALLOW_METHODS).is_some(),
        "CORS layer must answer OPTIONS"
    );
    assert_ne!(content_type(&headers), "application/json", "OPTIONS");

    // `/api/stats/me/roles` is a registered route (401 without a session).
    // This path is not.
    let (status, headers, body) = exchange(app.clone(), Method::GET, "/api/does-not-exist").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_json_not_found(&headers, &body, "unknown nested path");

    let (status, headers, body) = exchange(app.clone(), Method::GET, "/api").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_json_not_found(&headers, &body, "exact /api");

    let (status, headers, body) = exchange(app.clone(), Method::POST, "/api/nope?x=1").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_json_not_found(&headers, &body, "query string");

    let (status, _, body) = exchange(app.clone(), Method::GET, "/api/stats/me/roles").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(!body.contains(SHELL));

    let (status, _, body) = exchange(app.clone(), Method::GET, "/api/health").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "known GET must not become the API 404"
    );
    assert!(!body.contains(SHELL));

    let (status, _, body) = exchange(app.clone(), Method::POST, "/api/health").await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert!(!body.contains(SHELL));
    assert!(!body.contains("\"error\""));

    let (status, _, body) = exchange(app.clone(), Method::GET, "/api/route-probe").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "route-ok");

    let (status, _, body) = exchange(app.clone(), Method::GET, "/api/merge-probe").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "merge-ok");

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
    let value: Value = serde_json::from_str(body).unwrap_or_else(|err| {
        panic!("{label}: body is not JSON ({err}): {body}");
    });
    assert_eq!(value, serde_json::json!({"error": "Not found"}), "{label}");
}
