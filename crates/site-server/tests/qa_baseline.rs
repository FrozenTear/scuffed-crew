//! QA baseline regressions. Each test is ignored so CI stays green.
//!
//! Run them with:
//! `cargo test -p scuffed-site-server --test qa_baseline -- --ignored --test-threads=1`

use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

use scuffed_auth::SessionConfig;
use scuffed_auth::crypto::hash_session_token;
use scuffed_db::Database;
use scuffed_db::migrations::run_migrations;
use scuffed_site_server::create_router_with_dist;
use scuffed_site_server::state::{AppState, OAuthConfig};

const OFFICER_TOKEN: &str = "qa-officer-token";
const MEMBER_TOKEN: &str = "qa-member-token";

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
        notifier: None,
        nostr_challenge_key: *blake3::hash(b"qa-nostr-challenge-key").as_bytes(),
        consumed_challenges: scuffed_site_server::challenge_store::ConsumedChallengeStore::new(),
        nostr_rate_limiter: scuffed_site_server::nostr_rate_limit::NostrRateLimiter::new(),
        login_lockout: scuffed_site_server::login_lockout::LoginLockout::new(),
        crypto: None,
        relay_url: None,
        dm_events: None,
        nip05_domain: None,
        nip05_republish_enabled: false,
        public_settings: scuffed_site_server::state::PublicSettingsCache::new(),
    }
}

async fn router() -> (axum::Router, PathBuf) {
    let state = test_state().await;
    let dist = std::env::temp_dir().join(format!("scuffed-qa-dist-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dist).expect("dist dir");
    std::fs::write(
        dist.join("index.html"),
        "<!DOCTYPE html><html><body>SPA-SHELL-MARKER</body></html>",
    )
    .expect("index.html");
    let app = create_router_with_dist(state, &dist);
    (app, dist)
}

async fn seed_user(
    db: &Database,
    user_key: &str,
    member_key: &str,
    username: &str,
    role: &str,
    token: &str,
) {
    let token_hash = hash_session_token(token);
    let pid = format!("{user_key}-provider-id");
    let pid_hash = hash_session_token(&pid);
    db.client
        .query(format!(
            r#"CREATE user:{user_key} SET
                provider = 'discord',
                username = '{username}',
                avatar_url = NONE,
                provider_id = '{pid}',
                provider_id_hash = '{pid_hash}',
                provider_id_encrypted = NONE,
                created_at = time::now()"#
        ))
        .await
        .unwrap_or_else(|e| panic!("seed user {user_key}: {e}"));
    db.client
        .query(format!(
            r#"CREATE member:{member_key} SET
                user_id = '{user_key}',
                org_role = '{role}',
                display_name = '{username}',
                bio = 'secret bio',
                avatar_url = NONE,
                timezone = NONE,
                pronouns = NONE,
                availability_status = NONE,
                joined_at = time::now(),
                is_active = true"#
        ))
        .await
        .unwrap_or_else(|e| panic!("seed member {member_key}: {e}"));
    db.client
        .query(format!(
            r#"CREATE session:sess_{member_key} SET
                user_id = '{user_key}',
                token = $tok,
                expires_at = time::now() + 365d,
                created_at = time::now()"#
        ))
        .bind(("tok", token_hash))
        .await
        .unwrap_or_else(|e| panic!("seed session {member_key}: {e}"));
}

async fn seed_game_and_team(db: &Database) {
    db.client
        .query(
            r#"CREATE game:ow SET
                name = 'Overwatch',
                abbreviation = NONE,
                is_active = true,
                created_at = time::now()"#,
        )
        .await
        .expect("seed game");
    db.client
        .query(
            r#"CREATE team:alpha SET
                name = 'Alpha',
                game_id = 'ow',
                color = NONE,
                division = NONE,
                lore_quote = NONE,
                logo_url = NONE,
                is_active = true,
                created_at = time::now()"#,
        )
        .await
        .expect("seed team");
}

fn with_peer(builder: axum::http::request::Builder) -> axum::http::request::Builder {
    // Public routes sit behind GovernorLayer. oneshot does not inject the
    // peer socket that production gets from `into_make_service_with_connect_info`.
    builder
        .header("x-forwarded-for", "127.0.0.1")
        .extension(axum::extract::ConnectInfo(std::net::SocketAddr::from((
            [127, 0, 0, 1],
            40000,
        ))))
}

fn authed(method: Method, uri: &str, token: &str, body: Option<Value>) -> Request<Body> {
    let payload = body
        .map(|v| Body::from(serde_json::to_vec(&v).unwrap()))
        .unwrap_or_else(Body::empty);
    with_peer(
        Request::builder()
            .method(method)
            .uri(uri)
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header(header::CONTENT_TYPE, "application/json"),
    )
    .body(payload)
    .unwrap()
}

fn strings_at(value: &Value, field: &str) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| row.get(field).and_then(|v| v.as_str()).map(str::to_string))
        .collect()
}

fn anon(method: Method, uri: &str) -> Request<Body> {
    with_peer(Request::builder().method(method).uri(uri))
        .body(Body::empty())
        .unwrap()
}

async fn send(app: axum::Router, req: Request<Body>) -> (StatusCode, Value, String) {
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, json, text)
}

/// Deactivated members 404 on the public profile, but the public roster and
/// team page still list them (roster queries filter the `plays_on` edge, not
/// `member.is_active`). Ban sets `is_active = false` and does not drop the edge.
#[tokio::test]
#[ignore = "known bug: deactivated/banned members stay on public team rosters and inflate roster_count"]
async fn deactivated_member_is_absent_from_public_rosters() {
    let state = test_state().await;
    seed_user(
        &state.db,
        "memberuser",
        "membermember",
        "ListedMember",
        "member",
        MEMBER_TOKEN,
    )
    .await;
    seed_game_and_team(&state.db).await;
    state
        .db
        .add_to_roster("membermember", "alpha", scuffed_db::TeamRole::Player)
        .await
        .expect("roster add");
    state
        .db
        .client
        .query("UPDATE member:membermember SET is_active = false")
        .await
        .expect("deactivate");

    let app = create_router_with_dist(state, std::env::temp_dir());
    let (status, _, raw) = send(
        app.clone(),
        anon(Method::GET, "/api/public/members/membermember"),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "public profile already hides inactive members: {raw}"
    );

    let (status, roster, raw) =
        send(app.clone(), anon(Method::GET, "/api/teams/alpha/roster")).await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    let ids = strings_at(&roster, "member_id");
    assert!(
        !ids.iter().any(|id| id == "membermember"),
        "GET /api/teams/{{id}}/roster listed a deactivated member: {ids:?}"
    );

    let (status, detail, raw) =
        send(app.clone(), anon(Method::GET, "/api/public/teams/alpha")).await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    let public_ids = strings_at(&detail["roster"], "member_id");
    assert!(
        !public_ids.iter().any(|id| id == "membermember"),
        "GET /api/public/teams/{{id}} listed a deactivated member: {public_ids:?}"
    );

    let (status, overview, raw) = send(app, anon(Method::GET, "/api/public/overview")).await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    let count = overview["teams"]
        .as_array()
        .and_then(|teams| teams.first())
        .and_then(|team| team.get("roster_count"))
        .and_then(|v| v.as_u64());
    assert_eq!(
        count,
        Some(0),
        "overview roster_count must ignore deactivated members, got {overview}"
    );
}

/// Unregistered `/api/*` GETs are not API misses. They fall through to the SPA
/// shell (`classify_spa_route` treats any non-static multi-segment path as a
/// client route) and return 200 HTML.
#[tokio::test]
#[ignore = "known bug: unknown GET /api/* returns 200 text/html (the SPA shell) instead of 404"]
async fn unknown_api_get_is_json_404() {
    let (app, _dist) = router().await;
    let resp = app
        .oneshot(anon(Method::GET, "/api/qa-baseline-no-such-route"))
        .await
        .unwrap();
    let status = resp.status();
    let content_type = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let body = String::from_utf8_lossy(&bytes);
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "content-type={content_type} body={body}"
    );
    assert!(
        !body.contains("SPA-SHELL-MARKER"),
        "unknown API path must not be the HTML shell"
    );
    assert!(
        content_type.starts_with("application/json") || content_type.starts_with("text/plain"),
        "unexpected content-type {content_type}"
    );
}

async fn create_board(state: &AppState, slug: &str) -> String {
    seed_user(
        &state.db,
        "officeruser",
        "officermember",
        "QaOfficer",
        "officer",
        OFFICER_TOKEN,
    )
    .await;
    seed_user(
        &state.db,
        "memberuser",
        "membermember",
        "QaMember",
        "member",
        MEMBER_TOKEN,
    )
    .await;
    let app = create_router_with_dist(state.clone(), std::env::temp_dir());
    let (status, cat, raw) = send(
        app.clone(),
        authed(
            Method::POST,
            "/api/forum/categories",
            OFFICER_TOKEN,
            Some(json!({"name": "General", "slug": "general"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{raw}");
    let category_id = cat["id"].as_str().expect("category id").to_string();
    let (status, board, raw) = send(
        app,
        authed(
            Method::POST,
            "/api/forum/boards",
            OFFICER_TOKEN,
            Some(json!({
                "category_id": category_id,
                "name": slug,
                "slug": slug
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{raw}");
    board["id"].as_str().expect("board id").to_string()
}

/// `total` is `items.len()` after the SQL page is filtered, not the number of
/// matching threads. `limit=1` with two threads reports `total: 1`.
#[tokio::test]
#[ignore = "known bug: GET /api/forum/threads total is the current page length, not the match count"]
async fn forum_thread_total_counts_every_match() {
    let state = test_state().await;
    let board_id = create_board(&state, "public-board").await;
    let app = create_router_with_dist(state, std::env::temp_dir());
    for title in ["First", "Second"] {
        let (status, _, raw) = send(
            app.clone(),
            authed(
                Method::POST,
                "/api/forum/threads",
                MEMBER_TOKEN,
                Some(json!({
                    "title": title,
                    "content": "hello",
                    "board_id": board_id
                })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{raw}");
    }
    let (status, body, raw) = send(
        app,
        anon(Method::GET, "/api/forum/threads?board=public-board&limit=1"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    assert_eq!(body["threads"].as_array().map(|a| a.len()), Some(1));
    assert_eq!(
        body["total"].as_u64(),
        Some(2),
        "total must be the match count, not the page length: {body}"
    );
}

/// Role filtering happens after `LIMIT`. A newer restricted thread occupies
/// the only slot, the anonymous caller drops it, and the older public thread
/// disappears from the first page (`total` becomes 0).
#[tokio::test]
#[ignore = "known bug: forum list applies min_role after LIMIT, so a restricted row can hide older public threads"]
async fn forum_list_does_not_hide_public_threads_behind_restricted_ones() {
    let state = test_state().await;
    let public_id = create_board(&state, "public-board").await;
    let restricted_id = {
        let app = create_router_with_dist(state.clone(), std::env::temp_dir());
        let (status, cat, raw) = send(
            app.clone(),
            authed(
                Method::POST,
                "/api/forum/categories",
                OFFICER_TOKEN,
                Some(json!({"name": "Staff", "slug": "staff"})),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{raw}");
        let (status, board, raw) = send(
            app,
            authed(
                Method::POST,
                "/api/forum/boards",
                OFFICER_TOKEN,
                Some(json!({
                    "category_id": cat["id"].as_str().unwrap(),
                    "name": "Officers",
                    "slug": "officers-only"
                })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{raw}");
        board["id"].as_str().unwrap().to_string()
    };
    state
        .db
        .client
        .query("UPDATE forum_board SET min_role = 'officer' WHERE slug = 'officers-only'")
        .await
        .expect("set min_role");

    let app = create_router_with_dist(state.clone(), std::env::temp_dir());
    let (status, _, raw) = send(
        app.clone(),
        authed(
            Method::POST,
            "/api/forum/threads",
            MEMBER_TOKEN,
            Some(json!({
                "title": "Public hello",
                "content": "visible",
                "board_id": public_id
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{raw}");
    let (status, _, raw) = send(
        app.clone(),
        authed(
            Method::POST,
            "/api/forum/threads",
            OFFICER_TOKEN,
            Some(json!({
                "title": "Officer only",
                "content": "hidden",
                "board_id": restricted_id
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{raw}");
    state
        .db
        .client
        .query("UPDATE forum_thread SET updated_at = time::now() WHERE title = 'Officer only'")
        .await
        .expect("bump restricted thread timestamp");

    let (status, body, raw) = send(app, anon(Method::GET, "/api/forum/threads?limit=1")).await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    let titles = strings_at(&body["threads"], "title");
    assert!(
        titles.iter().any(|t| t == "Public hello"),
        "limit=1 must still surface a public thread when the newest row is restricted, got {body}"
    );
    assert!(
        !titles.iter().any(|t| t == "Officer only"),
        "anonymous list leaked a restricted thread: {body}"
    );
}

/// Missing forum threads, wiki topics, and article slugs are 404s, but the
/// body says `Internal error`. A real database failure uses that same string,
/// so clients cannot tell "not found" from "the server broke".
#[tokio::test]
#[ignore = "known bug: missing forum thread, wiki page, and article return 404 with error \"Internal error\""]
async fn missing_public_content_is_not_found_not_internal_error() {
    let (app, _dist) = router().await;
    for uri in [
        "/api/forum/threads/does-not-exist",
        "/api/wiki/does-not-exist",
        "/api/articles/does-not-exist",
    ] {
        let (status, body, raw) = send(app.clone(), anon(Method::GET, uri)).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}: {raw}");
        let msg = body["error"].as_str().unwrap_or("");
        assert!(
            !msg.eq_ignore_ascii_case("internal error"),
            "{uri} reported a missing row as an internal error: {body}"
        );
    }
}
