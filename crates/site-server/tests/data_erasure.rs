//! HTTP contract for member stats erasure and account erasure.

use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use chrono::{DateTime, Utc};
use http_body_util::BodyExt;
use scuffed_auth::SessionConfig;
use scuffed_auth::password::hash_password;
use scuffed_db::Database;
use scuffed_db::migrations::run_migrations;
use scuffed_db::types::OrgRole;
use scuffed_site_server::create_router;
use scuffed_site_server::state::{AppState, OAuthConfig};
use serde_json::{Value, json};
use tower::ServiceExt;

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
        auth_redirect_base: None,
        upload_dir: PathBuf::from("/tmp/scuffed-test-uploads"),
        reports_dir: PathBuf::from("/tmp/scuffed-test-reports"),
        reports_enabled: true,
        packs_dir: None,
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

async fn call(
    state: &AppState,
    method: &str,
    uri: &str,
    bearer: Option<&str>,
    cookie: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value, Option<String>) {
    let mut builder =
        Request::builder()
            .method(method)
            .uri(uri)
            .extension(axum::extract::ConnectInfo(std::net::SocketAddr::from((
                [127, 0, 0, 1],
                40000,
            ))));
    if let Some(token) = bearer {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    if let Some(token) = cookie {
        builder = builder.header(header::COOKIE, format!("sc_session={token}"));
    }
    let req = if let Some(body) = body {
        builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap()
    } else {
        builder.body(Body::empty()).unwrap()
    };
    let response = create_router(state.clone()).oneshot(req).await.unwrap();
    let status = response.status();
    let set_cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let parsed = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, parsed, set_cookie)
}

fn match_row(session_id: &str, recognizer: &str) -> Value {
    json!({
        "session_id": session_id,
        "hero": "Ana",
        "map_name": "Oasis",
        "game_mode": "control",
        "role": "Support",
        "outcome": "victory",
        "elims": 4,
        "deaths": 1,
        "assists": 2,
        "damage": 1000,
        "healing": 4000,
        "mitigation": 0,
        "played_at": "2026-07-01T20:00:00Z",
        "edited": false,
        "recognizer": recognizer
    })
}

async fn member_with_session(state: &AppState, username: &str, role: OrgRole) -> (String, String) {
    let user = state
        .db
        .create_local_user(username, "not-a-real-hash")
        .await
        .unwrap();
    let member = state
        .db
        .create_member(&user.id, username, role)
        .await
        .unwrap();
    let token = format!("session-{username}");
    state.db.create_session(&user.id, &token, 24).await.unwrap();
    (member.id, token)
}

#[tokio::test]
async fn upload_while_soft_delete_pending_is_409_for_the_whole_batch() {
    let state = test_state().await;
    let (member_id, session) = member_with_session(&state, "tracker", OrgRole::Member).await;
    state
        .db
        .create_daemon_token(&member_id, "daemon-old", "pc")
        .await
        .unwrap();
    let (uploaded, upload_body, _) = call(
        &state,
        "POST",
        "/api/stats/upload",
        Some("daemon-old"),
        None,
        Some(json!({ "matches": [match_row("already", "ocr-v1")] })),
    )
    .await;
    assert_eq!(uploaded, StatusCode::OK, "{upload_body}");

    let (warm_status, warm, _) = call(
        &state,
        "GET",
        "/api/public/leaderboards?metric=games&limit=25",
        None,
        None,
        None,
    )
    .await;
    assert_eq!(warm_status, StatusCode::OK, "{warm}");
    assert!(warm.to_string().contains("tracker"), "{warm}");

    let (status, body, _) = call(
        &state,
        "DELETE",
        "/api/stats/me",
        Some("daemon-old"),
        None,
        Some(json!({ "confirm": "delete my stats" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["scheduled"], true);
    assert_eq!(body["tokens_revoked"], 1);
    let deadline = body["stats_delete_scheduled_at"]
        .as_str()
        .unwrap()
        .to_string();
    DateTime::parse_from_rfc3339(&deadline).unwrap();

    state
        .db
        .create_daemon_token(&member_id, "daemon-new", "other-pc")
        .await
        .unwrap();
    let (again_status, again, _) = call(
        &state,
        "DELETE",
        "/api/stats/me",
        Some("daemon-new"),
        None,
        Some(json!({ "confirm": "delete my stats" })),
    )
    .await;
    assert_eq!(again_status, StatusCode::OK, "{again}");
    assert_eq!(again["stats_delete_scheduled_at"], deadline);
    assert_eq!(again["tokens_revoked"], 1);

    state
        .db
        .create_daemon_token(&member_id, "daemon-newer", "third-pc")
        .await
        .unwrap();
    let (status, body, _) = call(
        &state,
        "POST",
        "/api/stats/upload",
        Some("daemon-newer"),
        None,
        Some(json!({
            "matches": [match_row("bad-row", "NOPE")],
            "deleted_sessions": ["already"]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "stats_delete_pending");
    assert_eq!(body["stats_delete_scheduled_at"], deadline);
    assert!(body.get("error").is_none());

    let (status, mine, _) = call(&state, "GET", "/api/stats/me", None, Some(&session), None).await;
    assert_eq!(status, StatusCode::OK, "{mine}");
    assert_eq!(mine["total_matches"], 0);
    assert_eq!(
        DateTime::parse_from_rfc3339(mine["stats_delete_scheduled_at"].as_str().unwrap()).unwrap(),
        DateTime::parse_from_rfc3339(&deadline).unwrap()
    );

    let (board_status, board, _) = call(
        &state,
        "GET",
        "/api/public/leaderboards?metric=games&limit=25",
        None,
        None,
        None,
    )
    .await;
    assert_eq!(board_status, StatusCode::OK, "{board}");
    let rendered = board.to_string();
    assert!(!rendered.contains("tracker"), "{board}");

    let rows = state
        .db
        .list_personal_matches(&member_id, 10, 0)
        .await
        .unwrap();
    assert!(rows.is_empty(), "pending delete hides reads");
    let hidden = state.db.get_personal_stats(&member_id).await.unwrap();
    assert_eq!(hidden.total_matches, 0);
    state.db.cancel_stats_delete(&member_id).await.unwrap();
    let rows = state
        .db
        .list_personal_matches(&member_id, 10, 0)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1, "409 must not delete or insert rows");
    assert_eq!(rows[0].session_id, "already");

    let (bad, mismatch, _) = call(
        &state,
        "DELETE",
        "/api/stats/me",
        Some("daemon-newer"),
        None,
        Some(json!({ "confirm": "Delete My Stats" })),
    )
    .await;
    assert_eq!(bad, StatusCode::BAD_REQUEST, "{mismatch}");
    assert_eq!(mismatch["code"], "confirm_mismatch");
}

#[tokio::test]
async fn session_delete_schedules_and_restore_brings_stats_back() {
    let state = test_state().await;
    let (member_id, session) = member_with_session(&state, "ada", OrgRole::Member).await;
    let (other_id, _) = member_with_session(&state, "bea", OrgRole::Member).await;
    state
        .db
        .create_daemon_token(&member_id, "ada-daemon", "pc")
        .await
        .unwrap();
    state
        .db
        .upsert_personal_matches(
            &member_id,
            &[sample(&member_id, "a"), sample(&member_id, "b")],
        )
        .await
        .unwrap();
    state
        .db
        .upsert_personal_matches(&other_id, &[sample(&other_id, "c")])
        .await
        .unwrap();

    state
        .db
        .schedule_stats_delete(&member_id, Utc::now())
        .await
        .unwrap();
    let (status, body, _) = call(
        &state,
        "POST",
        "/api/stats/me/restore",
        Some("ada-daemon"),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");

    let (status, body, _) = call(
        &state,
        "POST",
        "/api/stats/me/restore",
        None,
        Some(&session),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["restored"], true);
    assert_eq!(
        state
            .db
            .get_personal_stats(&member_id)
            .await
            .unwrap()
            .total_matches,
        2
    );

    let (status, body, _) = call(
        &state,
        "DELETE",
        "/api/stats/me",
        None,
        Some(&session),
        Some(json!({ "confirm": "nope" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["code"], "confirm_mismatch");

    state
        .db
        .create_daemon_token(&member_id, "ada-daemon-2", "pc2")
        .await
        .unwrap();
    let (status, body, _) = call(
        &state,
        "DELETE",
        "/api/stats/me",
        None,
        Some(&session),
        Some(json!({ "confirm": "delete my stats" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["scheduled"], true);
    assert!(body.get("deleted").is_none(), "{body}");
    let deadline = body["stats_delete_scheduled_at"]
        .as_str()
        .unwrap()
        .to_string();
    DateTime::parse_from_rfc3339(&deadline).unwrap();
    assert_eq!(body["tokens_revoked"], 1);

    let (again_status, again, _) = call(
        &state,
        "DELETE",
        "/api/stats/me",
        None,
        Some(&session),
        Some(json!({ "confirm": "delete my stats" })),
    )
    .await;
    assert_eq!(again_status, StatusCode::OK, "{again}");
    assert_eq!(again["stats_delete_scheduled_at"], deadline);
    assert_eq!(again["tokens_revoked"], 0);

    assert_eq!(
        state
            .db
            .validate_daemon_token("ada-daemon-2")
            .await
            .unwrap(),
        scuffed_db::DaemonTokenAuth::Missing
    );
    assert_eq!(
        state
            .db
            .get_personal_stats(&other_id)
            .await
            .unwrap()
            .total_matches,
        1
    );
    assert_eq!(
        state
            .db
            .get_personal_stats(&member_id)
            .await
            .unwrap()
            .total_matches,
        0,
        "a session delete hides reads immediately"
    );
    assert!(
        state
            .db
            .list_personal_matches(&member_id, 10, 0)
            .await
            .unwrap()
            .is_empty()
    );

    let (status, restored, _) = call(
        &state,
        "POST",
        "/api/stats/me/restore",
        None,
        Some(&session),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{restored}");
    assert_eq!(restored["restored"], true);
    assert_eq!(
        state
            .db
            .get_personal_stats(&member_id)
            .await
            .unwrap()
            .total_matches,
        2,
        "restore after a session delete brings the rows back"
    );
}

#[tokio::test]
async fn account_delete_refuses_bad_confirm_password_reauth_and_last_admin() {
    let state = test_state().await;
    let hash = hash_password("correct-horse").unwrap();
    let admin = state.db.create_local_user("solo", &hash).await.unwrap();
    let admin_member = state
        .db
        .create_member(&admin.id, "Solo Admin", OrgRole::Admin)
        .await
        .unwrap();
    state
        .db
        .create_session(&admin.id, "solo-session", 24)
        .await
        .unwrap();

    let (status, body, _) = call(
        &state,
        "DELETE",
        "/api/auth/me",
        Some("not-a-session"),
        None,
        Some(json!({ "confirm": "Solo Admin" })),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert!(
        body.get("code").is_none(),
        "daemon token keeps the existing 401 body"
    );

    let (status, body, _) = call(
        &state,
        "DELETE",
        "/api/auth/me",
        None,
        Some("solo-session"),
        Some(json!({ "password": "correct-horse", "confirm": "not-the-name" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["code"], "confirm_mismatch");

    let (status, body, _) = call(
        &state,
        "DELETE",
        "/api/auth/me",
        None,
        Some("solo-session"),
        Some(json!({ "password": "wrong-password", "confirm": "Solo Admin" })),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert_eq!(body["code"], "invalid_password");

    let (status, body, _) = call(
        &state,
        "DELETE",
        "/api/auth/me",
        None,
        Some("solo-session"),
        Some(json!({ "password": "correct-horse", "confirm": "Solo Admin" })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "last_admin");
    assert!(
        state
            .db
            .get_member(&admin_member.id)
            .await
            .unwrap()
            .is_some()
    );

    let oauth = state
        .db
        .create_local_user("oauthname", "placeholder")
        .await
        .unwrap();
    state
        .db
        .client
        .query("UPDATE user SET password_hash = NONE WHERE username = 'oauthname'")
        .await
        .unwrap()
        .check()
        .unwrap();
    let _oauth_member = state
        .db
        .create_member(&oauth.id, "OAuth Name", OrgRole::Member)
        .await
        .unwrap();
    state
        .db
        .create_session(&oauth.id, "oauth-old", 24)
        .await
        .unwrap();
    state
        .db
        .client
        .query("UPDATE session SET created_at = time::now() - 11m WHERE token = $tok")
        .bind(("tok", scuffed_auth::crypto::hash_session_token("oauth-old")))
        .await
        .unwrap()
        .check()
        .unwrap();

    let (status, body, _) = call(
        &state,
        "DELETE",
        "/api/auth/me",
        None,
        Some("oauth-old"),
        Some(json!({ "confirm": "OAuth Name" })),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert_eq!(body["code"], "reauth_required");

    state
        .db
        .create_session(&oauth.id, "oauth-fresh", 24)
        .await
        .unwrap();
    state
        .db
        .create_announcement("Kept", "still here", &_oauth_member.id, false)
        .await
        .unwrap();
    let (status, body, cookie) = call(
        &state,
        "DELETE",
        "/api/auth/me",
        None,
        Some("oauth-fresh"),
        Some(json!({ "confirm": " oauthname " })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["deleted"], true);
    let cookie = cookie.unwrap();
    assert!(cookie.contains("sc_session"), "{cookie}");
    assert!(
        cookie.contains("Max-Age=0") || cookie.contains("max-age=0"),
        "{cookie}"
    );
    assert!(state.db.get_user(&oauth.id).await.unwrap().is_none());
    assert!(
        state
            .db
            .get_session_user("oauth-fresh")
            .await
            .unwrap()
            .is_none()
    );
    let announcements = state.db.list_announcements_paginated(10, 0).await.unwrap();
    assert_eq!(announcements[0].author_id, scuffed_db::DELETED_MEMBER_ID);
}

#[tokio::test]
async fn auth_me_reports_local_password_presence_without_the_hash() {
    let state = test_state().await;
    let hash = hash_password("correct-horse").unwrap();
    let local = state.db.create_local_user("localme", &hash).await.unwrap();
    state
        .db
        .create_member(&local.id, "Local Me", OrgRole::Member)
        .await
        .unwrap();
    state
        .db
        .create_session(&local.id, "local-session", 24)
        .await
        .unwrap();

    let (status, body, _) = call(
        &state,
        "GET",
        "/api/auth/me",
        None,
        Some("local-session"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["has_password"], true);
    assert!(body["user"].get("has_password").is_none());
    assert!(body["member"].get("has_password").is_none());
    let rendered = body.to_string();
    assert!(!rendered.contains("password_hash"), "{rendered}");
    assert!(!rendered.contains(&hash), "{rendered}");
    assert!(!rendered.contains("correct-horse"), "{rendered}");

    let oauth = state
        .db
        .create_local_user("oauthme", "placeholder-secret")
        .await
        .unwrap();
    state
        .db
        .client
        .query("UPDATE user SET password_hash = NONE WHERE username = 'oauthme'")
        .await
        .unwrap()
        .check()
        .unwrap();
    state
        .db
        .create_member(&oauth.id, "OAuth Me", OrgRole::Member)
        .await
        .unwrap();
    state
        .db
        .create_session(&oauth.id, "oauth-session", 24)
        .await
        .unwrap();

    let (status, body, _) = call(
        &state,
        "GET",
        "/api/auth/me",
        None,
        Some("oauth-session"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["has_password"], false);
    assert!(body["user"].get("has_password").is_none());
    assert!(
        body.get("member")
            .and_then(|m| m.get("has_password"))
            .is_none()
    );
    let rendered = body.to_string();
    assert!(!rendered.contains("password_hash"), "{rendered}");
    assert!(!rendered.contains("placeholder-secret"), "{rendered}");
}

async fn call_raw(
    state: &AppState,
    method: &str,
    uri: &str,
    bearer: Option<&str>,
    cookie: Option<&str>,
    content_type: &str,
    body: &'static [u8],
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, content_type)
        .extension(axum::extract::ConnectInfo(std::net::SocketAddr::from((
            [127, 0, 0, 1],
            40000,
        ))));
    if let Some(token) = bearer {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    if let Some(token) = cookie {
        builder = builder.header(header::COOKIE, format!("sc_session={token}"));
    }
    let req = builder.body(Body::from(body)).unwrap();
    let response = create_router(state.clone()).oneshot(req).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let parsed = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, parsed)
}

#[tokio::test]
async fn fresh_setup_stays_open_and_member_lists_omit_the_tombstone() {
    let state = test_state().await;
    let (status, body, _) = call(
        &state,
        "POST",
        "/api/auth/setup",
        None,
        None,
        Some(json!({ "username": "firstadmin", "password": "short" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_ne!(body["error"], "setup already completed");

    let (_admin_id, admin_session) = member_with_session(&state, "keeper", OrgRole::Admin).await;
    let (leaver_id, _) = member_with_session(&state, "leaver", OrgRole::Member).await;
    let leaver = state.db.get_member(&leaver_id).await.unwrap().unwrap();
    state
        .db
        .erase_account(&leaver_id, &leaver.user_id, None, None)
        .await
        .unwrap();

    let (status, listed, _) = call(
        &state,
        "GET",
        "/api/members?include_inactive=true&limit=50",
        None,
        Some(&admin_session),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    let names: Vec<&str> = listed["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|row| row["display_name"].as_str())
        .collect();
    assert!(names.contains(&"keeper"), "{names:?}");
    assert!(!names.contains(&"Deleted member"), "{names:?}");
    assert!(
        listed["data"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["id"] != "deleted"),
        "{listed}"
    );
}

#[tokio::test]
async fn pending_delete_rejects_garbage_uploads_revoked_tokens_and_new_tokens() {
    let state = test_state().await;
    let (member_id, session) = member_with_session(&state, "tracker", OrgRole::Member).await;
    state
        .db
        .create_daemon_token(&member_id, "daemon-old", "pc")
        .await
        .unwrap();
    let (status, body, _) = call(
        &state,
        "DELETE",
        "/api/stats/me",
        Some("daemon-old"),
        None,
        Some(json!({ "confirm": "delete my stats" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let deadline = body["stats_delete_scheduled_at"]
        .as_str()
        .unwrap()
        .to_string();

    let (status, garbage) = call_raw(
        &state,
        "POST",
        "/api/stats/upload",
        Some("daemon-old"),
        None,
        "application/json",
        b"not-json",
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{garbage}");
    assert_eq!(garbage["code"], "stats_delete_pending");
    assert_eq!(garbage["stats_delete_scheduled_at"], deadline);
    assert!(garbage.get("error").is_none(), "{garbage}");

    state
        .db
        .create_daemon_token(&member_id, "daemon-live", "still-active")
        .await
        .unwrap();
    let (status, live_garbage) = call_raw(
        &state,
        "POST",
        "/api/stats/upload",
        Some("daemon-live"),
        None,
        "text/plain",
        b"not-json",
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{live_garbage}");
    assert_eq!(live_garbage["code"], "stats_delete_pending");
    assert!(live_garbage.get("error").is_none(), "{live_garbage}");

    let (status, minted, _) = call(
        &state,
        "POST",
        "/api/stats/tokens",
        None,
        Some(&session),
        Some(json!({ "label": "new-pc" })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{minted}");
    assert_eq!(minted["code"], "stats_delete_pending");
    assert_eq!(minted["stats_delete_scheduled_at"], deadline);
    assert!(minted.get("error").is_none(), "{minted}");
    assert!(minted.get("token").is_none(), "{minted}");

    let (status, report) = call_raw(
        &state,
        "POST",
        "/api/stat-reports",
        None,
        Some(&session),
        "application/zip",
        b"not-a-zip",
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{report}");
    assert_eq!(report["code"], "stats_delete_pending");
    assert_eq!(report["stats_delete_scheduled_at"], deadline);
    assert!(report.get("error").is_none(), "{report}");

    let (status, restored, _) = call(
        &state,
        "POST",
        "/api/stats/me/restore",
        None,
        Some(&session),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{restored}");

    let (status, after) = call_raw(
        &state,
        "POST",
        "/api/stats/upload",
        Some("daemon-old"),
        None,
        "application/json",
        b"not-json",
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{after}");
    assert_eq!(after["error"], "Invalid or revoked daemon token");
    assert!(after.get("code").is_none(), "{after}");

    let (status, minted, _) = call(
        &state,
        "POST",
        "/api/stats/tokens",
        None,
        Some(&session),
        Some(json!({ "label": "after-restore" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{minted}");
    assert!(minted["token"].as_str().is_some(), "{minted}");
}

/// The governor lives on the `Router`. One router per request would reset the bucket.
async fn oneshot(
    app: &axum::Router,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value, Option<u64>) {
    let mut builder =
        Request::builder()
            .method(method)
            .uri(uri)
            .extension(axum::extract::ConnectInfo(std::net::SocketAddr::from((
                [127, 0, 0, 1],
                40000,
            ))));
    if let Some(token) = cookie {
        builder = builder.header(header::COOKIE, format!("sc_session={token}"));
    }
    let req = if let Some(body) = body {
        builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap()
    } else {
        builder.body(Body::empty()).unwrap()
    };
    let response = app.clone().oneshot(req).await.unwrap();
    let status = response.status();
    let retry_after = response
        .headers()
        .get(header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok());
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let parsed = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, parsed, retry_after)
}

#[tokio::test]
async fn repeated_wrong_account_delete_passwords_hit_429() {
    let state = test_state().await;
    let hash = hash_password("correct-horse").unwrap();
    let user = state.db.create_local_user("pwuser", &hash).await.unwrap();
    state
        .db
        .create_member(&user.id, "Pw User", OrgRole::Member)
        .await
        .unwrap();
    state
        .db
        .create_session(&user.id, "pw-session", 24)
        .await
        .unwrap();
    let app = create_router(state);

    let mut invalid = 0;
    for _ in 0..5 {
        let (status, body, _) = oneshot(
            &app,
            "DELETE",
            "/api/auth/me",
            Some("pw-session"),
            Some(json!({ "password": "wrong-password", "confirm": "Pw User" })),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
        assert_eq!(body["code"], "invalid_password");
        invalid += 1;
    }
    let (status, body, retry_after) = oneshot(
        &app,
        "DELETE",
        "/api/auth/me",
        Some("pw-session"),
        Some(json!({ "password": "wrong-password", "confirm": "Pw User" })),
    )
    .await;
    assert_eq!(invalid, 5);
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert_eq!(body["error"], "rate_limited");
    assert!(retry_after.unwrap_or(0) >= 1, "{body}");
    assert!(body["retry_after"].as_u64().unwrap_or(0) >= 1, "{body}");

    // GET stays off the login governor, so a full bucket does not 429 the poll.
    let (status, body, _) = oneshot(&app, "GET", "/api/auth/me", Some("pw-session"), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["has_password"], true);
}

#[tokio::test]
async fn stats_delete_and_restore_share_a_rate_limit() {
    let state = test_state().await;
    let app = create_router(state);
    for _ in 0..5 {
        let (status, body, _) = oneshot(
            &app,
            "DELETE",
            "/api/stats/me",
            None,
            Some(json!({ "confirm": "delete my stats" })),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    }
    let (status, body, retry_after) =
        oneshot(&app, "POST", "/api/stats/me/restore", None, None).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert_eq!(body["error"], "rate_limited");
    assert!(retry_after.unwrap_or(0) >= 1, "{body}");

    let (status, body, _) = oneshot(&app, "GET", "/api/stats/me", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert_ne!(body["error"], "rate_limited");
}

#[tokio::test]
async fn restore_database_error_is_500_not_no_pending_delete() {
    let state = test_state().await;
    let (member_id, session) = member_with_session(&state, "boom", OrgRole::Member).await;
    state
        .db
        .schedule_stats_delete(&member_id, Utc::now())
        .await
        .unwrap();
    state
        .db
        .client
        .query(
            "DEFINE EVENT OVERWRITE restore_boom ON TABLE member \
             WHEN $event = 'UPDATE' THEN { THROW 'restore_boom' }",
        )
        .await
        .unwrap()
        .check()
        .unwrap();

    let (status, body, _) = call(
        &state,
        "POST",
        "/api/stats/me/restore",
        None,
        Some(&session),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    assert!(body.get("code").is_none(), "{body}");
    assert_eq!(body["error"], "Internal error");
}

fn sample(member_id: &str, session_id: &str) -> scuffed_db::PersonalMatch {
    scuffed_db::PersonalMatch {
        id: String::new(),
        member_id: member_id.to_string(),
        session_id: session_id.to_string(),
        hero: "Ana".into(),
        map_name: "Oasis".into(),
        game_mode: "control".into(),
        role: "Support".into(),
        outcome: "victory".into(),
        elims: 4,
        deaths: 1,
        assists: 2,
        damage: 100,
        healing: 200,
        mitigation: 0,
        played_at: Utc::now(),
        uploaded_at: Utc::now(),
        edited: false,
        recognizer: "ocr-v1".into(),
        name_recognizer: None,
        suspect_fields: Vec::new(),
    }
}
