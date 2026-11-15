//! Officer tournament write routes (PR 679, follow-up to PR 675 which covers
//! the public GET routes).
//!
//! - Unknown tournament, participant or match ids answer 404 with a fixed
//!   body and write nothing.
//! - Bracket generation with fewer than 2 participants is a 400; an internal
//!   failure inside a generator is a logged 500, never a 404.
//! - The changed routes still work on a real tournament.
//! - DB-009 on round-robin reports, and the standalone dev seed's
//!   round-robin report helper (it used to panic on match 1).

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
const MEMBER_TOKEN: &str = "tourn-member-token";
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

fn authed(method: Method, uri: &str, token: &str, body: Option<Value>) -> Request<Body> {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
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

/// Call as the officer.
async fn call(
    state: &AppState,
    method: Method,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    call_as(state, OFFICER_TOKEN, method, uri, body).await
}

async fn call_as(
    state: &AppState,
    token: &str,
    method: Method,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let resp = create_router(state.clone())
        .oneshot(authed(method, uri, token, body))
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
    seed_user(
        &state.db,
        "memberuser",
        "membermember",
        "TournMember",
        "member",
        MEMBER_TOKEN,
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

    // Expected body per case: an unmatched /api path also answers 404
    // ({"error":"Not found"}), so the body proves the handler ran.
    const T404: &str = "Tournament not found";
    let cases: Vec<(Method, String, Option<Value>, &str)> = vec![
        (
            Method::PUT,
            format!("/api/tournaments/{MISSING}"),
            Some(json!({ "name": "Renamed" })),
            T404,
        ),
        (
            Method::PATCH,
            format!("/api/tournaments/{MISSING}/status"),
            Some(json!({ "status": "registration" })),
            T404,
        ),
        (
            Method::POST,
            format!("/api/tournaments/{MISSING}/generate-bracket"),
            None,
            T404,
        ),
        (
            Method::POST,
            format!("/api/tournaments/{MISSING}/next-round"),
            None,
            T404,
        ),
        (
            Method::POST,
            format!("/api/tournaments/{MISSING}/participants"),
            Some(json!({ "external_name": "Ghost" })),
            T404,
        ),
        // Real participant id under a missing tournament.
        (
            Method::PUT,
            format!("/api/tournaments/{MISSING}/participants/{pid}"),
            Some(json!({ "seed": 1 })),
            T404,
        ),
        (
            Method::DELETE,
            format!("/api/tournaments/{MISSING}/participants/{pid}"),
            None,
            T404,
        ),
        (
            Method::PATCH,
            format!("/api/tournaments/{MISSING}/matches/{MISSING}/report"),
            Some(json!({ "score_a": 1, "score_b": 0, "winner_id": pid })),
            "Match not found",
        ),
    ];

    for (method, uri, body, error) in cases {
        let (status, resp) = call(&state, method.clone(), &uri, body).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method} {uri}: {resp}");
        assert_eq!(resp["error"], error, "{method} {uri}");
    }

    assert_eq!(snapshot(&state).await, before, "no rows written");
    // The real participant was not touched.
    let participants = state.db.list_tournament_participants(&tid).await.unwrap();
    assert_eq!(participants.len(), 1);
    assert_eq!(participants[0].seed, None);
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
        assert_eq!(body["error"], "Participant not found", "PUT {uri}");
        let (status, body) = call(&state, Method::DELETE, &uri, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "DELETE {uri}: {body}");
        assert_eq!(body["error"], "Participant not found", "DELETE {uri}");
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
    assert_eq!(body["error"], "Match not found");
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

/// DB-009 through the report route on a round-robin match: naming B the
/// winner while A has the higher score is refused (409), and a B win reported
/// 1-2 goes through. This check existed before PR 679; the dev-seed fix is
/// covered by `dev_seed_round_robin_reports_go_through` below.
#[tokio::test]
async fn round_robin_report_db009_b_win_needs_b_to_outscore_a() {
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

/// Generating a bracket with 0 or 1 participants is a bad request (400 with a
/// clear body), not a 500, for every format. Nothing is written.
#[tokio::test]
async fn generate_bracket_with_under_two_participants_is_400() {
    let state = seeded().await;
    for format in ["single_elim", "double_elim", "round_robin", "swiss"] {
        let tid = create_tournament(&state, &format!("Small {format}"), format).await;
        for n in 0..2 {
            if n == 1 {
                add_participant(&state, &tid, "Lonely Team").await;
            }
            let before = snapshot(&state).await;
            let (status, body) = call(
                &state,
                Method::POST,
                &format!("/api/tournaments/{tid}/generate-bracket"),
                None,
            )
            .await;
            assert_eq!(
                status,
                StatusCode::BAD_REQUEST,
                "{format} with {n} participant(s): {body}"
            );
            assert_eq!(
                body["error"], "Need at least 2 participants",
                "{format} with {n} participant(s)"
            );
            assert_eq!(snapshot(&state).await, before, "{format} n={n}: no writes");
        }
        if format == "swiss" {
            let (status, body) = call(
                &state,
                Method::POST,
                &format!("/api/tournaments/{tid}/next-round"),
                None,
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "swiss next-round: {body}");
            assert_eq!(body["error"], "Need at least 2 participants");
        }
    }
    assert_eq!(count(&state, "tournament_round").await, 0);
}

/// QA nit 2 from PR 675: the bracket read and generate routes answer 404
/// "Tournament not found" only for a missing tournament. The DB layer now
/// says "no such tournament" as `Ok(None)`, so no other NotFound can be
/// turned into that 404.
#[tokio::test]
async fn bracket_routes_404_only_for_missing_tournament() {
    let state = seeded().await;

    // DB layer: a missing tournament is Ok(None), not an error.
    assert!(
        state
            .db
            .get_tournament_bracket(MISSING)
            .await
            .expect("missing tournament is not an error")
            .is_none()
    );

    for (method, uri) in [
        (Method::GET, format!("/api/tournaments/{MISSING}/bracket")),
        (
            Method::POST,
            format!("/api/tournaments/{MISSING}/generate-bracket"),
        ),
    ] {
        let (status, body) = call(&state, method.clone(), &uri, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method} {uri}: {body}");
        assert_eq!(body["error"], "Tournament not found", "{method} {uri}");
    }

    // A real tournament: the read is 200 with the tournament and empty lists,
    // and generate returns the new bracket.
    let tid = create_tournament(&state, "Bracket Cup", "single_elim").await;
    let (status, body) = call(
        &state,
        Method::GET,
        &format!("/api/tournaments/{tid}/bracket"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["tournament"]["id"], tid.as_str());
    for key in ["participants", "rounds", "matches"] {
        assert_eq!(body[key], json!([]), "{key}: {body}");
    }

    for name in ["Alpha", "Bravo"] {
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
    assert_eq!(body["tournament"]["id"], tid.as_str());
    assert_eq!(body["participants"].as_array().unwrap().len(), 2);
    assert!(!body["matches"].as_array().unwrap().is_empty(), "{body}");
}

/// Dev-seed regression (standalone site-server, "Failed to report RR match
/// 1"). Runs the seed's own helper, which `main.rs` calls: with the old 2-1
/// score for every match, match 1 (a B win) was refused by DB-009 and this
/// returns `Err((1, Conflict))`.
#[tokio::test]
async fn dev_seed_round_robin_reports_go_through() {
    let state = seeded().await;
    let tid = create_tournament(&state, "Seed RR", "round_robin").await;
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

    let reported = scuffed_site_server::seed::report_round_robin_demo_matches(&state.db, &tid, 3)
        .await
        .unwrap_or_else(|(i, e)| panic!("seed RR match {i} refused: {e}"));
    assert_eq!(reported, 3);

    let completed: Vec<_> = state
        .db
        .list_tournament_matches(&tid)
        .await
        .unwrap()
        .into_iter()
        .filter(|m| m.status == scuffed_db::TournamentMatchStatus::Completed)
        .collect();
    assert_eq!(completed.len(), 3);
    for m in &completed {
        let (a, b) = (m.score_a.unwrap(), m.score_b.unwrap());
        let winner_is_a = m.winner_id == m.participant_a_id;
        assert!(
            if winner_is_a { a > b } else { b > a },
            "winner outscores loser: {m:?}"
        );
    }
    assert!(
        completed.iter().any(|m| m.winner_id == m.participant_b_id),
        "the seed reports at least one B win"
    );
}

/// The routes this PR changed still work on a real (draft) tournament.
#[tokio::test]
async fn changed_routes_work_on_a_real_tournament() {
    let state = seeded().await;

    // PUT rename.
    let tid = create_tournament(&state, "Old Name", "single_elim").await;
    let (status, body) = call(
        &state,
        Method::PUT,
        &format!("/api/tournaments/{tid}"),
        Some(json!({ "name": "New Name" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["name"], "New Name");
    assert_eq!(
        state.db.get_tournament(&tid).await.unwrap().unwrap().name,
        "New Name"
    );

    // Swiss, 2 participants: next-round straight away is round 1.
    let swiss = create_tournament(&state, "Swiss A", "swiss").await;
    add_participant(&state, &swiss, "Alpha").await;
    add_participant(&state, &swiss, "Bravo").await;
    let (status, body) = call(
        &state,
        Method::POST,
        &format!("/api/tournaments/{swiss}/next-round"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["round_number"], 1);

    // Swiss, 2 participants: generate-bracket makes round 1, next-round round 2.
    let swiss = create_tournament(&state, "Swiss B", "swiss").await;
    add_participant(&state, &swiss, "Alpha").await;
    add_participant(&state, &swiss, "Bravo").await;
    let (status, body) = call(
        &state,
        Method::POST,
        &format!("/api/tournaments/{swiss}/generate-bracket"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["rounds"].as_array().unwrap().len(), 1, "{body}");
    assert_eq!(body["matches"].as_array().unwrap().len(), 1, "{body}");
    let (status, body) = call(
        &state,
        Method::POST,
        &format!("/api/tournaments/{swiss}/next-round"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["round_number"], 2);
}

/// A member (not an officer) gets the same 403 for a real and an unknown
/// tournament id: the officer check runs before any lookup, so the response
/// does not reveal which ids exist.
#[tokio::test]
async fn member_gets_same_403_for_real_and_unknown_id() {
    let state = seeded().await;
    let tid = create_tournament(&state, "Real Cup", "single_elim").await;
    let before = snapshot(&state).await;
    let rename = Some(json!({ "name": "Hijacked" }));

    let real = call_as(
        &state,
        MEMBER_TOKEN,
        Method::PUT,
        &format!("/api/tournaments/{tid}"),
        rename.clone(),
    )
    .await;
    let missing = call_as(
        &state,
        MEMBER_TOKEN,
        Method::PUT,
        &format!("/api/tournaments/{MISSING}"),
        rename,
    )
    .await;
    assert_eq!(real.0, StatusCode::FORBIDDEN, "{}", real.1);
    assert_eq!(real, missing, "real vs unknown id");
    assert_eq!(snapshot(&state).await, before, "no rows written");
}

/// Site review finding 1: a `NotFound` raised inside a generator, on rows the
/// generator itself just wrote, is a logged 500 and not a 404. Forced here
/// with an event that deletes every first-round match (the ones with a
/// participant) as soon as it is created, so single-elim's wiring step finds
/// its own match gone, as a second concurrent generate-bracket run would
/// cause.
#[tokio::test]
async fn internal_not_found_in_generator_is_500() {
    let state = seeded().await;
    let tid = create_tournament(&state, "Vanishing Cup", "single_elim").await;
    for name in ["Alpha", "Bravo", "Charlie"] {
        add_participant(&state, &tid, name).await;
    }
    state
        .db
        .client
        .query(
            "DEFINE EVENT vanish_first_round ON TABLE tournament_match \
             WHEN $event = 'CREATE' \
               AND ($after.participant_a_id != NONE OR $after.participant_b_id != NONE) \
             THEN { DELETE $after.id };",
        )
        .await
        .unwrap()
        .check()
        .expect("define event");

    // The DB layer raises an internal NotFound here, not Invalid or a
    // driver error, so the route test below covers the real path.
    let err = state
        .db
        .generate_single_elim_bracket(&tid)
        .await
        .expect_err("first-round matches vanish");
    assert!(
        matches!(err, scuffed_db::DbError::NotFound(_)),
        "expected an internal NotFound, got {err:?}"
    );

    let (status, body) = call(
        &state,
        Method::POST,
        &format!("/api/tournaments/{tid}/generate-bracket"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    assert_eq!(body["error"], "Internal server error");
}
