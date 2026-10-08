//! QA baseline regressions. Each test is ignored so CI stays green.
//!
//! Run them with:
//! `cargo test -p scuffed-site-server --test qa_baseline -- --ignored --test-threads=1`

use std::path::PathBuf;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

use scuffed_auth::crypto::hash_session_token;
use scuffed_db::Database;
use scuffed_site_server::create_router_with_dist;
use scuffed_site_server::state::AppState;

const OFFICER_TOKEN: &str = "qa-officer-token";
const MEMBER_TOKEN: &str = "qa-member-token";
const RECRUIT_TOKEN: &str = "qa-recruit-token";

async fn test_state() -> AppState {
    // Same constructor unit tests use (`src/test_support.rs`). PR #164 adds
    // `leaderboard_cache` there, so this file does not repeat the field list.
    scuffed_site_server::test_support::test_state().await
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

/// The member list hides inactive rows from recruits. Fetch-by-id does not.
#[tokio::test]
#[ignore = "known bug: GET /api/members/:id returns a deactivated member to any org member"]
async fn recruit_cannot_read_deactivated_member_by_id() {
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
    seed_user(
        &state.db,
        "recruituser",
        "recruitmember",
        "Recruit",
        "recruit",
        RECRUIT_TOKEN,
    )
    .await;
    state
        .db
        .client
        .query("UPDATE member:membermember SET is_active = false")
        .await
        .expect("deactivate");

    let app = create_router_with_dist(state, std::env::temp_dir());
    let (status, body, raw) = send(
        app,
        authed(
            Method::GET,
            "/api/members/membermember",
            RECRUIT_TOKEN,
            None,
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a recruit must not load a deactivated profile: {status} {raw}"
    );
    let _ = body;
}

/// Overview and sitemap drop `is_active = false` teams. The public id route does not.
#[tokio::test]
#[ignore = "known bug: GET /api/public/teams/:id still returns a deactivated team"]
async fn inactive_team_is_absent_from_public_detail() {
    let state = test_state().await;
    seed_game_and_team(&state.db).await;
    state
        .db
        .client
        .query("UPDATE team:alpha SET is_active = false")
        .await
        .expect("deactivate team");

    let app = create_router_with_dist(state, std::env::temp_dir());
    let (status, overview, raw) =
        send(app.clone(), anon(Method::GET, "/api/public/overview")).await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    let names = strings_at(&overview["teams"], "name");
    assert!(
        !names.iter().any(|n| n == "Alpha"),
        "overview already hides inactive teams: {overview}"
    );

    let (status, _, raw) = send(app, anon(Method::GET, "/api/public/teams/alpha")).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "public team page must 404 an inactive team: {raw}"
    );
}

/// Public match lists drop scrims and `is_public = false`. `record` counts every completed row.
#[tokio::test]
#[ignore = "known bug: public team record counts private and scrim results"]
async fn public_record_ignores_private_and_scrim_results() {
    let state = test_state().await;
    seed_user(
        &state.db,
        "officeruser",
        "officermember",
        "QaOfficer",
        "officer",
        OFFICER_TOKEN,
    )
    .await;
    seed_game_and_team(&state.db).await;
    let app = create_router_with_dist(state, std::env::temp_dir());

    let (status, _, raw) = send(
        app.clone(),
        authed(
            Method::POST,
            "/api/matches",
            OFFICER_TOKEN,
            Some(json!({
                "team_id": "alpha",
                "opponent": "Public",
                "score_us": 2,
                "score_them": 0,
                "match_type": "official",
                "is_public": true
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{raw}");
    let (status, _, raw) = send(
        app.clone(),
        authed(
            Method::POST,
            "/api/matches",
            OFFICER_TOKEN,
            Some(json!({
                "team_id": "alpha",
                "opponent": "Private scrim",
                "score_us": 0,
                "score_them": 1,
                "match_type": "scrim",
                "is_public": false
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{raw}");

    let (status, detail, raw) = send(app, anon(Method::GET, "/api/public/teams/alpha")).await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    assert_eq!(detail["record"]["wins"].as_u64(), Some(1), "{detail}");
    assert_eq!(
        detail["record"]["losses"].as_u64(),
        Some(0),
        "private scrim loss must not appear in the public record: {detail}"
    );
}

/// Deactivating a board removes it from the tree. Thread reads only check `min_role`.
#[tokio::test]
#[ignore = "known bug: deactivating a forum board does not hide its threads"]
async fn deactivated_forum_board_hides_its_threads() {
    let state = test_state().await;
    let board_id = create_board(&state, "public-board").await;
    let app = create_router_with_dist(state.clone(), std::env::temp_dir());
    let (status, created, raw) = send(
        app.clone(),
        authed(
            Method::POST,
            "/api/forum/threads",
            MEMBER_TOKEN,
            Some(json!({
                "title": "Still visible",
                "content": "secret after hide",
                "board_id": board_id
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{raw}");
    let thread_id = created["id"].as_str().expect("thread id");
    state
        .db
        .client
        .query("UPDATE forum_board SET is_active = false WHERE slug = 'public-board'")
        .await
        .expect("deactivate board");

    let (status, _, raw) = send(
        app.clone(),
        anon(Method::GET, &format!("/api/forum/threads/{thread_id}")),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a deactivated board's thread must 404: {raw}"
    );

    let (status, body, raw) = send(app, anon(Method::GET, "/api/forum/threads")).await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    let titles = strings_at(&body["threads"], "title");
    assert!(
        !titles.iter().any(|t| t == "Still visible"),
        "unfiltered list still returned a thread from a deactivated board: {body}"
    );
}

/// Register and setup reject usernames outside 1–32 `[A-Za-z0-9_-]`. Login does not.
#[tokio::test]
#[ignore = "known bug: POST /api/auth/local/login accepts usernames that register rejects"]
async fn login_rejects_usernames_register_would_reject() {
    let (app, _dist) = router().await;
    let username = "a".repeat(33);
    let (status, body, raw) = send(
        app,
        with_peer(
            Request::builder()
                .method(Method::POST)
                .uri("/api/auth/local/login")
                .header(header::CONTENT_TYPE, "application/json"),
        )
        .body(Body::from(
            serde_json::to_vec(&json!({"username": username, "password": "whatever"})).unwrap(),
        ))
        .unwrap(),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "oversized username must be rejected before lookup: {status} {raw}"
    );
    let msg = body["error"].as_str().unwrap_or("");
    assert!(
        msg.contains("32"),
        "expected the register length error, got {body}"
    );
}

/// Replacing an avatar deletes whatever path is stored in `avatar_url` under `/uploads/`.
#[tokio::test]
#[ignore = "known bug: avatar replace deletes any /uploads path stored on the member, including another member's file"]
async fn avatar_replace_does_not_delete_another_members_file() {
    let mut state = test_state().await;
    let dir = std::env::temp_dir().join(format!("scuffed-qa-upl-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(dir.join("images/victim")).expect("victim dir");
    let victim = dir.join("images/victim/secret.png");
    std::fs::write(&victim, b"keep-me").expect("victim file");
    state.upload_dir = dir.clone();
    seed_user(
        &state.db,
        "memberuser",
        "membermember",
        "ListedMember",
        "member",
        MEMBER_TOKEN,
    )
    .await;

    let app = create_router_with_dist(state, std::env::temp_dir());
    let (status, _, raw) = send(
        app.clone(),
        authed(
            Method::PUT,
            "/api/members/membermember",
            MEMBER_TOKEN,
            Some(json!({"avatar_url": "/uploads/images/victim/secret.png"})),
        ),
    )
    .await;
    assert!(
        status == StatusCode::OK || status == StatusCode::BAD_REQUEST,
        "avatar_url write should succeed or be rejected up front: {status} {raw}"
    );
    if status == StatusCode::BAD_REQUEST {
        assert!(
            victim.exists(),
            "a rejected avatar_url must leave the other member's file in place"
        );
        std::fs::remove_dir_all(&dir).ok();
        return;
    }

    let resp = app
        .oneshot(avatar_upload(
            "/api/upload/avatar",
            MEMBER_TOKEN,
            &tiny_png(),
        ))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "avatar upload should succeed"
    );
    assert!(
        victim.exists(),
        "replacing an avatar deleted another member's upload at {}",
        victim.display()
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// Accepting an application from a banned member sets `is_active` true again.
/// Lift is supposed to leave them inactive. The public profile keys off `is_active`.
#[tokio::test]
#[ignore = "known bug: accepting an application reactivates a banned member"]
async fn accepting_application_does_not_reactivate_a_ban() {
    let state = test_state().await;
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
        "ListedMember",
        "member",
        MEMBER_TOKEN,
    )
    .await;
    let app = create_router_with_dist(state.clone(), std::env::temp_dir());
    let (status, _, raw) = send(
        app.clone(),
        authed(
            Method::POST,
            "/api/moderation",
            OFFICER_TOKEN,
            Some(json!({
                "member_id": "membermember",
                "action_type": "ban",
                "reason": "qa ban"
            })),
        ),
    )
    .await;
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "ban failed: {status} {raw}"
    );
    state
        .db
        .create_session("memberuser", MEMBER_TOKEN, 24)
        .await
        .expect("session after ban revoke");

    let (status, application, raw) = send(
        app.clone(),
        authed(
            Method::POST,
            "/api/applications",
            MEMBER_TOKEN,
            Some(json!({
                "preferred_games": ["Overwatch"],
                "preferred_roles": ["flex"]
            })),
        ),
    )
    .await;
    if status == StatusCode::FORBIDDEN {
        // Submit itself blocked the banned member, so accept cannot reactivate them.
        return;
    }
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "submit should be stored or blocked: {status} {raw}"
    );
    let app_id = application["id"].as_str().expect("application id");
    let (status, _, raw) = send(
        app.clone(),
        authed(
            Method::PATCH,
            &format!("/api/applications/{app_id}"),
            OFFICER_TOKEN,
            Some(json!({"status": "accepted"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");

    let (status, _, raw) = send(app, anon(Method::GET, "/api/public/members/membermember")).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a ban must keep the public profile hidden after the application is accepted: {raw}"
    );
}

/// `time` is stored as text. A clock that overflows `hour * 60`, or a normal
/// clock plus `duration_minutes` that overflows the same `u32` sum, is accepted.
/// This test does not call the ICS handler (that path panics in debug).
#[tokio::test]
#[ignore = "known bug: event time is not validated as a clock time, and duration_minutes can overflow the public ICS u32 sum"]
async fn event_time_must_be_a_clock_time() {
    let state = test_state().await;
    seed_user(
        &state.db,
        "officeruser",
        "officermember",
        "QaOfficer",
        "officer",
        OFFICER_TOKEN,
    )
    .await;
    let app = create_router_with_dist(state, std::env::temp_dir());
    let (clock_status, _, clock_raw) = send(
        app.clone(),
        authed(
            Method::POST,
            "/api/events",
            OFFICER_TOKEN,
            Some(json!({
                "title": "Clock overflow",
                "day_of_week": 1,
                "time": "71582789:00",
                "is_public": true
            })),
        ),
    )
    .await;
    let (duration_status, _, duration_raw) = send(
        app,
        authed(
            Method::POST,
            "/api/events",
            OFFICER_TOKEN,
            Some(json!({
                "title": "Duration overflow",
                "day_of_week": 1,
                "time": "20:00",
                "duration_minutes": 4294967295u64,
                "is_public": true
            })),
        ),
    )
    .await;
    assert!(
        clock_status == StatusCode::BAD_REQUEST && duration_status == StatusCode::BAD_REQUEST,
        "both the clock overflow and the duration_minutes overflow must be rejected: clock={clock_status} {clock_raw}; duration={duration_status} {duration_raw}"
    );
}

fn tiny_png() -> Vec<u8> {
    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc: u32 = 0xFFFF_FFFF;
        for &b in bytes {
            crc ^= b as u32;
            for _ in 0..8 {
                let mask = (crc & 1).wrapping_neg();
                crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
            }
        }
        !crc
    }
    let mut out = vec![0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(b"IHDR");
    ihdr.extend_from_slice(&1u32.to_be_bytes());
    ihdr.extend_from_slice(&1u32.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    out.extend_from_slice(&13u32.to_be_bytes());
    out.extend_from_slice(&ihdr);
    out.extend_from_slice(&crc32(&ihdr).to_be_bytes());
    let mut idat = Vec::new();
    idat.extend_from_slice(b"IDAT");
    idat.extend_from_slice(&[0x78, 0x9c, 0x63, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01]);
    out.extend_from_slice(&((idat.len() - 4) as u32).to_be_bytes());
    out.extend_from_slice(&idat);
    out.extend_from_slice(&crc32(&idat).to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(b"IEND");
    out.extend_from_slice(&crc32(b"IEND").to_be_bytes());
    out
}

fn avatar_upload(uri: &str, token: &str, data: &[u8]) -> Request<Body> {
    let boundary = "qabaselineboundary";
    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        b"Content-Disposition: form-data; name=\"file\"; filename=\"a.png\"\r\n",
    );
    body.extend_from_slice(b"Content-Type: image/png\r\n\r\n");
    body.extend_from_slice(data);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    with_peer(
        Request::builder()
            .method(Method::POST)
            .uri(uri)
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header(
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            ),
    )
    .body(Body::from(body))
    .unwrap()
}
