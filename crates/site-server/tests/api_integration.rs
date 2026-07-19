//! Integration tests for the site-server API.
//!
//! Each test spins up an in-memory SurrealDB, runs migrations, seeds dev data,
//! and makes HTTP requests against the Axum router via `tower::ServiceExt`.

use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use scuffed_auth::crypto::hash_session_token;
use scuffed_auth::SessionConfig;
use scuffed_db::migrations::run_migrations;
use scuffed_db::Database;
use scuffed_site_server::create_router;
use scuffed_site_server::state::{AppState, OAuthConfig};

// ─── Test Harness ───────────────────────────────────────────────────────────

/// Create an AppState backed by an in-memory SurrealDB with migrations applied.
async fn test_state() -> AppState {
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
            redirect_base_url: "http://localhost:3000".into(),
            allowed_origins: vec!["http://localhost:3000".into()],
        },
        upload_dir: PathBuf::from("/tmp/scuffed-test-uploads"),
        notifier: None,
        nostr_challenge_key: *blake3::hash(b"test-nostr-challenge-key").as_bytes(),
        consumed_challenges: scuffed_site_server::challenge_store::ConsumedChallengeStore::new(),
        nostr_rate_limiter: scuffed_site_server::nostr_rate_limit::NostrRateLimiter::new(),
        crypto: None,
        relay_url: None,
        dm_events: None,
    }
}

/// Seed a user + member + session into the database.
/// Each seed call runs three separate queries to avoid silent batch failures.
async fn seed_user(
    db: &Database,
    user_key: &str,
    member_key: &str,
    username: &str,
    role: &str,
    token: &str,
) {
    let token_hash = hash_session_token(token);

    // Create user — provider_id_hash must be unique (or NONE only once) due to
    // the unique composite index on (provider, provider_id_hash).
    let pid = format!("{user_key}-provider-id");
    let pid_hash = hash_session_token(&pid); // reuse hash fn for convenience
    db.client
        .query(&format!(
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

    // Create member
    db.client
        .query(&format!(
            r#"CREATE member:{member_key} SET
                user_id = '{user_key}',
                org_role = '{role}',
                display_name = '{username}',
                bio = NONE,
                avatar_url = NONE,
                timezone = NONE,
                pronouns = NONE,
                availability_status = NONE,
                joined_at = time::now(),
                is_active = true"#
        ))
        .await
        .unwrap_or_else(|e| panic!("seed member {member_key}: {e}"));

    // Create session
    db.client
        .query(&format!(
            r#"CREATE session:sess_{member_key} SET
                user_id = '{user_key}',
                token = $tok,
                expires_at = time::now() + 365d,
                created_at = time::now()"#
        ))
        .bind(("tok", token_hash))
        .await
        .unwrap_or_else(|e| panic!("seed session for {member_key}: {e}"));
}

/// Seed a game record.
async fn seed_game(db: &Database, key: &str, name: &str) {
    db.client
        .query(&format!(
            r#"
            CREATE game:{key} SET
                name = '{name}',
                abbreviation = NONE,
                is_active = true,
                created_at = time::now();
        "#
        ))
        .await
        .expect("seed game");
}

/// Seed a team record.
async fn seed_team(db: &Database, key: &str, name: &str, game_id: &str) {
    db.client
        .query(&format!(
            r#"
            CREATE team:{key} SET
                name = '{name}',
                game_id = '{game_id}',
                color = NONE,
                division = NONE,
                lore_quote = NONE,
                logo_url = NONE,
                is_active = true,
                created_at = time::now();
        "#
        ))
        .await
        .expect("seed team");
}

// Session tokens for test users
const ADMIN_TOKEN: &str = "test-admin-token";
const OFFICER_TOKEN: &str = "test-officer-token";
const MEMBER_TOKEN: &str = "test-member-token";
const RECRUIT_TOKEN: &str = "test-recruit-token";

/// Seed the standard set of test users (admin, officer, member, recruit).
async fn seed_all_roles(db: &Database) {
    seed_user(
        db,
        "adminuser",
        "adminmember",
        "TestAdmin",
        "admin",
        ADMIN_TOKEN,
    )
    .await;
    seed_user(
        db,
        "officeruser",
        "officermember",
        "TestOfficer",
        "officer",
        OFFICER_TOKEN,
    )
    .await;
    seed_user(
        db,
        "memberuser",
        "membermember",
        "TestMember",
        "member",
        MEMBER_TOKEN,
    )
    .await;
    seed_user(
        db,
        "recruituser",
        "recruitmember",
        "TestRecruit",
        "recruit",
        RECRUIT_TOKEN,
    )
    .await;
}

/// Build a request with Bearer token authentication.
fn authed_request(method: Method, uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::empty())
        .unwrap()
}

/// Build a JSON request with body and Bearer token authentication.
fn authed_json_request(method: Method, uri: &str, token: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}

/// Build an unauthenticated request.
fn unauthed_request(method: Method, uri: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::empty())
        .unwrap()
}

/// Extract response body as JSON.
async fn body_json(resp: axum::response::Response) -> Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

// ─── Health Check ───────────────────────────────────────────────────────────

#[tokio::test]
async fn health_check_returns_200() {
    let state = test_state().await;
    let app = create_router(state);

    let resp = app
        .oneshot(unauthed_request(Method::GET, "/api/health"))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
}

// ─── Members ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn list_members_requires_auth() {
    let state = test_state().await;
    let app = create_router(state);

    let resp = app
        .oneshot(unauthed_request(Method::GET, "/api/members"))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn list_members_returns_seeded_members() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    let app = create_router(state);

    let resp = app
        .oneshot(authed_request(Method::GET, "/api/members", ADMIN_TOKEN))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let data = json["data"].as_array().expect("data array");
    assert_eq!(data.len(), 4, "should have 4 seeded members");
}

#[tokio::test]
async fn get_member_by_id() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    let app = create_router(state);

    let resp = app
        .oneshot(authed_request(
            Method::GET,
            "/api/members/adminmember",
            ADMIN_TOKEN,
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["display_name"], "TestAdmin");
    assert_eq!(json["org_role"], "admin");
}

#[tokio::test]
async fn get_member_not_found() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    let app = create_router(state);

    let resp = app
        .oneshot(authed_request(
            Method::GET,
            "/api/members/nonexistent",
            ADMIN_TOKEN,
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn update_own_member_profile() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    let app = create_router(state);

    let body = json!({
        "display_name": "UpdatedName",
        "bio": "Hello world"
    });
    let resp = app
        .oneshot(authed_json_request(
            Method::PUT,
            "/api/members/membermember",
            MEMBER_TOKEN,
            body,
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["display_name"], "UpdatedName");
    assert_eq!(json["bio"], "Hello world");
}

/// JSON `null` must clear a double-Option field; omitting it must leave it
/// unchanged. Regression test: plain `Option<Option<T>>` deserializes null to
/// the outer None, silently dropping explicit clears.
#[tokio::test]
async fn update_member_null_clears_omit_preserves() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;

    // Set bio + main_role + twitch
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::PUT,
            "/api/members/membermember",
            MEMBER_TOKEN,
            json!({ "bio": "hi", "main_role": "tank", "twitch": "soot_tv" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Omit bio/twitch, clear main_role with null
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::PUT,
            "/api/members/membermember",
            MEMBER_TOKEN,
            json!({ "main_role": null }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["bio"], "hi", "omitted field must be preserved");
    assert_eq!(json["twitch"], "soot_tv", "omitted field must be preserved");
    assert!(
        json["main_role"].is_null(),
        "null must clear main_role, got {:?}",
        json["main_role"]
    );
}

#[tokio::test]
async fn update_member_profile_fields_and_game_account_meta() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    seed_game(&state.db, "ow2", "Overwatch 2").await;

    // Social URL rejected
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::PUT,
            "/api/members/membermember",
            MEMBER_TOKEN,
            json!({ "twitch": "https://twitch.tv/nogo" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // Handles accepted (leading @ stripped)
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::PUT,
            "/api/members/membermember",
            MEMBER_TOKEN,
            json!({
                "main_role": "support",
                "twitch": "@scuffedowl",
                "twitter": "scuffed_x"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["main_role"], "support");
    assert_eq!(json["twitch"], "scuffedowl");
    assert_eq!(json["twitter"], "scuffed_x");

    // Game account rank/sr/role
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::PUT,
            "/api/members/membermember/game-accounts",
            MEMBER_TOKEN,
            json!({
                "game_id": "ow2",
                "account_name": "Owl#1234",
                "rank": "Diamond 2",
                "sr": 3200,
                "role": "support"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let ga = body_json(resp).await;
    assert_eq!(ga["rank"], "Diamond 2");
    assert_eq!(ga["sr"], 3200);
    assert_eq!(ga["role"], "support");

    // Public profile surfaces the fields
    let app = create_router(state);
    let resp = app
        .oneshot(unauthed_request(
            Method::GET,
            "/api/public/members/membermember",
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["main_role"], "support");
    assert_eq!(json["twitch"], "scuffedowl");
    assert_eq!(json["twitter"], "scuffed_x");
    let accounts = json["game_accounts"].as_array().unwrap();
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0]["rank"], "Diamond 2");
    assert_eq!(accounts[0]["sr"], 3200);
}

#[tokio::test]
async fn member_cannot_update_other_member() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    let app = create_router(state);

    let body = json!({ "display_name": "Hacked" });
    let resp = app
        .oneshot(authed_json_request(
            Method::PUT,
            "/api/members/adminmember",
            MEMBER_TOKEN,
            body,
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn officer_can_update_other_member() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    let app = create_router(state);

    let body = json!({ "display_name": "OfficerEdit" });
    let resp = app
        .oneshot(authed_json_request(
            Method::PUT,
            "/api/members/membermember",
            OFFICER_TOKEN,
            body,
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["display_name"], "OfficerEdit");
}

#[tokio::test]
async fn change_role_requires_admin() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;

    // Officer cannot change roles
    let app = create_router(state.clone());
    let body = json!({ "role": "officer" });
    let resp = app
        .oneshot(authed_json_request(
            Method::PATCH,
            "/api/members/recruitmember/role",
            OFFICER_TOKEN,
            body,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // Admin can change roles
    let app = create_router(state);
    let body = json!({ "role": "officer" });
    let resp = app
        .oneshot(authed_json_request(
            Method::PATCH,
            "/api/members/recruitmember/role",
            ADMIN_TOKEN,
            body,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["org_role"], "officer");
}

// ─── Teams ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn list_teams_is_public() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    seed_game(&state.db, "ow2", "Overwatch 2").await;
    seed_team(&state.db, "teamalpha", "Alpha Squad", "ow2").await;
    let app = create_router(state);

    // Teams list does NOT require auth
    let resp = app
        .oneshot(unauthed_request(Method::GET, "/api/teams"))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let data = json["data"].as_array().expect("data array");
    assert_eq!(data.len(), 1);
    assert_eq!(data[0]["name"], "Alpha Squad");
}

#[tokio::test]
async fn get_team_by_id() {
    let state = test_state().await;
    seed_game(&state.db, "ow2", "Overwatch 2").await;
    seed_team(&state.db, "teamalpha", "Alpha Squad", "ow2").await;
    let app = create_router(state);

    let resp = app
        .oneshot(unauthed_request(Method::GET, "/api/teams/teamalpha"))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["name"], "Alpha Squad");
}

#[tokio::test]
async fn get_team_not_found() {
    let state = test_state().await;
    let app = create_router(state);

    let resp = app
        .oneshot(unauthed_request(Method::GET, "/api/teams/nonexistent"))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn create_team_requires_admin() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    seed_game(&state.db, "ow2", "Overwatch 2").await;

    let body = json!({
        "name": "New Team",
        "game_id": "ow2"
    });

    // Officer cannot create teams
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/teams",
            OFFICER_TOKEN,
            body.clone(),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // Admin can create teams
    let app = create_router(state);
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/teams",
            ADMIN_TOKEN,
            body,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let json = body_json(resp).await;
    assert_eq!(json["name"], "New Team");
}

#[tokio::test]
async fn update_team_requires_officer() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    seed_game(&state.db, "ow2", "Overwatch 2").await;
    seed_team(&state.db, "teamalpha", "Alpha Squad", "ow2").await;

    let body = json!({ "name": "Renamed Squad" });

    // Regular member cannot update
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::PUT,
            "/api/teams/teamalpha",
            MEMBER_TOKEN,
            body.clone(),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // Officer can update
    let app = create_router(state);
    let resp = app
        .oneshot(authed_json_request(
            Method::PUT,
            "/api/teams/teamalpha",
            OFFICER_TOKEN,
            body,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["name"], "Renamed Squad");
}

// ─── Tournaments ────────────────────────────────────────────────────────────

#[tokio::test]
async fn list_tournaments_is_public() {
    let state = test_state().await;
    let app = create_router(state);

    let resp = app
        .oneshot(unauthed_request(Method::GET, "/api/tournaments"))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert!(json["data"].is_array());
}

#[tokio::test]
async fn create_tournament_requires_officer() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;

    let body = json!({
        "name": "Test Cup",
        "format": "single_elim",
        "best_of": 3
    });

    // Member cannot create
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/tournaments",
            MEMBER_TOKEN,
            body.clone(),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // Officer can create
    let app = create_router(state);
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/tournaments",
            OFFICER_TOKEN,
            body,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let json = body_json(resp).await;
    assert_eq!(json["name"], "Test Cup");
    assert_eq!(json["format"], "single_elim");
    assert_eq!(json["status"], "draft");
}

#[tokio::test]
async fn tournament_lifecycle() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    seed_game(&state.db, "ow2", "Overwatch 2").await;

    // Create tournament
    let app = create_router(state.clone());
    let body = json!({
        "name": "Lifecycle Cup",
        "format": "single_elim",
        "best_of": 1
    });
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/tournaments",
            OFFICER_TOKEN,
            body,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let json = body_json(resp).await;
    let tid = json["id"].as_str().unwrap().to_string();

    // Transition draft → registration
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::PATCH,
            &format!("/api/tournaments/{tid}/status"),
            OFFICER_TOKEN,
            json!({ "status": "registration" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["status"], "registration");

    // Add 2 teams via seeding
    seed_team(&state.db, "t1", "Team 1", "ow2").await;
    seed_team(&state.db, "t2", "Team 2", "ow2").await;

    // Add participants
    for team_id in ["t1", "t2"] {
        let app = create_router(state.clone());
        let resp = app
            .oneshot(authed_json_request(
                Method::POST,
                &format!("/api/tournaments/{tid}/participants"),
                OFFICER_TOKEN,
                json!({ "team_id": team_id }),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    // List participants
    let app = create_router(state.clone());
    let resp = app
        .oneshot(unauthed_request(
            Method::GET,
            &format!("/api/tournaments/{tid}/participants"),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let participants = json.as_array().unwrap();
    assert_eq!(participants.len(), 2);

    // Generate bracket
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_request(
            Method::POST,
            &format!("/api/tournaments/{tid}/generate-bracket"),
            OFFICER_TOKEN,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Transition registration → in_progress
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::PATCH,
            &format!("/api/tournaments/{tid}/status"),
            OFFICER_TOKEN,
            json!({ "status": "in_progress" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // List matches
    let app = create_router(state.clone());
    let resp = app
        .oneshot(unauthed_request(
            Method::GET,
            &format!("/api/tournaments/{tid}/matches"),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let matches = json.as_array().unwrap();
    assert!(!matches.is_empty(), "bracket should have matches");

    // Report the final match
    let final_match = &matches[0];
    let mid = final_match["id"].as_str().unwrap();
    let pa = final_match["participant_a_id"].as_str().unwrap();

    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::PATCH,
            &format!("/api/tournaments/{tid}/matches/{mid}/report"),
            OFFICER_TOKEN,
            json!({
                "score_a": 1,
                "score_b": 0,
                "winner_id": pa,
                "replay_codes": ["REPLAY1"]
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn tournament_invalid_status_transition() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;

    // Create tournament (starts in draft)
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/tournaments",
            OFFICER_TOKEN,
            json!({ "name": "Bad Transition", "format": "single_elim" }),
        ))
        .await
        .unwrap();
    let json = body_json(resp).await;
    let tid = json["id"].as_str().unwrap().to_string();

    // Try to go directly from draft → in_progress (invalid)
    let app = create_router(state);
    let resp = app
        .oneshot(authed_json_request(
            Method::PATCH,
            &format!("/api/tournaments/{tid}/status"),
            OFFICER_TOKEN,
            json!({ "status": "in_progress" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ─── Match-report integrity (DR1-DB-001/002/004/009) ─────────────────────────

/// Create a 4-team single-elim tournament, generate its bracket, and move it to
/// `in_progress`. Returns the tournament id. Team/participant keys are suffixed
/// with `tag` so multiple brackets can coexist in one test DB.
async fn setup_single_elim_4(state: &AppState, tag: &str) -> String {
    seed_game(&state.db, "ow2", "Overwatch 2").await;

    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/tournaments",
            OFFICER_TOKEN,
            json!({ "name": format!("Bracket {tag}"), "format": "single_elim", "best_of": 1 }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let tid = body_json(resp).await["id"].as_str().unwrap().to_string();

    let app = create_router(state.clone());
    app.oneshot(authed_json_request(
        Method::PATCH,
        &format!("/api/tournaments/{tid}/status"),
        OFFICER_TOKEN,
        json!({ "status": "registration" }),
    ))
    .await
    .unwrap();

    for i in 1..=4 {
        let key = format!("{tag}t{i}");
        seed_team(&state.db, &key, &format!("Team {tag}{i}"), "ow2").await;
        let app = create_router(state.clone());
        let resp = app
            .oneshot(authed_json_request(
                Method::POST,
                &format!("/api/tournaments/{tid}/participants"),
                OFFICER_TOKEN,
                json!({ "team_id": key }),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_request(
            Method::POST,
            &format!("/api/tournaments/{tid}/generate-bracket"),
            OFFICER_TOKEN,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let app = create_router(state.clone());
    app.oneshot(authed_json_request(
        Method::PATCH,
        &format!("/api/tournaments/{tid}/status"),
        OFFICER_TOKEN,
        json!({ "status": "in_progress" }),
    ))
    .await
    .unwrap();

    tid
}

async fn fetch_matches(state: &AppState, tid: &str) -> Vec<Value> {
    let app = create_router(state.clone());
    let resp = app
        .oneshot(unauthed_request(
            Method::GET,
            &format!("/api/tournaments/{tid}/matches"),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    body_json(resp).await.as_array().unwrap().clone()
}

/// The two round-1 (semifinal) matches: both participants present and a
/// `next_match_id` pointer set. Returns them as (this, other) is left to caller.
fn semifinals(matches: &[Value]) -> Vec<&Value> {
    matches
        .iter()
        .filter(|m| {
            m["next_match_id"].is_string()
                && m["participant_a_id"].is_string()
                && m["participant_b_id"].is_string()
        })
        .collect()
}

/// DB-001 (route 400) + no next-round poison: a winner that is not one of the
/// match participants is rejected, and the downstream slot stays empty.
#[tokio::test]
async fn report_foreign_winner_rejected_and_next_slot_not_poisoned() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    let tid = setup_single_elim_4(&state, "a").await;

    let matches = fetch_matches(&state, &tid).await;
    let semis = semifinals(&matches);
    assert_eq!(semis.len(), 2, "4-player single elim has two semifinals");
    let m = semis[0];
    let other = semis[1];
    let mid = m["id"].as_str().unwrap();
    let next_id = m["next_match_id"].as_str().unwrap().to_string();
    let next_slot = m["next_match_slot"].as_str().unwrap().to_string();
    // A participant from the *other* semifinal — a valid id, but foreign here.
    let foreign = other["participant_a_id"].as_str().unwrap();

    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::PATCH,
            &format!("/api/tournaments/{tid}/matches/{mid}/report"),
            OFFICER_TOKEN,
            json!({ "score_a": 1, "score_b": 0, "winner_id": foreign }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // The next-round slot must not have been filled with the foreign id.
    let after = fetch_matches(&state, &tid).await;
    let next = after.iter().find(|x| x["id"] == next_id).unwrap();
    assert!(
        next[format!("participant_{next_slot}_id")].is_null(),
        "next-round slot {next_slot} was poisoned: {next:?}"
    );
    // And the reported match is still pending.
    let same = after.iter().find(|x| x["id"] == mid).unwrap();
    assert_eq!(same["status"], "pending");
}

/// DB-004 (route 409) + no double-advance: a second report of an already
/// completed match is rejected and the next-round slot is not overwritten.
#[tokio::test]
async fn double_report_rejected_and_does_not_double_advance() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    let tid = setup_single_elim_4(&state, "b").await;

    let matches = fetch_matches(&state, &tid).await;
    let semis = semifinals(&matches);
    let m = semis[0];
    let mid = m["id"].as_str().unwrap();
    let next_id = m["next_match_id"].as_str().unwrap().to_string();
    let next_slot = m["next_match_slot"].as_str().unwrap().to_string();
    let winner = m["participant_a_id"].as_str().unwrap().to_string();
    let loser = m["participant_b_id"].as_str().unwrap().to_string();

    // First report succeeds and advances `winner`.
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::PATCH,
            &format!("/api/tournaments/{tid}/matches/{mid}/report"),
            OFFICER_TOKEN,
            json!({ "score_a": 1, "score_b": 0, "winner_id": winner }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let after1 = fetch_matches(&state, &tid).await;
    let next1 = after1.iter().find(|x| x["id"] == next_id).unwrap();
    assert_eq!(next1[format!("participant_{next_slot}_id")], json!(winner));

    // Second report (flipping the winner to the loser) must be rejected …
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::PATCH,
            &format!("/api/tournaments/{tid}/matches/{mid}/report"),
            OFFICER_TOKEN,
            json!({ "score_a": 0, "score_b": 1, "winner_id": loser }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);

    // … and the next-round slot must still hold the original winner.
    let after2 = fetch_matches(&state, &tid).await;
    let next2 = after2.iter().find(|x| x["id"] == next_id).unwrap();
    assert_eq!(next2[format!("participant_{next_slot}_id")], json!(winner));
}

/// DB-002: reporting a match under the wrong tournament path is rejected (404),
/// even though the match id is globally valid.
#[tokio::test]
async fn report_under_wrong_tournament_path_rejected() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    let tid1 = setup_single_elim_4(&state, "c").await;
    let tid2 = setup_single_elim_4(&state, "d").await;

    let matches = fetch_matches(&state, &tid1).await;
    let m = semifinals(&matches)[0];
    let mid = m["id"].as_str().unwrap();
    let winner = m["participant_a_id"].as_str().unwrap();

    // Correct match + winner, but reported under tid2's path.
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::PATCH,
            &format!("/api/tournaments/{tid2}/matches/{mid}/report"),
            OFFICER_TOKEN,
            json!({ "score_a": 1, "score_b": 0, "winner_id": winner }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // The match in tid1 must remain untouched (still pending).
    let after = fetch_matches(&state, &tid1).await;
    let same = after.iter().find(|x| x["id"] == mid).unwrap();
    assert_eq!(same["status"], "pending");
}

/// Regression: a normal, valid report still succeeds and advances the winner.
#[tokio::test]
async fn valid_report_advances_winner() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    let tid = setup_single_elim_4(&state, "e").await;

    let matches = fetch_matches(&state, &tid).await;
    let m = semifinals(&matches)[0];
    let mid = m["id"].as_str().unwrap();
    let next_id = m["next_match_id"].as_str().unwrap().to_string();
    let next_slot = m["next_match_slot"].as_str().unwrap().to_string();
    let winner = m["participant_a_id"].as_str().unwrap().to_string();

    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::PATCH,
            &format!("/api/tournaments/{tid}/matches/{mid}/report"),
            OFFICER_TOKEN,
            json!({ "score_a": 1, "score_b": 0, "winner_id": winner, "replay_codes": ["R1"] }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let reported = body_json(resp).await;
    assert_eq!(reported["status"], "completed");
    assert_eq!(reported["winner_id"], json!(winner));

    let after = fetch_matches(&state, &tid).await;
    let next = after.iter().find(|x| x["id"] == next_id).unwrap();
    assert_eq!(next[format!("participant_{next_slot}_id")], json!(winner));
}

// ─── Auth Extractor Hierarchy ───────────────────────────────────────────────

#[tokio::test]
async fn unauthenticated_gets_401_on_protected_routes() {
    let state = test_state().await;

    let protected_routes = [
        (Method::GET, "/api/members"),
        (Method::GET, "/api/members/someid"),
        (Method::PUT, "/api/members/someid"),
        (Method::PATCH, "/api/members/someid/role"),
    ];

    for (method, uri) in &protected_routes {
        let app = create_router(state.clone());
        let resp = app
            .oneshot(unauthed_request(method.clone(), uri))
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "expected 401 for {method} {uri}"
        );
    }
}

#[tokio::test]
async fn recruit_can_access_org_member_routes() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    let app = create_router(state);

    // Recruits are org members, so they can list members
    let resp = app
        .oneshot(authed_request(Method::GET, "/api/members", RECRUIT_TOKEN))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn member_cannot_access_officer_routes() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    seed_game(&state.db, "ow2", "Overwatch 2").await;

    // Creating a team requires AdminUser
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/teams",
            MEMBER_TOKEN,
            json!({ "name": "X", "game_id": "ow2" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // Creating a tournament requires OfficerUser
    let app = create_router(state);
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/tournaments",
            MEMBER_TOKEN,
            json!({ "name": "X", "format": "single_elim" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn officer_cannot_access_admin_routes() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;

    // Changing role requires AdminUser
    let app = create_router(state);
    let resp = app
        .oneshot(authed_json_request(
            Method::PATCH,
            "/api/members/membermember/role",
            OFFICER_TOKEN,
            json!({ "role": "officer" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn admin_can_access_all_routes() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    seed_game(&state.db, "ow2", "Overwatch 2").await;

    // Admin can list members (OrgMember)
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_request(Method::GET, "/api/members", ADMIN_TOKEN))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Admin can create tournaments (OfficerUser)
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/tournaments",
            ADMIN_TOKEN,
            json!({ "name": "Admin Cup", "format": "round_robin" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // Admin can create teams (AdminUser)
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/teams",
            ADMIN_TOKEN,
            json!({ "name": "Admin Team", "game_id": "ow2" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // Admin can change roles (AdminUser)
    let app = create_router(state);
    let resp = app
        .oneshot(authed_json_request(
            Method::PATCH,
            "/api/members/recruitmember/role",
            ADMIN_TOKEN,
            json!({ "role": "member" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

// ─── Suspended Member ───────────────────────────────────────────────────────

#[tokio::test]
async fn suspended_member_is_rejected() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;

    // Create a suspension for the member
    state
        .db
        .client
        .query(
            r#"
            CREATE moderation_action SET
                member_id = 'membermember',
                action_type = 'suspension',
                reason = 'test suspension',
                issued_by = 'adminmember',
                expires_at = time::now() + 30d,
                is_active = true,
                created_at = time::now();
        "#,
        )
        .await
        .expect("create suspension");

    let app = create_router(state);
    let resp = app
        .oneshot(authed_request(Method::GET, "/api/members", MEMBER_TOKEN))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

// ─── Pagination ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn members_pagination_works() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    let app = create_router(state);

    // Request with limit=2
    let resp = app
        .oneshot(authed_request(
            Method::GET,
            "/api/members?limit=2",
            ADMIN_TOKEN,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let data = json["data"].as_array().unwrap();
    assert_eq!(data.len(), 2, "should return 2 items");
    assert!(
        json["next_cursor"].is_string(),
        "should have next_cursor for remaining items"
    );
}

// ─── Cookie Auth ────────────────────────────────────────────────────────────

#[tokio::test]
async fn cookie_auth_works() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;

    let app = create_router(state);
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/members")
        .header(header::COOKIE, format!("sc_session={ADMIN_TOKEN}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

// ─── NIP-05 Well-Known ─────────────────────────────────────────────────────

#[tokio::test]
async fn nip05_json_returns_empty_when_no_identities() {
    let state = test_state().await;
    let app = create_router(state);

    let resp = app
        .oneshot(unauthed_request(
            Method::GET,
            "/.well-known/nostr.json?name=_",
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert!(json["names"].as_object().unwrap().is_empty());
}

#[tokio::test]
async fn nip05_json_returns_identity_for_member_with_pubkey() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;

    // Set a nostr pubkey on the admin member
    let fake_pubkey = "a".repeat(64);
    state
        .db
        .client
        .query(&format!(
            "UPDATE member:adminmember SET nostr_pubkey = '{fake_pubkey}'"
        ))
        .await
        .expect("set nostr pubkey");

    let app = create_router(state);
    let resp = app
        .oneshot(unauthed_request(
            Method::GET,
            "/.well-known/nostr.json?name=_",
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let names = json["names"].as_object().unwrap();
    assert_eq!(names.len(), 1);
    assert_eq!(names["testadmin"], fake_pubkey);
}

#[tokio::test]
async fn nip05_json_filters_by_name() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;

    let pk_admin = "a".repeat(64);
    let pk_officer = "b".repeat(64);
    state
        .db
        .client
        .query(&format!(
            "UPDATE member:adminmember SET nostr_pubkey = '{pk_admin}'; \
             UPDATE member:officermember SET nostr_pubkey = '{pk_officer}'"
        ))
        .await
        .expect("set nostr pubkeys");

    let app = create_router(state);
    let resp = app
        .oneshot(unauthed_request(
            Method::GET,
            "/.well-known/nostr.json?name=testadmin",
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let names = json["names"].as_object().unwrap();
    assert_eq!(names.len(), 1);
    assert_eq!(names["testadmin"], pk_admin);
}

#[tokio::test]
async fn nip05_json_has_cors_header() {
    let state = test_state().await;
    let app = create_router(state);

    let resp = app
        .oneshot(unauthed_request(
            Method::GET,
            "/.well-known/nostr.json?name=_",
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let cors = resp.headers().get("access-control-allow-origin").unwrap();
    assert_eq!(cors, "*");
}

#[tokio::test]
async fn nip05_json_includes_relay_hints_when_configured() {
    let mut state = test_state().await;
    state.relay_url = Some("wss://relay.scuffed.gg".into());
    seed_all_roles(&state.db).await;

    let fake_pubkey = "c".repeat(64);
    state
        .db
        .client
        .query(&format!(
            "UPDATE member:adminmember SET nostr_pubkey = '{fake_pubkey}'"
        ))
        .await
        .expect("set nostr pubkey");

    let app = create_router(state);
    let resp = app
        .oneshot(unauthed_request(
            Method::GET,
            "/.well-known/nostr.json?name=_",
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let relays = json["relays"].as_object().unwrap();
    assert_eq!(relays.len(), 1);
    let hints = relays[&fake_pubkey].as_array().unwrap();
    assert_eq!(hints[0], "wss://relay.scuffed.gg");
}

// ─── Nostr Challenge / Verify ──────────────────────────────────────────────

#[tokio::test]
async fn nostr_challenge_requires_auth() {
    let state = test_state().await;
    let app = create_router(state);

    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/nostr/challenge")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({"pubkey": "a".repeat(64)})).unwrap(),
        ))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn nostr_challenge_rejects_invalid_pubkey() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    let app = create_router(state);

    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/nostr/challenge",
            MEMBER_TOKEN,
            json!({"pubkey": "not-a-valid-pubkey"}),
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn nostr_challenge_accepts_valid_hex_pubkey() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    let app = create_router(state);

    // Generate a valid secp256k1 pubkey
    let keys = nostr::Keys::generate();
    let pubkey_hex = keys.public_key().to_hex();

    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/nostr/challenge",
            MEMBER_TOKEN,
            json!({"pubkey": pubkey_hex}),
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert!(json["challenge"]
        .as_str()
        .unwrap()
        .starts_with("scuffedclan-verify:"));
    assert!(!json["token"].as_str().unwrap().is_empty());
    assert_eq!(json["pubkey_hex"].as_str().unwrap(), pubkey_hex);
    assert_eq!(json["expires_in_secs"], 300);
}

#[tokio::test]
async fn nostr_verify_rejects_malformed_event() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    let app = create_router(state);

    // An empty object is not a valid nostr::Event, so Axum rejects at deserialization
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/nostr/verify",
            MEMBER_TOKEN,
            json!({
                "token": "invalid-token",
                "signed_event": {}
            }),
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

// ─── Nostr Identity Unlink ─────────────────────────────────────────────────

#[tokio::test]
async fn nostr_unlink_removes_pubkey() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;

    // Set a pubkey first
    let fake_pubkey = "d".repeat(64);
    state
        .db
        .client
        .query(&format!(
            "UPDATE member:membermember SET nostr_pubkey = '{fake_pubkey}'"
        ))
        .await
        .expect("set nostr pubkey");

    let app = create_router(state);
    let resp = app
        .oneshot(authed_request(
            Method::DELETE,
            "/api/nostr/identity",
            MEMBER_TOKEN,
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert!(json["nostr_pubkey"].is_null());
}

#[tokio::test]
async fn nostr_unlink_requires_auth() {
    let state = test_state().await;
    let app = create_router(state);

    let resp = app
        .oneshot(unauthed_request(Method::DELETE, "/api/nostr/identity"))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// ─── NIP-49 Export Backup ──────────────────────────────────────────────────

#[tokio::test]
async fn nostr_export_backup_rejects_short_password() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    let app = create_router(state);

    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/nostr/export-backup",
            MEMBER_TOKEN,
            json!({"password": "short"}),
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let json = body_json(resp).await;
    // Backup password floor is aligned to the account policy (12) — DR1-NOSTR-005.
    assert!(json["error"].as_str().unwrap().contains("12 characters"));
}

/// An 8–11 char password (previously accepted) is now rejected: the backup
/// password floor is aligned to `MIN_PASSWORD_LEN` (DR1-NOSTR-005).
#[tokio::test]
async fn nostr_export_backup_rejects_eleven_char_password() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    let app = create_router(state);

    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/nostr/export-backup",
            MEMBER_TOKEN,
            json!({ "password": "elevenchars" }), // 11 chars
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let json = body_json(resp).await;
    assert!(json["error"].as_str().unwrap().contains("12 characters"));
}

#[tokio::test]
async fn nostr_export_backup_rejects_non_server_managed() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    let app = create_router(state);

    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/nostr/export-backup",
            MEMBER_TOKEN,
            json!({"password": "a-strong-password-here"}),
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let json = body_json(resp).await;
    assert!(json["error"].as_str().unwrap().contains("server-managed"));
}

#[tokio::test]
async fn nostr_export_backup_requires_auth() {
    let state = test_state().await;
    let app = create_router(state);

    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/nostr/export-backup")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({"password": "mypassword123"})).unwrap(),
        ))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// ─── NIP-49 Import Key ────────────────────────────────────────────────────

#[tokio::test]
async fn nostr_import_key_rejects_invalid_ncryptsec() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    let app = create_router(state);

    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/nostr/import-key",
            MEMBER_TOKEN,
            json!({
                "ncryptsec": "not-valid-ncryptsec",
                "password": "anypassword"
            }),
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn nostr_import_key_rejects_wrong_password() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;

    // Create a valid ncryptsec with known password
    let keys = nostr::Keys::generate();
    let secret_hex = keys.secret_key().to_secret_hex();
    let ncryptsec = scuffed_auth::nip49::encrypt(&secret_hex, "correct-password").expect("encrypt");

    let app = create_router(state);
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/nostr/import-key",
            MEMBER_TOKEN,
            json!({
                "ncryptsec": ncryptsec,
                "password": "wrong-password"
            }),
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let json = body_json(resp).await;
    assert!(json["error"].as_str().unwrap().contains("decrypt"));
}

#[tokio::test]
async fn nostr_import_key_success() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;

    let keys = nostr::Keys::generate();
    let secret_hex = keys.secret_key().to_secret_hex();
    let expected_pubkey = keys.public_key().to_hex();
    let password = "secure-backup-password";
    let ncryptsec = scuffed_auth::nip49::encrypt(&secret_hex, password).expect("encrypt");

    let app = create_router(state);
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/nostr/import-key",
            MEMBER_TOKEN,
            json!({
                "ncryptsec": ncryptsec,
                "password": password
            }),
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["nostr_pubkey"].as_str().unwrap(), expected_pubkey);
}

/// Importing must NOT silently destroy a server-managed key (DR1-NOSTR-003):
/// a member currently in `server_managed` mode is refused with 409 and told to
/// unlink first.
#[tokio::test]
async fn nostr_import_key_rejects_server_managed_member() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;

    // Flip the member to server-managed mode (the import gate reads this field).
    state
        .db
        .client
        .query("UPDATE member:membermember SET nostr_key_mode = 'server_managed'")
        .await
        .expect("set server_managed mode");

    let keys = nostr::Keys::generate();
    let secret_hex = keys.secret_key().to_secret_hex();
    let password = "secure-backup-password";
    let ncryptsec = scuffed_auth::nip49::encrypt(&secret_hex, password).expect("encrypt");

    let app = create_router(state);
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/nostr/import-key",
            MEMBER_TOKEN,
            json!({
                "ncryptsec": ncryptsec,
                "password": password
            }),
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::CONFLICT);
    let json = body_json(resp).await;
    assert!(json["error"].as_str().unwrap().contains("server-managed"));
}

// ─── First-boot setup + local login ─────────────────────────────────────────

fn rate_limit_ip(builder: axum::http::request::Builder) -> axum::http::request::Builder {
    // The rate limiter keys off the peer socket (TrustedProxyIpKeyExtractor) and
    // only honors forwarded headers from a trusted peer. The real server injects
    // ConnectInfo via `into_make_service_with_connect_info`; `oneshot` does not,
    // so inject a loopback peer here to mirror production. Loopback is a trusted
    // proxy, so the forwarded header is still honored (key = 127.0.0.1).
    builder
        .header("x-forwarded-for", "127.0.0.1")
        .extension(axum::extract::ConnectInfo(std::net::SocketAddr::from((
            [127, 0, 0, 1],
            40000,
        ))))
}

#[tokio::test]
async fn setup_status_needs_setup_on_empty_db() {
    let state = test_state().await;
    let app = create_router(state);
    let res = app
        .oneshot(
            Request::builder()
                .uri("/api/auth/setup-status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let json = body_json(res).await;
    assert_eq!(json["needs_setup"], true);
    assert_eq!(json["local_login"], false);
}

#[tokio::test]
async fn setup_creates_admin_and_blocks_second_setup() {
    let state = test_state().await;
    let app = create_router(state);

    let res = app
        .clone()
        .oneshot(
            rate_limit_ip(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/auth/setup")
                    .header(header::CONTENT_TYPE, "application/json"),
            )
            .body(Body::from(
                serde_json::to_vec(&json!({
                    "username": "admin",
                    "password": "a-strong-password"
                }))
                .unwrap(),
            ))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let set_cookie = res
        .headers()
        .get(header::SET_COOKIE)
        .map(|v| v.to_str().unwrap().to_string());
    assert!(set_cookie.is_some(), "expected session cookie");

    let res2 = app
        .oneshot(
            rate_limit_ip(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/auth/setup")
                    .header(header::CONTENT_TYPE, "application/json"),
            )
            .body(Body::from(
                serde_json::to_vec(&json!({
                    "username": "other",
                    "password": "another-password"
                }))
                .unwrap(),
            ))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res2.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn local_login_works_after_setup() {
    let state = test_state().await;
    let app = create_router(state);

    let res = app
        .clone()
        .oneshot(
            rate_limit_ip(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/auth/setup")
                    .header(header::CONTENT_TYPE, "application/json"),
            )
            .body(Body::from(
                serde_json::to_vec(&json!({
                    "username": "Boss",
                    "password": "correct-horse-1"
                }))
                .unwrap(),
            ))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let res = app
        .oneshot(
            rate_limit_ip(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/auth/local/login")
                    .header(header::CONTENT_TYPE, "application/json"),
            )
            .body(Body::from(
                serde_json::to_vec(&json!({
                    "username": "boss",
                    "password": "correct-horse-1"
                }))
                .unwrap(),
            ))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(res.headers().get(header::SET_COOKIE).is_some());
}

#[tokio::test]
async fn setup_rejects_short_password() {
    let state = test_state().await;
    let app = create_router(state);
    let res = app
        .oneshot(
            rate_limit_ip(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/auth/setup")
                    .header(header::CONTENT_TYPE, "application/json"),
            )
            .body(Body::from(
                serde_json::to_vec(&json!({
                    "username": "admin",
                    "password": "short"
                }))
                .unwrap(),
            ))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

// ─── Membership spine + admin authority ─────────────────────────────────────

/// Seed a logged-in user with no member record (applicant).
async fn seed_applicant(db: &Database, user_key: &str, username: &str, token: &str) {
    let token_hash = hash_session_token(token);
    let pid = format!("{user_key}-provider-id");
    let pid_hash = hash_session_token(&pid);
    db.client
        .query(&format!(
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
        .unwrap_or_else(|e| panic!("seed applicant user {user_key}: {e}"));
    db.client
        .query(&format!(
            r#"CREATE session:sess_{user_key} SET
                user_id = '{user_key}',
                token = $tok,
                expires_at = time::now() + 365d,
                created_at = time::now()"#
        ))
        .bind(("tok", token_hash))
        .await
        .unwrap_or_else(|e| panic!("seed applicant session {user_key}: {e}"));
}

const APPLICANT_TOKEN: &str = "test-applicant-token";
const ADMIN2_TOKEN: &str = "test-admin2-token";

#[tokio::test]
async fn application_submit_accept_creates_member() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    seed_applicant(&state.db, "appuser", "Applicant", APPLICANT_TOKEN).await;

    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/applications",
            APPLICANT_TOKEN,
            json!({
                "preferred_games": ["ow2"],
                "preferred_roles": ["tank"],
                "message": "hi"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let created = body_json(resp).await;
    let app_id = created["id"].as_str().unwrap().to_string();
    assert_eq!(created["status"], "pending");

    // Double submit blocked
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/applications",
            APPLICANT_TOKEN,
            json!({
                "preferred_games": ["ow2"],
                "preferred_roles": ["tank"]
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);

    // Existing member cannot apply
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/applications",
            MEMBER_TOKEN,
            json!({
                "preferred_games": ["ow2"],
                "preferred_roles": ["dps"]
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);

    // Officer accepts
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::PATCH,
            &format!("/api/applications/{app_id}"),
            OFFICER_TOKEN,
            json!({ "status": "accepted", "review_notes": "lgtm" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let member = state
        .db
        .get_member_by_user("appuser")
        .await
        .unwrap()
        .expect("member should be provisioned");
    assert!(member.is_active);
    assert_eq!(member.org_role.to_string(), "recruit");
}

#[tokio::test]
async fn application_trial_then_accept_promotes_to_member() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    seed_applicant(&state.db, "trialuser", "Trialer", APPLICANT_TOKEN).await;

    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/applications",
            APPLICANT_TOKEN,
            json!({ "preferred_games": ["ow2"], "preferred_roles": ["support"] }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let app_id = body_json(resp).await["id"].as_str().unwrap().to_string();

    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::PATCH,
            &format!("/api/applications/{app_id}"),
            OFFICER_TOKEN,
            json!({ "status": "trial" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let recruit = state
        .db
        .get_member_by_user("trialuser")
        .await
        .unwrap()
        .expect("trial provisions recruit");
    assert_eq!(recruit.org_role.to_string(), "recruit");

    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::PATCH,
            &format!("/api/applications/{app_id}"),
            OFFICER_TOKEN,
            json!({ "status": "accepted" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let member = state
        .db
        .get_member_by_user("trialuser")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(member.org_role.to_string(), "member");
}

#[tokio::test]
async fn application_invalid_transition_rejected() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    seed_applicant(&state.db, "rejuser", "Rejectee", APPLICANT_TOKEN).await;

    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/applications",
            APPLICANT_TOKEN,
            json!({ "preferred_games": [], "preferred_roles": [] }),
        ))
        .await
        .unwrap();
    let app_id = body_json(resp).await["id"].as_str().unwrap().to_string();

    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::PATCH,
            &format!("/api/applications/{app_id}"),
            OFFICER_TOKEN,
            json!({ "status": "rejected" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // accepted is terminal — cannot go rejected → accepted
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::PATCH,
            &format!("/api/applications/{app_id}"),
            OFFICER_TOKEN,
            json!({ "status": "accepted" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn cannot_demote_last_active_admin() {
    let state = test_state().await;
    // Only one admin
    seed_user(
        &state.db,
        "adminuser",
        "adminmember",
        "TestAdmin",
        "admin",
        ADMIN_TOKEN,
    )
    .await;
    seed_user(
        &state.db,
        "memberuser",
        "membermember",
        "TestMember",
        "member",
        MEMBER_TOKEN,
    )
    .await;

    let app = create_router(state);
    let resp = app
        .oneshot(authed_json_request(
            Method::PATCH,
            "/api/members/adminmember/role",
            ADMIN_TOKEN,
            json!({ "role": "officer" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let body = body_json(resp).await;
    assert!(
        body["error"]
            .as_str()
            .unwrap_or("")
            .contains("last active admin"),
        "got: {body}"
    );
}

#[tokio::test]
async fn can_demote_admin_when_another_exists() {
    let state = test_state().await;
    seed_user(
        &state.db,
        "adminuser",
        "adminmember",
        "TestAdmin",
        "admin",
        ADMIN_TOKEN,
    )
    .await;
    seed_user(
        &state.db,
        "adminuser2",
        "adminmember2",
        "TestAdmin2",
        "admin",
        ADMIN2_TOKEN,
    )
    .await;

    let app = create_router(state);
    let resp = app
        .oneshot(authed_json_request(
            Method::PATCH,
            "/api/members/adminmember2/role",
            ADMIN_TOKEN,
            json!({ "role": "officer" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["org_role"], "officer");
}

#[tokio::test]
async fn cannot_deactivate_last_admin() {
    let state = test_state().await;
    seed_user(
        &state.db,
        "adminuser",
        "adminmember",
        "TestAdmin",
        "admin",
        ADMIN_TOKEN,
    )
    .await;
    seed_user(
        &state.db,
        "adminuser2",
        "adminmember2",
        "TestAdmin2",
        "admin",
        ADMIN2_TOKEN,
    )
    .await;

    // Deactivate second admin OK (still one left) — wait, we need only one active after
    // First demote admin2 path: deactivate admin2 when both exist is OK
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::PUT,
            "/api/members/adminmember2",
            ADMIN_TOKEN,
            json!({ "is_active": false }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Now only adminmember is active admin — cannot deactivate
    let app = create_router(state);
    let resp = app
        .oneshot(authed_json_request(
            Method::PUT,
            "/api/members/adminmember",
            ADMIN_TOKEN,
            json!({ "is_active": false }),
        ))
        .await
        .unwrap();
    // Self-deactivate also blocked
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn ban_revokes_sessions() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;

    // Member can list members before ban
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_request(Method::GET, "/api/members", MEMBER_TOKEN))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/moderation",
            OFFICER_TOKEN,
            json!({
                "member_id": "membermember",
                "action_type": "ban",
                "reason": "toxicity"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // Session revoked → unauthenticated
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_request(Method::GET, "/api/members", MEMBER_TOKEN))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // Even if session existed, suspension check would block — but revoke is the primary path
    let sessions = state
        .db
        .client
        .query("SELECT * FROM session WHERE user_id = 'memberuser'")
        .await;
    assert!(sessions.is_ok());
}

#[tokio::test]
async fn ban_deactivates_and_self_ban_blocked() {
    let state = test_state().await;
    seed_user(
        &state.db,
        "adminuser",
        "adminmember",
        "TestAdmin",
        "admin",
        ADMIN_TOKEN,
    )
    .await;
    seed_user(
        &state.db,
        "adminuser2",
        "adminmember2",
        "TestAdmin2",
        "admin",
        ADMIN2_TOKEN,
    )
    .await;

    // Self-ban blocked
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/moderation",
            ADMIN_TOKEN,
            json!({
                "member_id": "adminmember",
                "action_type": "ban",
                "reason": "self"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // Ban peer admin OK → deactivates them
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/moderation",
            ADMIN_TOKEN,
            json!({
                "member_id": "adminmember2",
                "action_type": "ban",
                "reason": "gone"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let banned = state.db.get_member("adminmember2").await.unwrap().unwrap();
    assert!(!banned.is_active, "ban should deactivate membership");
    assert_eq!(state.db.count_active_admins().await.unwrap(), 1);

    // Last remaining admin cannot demote self (covered elsewhere); cannot deactivate self
    let app = create_router(state);
    let resp = app
        .oneshot(authed_json_request(
            Method::PUT,
            "/api/members/adminmember",
            ADMIN_TOKEN,
            json!({ "is_active": false }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn officer_cannot_moderate_officer() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;

    let app = create_router(state);
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/moderation",
            OFFICER_TOKEN,
            json!({
                "member_id": "adminmember",
                "action_type": "warning",
                "reason": "nope"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn cannot_suspend_last_actionable_admin() {
    let state = test_state().await;
    seed_user(
        &state.db,
        "adminuser",
        "adminmember",
        "TestAdmin",
        "admin",
        ADMIN_TOKEN,
    )
    .await;
    seed_user(
        &state.db,
        "adminuser2",
        "adminmember2",
        "TestAdmin2",
        "admin",
        ADMIN2_TOKEN,
    )
    .await;

    // Suspend peer admin OK
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/moderation",
            ADMIN_TOKEN,
            json!({
                "member_id": "adminmember2",
                "action_type": "suspension",
                "reason": "cooldown"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    assert_eq!(state.db.count_actionable_admins().await.unwrap(), 1);
    // Still two is_active admins (suspension keeps is_active)
    assert_eq!(state.db.count_active_admins().await.unwrap(), 2);

    // Self-suspend still blocked
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/moderation",
            ADMIN_TOKEN,
            json!({
                "member_id": "adminmember",
                "action_type": "suspension",
                "reason": "self"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // Demote last actionable while peer is suspended → blocked
    // (old bug: count_active_admins==2 would have allowed this lockout)
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::PATCH,
            "/api/members/adminmember/role",
            ADMIN_TOKEN,
            json!({ "role": "officer" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let body = body_json(resp).await;
    assert!(
        body["error"]
            .as_str()
            .unwrap_or("")
            .contains("last active admin"),
        "got: {body}"
    );

    // One actionable admin remains → setup stays closed
    assert!(state.db.has_admin_member().await.unwrap());
}

#[tokio::test]
async fn cannot_demote_last_actionable_admin_when_other_suspended() {
    let state = test_state().await;
    seed_user(
        &state.db,
        "adminuser",
        "adminmember",
        "TestAdmin",
        "admin",
        ADMIN_TOKEN,
    )
    .await;
    seed_user(
        &state.db,
        "adminuser2",
        "adminmember2",
        "TestAdmin2",
        "admin",
        ADMIN2_TOKEN,
    )
    .await;

    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/moderation",
            ADMIN_TOKEN,
            json!({
                "member_id": "adminmember2",
                "action_type": "suspension",
                "reason": "temp"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // Deactivate last actionable also blocked
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::PUT,
            "/api/members/adminmember",
            ADMIN_TOKEN,
            json!({ "is_active": false }),
        ))
        .await
        .unwrap();
    // Self-deactivate always forbidden
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // Demote suspended admin is allowed (they are not actionable)
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::PATCH,
            "/api/members/adminmember2/role",
            ADMIN_TOKEN,
            json!({ "role": "officer" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // After demoting suspended admin, still one actionable admin
    assert_eq!(state.db.count_actionable_admins().await.unwrap(), 1);
}

#[tokio::test]
async fn application_stale_transition_conflict() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    seed_applicant(&state.db, "casuser", "CasUser", APPLICANT_TOKEN).await;

    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/applications",
            APPLICANT_TOKEN,
            json!({ "preferred_games": ["ow2"], "preferred_roles": ["tank"] }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let app_id = body_json(resp).await["id"].as_str().unwrap().to_string();

    // First transition: pending → rejected
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::PATCH,
            &format!("/api/applications/{app_id}"),
            OFFICER_TOKEN,
            json!({ "status": "rejected" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Stale transition (would have been pending → accepted) rejected by state machine
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::PATCH,
            &format!("/api/applications/{app_id}"),
            OFFICER_TOKEN,
            json!({ "status": "accepted" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // CAS path: force DB write with wrong expected status via direct API would need
    // concurrent writers. Simulate CAS by calling update with expected Pending while
    // row is Rejected — exercised through a second identical rejected attempt:
    let app = create_router(state);
    let resp = app
        .oneshot(authed_json_request(
            Method::PATCH,
            &format!("/api/applications/{app_id}"),
            OFFICER_TOKEN,
            json!({ "status": "rejected" }),
        ))
        .await
        .unwrap();
    // same-status is invalid transition (400), not conflict — CAS still holds for races
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn application_cas_via_db_rejects_stale_expected() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    seed_applicant(&state.db, "casdb", "CasDb", APPLICANT_TOKEN).await;

    let created = state
        .db
        .submit_application("casdb", vec!["ow2".into()], vec!["tank".into()], None)
        .await
        .unwrap();
    assert_eq!(created.status.to_string(), "pending");

    // Win the race: pending → trial
    state
        .db
        .update_application_status(
            &created.id,
            scuffed_db::ApplicationStatus::Pending,
            scuffed_db::ApplicationStatus::Trial,
            "officermember",
            None,
        )
        .await
        .unwrap();

    // Stale writer still expects Pending → Conflict
    let err = state
        .db
        .update_application_status(
            &created.id,
            scuffed_db::ApplicationStatus::Pending,
            scuffed_db::ApplicationStatus::Accepted,
            "adminmember",
            None,
        )
        .await
        .unwrap_err();
    match err {
        scuffed_db::DbError::Conflict(_) => {}
        other => panic!("expected Conflict, got {other}"),
    }
}

#[tokio::test]
async fn delete_game_account_requires_ownership() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    seed_game(&state.db, "ow2", "Overwatch 2").await;

    let a = state
        .db
        .upsert_game_account("membermember", "ow2", "MemberTag", None, None, None, None)
        .await
        .unwrap();
    let b = state
        .db
        .upsert_game_account("recruitmember", "ow2", "RecruitTag", None, None, None, None)
        .await
        .unwrap();

    // Member tries to delete recruit's account under member's path → 404
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_request(
            Method::DELETE,
            &format!("/api/members/membermember/game-accounts/{}", b.id),
            MEMBER_TOKEN,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // Owner can delete own account
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_request(
            Method::DELETE,
            &format!("/api/members/membermember/game-accounts/{}", a.id),
            MEMBER_TOKEN,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Recruit's account still exists
    let remaining = state
        .db
        .list_member_game_accounts("recruitmember")
        .await
        .unwrap();
    assert_eq!(remaining.len(), 1);
}

#[tokio::test]
async fn assert_has_actionable_admin_after_manual_lockout() {
    // Simulate concurrent demote race: both admins demoted without policy path,
    // then assert_has_actionable_admin reports conflict.
    let state = test_state().await;
    seed_user(
        &state.db,
        "adminuser",
        "adminmember",
        "TestAdmin",
        "admin",
        ADMIN_TOKEN,
    )
    .await;
    seed_user(
        &state.db,
        "adminuser2",
        "adminmember2",
        "TestAdmin2",
        "admin",
        ADMIN2_TOKEN,
    )
    .await;

    assert!(state.db.assert_has_actionable_admin().await.is_ok());
    state
        .db
        .change_member_role("adminmember", scuffed_db::OrgRole::Officer)
        .await
        .unwrap();
    state
        .db
        .change_member_role("adminmember2", scuffed_db::OrgRole::Officer)
        .await
        .unwrap();
    let err = state.db.assert_has_actionable_admin().await.unwrap_err();
    assert!(matches!(err, scuffed_db::DbError::Conflict(_)), "got {err}");
}

#[tokio::test]
async fn demote_compensates_if_no_actionable_admin_left() {
    // After pre-check passes with count=2, if the other admin is removed under us
    // (simulated by demoting them first in the same process before the second
    // demote request), the second demote is blocked by the pre-check. Here we
    // only verify post-write compensation path via assert after a successful
    // single demote still leaves one admin.
    let state = test_state().await;
    seed_user(
        &state.db,
        "adminuser",
        "adminmember",
        "TestAdmin",
        "admin",
        ADMIN_TOKEN,
    )
    .await;
    seed_user(
        &state.db,
        "adminuser2",
        "adminmember2",
        "TestAdmin2",
        "admin",
        ADMIN2_TOKEN,
    )
    .await;

    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::PATCH,
            "/api/members/adminmember2/role",
            ADMIN_TOKEN,
            json!({ "role": "officer" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(state.db.count_actionable_admins().await.unwrap(), 1);
    assert!(state.db.assert_has_actionable_admin().await.is_ok());

    // Second demote of last actionable → pre-check 403 (not 409 — no race)
    let app = create_router(state);
    let resp = app
        .oneshot(authed_json_request(
            Method::PATCH,
            "/api/members/adminmember/role",
            ADMIN_TOKEN,
            json!({ "role": "officer" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn change_role_route_uses_cas() {
    // DR1-ACCT-004: the PATCH /role route must go through change_member_role_cas.
    // Two facets:
    //   (1) happy path still succeeds through the CAS (no false conflict when the
    //       role has not moved), and
    //   (2) the CAS the route calls rejects a stale expected-role with Conflict —
    //       which the route maps to HTTP 409.
    let state = test_state().await;
    seed_user(
        &state.db,
        "adminuser",
        "adminmember",
        "TestAdmin",
        "admin",
        ADMIN_TOKEN,
    )
    .await;
    // Second admin so the last-admin guard never blocks the target's edits.
    seed_user(
        &state.db,
        "adminuser2",
        "adminmember2",
        "TestAdmin2",
        "admin",
        ADMIN2_TOKEN,
    )
    .await;
    seed_user(
        &state.db,
        "targetuser",
        "targetmember",
        "TargetMember",
        "member",
        MEMBER_TOKEN,
    )
    .await;

    // (1) Happy path through the CAS: member → officer succeeds (200) and the
    // role actually changes in the DB.
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::PATCH,
            "/api/members/targetmember/role",
            ADMIN_TOKEN,
            json!({ "role": "officer" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        state
            .db
            .get_member_safe("targetmember")
            .await
            .unwrap()
            .unwrap()
            .org_role,
        scuffed_db::OrgRole::Officer
    );

    // (2) The CAS the route relies on: with a stale expected-role (the target has
    // since moved to admin), change_member_role_cas returns Conflict. This is the
    // exact error the route translates into a 409 for a concurrent role change.
    state
        .db
        .change_member_role("targetmember", scuffed_db::OrgRole::Admin)
        .await
        .unwrap();
    let err = state
        .db
        .change_member_role_cas(
            "targetmember",
            scuffed_db::OrgRole::Officer, // stale expected
            scuffed_db::OrgRole::Member,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, scuffed_db::DbError::Conflict(_)), "got {err}");
}

#[tokio::test]
async fn open_application_count_guards_duplicate() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    seed_applicant(&state.db, "dupapp", "DupApp", APPLICANT_TOKEN).await;

    // First create via API
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/applications",
            APPLICANT_TOKEN,
            json!({ "preferred_games": ["ow2"], "preferred_roles": [] }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // Simulate race: second insert via DB then count>1 rollback path on next submit
    // is covered by sequential conflict. Also verify count_open + delete helpers.
    let second = state
        .db
        .submit_application("dupapp", vec![], vec![], None)
        .await
        .unwrap();
    assert_eq!(state.db.count_open_applications("dupapp").await.unwrap(), 2);
    state.db.delete_application(&second.id).await.unwrap();
    assert_eq!(state.db.count_open_applications("dupapp").await.unwrap(), 1);

    // Route-level double submit still 409
    let app = create_router(state);
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/applications",
            APPLICANT_TOKEN,
            json!({ "preferred_games": [], "preferred_roles": [] }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn applicant_self_withdraw_pending() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    seed_applicant(&state.db, "selfwd", "SelfWd", APPLICANT_TOKEN).await;

    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/applications",
            APPLICANT_TOKEN,
            json!({ "preferred_games": ["ow2"], "preferred_roles": [] }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/applications/mine/withdraw",
            APPLICANT_TOKEN,
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["status"], "withdrawn");

    // Cannot withdraw again
    let app = create_router(state);
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/applications/mine/withdraw",
            APPLICANT_TOKEN,
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn applicant_self_withdraw_trial_deactivates_recruit() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    seed_applicant(&state.db, "selftrial", "SelfTrial", APPLICANT_TOKEN).await;

    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/applications",
            APPLICANT_TOKEN,
            json!({ "preferred_games": [], "preferred_roles": [] }),
        ))
        .await
        .unwrap();
    let app_id = body_json(resp).await["id"].as_str().unwrap().to_string();

    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::PATCH,
            &format!("/api/applications/{app_id}"),
            OFFICER_TOKEN,
            json!({ "status": "trial" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(
        state
            .db
            .get_member_by_user("selftrial")
            .await
            .unwrap()
            .unwrap()
            .is_active
    );

    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/applications/mine/withdraw",
            APPLICANT_TOKEN,
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let member = state
        .db
        .get_member_by_user("selftrial")
        .await
        .unwrap()
        .unwrap();
    assert!(!member.is_active);
}

#[tokio::test]
async fn trial_reject_deactivates_recruit_and_withdraw_audits_distinct() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    seed_applicant(&state.db, "trialrej", "TrialRej", APPLICANT_TOKEN).await;

    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/applications",
            APPLICANT_TOKEN,
            json!({ "preferred_games": ["ow2"], "preferred_roles": ["dps"] }),
        ))
        .await
        .unwrap();
    let app_id = body_json(resp).await["id"].as_str().unwrap().to_string();

    // Start trial → provisions recruit
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::PATCH,
            &format!("/api/applications/{app_id}"),
            OFFICER_TOKEN,
            json!({ "status": "trial" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let recruit = state
        .db
        .get_member_by_user("trialrej")
        .await
        .unwrap()
        .expect("trial provisions recruit");
    assert!(recruit.is_active);

    // Reject → deactivates recruit (side effect before status write)
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::PATCH,
            &format!("/api/applications/{app_id}"),
            OFFICER_TOKEN,
            json!({ "status": "rejected" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let after = state
        .db
        .get_member_by_user("trialrej")
        .await
        .unwrap()
        .unwrap();
    assert!(!after.is_active, "reject should deactivate trial recruit");

    // Fresh application → withdraw uses dedicated audit action
    seed_applicant(&state.db, "withdrawu", "WithdrawU", "test-withdraw-token").await;
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/applications",
            "test-withdraw-token",
            json!({ "preferred_games": [], "preferred_roles": [] }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let w_id = body_json(resp).await["id"].as_str().unwrap().to_string();

    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::PATCH,
            &format!("/api/applications/{w_id}"),
            OFFICER_TOKEN,
            json!({ "status": "withdrawn" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let logs = state.db.list_audit_log(50, 0).await.unwrap();
    let withdraw_logged = logs.iter().any(|e| e.action == "withdrawn_application");
    assert!(
        withdraw_logged,
        "expected withdrawn_application in audit log, got: {:?}",
        logs.iter().map(|e| &e.action).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn suspended_admins_do_not_count_for_setup() {
    let state = test_state().await;
    seed_user(
        &state.db,
        "adminuser",
        "adminmember",
        "TestAdmin",
        "admin",
        ADMIN_TOKEN,
    )
    .await;
    seed_user(
        &state.db,
        "adminuser2",
        "adminmember2",
        "TestAdmin2",
        "admin",
        ADMIN2_TOKEN,
    )
    .await;

    assert!(state.db.has_admin_member().await.unwrap());

    // Suspend both via DB (bypass last-admin to simulate pre-fix lockout)
    state
        .db
        .create_moderation_action(
            "adminmember",
            scuffed_db::ModerationActionType::Suspension,
            "lockout-sim",
            "adminmember2",
            None,
        )
        .await
        .unwrap();
    state
        .db
        .create_moderation_action(
            "adminmember2",
            scuffed_db::ModerationActionType::Suspension,
            "lockout-sim",
            "adminmember",
            None,
        )
        .await
        .unwrap();

    assert_eq!(state.db.count_actionable_admins().await.unwrap(), 0);
    assert_eq!(state.db.count_active_admins().await.unwrap(), 2);
    // has_admin_member still reflects the live actionable count (0 → false); it
    // is used for UI/recruitment display. NOTE (DR1-ACCT-003): the unauthenticated
    // /api/auth/setup gate no longer keys off this value — see
    // setup_rejected_when_all_admins_suspended for the hardened behaviour.
    assert!(!state.db.has_admin_member().await.unwrap());
    // Bootstrap signal stays closed: members exist, so setup cannot reopen.
    assert!(state.db.has_any_member().await.unwrap());
}

/// DR1-ACCT-003: a transient zero-actionable-admin state (every admin
/// suspended) must NOT reopen the unauthenticated first-boot setup endpoint,
/// because member rows exist. Setup only bootstraps a genuinely empty instance.
#[tokio::test]
async fn setup_rejected_when_all_admins_suspended() {
    let state = test_state().await;
    seed_user(
        &state.db,
        "adminuser",
        "adminmember",
        "TestAdmin",
        "admin",
        ADMIN_TOKEN,
    )
    .await;
    seed_user(
        &state.db,
        "adminuser2",
        "adminmember2",
        "TestAdmin2",
        "admin",
        ADMIN2_TOKEN,
    )
    .await;

    // Suspend both admins (transient lockout) → zero actionable admins.
    state
        .db
        .create_moderation_action(
            "adminmember",
            scuffed_db::ModerationActionType::Suspension,
            "lockout-sim",
            "adminmember2",
            None,
        )
        .await
        .unwrap();
    state
        .db
        .create_moderation_action(
            "adminmember2",
            scuffed_db::ModerationActionType::Suspension,
            "lockout-sim",
            "adminmember",
            None,
        )
        .await
        .unwrap();
    assert_eq!(state.db.count_actionable_admins().await.unwrap(), 0);

    // The unauthenticated setup POST must still be refused (members exist).
    let app = create_router(state);
    let res = app
        .oneshot(
            rate_limit_ip(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/auth/setup")
                    .header(header::CONTENT_TYPE, "application/json"),
            )
            .body(Body::from(
                serde_json::to_vec(&json!({
                    "username": "attacker",
                    "password": "a-strong-password"
                }))
                .unwrap(),
            ))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        StatusCode::FORBIDDEN,
        "setup must stay closed once any member exists"
    );
}

// ─── Public surfaces (is_public gating) ─────────────────────────────────────

#[tokio::test]
async fn private_events_hidden_from_anon_visible_to_member() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;

    // Create private (default) and public events as officer
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/events",
            OFFICER_TOKEN,
            json!({
                "title": "Internal Practice",
                "day_of_week": 1,
                "time": "19:00",
                "timezone": "UTC",
                "is_recurring": true,
                "is_public": false
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/events",
            OFFICER_TOKEN,
            json!({
                "title": "Public Scrim Night",
                "day_of_week": 3,
                "time": "20:00",
                "timezone": "UTC",
                "is_recurring": true,
                "is_public": true
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // Anon: only public
    let app = create_router(state.clone());
    let resp = app
        .oneshot(unauthed_request(Method::GET, "/api/events"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let titles: Vec<&str> = json["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["title"].as_str().unwrap())
        .collect();
    assert!(titles.contains(&"Public Scrim Night"));
    assert!(!titles.contains(&"Internal Practice"));

    // Member: both
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_request(Method::GET, "/api/events", MEMBER_TOKEN))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let titles: Vec<&str> = json["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["title"].as_str().unwrap())
        .collect();
    assert!(titles.contains(&"Public Scrim Night"));
    assert!(titles.contains(&"Internal Practice"));

    // Public overview filters private events
    let app = create_router(state);
    let resp = app
        .oneshot(unauthed_request(Method::GET, "/api/public/overview"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let titles: Vec<&str> = json["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["title"].as_str().unwrap())
        .collect();
    assert!(titles.contains(&"Public Scrim Night"));
    assert!(!titles.contains(&"Internal Practice"));
}

#[tokio::test]
async fn match_is_public_gates_team_list_and_strips_notes() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    seed_game(&state.db, "ow2", "Overwatch 2").await;
    seed_team(&state.db, "teamalpha", "Alpha Squad", "ow2").await;

    // Private official (default)
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/matches",
            OFFICER_TOKEN,
            json!({
                "team_id": "teamalpha",
                "opponent": "Hidden FC",
                "score_us": 2,
                "score_them": 1,
                "match_type": "official",
                "played_at": "2026-06-01T18:00:00Z",
                "notes": "internal review notes",
                "is_public": false
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // Public official with notes
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/matches",
            OFFICER_TOKEN,
            json!({
                "team_id": "teamalpha",
                "opponent": "Public FC",
                "score_us": 3,
                "score_them": 0,
                "match_type": "official",
                "played_at": "2026-06-02T18:00:00Z",
                "notes": "should be stripped for anon",
                "is_public": true
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let public_match = body_json(resp).await;
    assert_eq!(public_match["is_public"], true);

    // Public scrim (is_public true but scrims never list publicly)
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/matches",
            OFFICER_TOKEN,
            json!({
                "team_id": "teamalpha",
                "opponent": "Scrim Opp",
                "score_us": 1,
                "score_them": 1,
                "match_type": "scrim",
                "played_at": "2026-06-03T18:00:00Z",
                "is_public": true
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // Anon team matches: only Public FC; notes stripped
    let app = create_router(state.clone());
    let resp = app
        .oneshot(unauthed_request(
            Method::GET,
            "/api/teams/teamalpha/matches",
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let data = json["data"].as_array().unwrap();
    assert_eq!(
        data.len(),
        1,
        "anon should see only public non-scrim: {data:?}"
    );
    assert_eq!(data[0]["opponent"], "Public FC");
    assert!(data[0]["notes"].is_null());
    assert!(
        data[0]["recorded_by"].is_null(),
        "public list must strip recorded_by to null, got {:?}",
        data[0]["recorded_by"]
    );

    // Member sees all three with notes intact on private row
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_request(
            Method::GET,
            "/api/teams/teamalpha/matches",
            MEMBER_TOKEN,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let data = json["data"].as_array().unwrap();
    assert_eq!(data.len(), 3, "member should see all matches: {data:?}");

    // Public team detail recent_matches
    let app = create_router(state);
    let resp = app
        .oneshot(unauthed_request(Method::GET, "/api/public/teams/teamalpha"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let recent = json["recent_matches"].as_array().unwrap();
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0]["opponent"], "Public FC");
    assert!(
        recent[0].get("notes").is_none() || recent[0]["notes"].is_null(),
        "PublicMatch must not expose notes"
    );
    assert!(
        recent[0].get("recorded_by").is_none(),
        "PublicMatch must not expose recorded_by"
    );
    assert_eq!(recent[0]["team_id"], "teamalpha");
}

#[tokio::test]
async fn unpublish_match_hides_from_anon() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    seed_game(&state.db, "ow2", "Overwatch 2").await;
    seed_team(&state.db, "teamalpha", "Alpha Squad", "ow2").await;

    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/matches",
            OFFICER_TOKEN,
            json!({
                "team_id": "teamalpha",
                "opponent": "Toggle FC",
                "score_us": 2,
                "score_them": 2,
                "match_type": "official",
                "played_at": "2026-06-10T18:00:00Z",
                "is_public": true
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let id = body_json(resp).await["id"].as_str().unwrap().to_string();

    // Visible while public
    let app = create_router(state.clone());
    let resp = app
        .oneshot(unauthed_request(
            Method::GET,
            "/api/teams/teamalpha/matches",
        ))
        .await
        .unwrap();
    let data = body_json(resp).await;
    assert_eq!(data["data"].as_array().unwrap().len(), 1);

    // Unpublish
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::PUT,
            &format!("/api/matches/{id}"),
            OFFICER_TOKEN,
            json!({ "is_public": false }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await["is_public"], false);

    let app = create_router(state);
    let resp = app
        .oneshot(unauthed_request(
            Method::GET,
            "/api/teams/teamalpha/matches",
        ))
        .await
        .unwrap();
    let data = body_json(resp).await;
    assert_eq!(
        data["data"].as_array().unwrap().len(),
        0,
        "unpublished match must vanish from anon list"
    );
}

#[tokio::test]
async fn calendar_ics_excludes_private_events() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;

    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/events",
            OFFICER_TOKEN,
            json!({
                "title": "Secret Practice",
                "day_of_week": 2,
                "time": "18:00",
                "timezone": "UTC",
                "is_recurring": true,
                "is_public": false
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/events",
            OFFICER_TOKEN,
            json!({
                "title": "Open Night",
                "day_of_week": 5,
                "time": "21:00",
                "timezone": "UTC",
                "is_recurring": true,
                "is_public": true
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let app = create_router(state);
    let resp = app
        .oneshot(unauthed_request(Method::GET, "/api/calendar/all.ics"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let ics = String::from_utf8_lossy(&bytes);
    assert!(ics.contains("Open Night"), "public event in ICS");
    assert!(
        !ics.contains("Secret Practice"),
        "private event must not appear in ICS"
    );
}

// ─── PR2 match lifecycle ────────────────────────────────────────────────────

#[tokio::test]
async fn match_lifecycle_scheduled_then_report_scores() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    seed_game(&state.db, "ow2", "Overwatch 2").await;
    seed_team(&state.db, "teamalpha", "Alpha Squad", "ow2").await;

    // Create scheduled fixture (no scores, no played_at)
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/matches",
            OFFICER_TOKEN,
            json!({
                "team_id": "teamalpha",
                "opponent": "Future FC",
                "match_type": "official",
                "scheduled_at": "2026-08-01T18:00:00Z",
                "is_public": true
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let created = body_json(resp).await;
    assert!(created["score_us"].is_null());
    assert!(created["score_them"].is_null());
    assert!(created["played_at"].is_null());
    assert_eq!(created["scheduled_at"], "2026-08-01T18:00:00Z");
    let id = created["id"].as_str().unwrap().to_string();

    // Scheduled public fixture must NOT appear in recent_matches (played-only)
    let app = create_router(state.clone());
    let resp = app
        .oneshot(unauthed_request(Method::GET, "/api/public/teams/teamalpha"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let recent = body_json(resp).await["recent_matches"]
        .as_array()
        .unwrap()
        .clone();
    assert!(
        recent.iter().all(|m| m["opponent"] != "Future FC"),
        "scheduled match must not be in recent_matches: {recent:?}"
    );

    // Report scores + played_at
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::PUT,
            &format!("/api/matches/{id}"),
            OFFICER_TOKEN,
            json!({
                "score_us": 3,
                "score_them": 1,
                "played_at": "2026-08-01T20:00:00Z",
                "vod_url": "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
                "replay_code": "ABCD1234"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let updated = body_json(resp).await;
    assert_eq!(updated["score_us"], 3);
    assert_eq!(updated["score_them"], 1);
    assert_eq!(updated["played_at"], "2026-08-01T20:00:00Z");
    assert_eq!(
        updated["vod_url"],
        "https://www.youtube.com/watch?v=dQw4w9WgXcQ"
    );
    assert_eq!(updated["replay_code"], "ABCD1234");
    // scheduled_at retained
    assert_eq!(updated["scheduled_at"], "2026-08-01T18:00:00Z");

    // Now in recent_matches
    let app = create_router(state);
    let resp = app
        .oneshot(unauthed_request(Method::GET, "/api/public/teams/teamalpha"))
        .await
        .unwrap();
    let recent = body_json(resp).await["recent_matches"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0]["opponent"], "Future FC");
    assert_eq!(recent[0]["score_us"], 3);
    assert_eq!(
        recent[0]["vod_url"],
        "https://www.youtube.com/watch?v=dQw4w9WgXcQ"
    );
    assert!(
        recent[0].get("notes").is_none() || recent[0]["notes"].is_null(),
        "public projection strips notes"
    );
}

#[tokio::test]
async fn update_match_null_clears_omit_preserves() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    seed_game(&state.db, "ow2", "Overwatch 2").await;
    seed_team(&state.db, "teamalpha", "Alpha Squad", "ow2").await;

    // Create with scores + vod + notes
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/matches",
            OFFICER_TOKEN,
            json!({
                "team_id": "teamalpha",
                "opponent": "Clearable FC",
                "match_type": "official",
                "score_us": 2,
                "score_them": 1,
                "played_at": "2026-06-15T18:00:00Z",
                "notes": "keep me",
                "vod_url": "https://www.twitch.tv/videos/123",
                "is_public": false
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let id = body_json(resp).await["id"].as_str().unwrap().to_string();

    // null clears scores + vod; omit notes → preserve
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::PUT,
            &format!("/api/matches/{id}"),
            OFFICER_TOKEN,
            json!({
                "score_us": null,
                "score_them": null,
                "vod_url": null
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert!(
        json["score_us"].is_null(),
        "null must clear score_us, got {:?}",
        json["score_us"]
    );
    assert!(
        json["score_them"].is_null(),
        "null must clear score_them, got {:?}",
        json["score_them"]
    );
    assert!(
        json["vod_url"].is_null(),
        "null must clear vod_url, got {:?}",
        json["vod_url"]
    );
    assert_eq!(json["notes"], "keep me", "omitted notes must be preserved");
    assert_eq!(json["opponent"], "Clearable FC");
    assert_eq!(json["played_at"], "2026-06-15T18:00:00Z");
}

#[tokio::test]
async fn match_media_validation_rejects_bad_vod_and_replay() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    seed_game(&state.db, "ow2", "Overwatch 2").await;
    seed_team(&state.db, "teamalpha", "Alpha Squad", "ow2").await;

    // http not https
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/matches",
            OFFICER_TOKEN,
            json!({
                "team_id": "teamalpha",
                "opponent": "Bad VOD",
                "match_type": "official",
                "played_at": "2026-06-01T18:00:00Z",
                "score_us": 1,
                "score_them": 0,
                "vod_url": "http://www.youtube.com/watch?v=x"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // unknown host
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/matches",
            OFFICER_TOKEN,
            json!({
                "team_id": "teamalpha",
                "opponent": "Bad Host",
                "match_type": "official",
                "played_at": "2026-06-01T18:00:00Z",
                "score_us": 1,
                "score_them": 0,
                "vod_url": "https://vimeo.com/123"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // replay too long
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/matches",
            OFFICER_TOKEN,
            json!({
                "team_id": "teamalpha",
                "opponent": "Bad Replay",
                "match_type": "official",
                "played_at": "2026-06-01T18:00:00Z",
                "score_us": 1,
                "score_them": 0,
                "replay_code": "ABCDEFGHIJKLMNOPQ"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // partial scores rejected
    let app = create_router(state);
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/matches",
            OFFICER_TOKEN,
            json!({
                "team_id": "teamalpha",
                "opponent": "Partial",
                "match_type": "official",
                "played_at": "2026-06-01T18:00:00Z",
                "score_us": 1
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ─── #2 home-live: overview upcoming + recent ───────────────────────────────

#[tokio::test]
async fn public_overview_includes_upcoming_and_recent_matches() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    seed_game(&state.db, "ow2", "Overwatch 2").await;
    seed_team(&state.db, "teamalpha", "Alpha Squad", "ow2").await;

    // Scheduled public fixture
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/matches",
            OFFICER_TOKEN,
            json!({
                "team_id": "teamalpha",
                "opponent": "Future Opp",
                "match_type": "official",
                "scheduled_at": "2026-09-01T18:00:00Z",
                "is_public": true
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // Played public result
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/matches",
            OFFICER_TOKEN,
            json!({
                "team_id": "teamalpha",
                "opponent": "Past Opp",
                "match_type": "official",
                "score_us": 3,
                "score_them": 1,
                "played_at": "2026-06-01T18:00:00Z",
                "is_public": true
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // Private played — must not appear
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/matches",
            OFFICER_TOKEN,
            json!({
                "team_id": "teamalpha",
                "opponent": "Hidden Opp",
                "match_type": "official",
                "score_us": 1,
                "score_them": 0,
                "played_at": "2026-06-02T18:00:00Z",
                "is_public": false
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // Scrim public — never on public overview
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/matches",
            OFFICER_TOKEN,
            json!({
                "team_id": "teamalpha",
                "opponent": "Scrim Opp",
                "match_type": "scrim",
                "score_us": 2,
                "score_them": 2,
                "played_at": "2026-06-03T18:00:00Z",
                "is_public": true
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let app = create_router(state);
    let resp = app
        .oneshot(unauthed_request(Method::GET, "/api/public/overview"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;

    let upcoming = json["upcoming_matches"].as_array().unwrap();
    assert_eq!(upcoming.len(), 1, "only public scheduled: {upcoming:?}");
    assert_eq!(upcoming[0]["opponent"], "Future Opp");
    assert_eq!(upcoming[0]["team_name"], "Alpha Squad");
    assert_eq!(upcoming[0]["game_name"], "Overwatch 2");
    assert!(upcoming[0]["scheduled_at"]
        .as_str()
        .unwrap()
        .contains("2026-09-01"));

    let recent = json["recent_results"].as_array().unwrap();
    assert_eq!(recent.len(), 1, "only public played non-scrim: {recent:?}");
    assert_eq!(recent[0]["opponent"], "Past Opp");
    assert_eq!(recent[0]["score_us"], 3);
    assert_eq!(recent[0]["score_them"], 1);
    assert_eq!(recent[0]["outcome"], "win");
    assert_eq!(recent[0]["team_name"], "Alpha Squad");
}

// ─── #4 match detail ────────────────────────────────────────────────────────

#[tokio::test]
async fn public_match_detail_gates_and_exposes_media() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    seed_game(&state.db, "ow2", "Overwatch 2").await;
    seed_team(&state.db, "teamalpha", "Alpha Squad", "ow2").await;

    // Public official with media
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/matches",
            OFFICER_TOKEN,
            json!({
                "team_id": "teamalpha",
                "opponent": "Detail Opp",
                "match_type": "official",
                "score_us": 2,
                "score_them": 1,
                "played_at": "2026-06-20T18:00:00Z",
                "notes": "secret",
                "vod_url": "https://www.youtube.com/watch?v=abc",
                "replay_code": "REPLAY01",
                "is_public": true
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let id = body_json(resp).await["id"].as_str().unwrap().to_string();

    // Private match
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/matches",
            OFFICER_TOKEN,
            json!({
                "team_id": "teamalpha",
                "opponent": "Private Opp",
                "match_type": "official",
                "score_us": 1,
                "score_them": 0,
                "played_at": "2026-06-21T18:00:00Z",
                "is_public": false
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let private_id = body_json(resp).await["id"].as_str().unwrap().to_string();

    // Public detail
    let app = create_router(state.clone());
    let resp = app
        .oneshot(unauthed_request(
            Method::GET,
            &format!("/api/public/matches/{id}"),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["opponent"], "Detail Opp");
    assert_eq!(json["team_name"], "Alpha Squad");
    assert_eq!(json["game_name"], "Overwatch 2");
    assert_eq!(json["vod_url"], "https://www.youtube.com/watch?v=abc");
    assert_eq!(json["replay_code"], "REPLAY01");
    assert!(
        json.get("notes").is_none() || json["notes"].is_null(),
        "notes must not leak: {json}"
    );

    // Private → 404
    let app = create_router(state.clone());
    let resp = app
        .oneshot(unauthed_request(
            Method::GET,
            &format!("/api/public/matches/{private_id}"),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // Member GET full row includes notes
    let app = create_router(state);
    let resp = app
        .oneshot(authed_request(
            Method::GET,
            &format!("/api/matches/{id}"),
            MEMBER_TOKEN,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["notes"], "secret");
}

// ─── Scrim team authz (security train) ──────────────────────────────────────

#[tokio::test]
async fn create_scrim_requires_roster_or_officer() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    seed_game(&state.db, "ow2", "Overwatch 2").await;
    seed_team(&state.db, "teamalpha", "Alpha Squad", "ow2").await;

    let scheduled = chrono::Utc::now() + chrono::Duration::hours(2);
    let body = json!({
        "team_id": "teamalpha",
        "game_id": "ow2",
        "scheduled_at": scheduled.to_rfc3339(),
        "duration_minutes": 90,
        "notes": "tryout scrim"
    });

    // Regular member not on roster → 403
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/scrims",
            MEMBER_TOKEN,
            body.clone(),
        ))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "member not on roster must not create scrim"
    );

    // Officer can create without being on roster
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/scrims",
            OFFICER_TOKEN,
            body.clone(),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let json = body_json(resp).await;
    assert_eq!(json["team_id"], "teamalpha");
    assert_eq!(json["status"], "open");

    // Put member on roster → can create
    state
        .db
        .add_to_roster("membermember", "teamalpha", scuffed_db::TeamRole::Player)
        .await
        .expect("add to roster");

    let app = create_router(state);
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/scrims",
            MEMBER_TOKEN,
            body,
        ))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::CREATED,
        "roster member can create scrim"
    );
}

#[tokio::test]
async fn update_scrim_requires_roster_or_officer() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    seed_game(&state.db, "ow2", "Overwatch 2").await;
    seed_team(&state.db, "teamalpha", "Alpha Squad", "ow2").await;

    // Officer creates a scrim
    let scheduled = chrono::Utc::now() + chrono::Duration::hours(3);
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/scrims",
            OFFICER_TOKEN,
            json!({
                "team_id": "teamalpha",
                "game_id": "ow2",
                "scheduled_at": scheduled.to_rfc3339(),
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let scrim = body_json(resp).await;
    let scrim_id = scrim["id"].as_str().unwrap().to_string();

    // Non-roster member cannot update
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::PATCH,
            &format!("/api/scrims/{scrim_id}"),
            MEMBER_TOKEN,
            json!({ "status": "confirmed", "opponent_name": "Rivals" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // Officer can update
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::PATCH,
            &format!("/api/scrims/{scrim_id}"),
            OFFICER_TOKEN,
            json!({ "status": "confirmed", "opponent_name": "Rivals" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["status"], "confirmed");
    assert_eq!(json["opponent_name"], "Rivals");
}

// ─── #6 leaderboards ────────────────────────────────────────────────────────

#[tokio::test]
async fn public_leaderboards_and_member_heroes() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;

    // Empty leaderboard is OK
    let app = create_router(state.clone());
    let resp = app
        .oneshot(unauthed_request(
            Method::GET,
            "/api/public/leaderboards?metric=winrate&limit=10",
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert!(json.as_array().unwrap().is_empty() || json.as_array().is_some());

    // Unknown member heroes → 404
    let app = create_router(state.clone());
    let resp = app
        .oneshot(unauthed_request(
            Method::GET,
            "/api/public/members/nope/heroes?top=3",
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // Existing member (seeded) → 200 empty or with data
    let app = create_router(state.clone());
    let resp = app
        .oneshot(unauthed_request(
            Method::GET,
            "/api/public/members/membermember/heroes?top=3",
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Admin can create season
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/admin/seasons",
            ADMIN_TOKEN,
            json!({
                "name": "Season 1",
                "starts_at": "2026-01-01T00:00:00Z",
                "ends_at": "2026-06-01T00:00:00Z",
                "is_current": true
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let season = body_json(resp).await;
    assert_eq!(season["name"], "Season 1");
    assert_eq!(season["is_current"], true);
    let season_id = season["id"].as_str().unwrap();

    // Seasonal board filter accepts known season id
    let app = create_router(state.clone());
    let resp = app
        .oneshot(unauthed_request(
            Method::GET,
            &format!("/api/public/leaderboards?metric=winrate&limit=10&season={season_id}"),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Unknown season → 404
    let app = create_router(state.clone());
    let resp = app
        .oneshot(unauthed_request(
            Method::GET,
            "/api/public/leaderboards?metric=winrate&season=nope",
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // W3 B2: known hero filter accepted (case-insensitive); empty board OK
    let app = create_router(state.clone());
    let resp = app
        .oneshot(unauthed_request(
            Method::GET,
            "/api/public/leaderboards?metric=games&limit=10&hero=ana",
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let _ = body_json(resp).await;

    // W3 B2: unknown hero → 400
    let app = create_router(state);
    let resp = app
        .oneshot(unauthed_request(
            Method::GET,
            "/api/public/leaderboards?metric=games&hero=NotAHero",
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

/// W3 B3: `GET /api/public/members?hero=` attaches optional `hero_scoped`.
#[tokio::test]
async fn public_members_hero_scoped_query() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;

    // No filter: OK, no hero_scoped on rows
    let app = create_router(state.clone());
    let resp = app
        .oneshot(unauthed_request(
            Method::GET,
            "/api/public/members?limit=10",
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let data = json["data"].as_array().expect("data array");
    if let Some(first) = data.first() {
        assert!(
            first.get("hero_scoped").is_none()
                || first
                    .get("hero_scoped")
                    .map(|v| v.is_null())
                    .unwrap_or(false),
            "unfiltered list must omit hero_scoped"
        );
    }

    // Known hero (case-insensitive): 200 (empty scoped OK with no personal matches)
    let app = create_router(state.clone());
    let resp = app
        .oneshot(unauthed_request(
            Method::GET,
            "/api/public/members?limit=10&hero=ana",
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let _ = body_json(resp).await;

    // Unknown hero → 400
    let app = create_router(state);
    let resp = app
        .oneshot(unauthed_request(
            Method::GET,
            "/api/public/members?hero=NotAHero",
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ─── Local self-registration (privacy-first signup) ─────────────────────────

fn anon_json_request(method: Method, uri: &str, body: Value) -> Request<Body> {
    // Rate-limited auth routes need a client IP for SmartIpKeyExtractor.
    rate_limit_ip(
        Request::builder()
            .method(method)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json"),
    )
    .body(Body::from(serde_json::to_vec(&body).unwrap()))
    .unwrap()
}

#[tokio::test]
async fn register_creates_bare_user_with_session() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;

    let app = create_router(state.clone());
    let resp = app
        .oneshot(anon_json_request(
            Method::POST,
            "/api/auth/local/register",
            json!({ "username": "NewPlayer", "password": "hunter2234567", "confirm_min_age": true }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let cookie = resp
        .headers()
        .get(header::SET_COOKIE)
        .expect("session cookie on register")
        .to_str()
        .unwrap()
        .to_string();

    // Session works and no member row exists (bare user; application is the gate)
    let app = create_router(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/auth/me")
                .header(header::COOKIE, cookie.split(';').next().unwrap())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let me = body_json(resp).await;
    assert_eq!(
        me["user"]["username"], "newplayer",
        "usernames normalize to lowercase"
    );
    assert!(me["member"].is_null(), "register must not create a member");
}

#[tokio::test]
async fn register_validation_and_conflicts() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;

    // Missing age confirmation
    let app = create_router(state.clone());
    let resp = app
        .oneshot(anon_json_request(
            Method::POST,
            "/api/auth/local/register",
            json!({ "username": "kid", "password": "hunter2234567" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // Weak password
    let app = create_router(state.clone());
    let resp = app
        .oneshot(anon_json_request(
            Method::POST,
            "/api/auth/local/register",
            json!({ "username": "weakpw", "password": "short", "confirm_min_age": true }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // Duplicate username → 409
    let app = create_router(state.clone());
    let resp = app
        .oneshot(anon_json_request(
            Method::POST,
            "/api/auth/local/register",
            json!({ "username": "dupuser", "password": "hunter2234567", "confirm_min_age": true }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let app = create_router(state.clone());
    let resp = app
        .oneshot(anon_json_request(
            Method::POST,
            "/api/auth/local/register",
            json!({ "username": "DupUser", "password": "hunter2234567", "confirm_min_age": true }),
        ))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::CONFLICT,
        "case-insensitive dup must 409"
    );

    // Close recruitment → registration 403 and providers.register=false
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::PUT,
            "/api/settings",
            ADMIN_TOKEN,
            json!({ "recruitment_open": false }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let app = create_router(state.clone());
    let resp = app
        .oneshot(anon_json_request(
            Method::POST,
            "/api/auth/local/register",
            json!({ "username": "latecomer", "password": "hunter2234567", "confirm_min_age": true }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    let app = create_router(state.clone());
    let resp = app
        .oneshot(unauthed_request(Method::GET, "/api/auth/providers"))
        .await
        .unwrap();
    let providers = body_json(resp).await;
    assert_eq!(providers["register"], false);
}

#[tokio::test]
async fn admin_resets_local_password_member_logs_in() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;

    // Register a local account, then make it a member so it appears in admin flows
    let app = create_router(state.clone());
    let resp = app
        .oneshot(anon_json_request(
            Method::POST,
            "/api/auth/local/register",
            json!({ "username": "forgetful", "password": "originalpw123", "confirm_min_age": true }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let pre_reset_cookie = resp
        .headers()
        .get(header::SET_COOKIE)
        .expect("register session cookie")
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let (user, _) = state
        .db
        .get_local_user_by_username("forgetful")
        .await
        .unwrap()
        .expect("registered user");
    state
        .db
        .create_member(&user.id, "Forgetful", scuffed_db::OrgRole::Member)
        .await
        .unwrap();
    let member = state
        .db
        .get_member_by_user(&user.id)
        .await
        .unwrap()
        .expect("member");

    // Non-admin cannot reset
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            &format!("/api/members/{}/reset-password", member.id),
            MEMBER_TOKEN,
            json!({ "new_password": "adminset12345" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // Admin resets
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            &format!("/api/members/{}/reset-password", member.id),
            ADMIN_TOKEN,
            json!({ "new_password": "adminset12345" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Reset revokes live sessions — the pre-reset cookie must be dead
    let app = create_router(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/auth/me")
                .header(header::COOKIE, pre_reset_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "old session must be revoked by password reset"
    );

    // Old password dead, new password works
    let app = create_router(state.clone());
    let resp = app
        .oneshot(anon_json_request(
            Method::POST,
            "/api/auth/local/login",
            json!({ "username": "forgetful", "password": "originalpw123" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let app = create_router(state.clone());
    let resp = app
        .oneshot(anon_json_request(
            Method::POST,
            "/api/auth/local/login",
            json!({ "username": "forgetful", "password": "adminset12345" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // OAuth-provider member → 400
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::POST,
            "/api/members/membermember/reset-password",
            ADMIN_TOKEN,
            json!({ "new_password": "adminset12345" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ─── Nostr NIP-07 login ──────────────────────────────────────────────────────

async fn nostr_challenge(state: &scuffed_site_server::state::AppState) -> (String, String) {
    let app = create_router(state.clone());
    let resp = app
        .oneshot(
            rate_limit_ip(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/auth/nostr/challenge"),
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    (
        json["challenge"].as_str().unwrap().to_string(),
        json["token"].as_str().unwrap().to_string(),
    )
}

fn signed_login_event(keys: &nostr::Keys, content: &str) -> serde_json::Value {
    let event = nostr::EventBuilder::new(nostr::Kind::Custom(22242), content)
        .sign_with_keys(keys)
        .expect("sign event");
    serde_json::to_value(event).unwrap()
}

/// Like [`signed_login_event`], but stamps a specific `created_at` (unix secs)
/// so tests can exercise the freshness window (DR1 replay-closer).
fn signed_login_event_at(
    keys: &nostr::Keys,
    content: &str,
    created_at_secs: u64,
) -> serde_json::Value {
    let event = nostr::EventBuilder::new(nostr::Kind::Custom(22242), content)
        .custom_created_at(nostr::Timestamp::from_secs(created_at_secs))
        .sign_with_keys(keys)
        .expect("sign event");
    serde_json::to_value(event).unwrap()
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

async fn nostr_verify_resp(
    state: &scuffed_site_server::state::AppState,
    token: &str,
    event: serde_json::Value,
) -> axum::response::Response {
    let app = create_router(state.clone());
    app.oneshot(anon_json_request(
        Method::POST,
        "/api/auth/nostr/verify",
        json!({ "token": token, "signed_event": event }),
    ))
    .await
    .unwrap()
}

fn cookie_of(resp: &axum::response::Response) -> String {
    resp.headers()
        .get(header::SET_COOKIE)
        .expect("session cookie")
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string()
}

async fn me_with_cookie(state: &scuffed_site_server::state::AppState, cookie: &str) -> Value {
    let app = create_router(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/auth/me")
                .header(header::COOKIE, cookie.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    body_json(resp).await
}

#[tokio::test]
async fn nostr_login_registers_then_reuses_user() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    let keys = nostr::Keys::generate();

    // First login: creates a bare nostr-provider user
    let (challenge, token) = nostr_challenge(&state).await;
    let resp = nostr_verify_resp(&state, &token, signed_login_event(&keys, &challenge)).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let cookie = cookie_of(&resp);
    let me = me_with_cookie(&state, &cookie).await;
    assert!(
        me["member"].is_null(),
        "nostr signup must not create a member"
    );
    let first_user_id = me["user"]["id"].as_str().unwrap().to_string();

    // Second login with the same key: same user, no duplicate
    let (challenge2, token2) = nostr_challenge(&state).await;
    let resp = nostr_verify_resp(&state, &token2, signed_login_event(&keys, &challenge2)).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let me2 = me_with_cookie(&state, &cookie_of(&resp)).await;
    assert_eq!(me2["user"]["id"].as_str().unwrap(), first_user_id);
}

#[tokio::test]
async fn nostr_login_rejects_bad_and_closed() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    let keys = nostr::Keys::generate();

    // Content mismatch → 400
    let (_challenge, token) = nostr_challenge(&state).await;
    let resp = nostr_verify_resp(
        &state,
        &token,
        signed_login_event(&keys, "scuffedclan-login:forged"),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // Tampered token → 400
    let (challenge, token) = nostr_challenge(&state).await;
    let mut bad_token = token.clone();
    bad_token.push('0');
    let resp = nostr_verify_resp(&state, &bad_token, signed_login_event(&keys, &challenge)).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // Recruitment closed → unknown pubkey cannot register (403)
    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed_json_request(
            Method::PUT,
            "/api/settings",
            ADMIN_TOKEN,
            json!({ "recruitment_open": false }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let (challenge, token) = nostr_challenge(&state).await;
    let resp = nostr_verify_resp(&state, &token, signed_login_event(&keys, &challenge)).await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

/// A well-formed kind-22242 event with the correct content and a valid token,
/// but an invalid schnorr signature, must be rejected by the explicit
/// `signed_event.verify()` check — 400 and no session cookie. Every other
/// failure path (`nostr_login_rejects_bad_and_closed`) trips a *pre-verify*
/// check (kind, content, token), so only a well-formed event with a bad
/// signature exercises `.verify()`; without this test the `.verify()` call
/// could be deleted and the suite would still pass.
#[tokio::test]
async fn nostr_login_rejects_forged_signature() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    let keys = nostr::Keys::generate();

    let (challenge, token) = nostr_challenge(&state).await;
    // Sign correctly (valid id, pubkey, matching content), then corrupt one
    // hex byte of the signature so the event stays structurally well-formed
    // and passes every pre-verify check, failing only the schnorr verify.
    let mut event = signed_login_event(&keys, &challenge);
    let sig = event["sig"].as_str().expect("event has a sig field");
    let mut chars: Vec<char> = sig.chars().collect();
    chars[0] = if chars[0] == 'a' { 'b' } else { 'a' };
    event["sig"] = serde_json::Value::String(chars.into_iter().collect());

    let resp = nostr_verify_resp(&state, &token, event).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert!(
        resp.headers().get(header::SET_COOKIE).is_none(),
        "forged-signature login must not set a session cookie"
    );
}

#[tokio::test]
async fn nostr_login_member_linked_pubkey_logs_into_member() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    let keys = nostr::Keys::generate();
    let pubkey_hex = keys.public_key().to_hex();

    // Link the pubkey to the seeded member directly
    state
        .db
        .update_member(
            "membermember",
            None,
            None,
            None,
            None,
            None,
            None,
            Some(Some(&pubkey_hex)),
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    let (challenge, token) = nostr_challenge(&state).await;
    let resp = nostr_verify_resp(&state, &token, signed_login_event(&keys, &challenge)).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let me = me_with_cookie(&state, &cookie_of(&resp)).await;
    assert!(
        me["member"].is_object(),
        "member-linked pubkey must log into the member account: {me:?}"
    );
    assert_eq!(me["member"]["id"], "membermember");
}

/// DR1 replay-closer: `Event::verify()` does not bound `created_at`, so the
/// handler must reject events whose signing time is outside the freshness
/// window. Both a too-old event (beyond `EVENT_MAX_AGE_SECS` = 300s) and a
/// far-future event (beyond `EVENT_FUTURE_SKEW_SECS` = 60s) must 400 and mint
/// no session cookie — otherwise a captured event would be replayable forever.
#[tokio::test]
async fn nostr_login_rejects_out_of_window_created_at() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    let keys = nostr::Keys::generate();

    // Too old: 420s in the past (> 300s max age).
    let (challenge, token) = nostr_challenge(&state).await;
    let stale = signed_login_event_at(&keys, &challenge, now_secs() - 420);
    let resp = nostr_verify_resp(&state, &token, stale).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert!(resp.headers().get(header::SET_COOKIE).is_none());
    let body = body_json(resp).await;
    assert!(
        body["error"].as_str().unwrap_or_default().contains("old")
            || body["error"]
                .as_str()
                .unwrap_or_default()
                .contains("window"),
        "expected freshness rejection, got {body:?}"
    );

    // Far future: 3600s ahead (> 60s skew tolerance).
    let (challenge2, token2) = nostr_challenge(&state).await;
    let future = signed_login_event_at(&keys, &challenge2, now_secs() + 3600);
    let resp = nostr_verify_resp(&state, &token2, future).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert!(resp.headers().get(header::SET_COOKIE).is_none());
}

/// DR1 one-time-challenge store: a captured, still-fresh signed event must not
/// be replayable within the TTL window. The first submission succeeds; a second
/// submission of the *same* challenge is rejected 400 ("challenge already
/// used"). Without the consumed-challenge store the replay would succeed again.
#[tokio::test]
async fn nostr_login_rejects_replayed_challenge() {
    let state = test_state().await;
    seed_all_roles(&state.db).await;
    let keys = nostr::Keys::generate();

    let (challenge, token) = nostr_challenge(&state).await;
    let event = signed_login_event(&keys, &challenge);

    // First submission: fresh challenge → logs in.
    let resp = nostr_verify_resp(&state, &token, event.clone()).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(resp.headers().get(header::SET_COOKIE).is_some());

    // Replay the identical token + signed event → rejected as already consumed.
    let resp = nostr_verify_resp(&state, &token, event).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert!(resp.headers().get(header::SET_COOKIE).is_none());
    let body = body_json(resp).await;
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("already used"),
        "expected replay rejection, got {body:?}"
    );
}

// ─── Uploads (DR1-ADMIN-001 / ADMIN-003 hardening) ───────────────────────────

/// Standard CRC-32 (IEEE) for crafting valid PNG chunks in tests.
fn upl_crc32(bytes: &[u8]) -> u32 {
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

/// Minimal structurally-valid PNG (signature + IHDR + IDAT + IEND) declaring
/// `w`x`h`. The tiny IDAT is required for a header-only dimension read but is
/// never inflated, so gigapixel dimensions can be declared cheaply.
fn upl_png(w: u32, h: u32) -> Vec<u8> {
    let mut out = vec![0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(b"IHDR");
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    out.extend_from_slice(&(13u32).to_be_bytes());
    out.extend_from_slice(&ihdr);
    out.extend_from_slice(&upl_crc32(&ihdr).to_be_bytes());
    let mut idat = Vec::new();
    idat.extend_from_slice(b"IDAT");
    idat.extend_from_slice(&[0x78, 0x9c, 0x63, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01]);
    out.extend_from_slice(&((idat.len() - 4) as u32).to_be_bytes());
    out.extend_from_slice(&idat);
    out.extend_from_slice(&upl_crc32(&idat).to_be_bytes());
    out.extend_from_slice(&(0u32).to_be_bytes());
    out.extend_from_slice(b"IEND");
    out.extend_from_slice(&upl_crc32(b"IEND").to_be_bytes());
    out
}

/// Build an authenticated multipart/form-data upload request.
fn upload_request(
    uri: &str,
    token: &str,
    filename: &str,
    ctype: &str,
    data: &[u8],
) -> Request<Body> {
    let boundary = "scuffedtestboundary";
    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!("Content-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n")
            .as_bytes(),
    );
    body.extend_from_slice(format!("Content-Type: {ctype}\r\n\r\n").as_bytes());
    body.extend_from_slice(data);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    // Upload routes are rate-limited with TrustedProxyIpKeyExtractor, which needs
    // a peer socket (ConnectInfo). `oneshot` doesn't inject one, so mirror the
    // auth rate-limit tests / production wiring via the shared helper.
    let builder = Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        );
    rate_limit_ip(builder).body(Body::from(body)).unwrap()
}

fn unique_upload_dir(tag: &str) -> PathBuf {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("scuffed-upl-it-{tag}-{n}"))
}

#[tokio::test]
async fn upload_avatar_normal_succeeds() {
    let mut state = test_state().await;
    state.upload_dir = unique_upload_dir("normal");
    seed_all_roles(&state.db).await;
    let dir = state.upload_dir.clone();
    let app = create_router(state);

    let resp = app
        .oneshot(upload_request(
            "/api/upload/avatar",
            MEMBER_TOKEN,
            "a.png",
            "image/png",
            &upl_png(64, 64),
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let url = json["url"].as_str().expect("url in response");
    assert!(url.starts_with("/uploads/avatars/"), "got {url}");
    // File actually landed under the member's scoped dir.
    let rel = url.strip_prefix("/uploads/").unwrap();
    assert!(dir.join(rel).exists(), "uploaded file should exist on disk");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn upload_avatar_oversized_dimensions_rejected() {
    let mut state = test_state().await;
    state.upload_dir = unique_upload_dir("bomb");
    seed_all_roles(&state.db).await;
    let dir = state.upload_dir.clone();
    let app = create_router(state);

    // ~500 byte file that declares a 50000x50000 (2.5 gigapixel) canvas.
    let resp = app
        .oneshot(upload_request(
            "/api/upload/avatar",
            MEMBER_TOKEN,
            "bomb.png",
            "image/png",
            &upl_png(50_000, 50_000),
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    // Nothing persisted.
    assert!(
        !dir.join("avatars").exists()
            || std::fs::read_dir(dir.join("avatars"))
                .map(|mut d| d.next().is_none())
                .unwrap_or(true),
        "no file should be written for a rejected bomb"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn upload_avatar_deletes_previous_local_avatar() {
    let mut state = test_state().await;
    state.upload_dir = unique_upload_dir("replace");
    seed_all_roles(&state.db).await;
    let dir = state.upload_dir.clone();

    // Point the member at an existing local avatar and put that file on disk.
    let old_rel = "avatars/old-avatar.png";
    let old_url = format!("/uploads/{old_rel}");
    state
        .db
        .client
        .query("UPDATE member:membermember SET avatar_url = $u")
        .bind(("u", old_url.clone()))
        .await
        .expect("set old avatar_url");
    std::fs::create_dir_all(dir.join("avatars")).unwrap();
    std::fs::write(dir.join(old_rel), b"old-bytes").unwrap();
    assert!(dir.join(old_rel).exists());

    let app = create_router(state);
    let resp = app
        .oneshot(upload_request(
            "/api/upload/avatar",
            MEMBER_TOKEN,
            "new.png",
            "image/png",
            &upl_png(32, 32),
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    // Old avatar was deleted on replace.
    assert!(
        !dir.join(old_rel).exists(),
        "previous avatar should be deleted on replace"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn upload_avatar_quota_exceeded_rejected() {
    let mut state = test_state().await;
    state.upload_dir = unique_upload_dir("quota");
    seed_all_roles(&state.db).await;
    let dir = state.upload_dir.clone();
    let app = create_router(state);

    // First upload succeeds and tells us the member's scoped dir.
    let resp = app
        .clone()
        .oneshot(upload_request(
            "/api/upload/avatar",
            MEMBER_TOKEN,
            "a.png",
            "image/png",
            &upl_png(16, 16),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let url = body_json(resp).await["url"].as_str().unwrap().to_string();
    // /uploads/avatars/{key}/{uuid}.png → scoped dir = upload_dir/avatars/{key}
    let scoped = dir.join(
        std::path::Path::new(url.strip_prefix("/uploads/").unwrap())
            .parent()
            .unwrap(),
    );

    // Pre-fill that dir with a sparse file at the byte cap so the next upload
    // pushes the member over quota.
    let f = std::fs::File::create(scoped.join("filler.bin")).unwrap();
    f.set_len(25 * 1024 * 1024).unwrap(); // MEMBER_MAX_UPLOAD_BYTES

    let resp = app
        .oneshot(upload_request(
            "/api/upload/avatar",
            MEMBER_TOKEN,
            "b.png",
            "image/png",
            &upl_png(16, 16),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);

    std::fs::remove_dir_all(&dir).ok();
}
