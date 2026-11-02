//! Officer match view: `GET /api/stats/member/{id}/matches` and
//! `GET /api/stats/member/{id}/matches/{match_id}`.
//!
//! Officers read a member's games when that member's `stats_visibility` is
//! `public` or `members`. `private` answers 403 with the stable
//! `hidden_by_member` body before any match query. Officers always read their
//! own games. Members read their own games on `/api/stats/me/matches`.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode, header};
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
use scuffed_types::{StatsHiddenError, StatsVisibility};

const OFFICER: &str = "officer";
const INACTIVE: &str = "inactiveofficer";
const PUB: &str = "pubmember";
const MEM: &str = "memmember";
const PRIV: &str = "privmember";
const PRIV_EMPTY: &str = "privempty";

const OFFICER_SESSION: &str = "matchview-officer-session";
const INACTIVE_SESSION: &str = "matchview-inactive-session";
const PUB_SESSION: &str = "matchview-pub-session";
const MEM_SESSION: &str = "matchview-mem-session";
const PRIV_SESSION: &str = "matchview-priv-session";
const PRIV_EMPTY_SESSION: &str = "matchview-privempty-session";

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

/// One game with a non-empty `suspect_fields`, so officer reads can be
/// checked for the same marker the owner sees.
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
        suspect_fields: vec!["dmg".into()],
    }
}

/// Officer (private, one game), an inactive officer, three members with one
/// game each (public, members, private), and a private member with no games.
async fn app() -> axum::Router {
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
        "inactiveuser",
        INACTIVE,
        "Inactive",
        "officer",
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
    seed_member(
        &state.db,
        "privemptyuser",
        PRIV_EMPTY,
        "PrivEmpty",
        "member",
        true,
        PRIV_EMPTY_SESSION,
    )
    .await;
    for (id, vis) in [
        (MEM, StatsVisibility::Members),
        (PRIV, StatsVisibility::Private),
        (PRIV_EMPTY, StatsVisibility::Private),
        (OFFICER, StatsVisibility::Private),
    ] {
        state
            .db
            .upsert_member_settings(id, None, Some(vis))
            .await
            .unwrap();
    }
    for (id, sid) in [
        (PUB, "g-pub"),
        (MEM, "g-mem"),
        (PRIV, "g-priv"),
        (OFFICER, "g-officer"),
    ] {
        state
            .db
            .upsert_personal_matches(id, &[game(id, sid)])
            .await
            .unwrap();
    }
    create_router(state)
}

fn request(uri: &str, token: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .uri(uri)
        .extension(ConnectInfo(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(203, 0, 113, 50)),
            9,
        )))
        .header("x-forwarded-for", "203.0.113.50");
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    builder.body(Body::empty()).unwrap()
}

async fn get(app: &axum::Router, uri: &str, token: Option<&str>) -> (StatusCode, Value) {
    let resp = app.clone().oneshot(request(uri, token)).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let parsed = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, parsed)
}

/// Id of the caller's only game, read on their own `/api/stats/me/matches`.
async fn own_match_id(app: &axum::Router, session: &str) -> String {
    let (status, body) = get(app, "/api/stats/me/matches", Some(session)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["data"][0]["id"].as_str().expect("match id").to_string()
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

/// 403 with exactly `{"error","message"}`: no counts, dates or other keys.
fn assert_hidden(label: &str, status: StatusCode, body: &Value) {
    assert_eq!(status, StatusCode::FORBIDDEN, "{label}: {body}");
    let expected = json!({
        "error": "hidden_by_member",
        "message": "This member keeps their stats private.",
    });
    assert_eq!(body, &expected, "{label}");
    let typed = serde_json::to_value(StatsHiddenError::hidden_by_member()).unwrap();
    assert_eq!(body, &typed, "{label}");
}

#[tokio::test]
async fn officer_reads_public_and_members_matches() {
    let app = app().await;
    for (subject, session) in [(PUB, PUB_SESSION), (MEM, MEM_SESSION)] {
        let uri = format!("/api/stats/member/{subject}/matches");
        let (status, body) = get(&app, &uri, Some(OFFICER_SESSION)).await;
        assert_eq!(status, StatusCode::OK, "{uri}: {body}");
        let rows = body["data"].as_array().expect("data array");
        assert_eq!(rows.len(), 1, "{uri}: {body}");
        assert_eq!(rows[0]["member_id"], subject, "{uri}: {body}");
        assert_eq!(rows[0]["suspect_fields"], json!(["dmg"]), "{uri}: {body}");
        assert!(body.get("next_cursor").is_none(), "{uri}: {body}");

        let match_id = rows[0]["id"].as_str().expect("match id");
        let one = format!("/api/stats/member/{subject}/matches/{match_id}");
        let (status, officer_view) = get(&app, &one, Some(OFFICER_SESSION)).await;
        assert_eq!(status, StatusCode::OK, "{one}: {officer_view}");
        assert_eq!(officer_view["suspect_fields"], json!(["dmg"]), "{one}");

        let own = format!("/api/stats/me/matches/{match_id}");
        let (status, owner_view) = get(&app, &own, Some(session)).await;
        assert_eq!(status, StatusCode::OK, "{own}: {owner_view}");
        assert_eq!(officer_view, owner_view, "{one}");
    }
}

#[tokio::test]
async fn officer_blocked_for_private_member_without_leaks() {
    let app = app().await;
    let match_id = own_match_id(&app, PRIV_SESSION).await;
    let uris = [
        format!("/api/stats/member/{PRIV}/matches"),
        format!("/api/stats/member/{PRIV}/matches?limit=1"),
        format!("/api/stats/member/{PRIV}/matches/{match_id}"),
        format!("/api/stats/member/{PRIV}/matches/no-such-match"),
        format!("/api/stats/member/{PRIV_EMPTY}/matches"),
        format!("/api/stats/member/{PRIV_EMPTY}/matches/{match_id}"),
    ];
    for uri in &uris {
        let (status, body) = get(&app, uri, Some(OFFICER_SESSION)).await;
        assert_hidden(uri, status, &body);
    }
}

#[tokio::test]
async fn officer_reads_own_private_matches() {
    let app = app().await;
    let uri = format!("/api/stats/member/{OFFICER}/matches");
    let (status, body) = get(&app, &uri, Some(OFFICER_SESSION)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let rows = body["data"].as_array().expect("data array");
    assert_eq!(rows.len(), 1, "{body}");
    assert_eq!(rows[0]["suspect_fields"], json!(["dmg"]), "{body}");

    let match_id = rows[0]["id"].as_str().expect("match id");
    let one = format!("/api/stats/member/{OFFICER}/matches/{match_id}");
    let (status, row) = get(&app, &one, Some(OFFICER_SESSION)).await;
    assert_eq!(status, StatusCode::OK, "{row}");
    assert_eq!(row["suspect_fields"], json!(["dmg"]), "{row}");
}

#[tokio::test]
async fn owners_keep_their_me_routes_and_members_cannot_use_officer_route() {
    let app = app().await;
    for (subject, session) in [
        (PUB, PUB_SESSION),
        (MEM, MEM_SESSION),
        (PRIV, PRIV_SESSION),
        (OFFICER, OFFICER_SESSION),
    ] {
        let (status, body) = get(&app, "/api/stats/me/matches", Some(session)).await;
        assert_eq!(status, StatusCode::OK, "{subject}: {body}");
        let rows = body["data"].as_array().expect("data array");
        assert_eq!(rows.len(), 1, "{subject}: {body}");
        assert_eq!(rows[0]["suspect_fields"], json!(["dmg"]), "{subject}");

        let match_id = rows[0]["id"].as_str().expect("match id");
        let own = format!("/api/stats/me/matches/{match_id}");
        let (status, body) = get(&app, &own, Some(session)).await;
        assert_eq!(status, StatusCode::OK, "{own}: {body}");

        let uri = format!("/api/stats/member/{subject}/matches");
        let (status, body) = get(&app, &uri, Some(session)).await;
        if subject == OFFICER {
            assert_eq!(status, StatusCode::OK, "{uri}: {body}");
        } else {
            assert_status(
                &uri,
                status,
                &body,
                StatusCode::FORBIDDEN,
                Some("Officer access required"),
            );
        }
    }
}

#[tokio::test]
async fn non_officer_inactive_and_signed_out_are_refused() {
    let app = app().await;
    for subject in [PUB, MEM, PRIV] {
        for route in ["/matches", "/matches/any-id"] {
            let uri = format!("/api/stats/member/{subject}{route}");

            let (status, body) = get(&app, &uri, Some(PUB_SESSION)).await;
            assert_status(
                &uri,
                status,
                &body,
                StatusCode::FORBIDDEN,
                Some("Officer access required"),
            );

            let (status, body) = get(&app, &uri, Some(INACTIVE_SESSION)).await;
            assert_status(
                &uri,
                status,
                &body,
                StatusCode::FORBIDDEN,
                Some("Membership inactive"),
            );

            let (status, body) = get(&app, &uri, None).await;
            assert_status(
                &uri,
                status,
                &body,
                StatusCode::UNAUTHORIZED,
                Some("Authentication required"),
            );
        }
    }
}

#[tokio::test]
async fn unknown_member_and_foreign_match_are_404() {
    let app = app().await;
    for uri in [
        "/api/stats/member/nosuchmember/matches",
        "/api/stats/member/nosuchmember/matches/any-id",
    ] {
        let (status, body) = get(&app, uri, Some(OFFICER_SESSION)).await;
        assert_status(
            uri,
            status,
            &body,
            StatusCode::NOT_FOUND,
            Some("Member not found"),
        );
    }

    let mem_match = own_match_id(&app, MEM_SESSION).await;
    for match_id in [mem_match.as_str(), "no-such-match"] {
        let uri = format!("/api/stats/member/{PUB}/matches/{match_id}");
        let (status, body) = get(&app, &uri, Some(OFFICER_SESSION)).await;
        assert_status(
            &uri,
            status,
            &body,
            StatusCode::NOT_FOUND,
            Some("Match not found"),
        );
    }
}
