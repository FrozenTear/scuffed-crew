//! HTTP coverage for Opus #118 findings M5–M9, L3, and L11.
//!
//! Each test is named after the finding it would fail without the fix.

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

const ADMIN_TOKEN: &str = "gap-admin-token";
const OFFICER_TOKEN: &str = "gap-officer-token";
const MEMBER_TOKEN: &str = "gap-member-token";
const APPLICANT_TOKEN: &str = "gap-applicant-token";

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
        nostr_challenge_key: *blake3::hash(b"test-nostr-challenge-key").as_bytes(),
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

async fn seed_staff(db: &Database) {
    seed_user(
        db,
        "adminuser",
        "adminmember",
        "GapAdmin",
        "admin",
        ADMIN_TOKEN,
    )
    .await;
    seed_user(
        db,
        "officeruser",
        "officermember",
        "GapOfficer",
        "officer",
        OFFICER_TOKEN,
    )
    .await;
    seed_user(
        db,
        "memberuser",
        "membermember",
        "GapMember",
        "member",
        MEMBER_TOKEN,
    )
    .await;
}

async fn seed_applicant(db: &Database) {
    let token_hash = hash_session_token(APPLICANT_TOKEN);
    let pid_hash = hash_session_token("appuser-provider-id");
    db.client
        .query(
            r#"CREATE user:appuser SET
                provider = 'discord',
                username = 'GapApplicant',
                avatar_url = NONE,
                provider_id = 'appuser-provider-id',
                provider_id_hash = $pid,
                provider_id_encrypted = NONE,
                created_at = time::now()"#,
        )
        .bind(("pid", pid_hash))
        .await
        .expect("seed applicant");
    db.client
        .query(
            r#"CREATE session:sess_appuser SET
                user_id = 'appuser',
                token = $tok,
                expires_at = time::now() + 365d,
                created_at = time::now()"#,
        )
        .bind(("tok", token_hash))
        .await
        .expect("seed applicant session");
}

fn rate_limit_ip(builder: axum::http::request::Builder) -> axum::http::request::Builder {
    builder
        .header("x-forwarded-for", "127.0.0.1")
        .extension(axum::extract::ConnectInfo(std::net::SocketAddr::from((
            [127, 0, 0, 1],
            40000,
        ))))
}

fn authed(method: Method, uri: &str, token: &str, body: Option<Value>) -> Request<Body> {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_TYPE, "application/json");
    let req = rate_limit_ip(req);
    match body {
        Some(v) => req
            .body(Body::from(serde_json::to_vec(&v).unwrap()))
            .unwrap(),
        None => req.body(Body::empty()).unwrap(),
    }
}

fn anon(method: Method, uri: &str) -> Request<Body> {
    rate_limit_ip(
        Request::builder()
            .method(method)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json"),
    )
    .body(Body::empty())
    .unwrap()
}

fn anon_json(method: Method, uri: &str, body: Value) -> Request<Body> {
    rate_limit_ip(
        Request::builder()
            .method(method)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json"),
    )
    .body(Body::from(serde_json::to_vec(&body).unwrap()))
    .unwrap()
}

async fn body_bytes(resp: axum::response::Response) -> Vec<u8> {
    resp.into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec()
}

async fn body_json(resp: axum::response::Response) -> Value {
    let bytes = body_bytes(resp).await;
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

fn assert_ics_unbroken(ics: &str, vevents: usize) {
    let flat = ics.replace("\r\n", "\u{0}");
    assert!(
        !flat.contains('\r') && !flat.contains('\n'),
        "CR/LF outside ICS line endings:\n{ics}"
    );
    let lines: Vec<&str> = ics.split("\r\n").filter(|l| !l.is_empty()).collect();
    assert!(
        lines.iter().all(|l| !l.starts_with("X-EVIL")),
        "injected property: {lines:?}"
    );
    assert_eq!(
        lines.iter().filter(|l| **l == "BEGIN:VEVENT").count(),
        vevents
    );
}

// ─── M5 ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn m5_event_write_rejects_control_chars_and_ics_feed_stays_intact() {
    let state = test_state().await;
    seed_staff(&state.db).await;

    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed(
            Method::POST,
            "/api/events",
            OFFICER_TOKEN,
            Some(json!({
                "title": "Scrim\r\nX-EVIL:1",
                "day_of_week": 1,
                "time": "20:00",
                "timezone": "UTC",
                "is_public": true
            })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let err = body_json(resp).await;
    assert!(
        err["error"].as_str().unwrap_or("").contains("title"),
        "create must name the bad field, got {err}"
    );

    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed(
            Method::POST,
            "/api/events",
            OFFICER_TOKEN,
            Some(json!({
                "title": "Public Scrim",
                "day_of_week": 1,
                "time": "20:00",
                "timezone": "UTC\nX-EVIL:tz",
                "is_public": true
            })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(resp).await["error"], "invalid timezone");

    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed(
            Method::POST,
            "/api/events",
            OFFICER_TOKEN,
            Some(json!({
                "title": "Public Scrim",
                "day_of_week": 1,
                "time": "20:00",
                "timezone": "Europe/Berlin",
                "team_id": "alpha",
                "is_public": true
            })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let id = body_json(resp).await["id"].as_str().unwrap().to_string();

    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed(
            Method::PUT,
            &format!("/api/events/{id}"),
            OFFICER_TOKEN,
            Some(json!({ "timezone": "UTC\rX-EVIL:1" })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // A row already in the database (pre-validation) must still not break the feed.
    state
        .db
        .client
        .query(
            "CREATE event:injected SET \
                title = $title, day_of_week = 2, time = '18:00', timezone = $tz, \
                duration_minutes = 60, is_recurring = false, team_id = $team, \
                created_by = 'officermember', is_active = true, is_public = true",
        )
        .bind(("title", "Hello\r\nX-EVIL:1"))
        .bind(("tz", "UTC\r\nX-EVIL:tz"))
        .bind(("team", "alpha\r\nDESCRIPTION:pwned"))
        .await
        .expect("insert hostile event");

    let app = create_router(state);
    let resp = app
        .oneshot(anon(Method::GET, "/api/calendar/all.ics"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let ics = String::from_utf8(body_bytes(resp).await).unwrap();
    assert_ics_unbroken(&ics, 2);
    assert!(
        ics.contains("SUMMARY:Public Scrim"),
        "clean event still exported"
    );
    assert!(
        ics.contains("SUMMARY:Hello\\nX-EVIL:1"),
        "hostile title escaped inside SUMMARY, got {ics}"
    );
    assert!(
        ics.contains("DESCRIPTION:Team: alpha\\nDESCRIPTION:pwned"),
        "hostile team id escaped inside DESCRIPTION, got {ics}"
    );
}

// ─── M6 ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn m6_rsvps_anonymous_only_for_public_active_events() {
    let state = test_state().await;
    seed_staff(&state.db).await;

    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed(
            Method::POST,
            "/api/events",
            OFFICER_TOKEN,
            Some(json!({
                "title": "Private Practice",
                "day_of_week": 3,
                "time": "19:00",
                "timezone": "UTC",
                "is_public": false
            })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let private_id = body_json(resp).await["id"].as_str().unwrap().to_string();

    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed(
            Method::POST,
            &format!("/api/events/{private_id}/rsvp"),
            MEMBER_TOKEN,
            Some(json!({ "status": "yes" })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    for path in ["rsvps", "rsvp-summary"] {
        let app = create_router(state.clone());
        let resp = app
            .oneshot(anon(
                Method::GET,
                &format!("/api/events/{private_id}/{path}"),
            ))
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "anon {path} on a private event must be 404"
        );
    }

    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed(
            Method::GET,
            &format!("/api/events/{private_id}/rsvps"),
            MEMBER_TOKEN,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let rows = body_json(resp).await;
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert_eq!(rows[0]["member_id"], "membermember");

    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed(
            Method::POST,
            "/api/events",
            OFFICER_TOKEN,
            Some(json!({
                "title": "Open Night",
                "day_of_week": 4,
                "time": "19:00",
                "timezone": "UTC",
                "is_public": true
            })),
        ))
        .await
        .unwrap();
    let public_id = body_json(resp).await["id"].as_str().unwrap().to_string();

    let app = create_router(state.clone());
    let resp = app
        .oneshot(anon(
            Method::GET,
            &format!("/api/events/{public_id}/rsvp-summary"),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed(
            Method::DELETE,
            &format!("/api/events/{public_id}"),
            OFFICER_TOKEN,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    for token in [None, Some(MEMBER_TOKEN)] {
        let app = create_router(state.clone());
        let req = match token {
            Some(t) => authed(
                Method::GET,
                &format!("/api/events/{public_id}/rsvps"),
                t,
                None,
            ),
            None => anon(Method::GET, &format!("/api/events/{public_id}/rsvps")),
        };
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "inactive event RSVPs are hidden"
        );
    }

    let app = create_router(state);
    let resp = app
        .oneshot(anon(Method::GET, "/api/events/missing-event/rsvps"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ─── M7 ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn m7_public_member_by_id_hides_inactive() {
    let state = test_state().await;
    seed_staff(&state.db).await;
    state
        .db
        .client
        .query("UPDATE member:membermember SET bio = 'secret bio'")
        .await
        .unwrap();

    let app = create_router(state.clone());
    let resp = app
        .oneshot(anon(Method::GET, "/api/public/members/membermember"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await["display_name"], "GapMember");

    state
        .db
        .client
        .query("UPDATE member:membermember SET is_active = false")
        .await
        .unwrap();

    let app = create_router(state.clone());
    let resp = app
        .oneshot(anon(Method::GET, "/api/public/members/membermember"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let body = body_json(resp).await;
    assert_eq!(body["error"], "Member not found");
    assert!(
        !body.to_string().contains("secret bio"),
        "inactive profile must not leak bio"
    );

    let app = create_router(state.clone());
    let resp = app
        .oneshot(anon(Method::GET, "/api/public/members/membermember/heroes"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    let app = create_router(state.clone());
    let resp = app
        .oneshot(anon(Method::GET, "/api/public/members"))
        .await
        .unwrap();
    let listed = body_json(resp).await;
    assert!(
        listed["data"]
            .as_array()
            .unwrap()
            .iter()
            .all(|m| m["id"] != "membermember")
    );

    let app = create_router(state);
    let resp = app
        .oneshot(anon(Method::GET, "/api/public/members/adminmember"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await["display_name"], "GapAdmin");
}

// ─── M8 ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn m8_draft_tournaments_visible_only_to_staff() {
    let state = test_state().await;
    seed_staff(&state.db).await;

    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed(
            Method::POST,
            "/api/tournaments",
            OFFICER_TOKEN,
            Some(json!({
                "name": "Secret Cup",
                "format": "single_elim",
                "rules": "unreleased seeding plan",
                "best_of": 3
            })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let created = body_json(resp).await;
    assert_eq!(created["status"], "draft");
    let tid = created["id"].as_str().unwrap().to_string();

    let app = create_router(state.clone());
    let resp = app
        .oneshot(anon(Method::GET, "/api/tournaments"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let list = body_json(resp).await;
    assert!(
        list["data"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["name"] != "Secret Cup" && t["status"] != "draft"),
        "anon list leaked a draft: {list}"
    );

    let app = create_router(state.clone());
    let resp = app
        .oneshot(anon(Method::GET, "/api/tournaments?status=draft"))
        .await
        .unwrap();
    let filtered = body_json(resp).await;
    assert!(
        filtered["data"].as_array().unwrap().is_empty(),
        "status=draft must not list drafts to the public: {filtered}"
    );

    for path in [
        format!("/api/tournaments/{tid}"),
        format!("/api/tournaments/{tid}/bracket"),
        format!("/api/tournaments/{tid}/participants"),
        format!("/api/tournaments/{tid}/matches"),
        format!("/api/tournaments/{tid}/standings"),
    ] {
        let app = create_router(state.clone());
        let resp = app.oneshot(anon(Method::GET, &path)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND, "anon {path}");
        let app = create_router(state.clone());
        let resp = app
            .oneshot(authed(Method::GET, &path, MEMBER_TOKEN, None))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND, "member {path}");
    }

    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed(
            Method::GET,
            &format!("/api/tournaments/{tid}"),
            OFFICER_TOKEN,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let detail = body_json(resp).await;
    assert_eq!(detail["name"], "Secret Cup");
    assert_eq!(detail["rules"], "unreleased seeding plan");

    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed(
            Method::GET,
            &format!("/api/tournaments/{tid}/participants"),
            OFFICER_TOKEN,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed(
            Method::GET,
            "/api/tournaments?status=draft",
            ADMIN_TOKEN,
            None,
        ))
        .await
        .unwrap();
    let drafts = body_json(resp).await;
    assert!(
        drafts["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["id"] == tid),
        "admin list must still include drafts: {drafts}"
    );

    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed(
            Method::PATCH,
            &format!("/api/tournaments/{tid}/status"),
            OFFICER_TOKEN,
            Some(json!({ "status": "registration" })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let app = create_router(state.clone());
    let resp = app
        .oneshot(anon(Method::GET, &format!("/api/tournaments/{tid}")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await["status"], "registration");

    let app = create_router(state);
    let resp = app
        .oneshot(anon(Method::GET, "/api/tournaments"))
        .await
        .unwrap();
    let list = body_json(resp).await;
    assert!(
        list["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["id"] == tid)
    );
}

// ─── L3 ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn l3_my_application_omits_officer_review_fields() {
    let state = test_state().await;
    seed_staff(&state.db).await;
    seed_applicant(&state.db).await;

    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed(
            Method::POST,
            "/api/applications",
            APPLICANT_TOKEN,
            Some(json!({
                "preferred_games": [],
                "preferred_roles": ["flex"],
                "message": "hello"
            })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let app_id = body_json(resp).await["id"].as_str().unwrap().to_string();

    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed(
            Method::PATCH,
            &format!("/api/applications/{app_id}"),
            OFFICER_TOKEN,
            Some(json!({
                "status": "trial",
                "review_notes": "internal scouting note"
            })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let app = create_router(state.clone());
    let resp = app
        .oneshot(authed(
            Method::GET,
            "/api/applications/mine",
            APPLICANT_TOKEN,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let mine = body_json(resp).await;
    assert_eq!(mine["status"], "trial");
    assert_eq!(mine["message"], "hello");
    assert!(
        mine.get("review_notes").is_none(),
        "mine leaked notes: {mine}"
    );
    assert!(
        mine.get("reviewed_by").is_none(),
        "mine leaked reviewer: {mine}"
    );
    assert!(
        !mine.to_string().contains("internal scouting note"),
        "note text leaked: {mine}"
    );

    let app = create_router(state);
    let resp = app
        .oneshot(authed(
            Method::GET,
            "/api/applications",
            OFFICER_TOKEN,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let list = body_json(resp).await;
    let row = list["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == app_id)
        .expect("officer list includes the application");
    assert_eq!(row["review_notes"], "internal scouting note");
    assert!(row["reviewed_by"].is_string());
}

// ─── L11 ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn l11_register_conflict_does_not_say_username_taken() {
    let state = test_state().await;

    let app = create_router(state.clone());
    let resp = app
        .oneshot(anon_json(
            Method::POST,
            "/api/auth/local/register",
            json!({
                "username": "TakenName",
                "password": "hunter2234567",
                "confirm_min_age": true
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let app = create_router(state);
    let resp = app
        .oneshot(anon_json(
            Method::POST,
            "/api/auth/local/register",
            json!({
                "username": "takenname",
                "password": "hunter2234567",
                "confirm_min_age": true
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    let body = body_json(resp).await;
    let msg = body["error"].as_str().unwrap_or("");
    assert_eq!(msg, "could not create account");
    assert!(
        !msg.to_lowercase().contains("taken") && !msg.to_lowercase().contains("username"),
        "conflict message still enumerates: {msg}"
    );
}
