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
use scuffed_site_server::state::{AppState, OAuthConfig};
use scuffed_site_server::create_router;

// ─── Test Harness ───────────────────────────────────────────────────────────

/// Create an AppState backed by an in-memory SurrealDB with migrations applied.
async fn test_state() -> AppState {
    let db = Database::connect_memory()
        .await
        .expect("in-memory DB connect");

    run_migrations(&db.client)
        .await
        .expect("migrations");

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
        crypto: None,
        relay_url: None,
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
    seed_user(db, "adminuser", "adminmember", "TestAdmin", "admin", ADMIN_TOKEN).await;
    seed_user(db, "officeruser", "officermember", "TestOfficer", "officer", OFFICER_TOKEN).await;
    seed_user(db, "memberuser", "membermember", "TestMember", "member", MEMBER_TOKEN).await;
    seed_user(db, "recruituser", "recruitmember", "TestRecruit", "recruit", RECRUIT_TOKEN).await;
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
