//! O3: a game deleted in the tracker app must leave every site view.
//!
//! The tracker sends the deleted session id in `deleted_sessions` on
//! `POST /api/stats/upload`. These tests upload games, delete one through
//! that list, and check the stored rows, the member's own stats and match
//! list, the member stats page, and the public leaderboard.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::ConnectInfo;
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

const OWNER_SESSION: &str = "delete-sync-owner-session";
const OWNER_DAEMON: &str = "delete-sync-owner-daemon";
const OTHER_DAEMON: &str = "delete-sync-other-daemon";
const OWNER: &str = "ownermember";
const OTHER: &str = "othermember";

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

/// A user, an active member with a fixed record key, and a login session.
async fn seed_member(db: &Database, user: &str, member: &str, name: &str, session: &str) {
    let pid = format!("{user}-provider-id");
    let pid_hash = hash_session_token(&pid);
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
                org_role = 'member',
                display_name = '{name}',
                bio = NONE,
                avatar_url = NONE,
                timezone = NONE,
                pronouns = NONE,
                availability_status = NONE,
                joined_at = time::now(),
                is_active = true"#
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

async fn harness() -> (AppState, axum::Router) {
    let state = test_state().await;
    seed_member(&state.db, "owneruser", OWNER, "OwnerOne", OWNER_SESSION).await;
    seed_member(
        &state.db,
        "otheruser",
        OTHER,
        "OtherOne",
        "delete-sync-other-session",
    )
    .await;
    state
        .db
        .create_daemon_token(OWNER, OWNER_DAEMON, "tracker")
        .await
        .expect("owner daemon token");
    state
        .db
        .create_daemon_token(OTHER, OTHER_DAEMON, "tracker")
        .await
        .expect("other daemon token");
    let app = create_router(state.clone());
    (state, app)
}

fn game(session_id: &str, outcome: &str) -> Value {
    json!({
        "session_id": session_id,
        "hero": "Ana",
        "map_name": "Oasis",
        "game_mode": "control",
        "role": "Support",
        "outcome": outcome,
        "elims": 10,
        "deaths": 2,
        "assists": 5,
        "damage": 4000,
        "healing": 8000,
        "mitigation": 0,
        "played_at": "2026-10-21T13:40:00Z",
        "edited": false,
        "recognizer": "ocr-v1"
    })
}

fn request(method: Method, uri: &str, token: &str, body: Option<Value>) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .extension(ConnectInfo(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(203, 0, 113, 40)),
            9,
        )));
    if !token.is_empty() {
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

async fn send(app: &axum::Router, req: Request<Body>) -> Value {
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    serde_json::from_slice(&bytes).unwrap()
}

async fn upload(app: &axum::Router, token: &str, matches: Vec<Value>, deleted: &[&str]) -> Value {
    send(
        app,
        request(
            Method::POST,
            "/api/stats/upload",
            token,
            Some(json!({ "matches": matches, "deleted_sessions": deleted })),
        ),
    )
    .await
}

async fn session_ids(app: &axum::Router, uri: &str) -> Vec<String> {
    let body = send(app, request(Method::GET, uri, OWNER_SESSION, None)).await;
    let mut ids: Vec<String> = body["data"]
        .as_array()
        .expect("match list")
        .iter()
        .map(|m| m["session_id"].as_str().unwrap().to_string())
        .collect();
    ids.sort();
    ids
}

fn leaderboard_games(board: &Value, member_id: &str) -> Option<u64> {
    board["rows"]
        .as_array()
        .expect("leaderboard rows")
        .iter()
        .find(|row| row["member_id"] == member_id)
        .map(|row| row["games"].as_u64().unwrap())
}

#[tokio::test]
async fn deleted_session_leaves_rows_stats_and_leaderboard() {
    let (state, app) = harness().await;

    let resp = upload(
        &app,
        OWNER_DAEMON,
        vec![game("keep-1", "victory"), game("drop-1", "defeat")],
        &[],
    )
    .await;
    assert_eq!(resp["inserted"], 2);
    // Another member's game under the same session id string.
    let resp = upload(&app, OTHER_DAEMON, vec![game("drop-1", "victory")], &[]).await;
    assert_eq!(resp["inserted"], 1);
    assert_eq!(
        session_ids(&app, "/api/stats/me/matches").await,
        ["drop-1", "keep-1"]
    );

    // The tracker's delete: no matches, one tombstone.
    let resp = upload(&app, OWNER_DAEMON, vec![], &["drop-1"]).await;
    assert_eq!(
        (&resp["inserted"], &resp["deleted"]),
        (&json!(0), &json!(1))
    );

    // Stored rows: the owner's copy is gone, the other member's stays.
    let owner_rows = state.db.list_personal_matches(OWNER, 50, 0).await.unwrap();
    let owner_ids: Vec<&str> = owner_rows.iter().map(|m| m.session_id.as_str()).collect();
    assert_eq!(owner_ids, ["keep-1"]);
    let other_rows = state.db.list_personal_matches(OTHER, 50, 0).await.unwrap();
    assert_eq!(
        other_rows.len(),
        1,
        "a tombstone only deletes its own member's row"
    );

    // The owner's views.
    assert_eq!(session_ids(&app, "/api/stats/me/matches").await, ["keep-1"]);
    let me = send(
        &app,
        request(Method::GET, "/api/stats/me", OWNER_SESSION, None),
    )
    .await;
    assert_eq!(me["total_matches"], 1, "{me}");
    assert_eq!((&me["wins"], &me["losses"]), (&json!(1), &json!(0)), "{me}");
    let heroes = send(
        &app,
        request(Method::GET, "/api/stats/me/heroes", OWNER_SESSION, None),
    )
    .await;
    let ana = heroes
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["hero"] == "Ana")
        .expect("Ana row");
    assert_eq!(ana["matches"], 1, "{heroes}");

    // The member stats page another member sees.
    let page = send(
        &app,
        request(
            Method::GET,
            &format!("/api/stats/member/{OWNER}"),
            "delete-sync-other-session",
            None,
        ),
    )
    .await;
    assert_eq!(page["total_matches"], 1, "{page}");

    // Public leaderboard (first read happens after the delete, so the
    // board is built from the current rows).
    let board = send(
        &app,
        request(
            Method::GET,
            "/api/public/leaderboards?metric=games",
            "",
            None,
        ),
    )
    .await;
    assert_eq!(leaderboard_games(&board, OWNER), Some(1), "{board}");
    assert_eq!(leaderboard_games(&board, OTHER), Some(1), "{board}");

    // A resent tombstone (the ack was lost) is a harmless no-op.
    let resp = upload(&app, OWNER_DAEMON, vec![], &["drop-1"]).await;
    assert_eq!(resp["deleted"], 0);
    assert_eq!(
        state
            .db
            .list_personal_matches(OTHER, 50, 0)
            .await
            .unwrap()
            .len(),
        1
    );
}

/// A tombstone that rides with a newer upload of a different game applies
/// after the upsert, so the same request both stores and deletes.
#[tokio::test]
async fn tombstone_in_a_mixed_upload_deletes_the_old_game_and_keeps_the_new_one() {
    let (state, app) = harness().await;
    upload(&app, OWNER_DAEMON, vec![game("old", "victory")], &[]).await;

    let resp = upload(&app, OWNER_DAEMON, vec![game("new", "defeat")], &["old"]).await;
    assert_eq!(
        (&resp["inserted"], &resp["deleted"]),
        (&json!(1), &json!(1))
    );
    let ids: Vec<String> = state
        .db
        .list_personal_matches(OWNER, 50, 0)
        .await
        .unwrap()
        .into_iter()
        .map(|m| m.session_id)
        .collect();
    assert_eq!(ids, ["new"]);
}
