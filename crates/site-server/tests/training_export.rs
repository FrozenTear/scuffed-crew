//! Training crop export for Scuffed Vision: export tokens (admin session
//! only, read-only, 401 everywhere else), the export list (consent and
//! membership filter, member keys, cursor, tombstones on every removal
//! path, pruning), the export image route and the per-IP limit.
//!
//! Members: an owner who contributes, another member, an officer and an
//! admin, each with a session and a tracker sync token. Crops are written
//! to a fresh directory under the system temp dir, which each test removes.

use std::collections::HashSet;
use std::io::Cursor;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Method, Request, StatusCode, header};
use axum::response::Response;
use chrono::{Duration, Utc};
use http_body_util::BodyExt;
use image::{ImageFormat, Rgba, RgbaImage};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

use scuffed_auth::SessionConfig;
use scuffed_auth::crypto::hash_session_token;
use scuffed_db::Database;
use scuffed_db::migrations::run_migrations;
use scuffed_site_server::create_router;
use scuffed_site_server::hero_icons::HeroIconStore;
use scuffed_site_server::state::{AppState, OAuthConfig};
use scuffed_types::api::{TRAINING_EXPORT_IP_BURST, TRAINING_TOMBSTONE_RETENTION_DAYS};

const OWNER: &str = "expowner";
const OTHER: &str = "expother";
const OFFICER: &str = "expofficer";
const ADMIN: &str = "expadmin";

const OWNER_SESSION: &str = "exp-owner-session";
const OTHER_SESSION: &str = "exp-other-session";
const OFFICER_SESSION: &str = "exp-officer-session";
const ADMIN_SESSION: &str = "exp-admin-session";

const OWNER_TOKEN: &str = "exp-owner-sync-token";
const OTHER_TOKEN: &str = "exp-other-sync-token";
const OFFICER_TOKEN: &str = "exp-officer-sync-token";
const ADMIN_TOKEN: &str = "exp-admin-sync-token";

const BOUNDARY: &str = "training-export-test-boundary";
const TRACKER_UA: &str = "scuffed-stat-tracker/0.6.0-alpha.3";
const EXPORT: &str = "/api/training-crops/export";
const TOKENS: &str = "/api/training-crops/export-tokens";

type Got = (StatusCode, Value);

async fn test_state(hero_icons: HeroIconStore) -> AppState {
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
        hero_icons,
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

/// Seed a user, an active member, a session and a sync token.
async fn seed_member(db: &Database, member: &str, role: &str, session: &str, token: &str) {
    let user = format!("u_{member}");
    let pid = format!("{user}-provider-id");
    let pid_hash = hash_session_token(&pid);
    db.client
        .query(format!(
            r#"CREATE user:{user} SET
                provider = 'discord',
                username = '{member}',
                avatar_url = NONE,
                provider_id = '{pid}',
                provider_id_hash = '{pid_hash}',
                provider_id_encrypted = NONE,
                created_at = time::now();
            CREATE member:{member} SET
                user_id = '{user}',
                org_role = '{role}',
                display_name = '{member}',
                bio = NONE,
                avatar_url = NONE,
                timezone = NONE,
                pronouns = NONE,
                availability_status = NONE,
                joined_at = time::now(),
                is_active = true;
            CREATE session:sess_{member} SET
                user_id = '{user}',
                token = $tok,
                expires_at = time::now() + 365d,
                created_at = time::now();"#
        ))
        .bind(("tok", hash_session_token(session)))
        .await
        .expect("seed member")
        .check()
        .expect("seed member");
    db.create_daemon_token(member, token, "tracker")
        .await
        .expect("sync token");
}

struct World {
    app: axum::Router,
    state: AppState,
}

impl World {
    fn dir(&self) -> &Path {
        self.state.hero_icons.dir()
    }

    fn db(&self) -> &Database {
        &self.state.db
    }

    fn cleanup(&self) {
        let _ = std::fs::remove_dir_all(self.dir());
    }

    async fn sql(&self, query: &str) {
        self.db()
            .client
            .query(query)
            .await
            .expect("query")
            .check()
            .expect("query ok");
    }
}

async fn world() -> World {
    let state = test_state(HeroIconStore::in_temp_dir()).await;
    for (member, role, session, token) in [
        (OWNER, "member", OWNER_SESSION, OWNER_TOKEN),
        (OTHER, "member", OTHER_SESSION, OTHER_TOKEN),
        (OFFICER, "officer", OFFICER_SESSION, OFFICER_TOKEN),
        (ADMIN, "admin", ADMIN_SESSION, ADMIN_TOKEN),
    ] {
        seed_member(&state.db, member, role, session, token).await;
    }
    let app = create_router(state.clone());
    World { app, state }
}

fn png(width: u32, height: u32, shade: u8) -> Vec<u8> {
    let canvas = RgbaImage::from_pixel(width, height, Rgba([30, shade, 220, 255]));
    let mut out = Cursor::new(Vec::new());
    canvas.write_to(&mut out, ImageFormat::Png).unwrap();
    out.into_inner()
}

fn multipart_body(count: usize, meta: &Value) -> Vec<u8> {
    let mut body = Vec::new();
    for i in 0..count {
        body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
        body.extend_from_slice(
            b"Content-Disposition: form-data; name=\"meta\"\r\n\
              Content-Type: application/json\r\n\r\n",
        );
        body.extend_from_slice(&serde_json::to_vec(meta).unwrap());
        body.extend_from_slice(b"\r\n");
        body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
        body.extend_from_slice(
            b"Content-Disposition: form-data; name=\"crop\"; filename=\"crop.png\"\r\n\
              Content-Type: image/png\r\n\r\n",
        );
        body.extend_from_slice(&png(40, 16, i as u8));
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
    body
}

/// A fresh client address per request, so the per-IP limits only trip in
/// the test that is about them.
fn next_ip() -> IpAddr {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed) % (1 << 17);
    IpAddr::V4(Ipv4Addr::from(0xC612_0000u32 + n))
}

fn request(
    ip: IpAddr,
    method: Method,
    uri: &str,
    bearer: Option<&str>,
) -> axum::http::request::Builder {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .extension(ConnectInfo(SocketAddr::new(ip, 9)))
        .header(header::USER_AGENT, TRACKER_UA);
    if let Some(bearer) = bearer {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {bearer}"));
    }
    builder
}

async fn read_json(resp: Response) -> Got {
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn call_from(
    app: &axum::Router,
    ip: IpAddr,
    method: Method,
    uri: &str,
    bearer: Option<&str>,
    body: Option<Value>,
) -> Got {
    let mut builder = request(ip, method, uri, bearer);
    let body = match body {
        Some(value) => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            Body::from(serde_json::to_vec(&value).unwrap())
        }
        None => Body::empty(),
    };
    read_json(
        app.clone()
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap(),
    )
    .await
}

async fn call(
    app: &axum::Router,
    method: Method,
    uri: &str,
    bearer: &str,
    body: Option<Value>,
) -> Got {
    call_from(app, next_ip(), method, uri, Some(bearer), body).await
}

async fn get(app: &axum::Router, uri: &str, bearer: &str) -> Got {
    call(app, Method::GET, uri, bearer, None).await
}

async fn delete(app: &axum::Router, uri: &str, bearer: &str) -> Got {
    call(app, Method::DELETE, uri, bearer, None).await
}

async fn raw_get(app: &axum::Router, uri: &str, bearer: &str) -> Response {
    let req = request(next_ip(), Method::GET, uri, Some(bearer))
        .body(Body::empty())
        .unwrap();
    app.clone().oneshot(req).await.unwrap()
}

async fn set_consent(app: &axum::Router, session: &str, body: Value) {
    let got = call(
        app,
        Method::PUT,
        "/api/training-crops/consent",
        session,
        Some(body),
    )
    .await;
    assert_eq!(got.0, StatusCode::OK, "consent: {}", got.1);
}

/// Upload `count` crops of `kind` with the sync token; returns their ids.
async fn upload(app: &axum::Router, token: &str, kind: &str, count: usize) -> Vec<String> {
    let meta = if kind == "hero_icon" {
        json!({ "hero": "Ana" })
    } else {
        json!({ "screen_width": 1920, "screen_height": 1080 })
    };
    let uri = format!("/api/training-crops?kind={kind}");
    let req = request(next_ip(), Method::POST, &uri, Some(token))
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .body(Body::from(multipart_body(count, &meta)))
        .unwrap();
    let got = read_json(app.clone().oneshot(req).await.unwrap()).await;
    assert_eq!(got.0, StatusCode::CREATED, "upload: {}", got.1);
    got.1["crops"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap().to_string())
        .collect()
}

/// Mint an export token with the admin session; returns (token, id).
async fn mint(app: &axum::Router, label: &str) -> (String, String) {
    let got = call(
        app,
        Method::POST,
        TOKENS,
        ADMIN_SESSION,
        Some(json!({ "label": label })),
    )
    .await;
    assert_eq!(got.0, StatusCode::CREATED, "mint: {}", got.1);
    (
        got.1["token"].as_str().unwrap().to_string(),
        got.1["id"].as_str().unwrap().to_string(),
    )
}

async fn export(app: &axum::Router, token: &str, query: &str) -> Value {
    let got = get(app, &format!("{EXPORT}{query}"), token).await;
    assert_eq!(got.0, StatusCode::OK, "export {query}: {}", got.1);
    got.1
}

fn item_ids(page: &Value) -> Vec<String> {
    page["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_string())
        .collect()
}

fn removed_ids(page: &Value) -> Vec<String> {
    page["removed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_string())
        .collect()
}

fn since(page: &Value) -> String {
    format!("?since={}", page["next_cursor"].as_str().unwrap())
}

async fn tombstone_reason(w: &World, crop_id: &str) -> Vec<String> {
    let mut result = w
        .db()
        .client
        .query("SELECT VALUE reason FROM training_crop_tombstone WHERE crop_id = $cid ORDER BY at")
        .bind(("cid", crop_id.to_string()))
        .await
        .unwrap();
    result.take(0).unwrap()
}

async fn audit_actions(w: &World) -> Vec<String> {
    w.db()
        .list_audit_log(100, 0)
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.action)
        .collect()
}

#[tokio::test]
async fn tokens_are_minted_listed_and_revoked_by_admin_sessions_only() {
    let w = world().await;
    let (token, id) = mint(&w.app, "vision weekly").await;
    assert!(
        token.starts_with("sctx_") && token.len() == 5 + 64,
        "{token}"
    );

    let (status, list) = get(&w.app, TOKENS, ADMIN_SESSION).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let row = &list[0];
    assert_eq!(row["id"], json!(id));
    assert_eq!(row["label"], json!("vision weekly"));
    assert_eq!(row["created_by"], json!(ADMIN));
    assert_eq!(row["expired"], json!(false));
    for secret in ["token", "token_hash", "pseudonym_salt"] {
        assert!(row.get(secret).is_none(), "{secret} leaked: {row}");
    }
    let expires: chrono::DateTime<Utc> = serde_json::from_value(row["expires_at"].clone()).unwrap();
    let days = (expires - Utc::now()).num_hours();
    assert!(
        (89 * 24..=90 * 24).contains(&days),
        "default 90 days: {days}h"
    );

    // Not an admin session: officer 403, member 403, sync token, export
    // token and nothing at all 401.
    let body = || Some(json!({ "label": "x" }));
    for (bearer, status) in [
        (OFFICER_SESSION, StatusCode::FORBIDDEN),
        (OWNER_SESSION, StatusCode::FORBIDDEN),
        (ADMIN_TOKEN, StatusCode::UNAUTHORIZED),
        (token.as_str(), StatusCode::UNAUTHORIZED),
    ] {
        assert_eq!(
            call(&w.app, Method::POST, TOKENS, bearer, body()).await.0,
            status
        );
        assert_eq!(get(&w.app, TOKENS, bearer).await.0, status);
        let one = format!("{TOKENS}/{id}");
        assert_eq!(delete(&w.app, &one, bearer).await.0, status);
    }
    let got = call_from(&w.app, next_ip(), Method::GET, TOKENS, None, None).await;
    assert_eq!(got.0, StatusCode::UNAUTHORIZED);

    // Bad bodies.
    for bad in [
        json!({ "label": "" }),
        json!({ "label": "   " }),
        json!({ "label": "x".repeat(65) }),
        json!({ "label": "x", "expires_in_days": 0 }),
        json!({ "label": "x", "expires_in_days": 366 }),
        json!({ "label": "x", "extra": true }),
        json!({}),
    ] {
        let got = call(
            &w.app,
            Method::POST,
            TOKENS,
            ADMIN_SESSION,
            Some(bad.clone()),
        )
        .await;
        assert_eq!(got.0, StatusCode::BAD_REQUEST, "{bad}");
        assert_eq!(got.1["error"], json!("invalid_export_token_request"));
    }
    let got = call(
        &w.app,
        Method::POST,
        TOKENS,
        ADMIN_SESSION,
        Some(json!({ "label": "long", "expires_in_days": 365 })),
    )
    .await;
    assert_eq!(got.0, StatusCode::CREATED, "{}", got.1);

    // Revoke: once, then 404.
    let one = format!("{TOKENS}/{id}");
    let (status, revoked) = delete(&w.app, &one, ADMIN_SESSION).await;
    assert_eq!(status, StatusCode::OK, "{revoked}");
    assert!(revoked["revoked_at"].is_string(), "{revoked}");
    let got = delete(&w.app, &one, ADMIN_SESSION).await;
    assert_eq!(got.0, StatusCode::NOT_FOUND);
    assert_eq!(got.1["error"], json!("export_token_not_found"));
    let got = delete(&w.app, &format!("{TOKENS}/nope"), ADMIN_SESSION).await;
    assert_eq!(got.0, StatusCode::NOT_FOUND);

    let actions = audit_actions(&w).await;
    let minted = actions
        .iter()
        .filter(|a| *a == "created_training_export_token")
        .count();
    assert_eq!(minted, 2, "{actions:?}");
    assert_eq!(
        actions
            .iter()
            .filter(|a| *a == "revoked_training_export_token")
            .count(),
        1
    );
    w.cleanup();
}

#[tokio::test]
async fn revoked_and_expired_tokens_are_401() {
    let w = world().await;
    set_consent(&w.app, OWNER_SESSION, json!({ "stat_digits": true })).await;
    let ids = upload(&w.app, OWNER_TOKEN, "stat_digits", 1).await;
    let image = format!("{EXPORT}/{}/image", ids[0]);
    let (token, id) = mint(&w.app, "revoke me").await;
    assert_eq!(item_ids(&export(&w.app, &token, "").await), ids);
    assert_eq!(
        raw_get(&w.app, &image, &token).await.status(),
        StatusCode::OK
    );

    delete(&w.app, &format!("{TOKENS}/{id}"), ADMIN_SESSION).await;
    assert_eq!(
        get(&w.app, EXPORT, &token).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        raw_get(&w.app, &image, &token).await.status(),
        StatusCode::UNAUTHORIZED
    );

    let expired = "sctx_expired-for-the-test";
    w.db()
        .create_training_export_token(
            ADMIN,
            expired,
            "old",
            Utc::now() - Duration::seconds(1),
            &"ab".repeat(32),
        )
        .await
        .unwrap();
    let got = get(&w.app, EXPORT, expired).await;
    assert_eq!(got.0, StatusCode::UNAUTHORIZED, "{}", got.1);
    assert_eq!(
        raw_get(&w.app, &image, expired).await.status(),
        StatusCode::UNAUTHORIZED
    );
    let (_, list) = get(&w.app, TOKENS, ADMIN_SESSION).await;
    let old = list
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["label"] == json!("old"))
        .unwrap();
    assert_eq!(old["expired"], json!(true));

    // Sessions and sync tokens are not export tokens.
    for bearer in [
        ADMIN_SESSION,
        OWNER_SESSION,
        OWNER_TOKEN,
        ADMIN_TOKEN,
        "junk",
    ] {
        assert_eq!(
            get(&w.app, EXPORT, bearer).await.0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            raw_get(&w.app, &image, bearer).await.status(),
            StatusCode::UNAUTHORIZED
        );
    }
    w.cleanup();
}

#[tokio::test]
async fn export_token_is_401_on_every_other_route() {
    let w = world().await;
    set_consent(&w.app, OWNER_SESSION, json!({ "stat_digits": true })).await;
    let ids = upload(&w.app, OWNER_TOKEN, "stat_digits", 1).await;
    let (token, _) = mint(&w.app, "elsewhere").await;
    let crop = &ids[0];
    let routes = [
        (Method::GET, "/api/training-crops".to_string()),
        (
            Method::POST,
            "/api/training-crops?kind=stat_digits".to_string(),
        ),
        (Method::GET, "/api/training-crops/mine".to_string()),
        (Method::DELETE, "/api/training-crops/mine".to_string()),
        (Method::GET, "/api/training-crops/consent".to_string()),
        (Method::PUT, "/api/training-crops/consent".to_string()),
        (Method::GET, "/api/training-crops/members".to_string()),
        (Method::GET, "/api/training-crops/resolutions".to_string()),
        (Method::GET, format!("/api/training-crops/{crop}/image")),
        (Method::DELETE, format!("/api/training-crops/{crop}")),
        (Method::GET, "/api/stat-reports".to_string()),
        (Method::POST, "/api/stat-reports".to_string()),
        (Method::GET, "/api/hero-icons".to_string()),
        (Method::GET, "/api/hero-icons/mine".to_string()),
        (Method::DELETE, "/api/hero-icons/mine".to_string()),
        (Method::GET, "/api/stats/token-check".to_string()),
        (Method::GET, "/api/stats/tokens".to_string()),
        (Method::GET, "/api/audit-log".to_string()),
    ];
    for (method, uri) in routes {
        let got = call(&w.app, method.clone(), &uri, &token, Some(json!({}))).await;
        assert_eq!(got.0, StatusCode::UNAUTHORIZED, "{method} {uri}: {}", got.1);
    }
    // Nothing was touched.
    assert_eq!(item_ids(&export(&w.app, &token, "").await), ids);
    w.cleanup();
}

#[tokio::test]
async fn consent_decides_what_is_exported_now() {
    let w = world().await;
    let all = json!({ "stat_digits": true, "map_name": true });
    set_consent(&w.app, OWNER_SESSION, all).await;
    let digits = upload(&w.app, OWNER_TOKEN, "stat_digits", 2).await;
    let maps = upload(&w.app, OWNER_TOKEN, "map_name", 1).await;
    // map_name consent goes off before the first export: never listed.
    set_consent(&w.app, OWNER_SESSION, json!({ "map_name": false })).await;
    let (token, _) = mint(&w.app, "consent").await;

    let first = export(&w.app, &token, "").await;
    let mut got = item_ids(&first);
    got.sort();
    let mut want = digits.clone();
    want.sort();
    assert_eq!(got, want);
    assert_eq!(removed_ids(&first), Vec::<String>::new());
    assert_eq!(first["has_more"], json!(false));
    assert_eq!(first["tombstone_retention_days"], json!(60));
    let item = &first["items"][0];
    assert_eq!(item["kind"], json!("stat_digits"));
    assert_eq!(item["meta"]["screen_width"], json!(1920));
    let at = item["created_at"].as_str().unwrap();
    assert!(at.contains(":00:00"), "cut to the hour: {at}");
    for leak in ["member_id", "display_name", "tracker_version"] {
        assert!(item.get(leak).is_none(), "{leak}: {item}");
    }
    let map_image = format!("{EXPORT}/{}/image", maps[0]);
    assert_eq!(
        raw_get(&w.app, &map_image, &token).await.status(),
        StatusCode::NOT_FOUND
    );
    let only_maps = export(&w.app, &token, "?kind=map_name").await;
    assert_eq!(item_ids(&only_maps), Vec::<String>::new());

    // Withdraw stat_digits: a tombstone for each, and gone from a full pull.
    set_consent(&w.app, OWNER_SESSION, json!({ "stat_digits": false })).await;
    let after = export(&w.app, &token, &since(&first)).await;
    let mut gone = removed_ids(&after);
    gone.sort();
    assert_eq!(gone, want);
    assert_eq!(item_ids(&after), Vec::<String>::new());
    assert_eq!(
        item_ids(&export(&w.app, &token, "").await),
        Vec::<String>::new()
    );
    assert_eq!(
        tombstone_reason(&w, &digits[0]).await,
        vec!["consent_withdrawn"]
    );

    // Re-grant: the same crops come back after the tombstones.
    set_consent(&w.app, OWNER_SESSION, json!({ "stat_digits": true })).await;
    let back = export(&w.app, &token, &since(&after)).await;
    let mut again = item_ids(&back);
    again.sort();
    assert_eq!(again, want);
    assert_eq!(removed_ids(&back), Vec::<String>::new());
    w.cleanup();
}

#[tokio::test]
async fn inactive_suspended_and_banned_members_are_not_exported() {
    let w = world().await;
    for session in [OWNER_SESSION, OTHER_SESSION, OFFICER_SESSION] {
        set_consent(&w.app, session, json!({ "stat_digits": true })).await;
    }
    let owner = upload(&w.app, OWNER_TOKEN, "stat_digits", 1).await;
    let other = upload(&w.app, OTHER_TOKEN, "stat_digits", 1).await;
    let officer = upload(&w.app, OFFICER_TOKEN, "stat_digits", 1).await;
    let (token, _) = mint(&w.app, "members").await;
    let first = export(&w.app, &token, "").await;
    assert_eq!(item_ids(&first).len(), 3);

    w.sql(&format!("UPDATE member:{OTHER} SET is_active = false"))
        .await;
    w.sql(&format!(
        "CREATE moderation_action SET member_id = '{OWNER}', action_type = 'suspension', \
         reason = 'test', issued_by = '{ADMIN}', is_active = true, \
         expires_at = time::now() + 7d, created_at = time::now()"
    ))
    .await;
    w.sql(&format!(
        "CREATE moderation_action SET member_id = '{OFFICER}', action_type = 'ban', \
         reason = 'test', issued_by = '{ADMIN}', is_active = true, created_at = time::now()"
    ))
    .await;

    let after = export(&w.app, &token, &since(&first)).await;
    let mut gone = removed_ids(&after);
    gone.sort();
    let mut want = vec![owner[0].clone(), other[0].clone(), officer[0].clone()];
    want.sort();
    assert_eq!(gone, want);
    assert_eq!(
        item_ids(&export(&w.app, &token, "").await),
        Vec::<String>::new()
    );
    for id in &want {
        assert_eq!(tombstone_reason(&w, id).await, vec!["member_inactive"]);
        let image = format!("{EXPORT}/{id}/image");
        let got = read_json(raw_get(&w.app, &image, &token).await).await;
        assert_eq!(got.0, StatusCode::NOT_FOUND);
        assert_eq!(got.1["error"], json!("crop_not_found"));
    }

    // Lifting the suspension makes the owner's crop eligible again.
    w.sql(&format!(
        "UPDATE moderation_action SET is_active = false WHERE member_id = '{OWNER}'"
    ))
    .await;
    let back = export(&w.app, &token, &since(&after)).await;
    assert_eq!(item_ids(&back), owner);
    w.cleanup();
}

#[tokio::test]
async fn member_key_is_per_token_and_never_the_member_id() {
    let w = world().await;
    set_consent(&w.app, OWNER_SESSION, json!({ "stat_digits": true })).await;
    set_consent(&w.app, OTHER_SESSION, json!({ "stat_digits": true })).await;
    upload(&w.app, OWNER_TOKEN, "stat_digits", 2).await;
    let other = upload(&w.app, OTHER_TOKEN, "stat_digits", 1).await;
    let (a, _) = mint(&w.app, "a").await;
    let (b, _) = mint(&w.app, "b").await;

    let keys = |page: &Value| -> Vec<(String, String)> {
        page["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| {
                (
                    i["id"].as_str().unwrap().to_string(),
                    i["member_key"].as_str().unwrap().to_string(),
                )
            })
            .collect()
    };
    let a1 = keys(&export(&w.app, &a, "").await);
    let a2 = keys(&export(&w.app, &a, "").await);
    let b1 = keys(&export(&w.app, &b, "").await);
    assert_eq!(a1.len(), 3);
    assert_eq!(a1, a2, "stable for one token");
    let key_of =
        |list: &[(String, String)], id: &str| list.iter().find(|(i, _)| i == id).unwrap().1.clone();
    for (id, key) in &a1 {
        assert_eq!(key.len(), 64);
        assert!(key.chars().all(|c| c.is_ascii_hexdigit()));
        for member in [OWNER, OTHER] {
            assert_ne!(key, member);
            assert!(!key.contains(member));
        }
        assert_ne!(*key, key_of(&b1, id), "differs across tokens");
    }
    let owner_keys: HashSet<String> = a1
        .iter()
        .filter(|(id, _)| *id != other[0])
        .map(|(_, k)| k.clone())
        .collect();
    assert_eq!(owner_keys.len(), 1, "one key per member");
    assert!(!owner_keys.contains(&key_of(&a1, &other[0])));
    w.cleanup();
}

/// Put `ids` in the feed, run `action`, and check the next page removes them.
async fn assert_removed_after<F, Fut>(
    w: &World,
    token: &str,
    ids: &[String],
    reason: &str,
    action: F,
) where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let before = export(&w.app, token, "").await;
    let listed = item_ids(&before);
    for id in ids {
        assert!(listed.contains(id), "{id} not listed before");
    }
    action().await;
    let after = export(&w.app, token, &since(&before)).await;
    let removed = removed_ids(&after);
    for id in ids {
        assert!(removed.contains(id), "{id} not removed ({reason}): {after}");
        assert_eq!(
            tombstone_reason(w, id).await.last().map(String::as_str),
            Some(reason)
        );
    }
    assert!(
        !item_ids(&export(&w.app, token, "").await)
            .iter()
            .any(|id| ids.contains(id))
    );
}

#[tokio::test]
async fn every_delete_path_leaves_a_tombstone() {
    let w = world().await;
    let all = json!({ "stat_digits": true, "map_name": true, "hero_icon": true });
    set_consent(&w.app, OWNER_SESSION, all).await;
    let (token, _) = mint(&w.app, "deletes").await;
    let app = &w.app;

    // Own single delete, sync token and session.
    let ids = upload(app, OWNER_TOKEN, "stat_digits", 2).await;
    let one = ids[0].clone();
    assert_removed_after(
        &w,
        &token,
        std::slice::from_ref(&one),
        "deleted",
        || async {
            let got = delete(app, &format!("/api/training-crops/{one}"), OWNER_TOKEN).await;
            assert_eq!(got.0, StatusCode::OK, "{}", got.1);
        },
    )
    .await;
    let two = ids[1].clone();
    assert_removed_after(
        &w,
        &token,
        std::slice::from_ref(&two),
        "deleted",
        || async {
            let got = delete(app, &format!("/api/training-crops/{two}"), OWNER_SESSION).await;
            assert_eq!(got.0, StatusCode::OK, "{}", got.1);
        },
    )
    .await;

    // Admin single delete.
    let ids = upload(app, OWNER_TOKEN, "stat_digits", 1).await;
    let id = ids[0].clone();
    assert_removed_after(&w, &token, &ids, "deleted", || async {
        let got = delete(app, &format!("/api/training-crops/{id}"), ADMIN_SESSION).await;
        assert_eq!(got.0, StatusCode::OK, "{}", got.1);
    })
    .await;

    // Own delete-all for one kind keeps the other kinds.
    let maps = upload(app, OWNER_TOKEN, "map_name", 2).await;
    let kept = upload(app, OWNER_TOKEN, "stat_digits", 1).await;
    assert_removed_after(&w, &token, &maps, "deleted", || async {
        let got = delete(app, "/api/training-crops/mine?kind=map_name", OWNER_TOKEN).await;
        assert_eq!(got.1["deleted"], json!(2), "{}", got.1);
    })
    .await;
    assert!(item_ids(&export(app, &token, "").await).contains(&kept[0]));

    // Own delete-all.
    let more = upload(app, OWNER_TOKEN, "map_name", 1).await;
    let everything = [kept.clone(), more].concat();
    assert_removed_after(&w, &token, &everything, "deleted", || async {
        let got = delete(app, "/api/training-crops/mine", OWNER_SESSION).await;
        assert_eq!(got.1["deleted"], json!(2), "{}", got.1);
    })
    .await;

    // Hero icon routes: single, own delete-all, admin per member.
    let heroes = upload(app, OWNER_TOKEN, "hero_icon", 3).await;
    let h0 = heroes[0].clone();
    assert_removed_after(&w, &token, std::slice::from_ref(&h0), "deleted", || async {
        let got = delete(app, &format!("/api/hero-icons/{h0}"), OWNER_SESSION).await;
        assert_eq!(got.0, StatusCode::OK, "{}", got.1);
    })
    .await;
    let h1 = heroes[1].clone();
    assert_removed_after(&w, &token, &[h1], "deleted", || async {
        let got = delete(app, "/api/hero-icons/mine", OWNER_SESSION).await;
        assert_eq!(got.0, StatusCode::OK, "{}", got.1);
    })
    .await;
    let admin_heroes = upload(app, OWNER_TOKEN, "hero_icon", 2).await;
    assert_removed_after(&w, &token, &admin_heroes, "deleted", || async {
        let got = delete(
            app,
            &format!("/api/hero-icons/members/{OWNER}"),
            ADMIN_SESSION,
        )
        .await;
        assert_eq!(got.0, StatusCode::OK, "{}", got.1);
    })
    .await;

    // Consent withdrawal for one kind.
    let digits = upload(app, OWNER_TOKEN, "stat_digits", 2).await;
    assert_removed_after(&w, &token, &digits, "consent_withdrawn", || async {
        set_consent(app, OWNER_SESSION, json!({ "stat_digits": false })).await;
    })
    .await;

    // Deactivation (leaving).
    let maps = upload(app, OWNER_TOKEN, "map_name", 1).await;
    assert_removed_after(&w, &token, &maps, "member_inactive", || async {
        w.sql(&format!("UPDATE member:{OWNER} SET is_active = false"))
            .await;
    })
    .await;
    w.cleanup();
}

#[tokio::test]
async fn cursor_resumes_without_duplicates() {
    let w = world().await;
    set_consent(
        &w.app,
        OWNER_SESSION,
        json!({ "stat_digits": true, "map_name": true }),
    )
    .await;
    let mut want = upload(&w.app, OWNER_TOKEN, "stat_digits", 7).await;
    let (token, _) = mint(&w.app, "pages").await;

    let mut seen: Vec<String> = Vec::new();
    let mut query = "?limit=3".to_string();
    let mut pages = 0;
    let last = loop {
        let page = export(&w.app, &token, &query).await;
        pages += 1;
        seen.extend(item_ids(&page));
        query = format!("{}&limit=3", since(&page));
        if page["has_more"] == json!(false) {
            break page;
        }
        assert!(pages < 10, "runaway paging");
    };
    assert_eq!(pages, 3);
    let unique: HashSet<&String> = seen.iter().collect();
    assert_eq!(unique.len(), seen.len(), "duplicates: {seen:?}");
    seen.sort();
    want.sort();
    assert_eq!(seen, want);

    // Nothing new: empty page, the cursor still moves.
    let quiet = export(&w.app, &token, &since(&last)).await;
    assert_eq!(item_ids(&quiet), Vec::<String>::new());
    assert_eq!(removed_ids(&quiet), Vec::<String>::new());
    assert_ne!(quiet["next_cursor"], last["next_cursor"]);

    // A delete and an upload after that: one removal, one item, once each.
    let fresh = upload(&w.app, OWNER_TOKEN, "map_name", 1).await;
    delete(
        &w.app,
        &format!("/api/training-crops/{}", want[0]),
        OWNER_TOKEN,
    )
    .await;
    let next = export(&w.app, &token, &format!("{}&limit=1", since(&quiet))).await;
    assert_eq!(next["has_more"], json!(true));
    let rest = export(&w.app, &token, &since(&next)).await;
    let items = [item_ids(&next), item_ids(&rest)].concat();
    let removed = [removed_ids(&next), removed_ids(&rest)].concat();
    assert_eq!(items, fresh);
    assert_eq!(removed, vec![want[0].clone()]);
    // The kind filter keeps its own stream.
    let maps = export(&w.app, &token, "?kind=map_name").await;
    assert_eq!(item_ids(&maps), fresh);

    // Bad cursors.
    for bad in ["?since=nope", "?since=v1.1.0.zz", "?kind=replay"] {
        let got = get(&w.app, &format!("{EXPORT}{bad}"), &token).await;
        assert_eq!(got.0, StatusCode::BAD_REQUEST, "{bad}: {}", got.1);
    }
    let got = get(&w.app, &format!("{EXPORT}?kind=full_screen"), &token).await;
    assert_eq!(got.0, StatusCode::UNPROCESSABLE_ENTITY, "{}", got.1);
    w.cleanup();
}

#[tokio::test]
async fn image_route_serves_eligible_crops_only() {
    let w = world().await;
    set_consent(&w.app, OWNER_SESSION, json!({ "stat_digits": true })).await;
    let ids = upload(&w.app, OWNER_TOKEN, "stat_digits", 1).await;
    let (token, _) = mint(&w.app, "images").await;
    let page = export(&w.app, &token, "").await;
    let item = &page["items"][0];
    let image = format!("{EXPORT}/{}/image", ids[0]);

    let resp = raw_get(&w.app, &image, &token).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let headers = resp.headers().clone();
    assert_eq!(headers[header::CONTENT_TYPE], "image/png");
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert_eq!(headers[header::CACHE_CONTROL], "private, no-store");
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let sha: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(item["sha256"], json!(sha));
    assert_eq!(item["size_bytes"], json!(bytes.len()));

    // Unknown and malformed ids, then consent off: 404 crop_not_found.
    for uri in [
        format!("{EXPORT}/{}/image", "0".repeat(32)),
        format!("{EXPORT}/not-an-id/image"),
    ] {
        let got = read_json(raw_get(&w.app, &uri, &token).await).await;
        assert_eq!(got.0, StatusCode::NOT_FOUND);
        assert_eq!(got.1["error"], json!("crop_not_found"));
    }
    set_consent(&w.app, OWNER_SESSION, json!({ "stat_digits": false })).await;
    let got = read_json(raw_get(&w.app, &image, &token).await).await;
    assert_eq!(got.0, StatusCode::NOT_FOUND);
    assert_eq!(got.1["error"], json!("crop_not_found"));
    w.cleanup();
}

#[tokio::test]
async fn old_tombstones_are_pruned_and_old_cursors_are_410() {
    let w = world().await;
    let (token, _) = mint(&w.app, "prune").await;
    w.sql(
        "CREATE training_crop_tombstone SET crop_id = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', \
         kind = 'map_name', reason = 'deleted', at = time::now() - 61d; \
         CREATE training_crop_tombstone SET crop_id = 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb', \
         kind = 'map_name', reason = 'deleted', at = time::now() - 59d;",
    )
    .await;
    let page = export(&w.app, &token, "").await;
    assert_eq!(removed_ids(&page), vec!["b".repeat(32)]);
    assert!(
        tombstone_reason(&w, &"a".repeat(32)).await.is_empty(),
        "pruned"
    );
    assert_eq!(tombstone_reason(&w, &"b".repeat(32)).await, vec!["deleted"]);

    let days = i64::from(TRAINING_TOMBSTONE_RETENTION_DAYS);
    let old = (Utc::now() - Duration::days(days) - Duration::hours(1)).timestamp_micros();
    let got = get(&w.app, &format!("{EXPORT}?since=v1.{old}.2."), &token).await;
    assert_eq!(got.0, StatusCode::GONE, "{}", got.1);
    assert_eq!(got.1["error"], json!("export_cursor_expired"));
    let recent = (Utc::now() - Duration::days(days - 1)).timestamp_micros();
    let got = get(&w.app, &format!("{EXPORT}?since=v1.{recent}.2."), &token).await;
    assert_eq!(got.0, StatusCode::OK, "{}", got.1);
    w.cleanup();
}

#[tokio::test]
async fn export_routes_have_their_own_per_ip_limit() {
    let w = world().await;
    let (token, _) = mint(&w.app, "limit").await;
    let hot = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 91));
    let image = format!("{EXPORT}/{}/image", "0".repeat(32));
    let first = call_from(&w.app, hot, Method::GET, &image, Some(&token), None).await;
    assert_eq!(first.0, StatusCode::NOT_FOUND, "{}", first.1);
    // The bucket refills one a second, so allow a few refills on a slow run.
    let mut limited = None;
    for i in 1..TRAINING_EXPORT_IP_BURST + 10 {
        let got = call_from(&w.app, hot, Method::GET, &image, Some("junk"), None).await;
        if got.0 == StatusCode::TOO_MANY_REQUESTS {
            limited = Some(i);
            break;
        }
    }
    let limited = limited.expect("never limited");
    assert!(
        limited >= TRAINING_EXPORT_IP_BURST - 1,
        "limited after {limited}"
    );
    // The list shares the bucket; another address does not.
    let got = call_from(&w.app, hot, Method::GET, EXPORT, Some(&token), None).await;
    assert_eq!(got.0, StatusCode::TOO_MANY_REQUESTS);
    let got = call_from(&w.app, next_ip(), Method::GET, EXPORT, Some(&token), None).await;
    assert_eq!(got.0, StatusCode::OK, "{}", got.1);
    // Session routes on the hot address are not in this bucket.
    let got = call_from(&w.app, hot, Method::GET, TOKENS, Some(ADMIN_SESSION), None).await;
    assert_eq!(got.0, StatusCode::OK, "{}", got.1);
    w.cleanup();
}
