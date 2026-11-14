//! Officer tournament routes answer 404 on an unknown tournament,
//! participant, or match id, and write nothing. Deferred follow-up from
//! PR 675, which covers the public GET routes.

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
use scuffed_site_server::create_router;
use scuffed_site_server::state::{AppState, OAuthConfig};

const OFFICER_TOKEN: &str = "tourn-officer-token";
const MISSING: &str = "doesnotexist";

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
        auth_redirect_base: None,
        upload_dir: PathBuf::from("/tmp/scuffed-test-uploads"),
        reports_dir: PathBuf::from("/tmp/scuffed-test-reports"),
        reports_enabled: true,
        packs_dir: None,
        hero_icons: scuffed_site_server::hero_icons::HeroIconStore::in_temp_dir(),
        notifier: None,
        nostr_challenge_key: *blake3::hash(b"test-nostr-challenge-key").as_bytes(),
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

fn authed(method: Method, uri: &str, body: Option<Value>) -> Request<Body> {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {OFFICER_TOKEN}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-forwarded-for", "127.0.0.1")
        .extension(axum::extract::ConnectInfo(std::net::SocketAddr::from((
            [127, 0, 0, 1],
            40000,
        ))));
    match body {
        Some(v) => req
            .body(Body::from(serde_json::to_vec(&v).unwrap()))
            .unwrap(),
        None => req.body(Body::empty()).unwrap(),
    }
}

async fn call(
    state: &AppState,
    method: Method,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let resp = create_router(state.clone())
        .oneshot(authed(method, uri, body))
        .await
        .unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn seeded() -> AppState {
    let state = test_state().await;
    seed_user(
        &state.db,
        "officeruser",
        "officermember",
        "TournOfficer",
        "officer",
        OFFICER_TOKEN,
    )
    .await;
    state
}

async fn create_tournament(state: &AppState, name: &str, format: &str) -> String {
    let (status, body) = call(
        state,
        Method::POST,
        "/api/tournaments",
        Some(json!({ "name": name, "format": format })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["id"].as_str().unwrap().to_string()
}

async fn add_participant(state: &AppState, tid: &str, name: &str) -> String {
    let (status, body) = call(
        state,
        Method::POST,
        &format!("/api/tournaments/{tid}/participants"),
        Some(json!({ "external_name": name })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["id"].as_str().unwrap().to_string()
}

async fn count(state: &AppState, table: &str) -> usize {
    let mut r = state
        .db
        .client
        .query(format!("SELECT count() FROM {table} GROUP ALL"))
        .await
        .unwrap();
    let rows: Vec<Value> = r.take(0).unwrap();
    rows.first().and_then(|v| v["count"].as_u64()).unwrap_or(0) as usize
}

async fn snapshot(state: &AppState) -> [usize; 4] {
    [
        count(state, "tournament").await,
        count(state, "tournament_participant").await,
        count(state, "tournament_round").await,
        count(state, "tournament_match").await,
    ]
}

#[tokio::test]
async fn officer_routes_404_on_unknown_tournament_and_write_nothing() {
    let state = seeded().await;
    // A real tournament with a participant, so a stray write would show up.
    let tid = create_tournament(&state, "Real Cup", "swiss").await;
    let pid = add_participant(&state, &tid, "Team A").await;
    let before = snapshot(&state).await;

    let cases: Vec<(Method, String, Option<Value>)> = vec![
        (
            Method::PUT,
            format!("/api/tournaments/{MISSING}"),
            Some(json!({ "name": "Renamed" })),
        ),
        (
            Method::PATCH,
            format!("/api/tournaments/{MISSING}/status"),
            Some(json!({ "status": "registration" })),
        ),
        (
            Method::POST,
            format!("/api/tournaments/{MISSING}/generate-bracket"),
            None,
        ),
        (
            Method::POST,
            format!("/api/tournaments/{MISSING}/next-round"),
            None,
        ),
        (
            Method::POST,
            format!("/api/tournaments/{MISSING}/participants"),
            Some(json!({ "external_name": "Ghost" })),
        ),
        // Real participant id under a missing tournament.
        (
            Method::PUT,
            format!("/api/tournaments/{MISSING}/participants/{pid}"),
            Some(json!({ "seed": 1 })),
        ),
        (
            Method::DELETE,
            format!("/api/tournaments/{MISSING}/participants/{pid}"),
            None,
        ),
        (
            Method::PATCH,
            format!("/api/tournaments/{MISSING}/matches/{MISSING}/report"),
            Some(json!({ "score_a": 1, "score_b": 0, "winner_id": pid })),
        ),
    ];

    for (method, uri, body) in cases {
        let (status, resp) = call(&state, method.clone(), &uri, body).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method} {uri}: {resp}");
    }

    assert_eq!(snapshot(&state).await, before, "no rows written");
    // The real participant was not touched.
    let participants = state.db.list_tournament_participants(&tid).await.unwrap();
    assert_eq!(participants.len(), 1);
    assert_eq!(participants[0].seed, None);
}

#[tokio::test]
async fn next_round_on_missing_tournament_creates_no_round() {
    let state = seeded().await;
    let (status, body) = call(
        &state,
        Method::POST,
        &format!("/api/tournaments/{MISSING}/next-round"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(count(&state, "tournament_round").await, 0);
    assert!(
        state
            .db
            .list_tournament_rounds(MISSING)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn participant_routes_404_on_unknown_or_foreign_participant() {
    let state = seeded().await;
    let tid = create_tournament(&state, "Cup One", "single_elim").await;
    let other = create_tournament(&state, "Cup Two", "single_elim").await;
    let foreign = add_participant(&state, &other, "Other Team").await;
    let before = snapshot(&state).await;

    for pid in [MISSING, foreign.as_str()] {
        let uri = format!("/api/tournaments/{tid}/participants/{pid}");
        let (status, body) = call(&state, Method::PUT, &uri, Some(json!({ "seed": 3 }))).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "PUT {uri}: {body}");
        let (status, body) = call(&state, Method::DELETE, &uri, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "DELETE {uri}: {body}");
    }

    assert_eq!(snapshot(&state).await, before, "no rows written");
    let others = state.db.list_tournament_participants(&other).await.unwrap();
    assert_eq!(others.len(), 1, "foreign participant not deleted");
    assert_eq!(others[0].seed, None, "foreign participant not updated");
}

#[tokio::test]
async fn report_404_on_unknown_match_in_real_tournament() {
    let state = seeded().await;
    let tid = create_tournament(&state, "Cup", "single_elim").await;
    let pid = add_participant(&state, &tid, "Team A").await;
    let before = snapshot(&state).await;
    let (status, body) = call(
        &state,
        Method::PATCH,
        &format!("/api/tournaments/{tid}/matches/{MISSING}/report"),
        Some(json!({ "score_a": 1, "score_b": 0, "winner_id": pid })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(snapshot(&state).await, before);
}

#[tokio::test]
async fn existing_participant_update_and_delete_still_work() {
    let state = seeded().await;
    let tid = create_tournament(&state, "Cup", "single_elim").await;
    let pid = add_participant(&state, &tid, "Team A").await;
    let uri = format!("/api/tournaments/{tid}/participants/{pid}");
    let (status, body) = call(&state, Method::PUT, &uri, Some(json!({ "seed": 2 }))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["seed"], 2);
    let (status, _) = call(&state, Method::DELETE, &uri, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(
        state
            .db
            .list_tournament_participants(&tid)
            .await
            .unwrap()
            .is_empty()
    );
}

/// Dev-seed regression (site-server main.rs, "Failed to report RR match 1"):
/// the seed reported a B win as 2-1. The report path is right to refuse that
/// (DB-009, the winner must outscore the loser). A B win reported 1-2 on a
/// round-robin match goes through.
#[tokio::test]
async fn round_robin_b_win_needs_b_to_outscore_a() {
    let state = seeded().await;
    let tid = create_tournament(&state, "RR Cup", "round_robin").await;
    for name in ["Alpha", "Bravo", "Charlie", "Echo"] {
        add_participant(&state, &tid, name).await;
    }
    let (status, body) = call(
        &state,
        Method::POST,
        &format!("/api/tournaments/{tid}/generate-bracket"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let matches = state.db.list_tournament_matches(&tid).await.unwrap();
    let pending: Vec<_> = matches
        .iter()
        .filter(|m| m.participant_a_id.is_some() && m.participant_b_id.is_some())
        .collect();
    assert_eq!(pending.len(), 6, "4 teams play 6 round-robin matches");
    let m = pending[1];
    let b = m.participant_b_id.clone().unwrap();
    let uri = format!("/api/tournaments/{tid}/matches/{}/report", m.id);

    // The old seed data: B named winner with A's score higher. Still refused.
    let (status, body) = call(
        &state,
        Method::PATCH,
        &uri,
        Some(json!({ "score_a": 2, "score_b": 1, "winner_id": b })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");

    // The fixed seed data: B wins 1-2.
    let (status, body) = call(
        &state,
        Method::PATCH,
        &uri,
        Some(json!({ "score_a": 1, "score_b": 2, "winner_id": b })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["winner_id"], b);
    assert_eq!(body["status"], "completed");
}
