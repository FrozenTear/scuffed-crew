//! `stats_visibility` on member stat routes, public heroes, and leaderboards.
//!
//! Viewers: the subject (owner), an officer, another active member, an inactive
//! member, and a signed-out caller. Subjects: one member in each of public,
//! members, and private.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Method, Request, StatusCode, header};
use chrono::{TimeZone, Utc};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

use scuffed_auth::SessionConfig;
use scuffed_auth::crypto::hash_session_token;
use scuffed_db::migrations::run_migrations;
use scuffed_db::{Database, PersonalMatch};
use scuffed_site_server::create_router;
use scuffed_site_server::state::{AppState, OAuthConfig};
use scuffed_types::StatsVisibility;

const OFFICER: &str = "officer";
const OTHER: &str = "othermember";
const INACTIVE: &str = "inactivemember";
const PUB: &str = "pubmember";
const MEM: &str = "memmember";
const PRIV: &str = "privmember";

const OFFICER_SESSION: &str = "privacy-officer-session";
const OTHER_SESSION: &str = "privacy-other-session";
const INACTIVE_SESSION: &str = "privacy-inactive-session";
const PUB_SESSION: &str = "privacy-pub-session";
const MEM_SESSION: &str = "privacy-mem-session";
const PRIV_SESSION: &str = "privacy-priv-session";

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

async fn seed_member(
    db: &Database,
    user: &str,
    member: &str,
    name: &str,
    role: &str,
    active: bool,
    session: &str,
) {
    let pid = format!("{user}-provider-id");
    let pid_hash = hash_session_token(&pid);
    let active_sql = if active { "true" } else { "false" };
    db.client
        .query(format!(
            r#"CREATE user:{user} SET
                provider = 'discord',
                username = '{name}',
                avatar_url = NONE,
                provider_id = '{pid}',
                provider_id_hash = '{pid_hash}',
                provider_id_encrypted = NONE,
                created_at = time::now()"#
        ))
        .await
        .expect("seed user")
        .check()
        .expect("seed user");
    db.client
        .query(format!(
            r#"CREATE member:{member} SET
                user_id = '{user}',
                org_role = '{role}',
                display_name = '{name}',
                bio = NONE,
                avatar_url = NONE,
                timezone = NONE,
                pronouns = NONE,
                availability_status = NONE,
                joined_at = time::now(),
                is_active = {active_sql}"#
        ))
        .await
        .expect("seed member")
        .check()
        .expect("seed member");
    db.client
        .query(format!(
            r#"CREATE session:sess_{member} SET
                user_id = '{user}',
                token = $tok,
                expires_at = time::now() + 365d,
                created_at = time::now()"#
        ))
        .bind(("tok", hash_session_token(session)))
        .await
        .expect("seed session")
        .check()
        .expect("seed session");
}

fn game(member_id: &str, session_id: &str) -> PersonalMatch {
    PersonalMatch {
        id: String::new(),
        member_id: member_id.to_string(),
        session_id: session_id.to_string(),
        hero: "Ana".into(),
        map_name: "Oasis".into(),
        game_mode: "control".into(),
        role: "Support".into(),
        outcome: "victory".into(),
        elims: 10,
        deaths: 2,
        assists: 5,
        damage: 4000,
        healing: 8000,
        mitigation: 0,
        played_at: Utc.with_ymd_and_hms(2026, 7, 1, 20, 0, 0).unwrap(),
        uploaded_at: Utc::now(),
        edited: false,
        recognizer: scuffed_types::RECOGNIZER_OCR_V1.into(),
        name_recognizer: None,
        suspect_fields: Vec::new(),
    }
}

struct World {
    app: axum::Router,
}

async fn world() -> World {
    let state = test_state().await;
    seed_member(
        &state.db,
        "officeruser",
        OFFICER,
        "Officer",
        "officer",
        true,
        OFFICER_SESSION,
    )
    .await;
    seed_member(
        &state.db,
        "otheruser",
        OTHER,
        "Other",
        "member",
        true,
        OTHER_SESSION,
    )
    .await;
    seed_member(
        &state.db,
        "inactiveuser",
        INACTIVE,
        "Inactive",
        "member",
        false,
        INACTIVE_SESSION,
    )
    .await;
    seed_member(
        &state.db,
        "pubuser",
        PUB,
        "PubVis",
        "member",
        true,
        PUB_SESSION,
    )
    .await;
    seed_member(
        &state.db,
        "memuser",
        MEM,
        "MemVis",
        "member",
        true,
        MEM_SESSION,
    )
    .await;
    seed_member(
        &state.db,
        "privuser",
        PRIV,
        "PrivVis",
        "member",
        true,
        PRIV_SESSION,
    )
    .await;
    state
        .db
        .upsert_member_settings(MEM, None, Some(StatsVisibility::Members))
        .await
        .unwrap();
    state
        .db
        .upsert_member_settings(PRIV, None, Some(StatsVisibility::Private))
        .await
        .unwrap();
    for (id, sid) in [(PUB, "g-pub"), (MEM, "g-mem"), (PRIV, "g-priv")] {
        state
            .db
            .upsert_personal_matches(id, &[game(id, sid)])
            .await
            .unwrap();
    }
    let app = create_router(state);
    World { app }
}

fn request(method: Method, uri: &str, token: Option<&str>, body: Option<Value>) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .extension(ConnectInfo(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(203, 0, 113, 50)),
            9,
        )))
        .header("x-forwarded-for", "203.0.113.50");
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let body = match body {
        Some(value) => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            Body::from(serde_json::to_vec(&value).unwrap())
        }
        None => Body::empty(),
    };
    builder.body(body).unwrap()
}

async fn call(
    app: &axum::Router,
    method: Method,
    uri: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let resp = app
        .clone()
        .oneshot(request(method, uri, token, body))
        .await
        .unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let parsed = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, parsed)
}

fn owner_session(subject: &str) -> &'static str {
    match subject {
        PUB => PUB_SESSION,
        MEM => MEM_SESSION,
        PRIV => PRIV_SESSION,
        _ => panic!("unknown subject {subject}"),
    }
}

fn viewer_token(viewer: &str, subject: &str) -> Option<&'static str> {
    match viewer {
        "owner" => Some(owner_session(subject)),
        "officer" => Some(OFFICER_SESSION),
        "other" => Some(OTHER_SESSION),
        "inactive" => Some(INACTIVE_SESSION),
        "signed_out" => None,
        _ => panic!("unknown viewer {viewer}"),
    }
}

fn member_route_expect(viewer: &str, visibility: &str) -> (StatusCode, Option<&'static str>) {
    match viewer {
        "signed_out" => (StatusCode::UNAUTHORIZED, Some("Authentication required")),
        "inactive" => (StatusCode::FORBIDDEN, Some("Membership inactive")),
        "other" if visibility == "private" => (StatusCode::FORBIDDEN, Some("stats_private")),
        _ => (StatusCode::OK, None),
    }
}

fn heroes_expect(viewer: &str, visibility: &str) -> (StatusCode, Option<&'static str>) {
    match (viewer, visibility) {
        (_, "public") => (StatusCode::OK, None),
        ("owner" | "officer" | "other", "members") => (StatusCode::OK, None),
        ("inactive" | "signed_out", "members") => {
            (StatusCode::FORBIDDEN, Some("stats_members_only"))
        }
        ("owner" | "officer", "private") => (StatusCode::OK, None),
        (_, "private") => (StatusCode::FORBIDDEN, Some("stats_private")),
        _ => panic!("uncovered heroes case {viewer} {visibility}"),
    }
}

fn assert_status(
    label: &str,
    status: StatusCode,
    body: &Value,
    expected: StatusCode,
    error: Option<&str>,
) {
    assert_eq!(status, expected, "{label}: {body}");
    if let Some(error) = error {
        assert_eq!(body["error"], error, "{label}: {body}");
    }
}

#[tokio::test]
async fn five_viewers_against_each_visibility() {
    let world = world().await;
    let subjects = [(PUB, "public"), (MEM, "members"), (PRIV, "private")];
    let viewers = ["owner", "officer", "other", "inactive", "signed_out"];
    let routes = ["", "/heroes", "/roles", "/maps"];

    for (subject, visibility) in subjects {
        for viewer in viewers {
            let token = viewer_token(viewer, subject);
            let (expect_status, expect_error) = member_route_expect(viewer, visibility);
            for route in routes {
                let uri = format!("/api/stats/member/{subject}{route}");
                let (status, body) = call(&world.app, Method::GET, &uri, token, None).await;
                let label = format!("member {uri} as {viewer} ({visibility})");
                assert_status(&label, status, &body, expect_status, expect_error);
                if status == StatusCode::OK && route.is_empty() {
                    assert_eq!(body["total_matches"], 1, "{label}: {body}");
                }
                if status == StatusCode::OK && !route.is_empty() {
                    assert!(
                        body.as_array().is_some_and(|rows| !rows.is_empty()),
                        "{label}: {body}"
                    );
                }
            }

            let (hero_status, hero_error) = heroes_expect(viewer, visibility);
            let hero_uri = format!("/api/public/members/{subject}/heroes");
            let (status, body) = call(&world.app, Method::GET, &hero_uri, token, None).await;
            let label = format!("public heroes {subject} as {viewer} ({visibility})");
            assert_status(&label, status, &body, hero_status, hero_error);
            if status == StatusCode::OK {
                assert!(
                    body.as_array().is_some_and(|rows| !rows.is_empty()),
                    "{label}: {body}"
                );
            }
        }
    }

    for viewer in viewers {
        let token = match viewer {
            "owner" => Some(PUB_SESSION),
            other => viewer_token(other, PUB),
        };
        let (status, body) = call(
            &world.app,
            Method::GET,
            "/api/public/leaderboards?metric=games&limit=25",
            token,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{viewer} board: {body}");
        let mut ids: Vec<&str> = body["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["member_id"].as_str().unwrap())
            .collect();
        ids.sort();
        let expect = if matches!(viewer, "signed_out" | "inactive") {
            vec![PUB]
        } else {
            vec![MEM, PUB]
        };
        assert_eq!(ids, expect, "{viewer} board ids");
    }

    let (status, body) = call(
        &world.app,
        Method::GET,
        "/api/public/leaderboards?metric=games&limit=25",
        Some(PRIV_SESSION),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let ids: Vec<&str> = body["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["member_id"].as_str().unwrap())
        .collect();
    assert!(
        !ids.contains(&PRIV),
        "private member is off their own board"
    );
    assert!(ids.contains(&MEM) && ids.contains(&PUB));
}

#[tokio::test]
async fn public_hero_scoped_follows_leaderboard_visibility() {
    let world = world().await;
    for (token, crew) in [(None, false), (Some(OFFICER_SESSION), true)] {
        let (status, body) = call(
            &world.app,
            Method::GET,
            "/api/public/members?hero=Ana&limit=50",
            token,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let data = body["data"].as_array().expect("member list");
        let row = |id: &str| data.iter().find(|m| m["id"] == id).unwrap();
        for id in [PUB, MEM, PRIV] {
            assert!(row(id).get("id").is_some(), "{id} stays on the roster");
        }
        assert!(row(PUB).get("hero_scoped").is_some(), "{body}");
        assert!(row(PRIV).get("hero_scoped").is_none(), "{body}");
        if crew {
            assert!(row(MEM).get("hero_scoped").is_some(), "{body}");
        } else {
            assert!(row(MEM).get("hero_scoped").is_none(), "{body}");
        }
    }
}

#[tokio::test]
async fn settings_put_keeps_omitted_visibility_and_cache_drops_the_old_board() {
    let world = world().await;

    let (status, body) = call(
        &world.app,
        Method::GET,
        "/api/stats/settings",
        Some(PUB_SESSION),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["stats_visibility"], "public");
    assert!(body["player_name"].is_null());

    let (status, body) = call(
        &world.app,
        Method::PUT,
        "/api/stats/settings",
        Some(MEM_SESSION),
        Some(json!({"player_name": "Ana"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["player_name"], "Ana");
    assert_eq!(body["stats_visibility"], "members");

    // Omitting player_name still clears the name. It does not reject the body.
    let (status, body) = call(
        &world.app,
        Method::PUT,
        "/api/stats/settings",
        Some(MEM_SESSION),
        Some(json!({"stats_visibility": "members"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["stats_visibility"], "members");
    assert!(body["player_name"].is_null(), "{body}");
    let (status, body) = call(
        &world.app,
        Method::PUT,
        "/api/stats/settings",
        Some(MEM_SESSION),
        Some(json!({"player_name": "Ana"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["player_name"], "Ana");
    assert_eq!(body["stats_visibility"], "members");

    let (status, body) = call(
        &world.app,
        Method::PUT,
        "/api/stats/settings",
        Some(MEM_SESSION),
        Some(json!({"player_name": "Ana", "stats_visibility": "nope"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"], "invalid_stats_visibility");
    let (status, body) = call(
        &world.app,
        Method::PUT,
        "/api/stats/settings",
        Some(MEM_SESSION),
        Some(json!({"player_name": "Ana", "stats_visibility": 1})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"], "invalid_stats_visibility");

    let (status, kept) = call(
        &world.app,
        Method::GET,
        "/api/stats/settings",
        Some(MEM_SESSION),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{kept}");
    assert_eq!(kept["stats_visibility"], "members");
    assert_eq!(kept["player_name"], "Ana");

    let leaderboard = "/api/public/leaderboards?metric=games&limit=25";
    let (status, crew_before) = call(
        &world.app,
        Method::GET,
        leaderboard,
        Some(OFFICER_SESSION),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{crew_before}");
    let (status, anon_before) = call(&world.app, Method::GET, leaderboard, None, None).await;
    assert_eq!(status, StatusCode::OK, "{anon_before}");
    let ids = |body: &Value| -> Vec<String> {
        let mut ids: Vec<String> = body["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["member_id"].as_str().unwrap().to_string())
            .collect();
        ids.sort();
        ids
    };
    assert_eq!(ids(&crew_before), vec![MEM.to_string(), PUB.to_string()]);
    assert_eq!(ids(&anon_before), vec![PUB.to_string()]);
    let (status, crew_again) = call(
        &world.app,
        Method::GET,
        leaderboard,
        Some(OFFICER_SESSION),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(crew_again["cached_at"], crew_before["cached_at"]);
    let (status, anon_again) = call(&world.app, Method::GET, leaderboard, None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(anon_again["cached_at"], anon_before["cached_at"]);

    let (status, body) = call(
        &world.app,
        Method::PUT,
        "/api/stats/settings",
        Some(MEM_SESSION),
        Some(json!({"player_name": "Ana", "stats_visibility": "private"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["stats_visibility"], "private");

    let (status, crew_after) = call(
        &world.app,
        Method::GET,
        leaderboard,
        Some(OFFICER_SESSION),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{crew_after}");
    assert_eq!(ids(&crew_after), vec![PUB.to_string()]);
    assert_ne!(crew_after["cached_at"], crew_before["cached_at"]);
    let (status, anon_after) = call(&world.app, Method::GET, leaderboard, None, None).await;
    assert_eq!(status, StatusCode::OK, "{anon_after}");
    assert_eq!(ids(&anon_after), vec![PUB.to_string()]);

    let (status, body) = call(
        &world.app,
        Method::PUT,
        "/api/stats/settings",
        Some(MEM_SESSION),
        Some(json!({"player_name": "Bee"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["player_name"], "Bee");
    assert_eq!(body["stats_visibility"], "private");
}
