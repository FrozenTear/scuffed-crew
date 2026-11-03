//! Contributed hero icons: upload, opt-in, owner and admin reads, deletes,
//! and officer counts.
//!
//! Members: an owner who contributes, another member, an officer and an admin.
//! Only the admin reads or deletes another member's icons. The officer gets
//! per-member counts only. Icons
//! are written to a fresh directory under the system temp dir, which each
//! test removes at the end. Nothing is written into the repo tree.

use std::io::Cursor;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{HeaderMap, HeaderName, Method, Request, StatusCode, header};
use axum::response::Response;
use chrono::{DateTime, Utc};
use http_body_util::BodyExt;
use image::{ImageFormat, Rgba, RgbaImage};
use serde_json::{Value, json};
use tower::ServiceExt;

use scuffed_auth::SessionConfig;
use scuffed_auth::crypto::hash_session_token;
use scuffed_db::migrations::run_migrations;
use scuffed_db::{Database, MemberSettings, NewHeroIcon};
use scuffed_site_server::create_router;
use scuffed_site_server::hero_icons::HeroIconStore;
use scuffed_site_server::state::{AppState, OAuthConfig};
use scuffed_types::{
    HERO_ICON_MAX_BYTES, HERO_ICON_MAX_FILES_PER_UPLOAD, HERO_ICON_MAX_PER_MEMBER,
    HERO_ICON_RATE_BURST, StatsVisibility,
};

const OWNER: &str = "iconowner";
const OTHER: &str = "iconother";
const OFFICER: &str = "iconofficer";
const ADMIN: &str = "iconadmin";

const OWNER_SESSION: &str = "icons-owner-session";
const OTHER_SESSION: &str = "icons-other-session";
const OFFICER_SESSION: &str = "icons-officer-session";
const ADMIN_SESSION: &str = "icons-admin-session";

const BOUNDARY: &str = "hero-icon-test-boundary";
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

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

/// Seed a user, an active member and a session. The display name is the member id.
async fn seed_member(db: &Database, member: &str, role: &str, session: &str) {
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
                display_name = '{member}',
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
}

async fn world_with(hero_icons: HeroIconStore) -> World {
    let state = test_state(hero_icons).await;
    seed_member(&state.db, OWNER, "member", OWNER_SESSION).await;
    seed_member(&state.db, OTHER, "member", OTHER_SESSION).await;
    seed_member(&state.db, OFFICER, "officer", OFFICER_SESSION).await;
    seed_member(&state.db, ADMIN, "admin", ADMIN_SESSION).await;
    let app = create_router(state.clone());
    World { app, state }
}

async fn world() -> World {
    world_with(HeroIconStore::in_temp_dir()).await
}

/// A small valid PNG with one solid colour.
fn png(width: u32, height: u32) -> Vec<u8> {
    let canvas = RgbaImage::from_pixel(width, height, Rgba([200, 40, 90, 255]));
    let mut out = Cursor::new(Vec::new());
    canvas.write_to(&mut out, ImageFormat::Png).unwrap();
    out.into_inner()
}

/// One `icon` part per PNG.
fn icons(pngs: Vec<Vec<u8>>) -> Vec<(&'static str, Vec<u8>)> {
    pngs.into_iter().map(|bytes| ("icon", bytes)).collect()
}

fn multipart_body(parts: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut body = Vec::new();
    for (name, bytes) in parts {
        let disposition = format!("Content-Disposition: form-data; name=\"{name}\"");
        body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
        body.extend_from_slice(disposition.as_bytes());
        body.extend_from_slice(b"; filename=\"icon.png\"\r\nContent-Type: image/png\r\n\r\n");
        body.extend_from_slice(bytes);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
    body
}

fn base_request(method: Method, uri: &str, token: Option<&str>) -> axum::http::request::Builder {
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
    builder
}

async fn read_json(resp: Response) -> Got {
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let parsed = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, parsed)
}

async fn call(
    app: &axum::Router,
    method: Method,
    uri: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> Got {
    let mut builder = base_request(method, uri, token);
    let body = match body {
        Some(value) => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            Body::from(serde_json::to_vec(&value).unwrap())
        }
        None => Body::empty(),
    };
    let request = builder.body(body).unwrap();
    let resp = app.clone().oneshot(request).await.unwrap();
    read_json(resp).await
}

async fn get(app: &axum::Router, uri: &str, token: &str) -> Got {
    call(app, Method::GET, uri, Some(token), None).await
}

async fn delete(app: &axum::Router, uri: &str, token: &str) -> Got {
    call(app, Method::DELETE, uri, Some(token), None).await
}

async fn upload_raw(
    app: &axum::Router,
    token: Option<&str>,
    uri: &str,
    parts: &[(&str, Vec<u8>)],
) -> Response {
    let content_type = format!("multipart/form-data; boundary={BOUNDARY}");
    let request = base_request(Method::POST, uri, token)
        .header(header::CONTENT_TYPE, content_type)
        .body(Body::from(multipart_body(parts)))
        .unwrap();
    app.clone().oneshot(request).await.unwrap()
}

async fn upload(app: &axum::Router, token: &str, hero: &str, parts: &[(&str, Vec<u8>)]) -> Got {
    let uri = format!("/api/hero-icons?hero={hero}");
    let resp = upload_raw(app, Some(token), &uri, parts).await;
    read_json(resp).await
}

/// Upload one 16 by 16 icon.
async fn upload_one(app: &axum::Router, token: &str, hero: &str) -> Got {
    let parts = icons(vec![png(16, 16)]);
    upload(app, token, hero, &parts).await
}

async fn get_raw(app: &axum::Router, uri: &str, token: &str) -> (StatusCode, HeaderMap, Vec<u8>) {
    let request = base_request(Method::GET, uri, Some(token))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(request).await.unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, headers, bytes.to_vec())
}

async fn set_opt_in(app: &axum::Router, token: &str, value: bool) {
    let body = json!({ "icon_contribution_opt_in": value });
    let uri = "/api/hero-icons/opt-in";
    let (status, got) = call(app, Method::PUT, uri, Some(token), Some(body)).await;
    assert_eq!(status, StatusCode::OK, "opt-in put: {got}");
    assert_eq!(got["icon_contribution_opt_in"], json!(value));
}

/// File names in the icons directory. A missing directory is empty.
fn files_on_disk(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn ids_of(list: &Value) -> Vec<String> {
    let items = list.as_array().expect("array");
    items
        .iter()
        .map(|icon| icon["id"].as_str().expect("id").to_string())
        .collect()
}

fn first_id(got: &Got) -> String {
    assert_eq!(got.0, StatusCode::CREATED, "{}", got.1);
    ids_of(&got.1["icons"])[0].clone()
}

fn assert_error(got: &Got, status: StatusCode, code: &str) {
    assert_eq!(got.0, status, "{}", got.1);
    assert_eq!(got.1["error"], json!(code), "{}", got.1);
}

fn assert_header(headers: &HeaderMap, name: HeaderName, expected: &str) {
    let value = headers.get(&name).and_then(|v| v.to_str().ok());
    assert_eq!(value, Some(expected), "{name}");
}

async fn settings_of(db: &Database, member: &str) -> MemberSettings {
    let found = db.get_member_settings(member).await.expect("read");
    found.expect("settings row")
}

async fn deactivate(db: &Database, member: &str) {
    db.client
        .query(format!("UPDATE member:{member} SET is_active = false"))
        .await
        .expect("deactivate")
        .check()
        .expect("deactivate");
}

/// Audit rows as (action, target_type, target_id, details), oldest first.
async fn audit_rows(db: &Database) -> Vec<(String, String, String, Option<String>)> {
    let mut rows: Vec<_> = db
        .list_audit_log(100, 0)
        .await
        .expect("audit log")
        .into_iter()
        .map(|e| {
            (
                e.created_at,
                e.action,
                e.target_type,
                e.target_id,
                e.details,
            )
        })
        .collect();
    rows.sort_by_key(|row| row.0);
    rows.into_iter()
        .map(|(_, action, target_type, target_id, details)| {
            (action, target_type, target_id, details)
        })
        .collect()
}

/// Audit rows with this action, as (target_type, target_id, details).
async fn audits_of(db: &Database, action: &str) -> Vec<(String, String, Option<String>)> {
    audit_rows(db)
        .await
        .into_iter()
        .filter(|row| row.0 == action)
        .map(|(_, target_type, target_id, details)| (target_type, target_id, details))
        .collect()
}

fn member_ids_of(page: &Value) -> Vec<String> {
    let rows = page["data"].as_array().expect("data");
    rows.iter()
        .map(|row| row["member_id"].as_str().expect("member_id").to_string())
        .collect()
}

#[tokio::test]
async fn upload_then_list_mine_with_thumbnail_and_rfc3339_time() {
    let w = world().await;
    set_opt_in(&w.app, OWNER_SESSION, true).await;

    let parts = icons(vec![png(32, 32), png(64, 48)]);
    let (status, body) = upload(&w.app, OWNER_SESSION, "ana", &parts).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let uploaded = ids_of(&body["icons"]);
    assert_eq!(uploaded.len(), 2);
    for (index, id) in uploaded.iter().enumerate() {
        let icon = &body["icons"][index];
        assert_eq!(id.len(), 32);
        assert!(id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')));
        assert_eq!(icon["hero"], json!("Ana"));
        let thumbnail = format!("/api/hero-icons/{id}/image");
        assert_eq!(icon["thumbnail_url"], json!(thumbnail));
        assert!(icon["size_bytes"].as_u64().expect("size") > 0);
    }

    // Accent-folded key, stored as the display name.
    let (status, body) = upload_one(&w.app, OWNER_SESSION, "lucio").await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["icons"][0]["hero"], json!("Lúcio"));

    let (status, mine) = get(&w.app, "/api/hero-icons/mine", OWNER_SESSION).await;
    assert_eq!(status, StatusCode::OK, "{mine}");
    assert_eq!(mine["icon_contribution_opt_in"], json!(true));
    assert_eq!(mine["count"], json!(3));
    assert_eq!(mine["max_icons"], json!(HERO_ICON_MAX_PER_MEMBER));
    let listed = mine["icons"].as_array().expect("icons");
    assert_eq!(listed.len(), 3);
    let mut saw_wide = false;
    for icon in listed {
        let when = icon["uploaded_at"].as_str().expect("uploaded_at");
        assert!(DateTime::parse_from_rfc3339(when).is_ok(), "{when}");
        assert!(when.ends_with('Z'), "UTC timestamp: {when}");
        if icon["width"] == json!(64) && icon["height"] == json!(48) {
            saw_wide = true;
        }
    }
    assert!(saw_wide, "{mine}");

    // Files are named by id only.
    let id = uploaded[0].clone();
    let file = w.dir().join(format!("{id}.png"));
    let files = files_on_disk(w.dir());
    assert_eq!(files.len(), 3, "{files:?}");
    assert!(file.exists(), "{files:?}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let file_mode = std::fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(file_mode & 0o777, 0o600);
        let dir_mode = std::fs::metadata(w.dir()).unwrap().permissions().mode();
        assert_eq!(dir_mode & 0o777, 0o700);
    }

    // The thumbnail is the stored file, served privately.
    let uri = format!("/api/hero-icons/{id}/image");
    let (status, headers, bytes) = get_raw(&w.app, &uri, OWNER_SESSION).await;
    assert_eq!(status, StatusCode::OK);
    assert_header(&headers, header::CONTENT_TYPE, "image/png");
    assert_header(&headers, header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    assert_header(&headers, header::CACHE_CONTROL, "private, no-store");
    assert_header(&headers, header::CONTENT_DISPOSITION, "inline");
    assert!(bytes.starts_with(&PNG_SIGNATURE));
    assert_eq!(bytes, std::fs::read(&file).unwrap());

    w.cleanup();
}

#[tokio::test]
async fn not_signed_in_is_401() {
    let w = world().await;
    let parts = icons(vec![png(16, 16)]);
    let resp = upload_raw(&w.app, None, "/api/hero-icons?hero=ana", &parts).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    for uri in [
        "/api/hero-icons/mine",
        "/api/hero-icons/opt-in",
        "/api/hero-icons",
        "/api/hero-icons/members",
        "/api/hero-icons/0123456789abcdef0123456789abcdef/image",
    ] {
        let (status, body) = call(&w.app, Method::GET, uri, None, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{uri}: {body}");
    }
    assert!(files_on_disk(w.dir()).is_empty());
    w.cleanup();
}

#[tokio::test]
async fn upload_without_opt_in_is_403_and_stores_nothing() {
    let w = world().await;
    let got = upload_one(&w.app, OTHER_SESSION, "ana").await;
    assert_error(&got, StatusCode::FORBIDDEN, "icon_opt_in_required");
    let (_, mine) = get(&w.app, "/api/hero-icons/mine", OTHER_SESSION).await;
    assert_eq!(mine["count"], json!(0));
    assert_eq!(mine["icon_contribution_opt_in"], json!(false));
    assert!(files_on_disk(w.dir()).is_empty());
    w.cleanup();
}

#[tokio::test]
async fn wrong_type_bad_png_and_bad_size_are_refused() {
    let w = world().await;
    set_opt_in(&w.app, OWNER_SESSION, true).await;
    let media = StatusCode::UNSUPPORTED_MEDIA_TYPE;
    let bad = StatusCode::BAD_REQUEST;

    let jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, b'J', b'F', b'I', b'F'];
    let got = upload(&w.app, OWNER_SESSION, "ana", &icons(vec![jpeg])).await;
    assert_error(&got, media, "not_png");

    let garbage = b"hello, not an image".to_vec();
    let got = upload(&w.app, OWNER_SESSION, "ana", &icons(vec![garbage])).await;
    assert_error(&got, media, "not_png");

    let mut broken = PNG_SIGNATURE.to_vec();
    broken.extend_from_slice(b"not really chunks");
    let got = upload(&w.app, OWNER_SESSION, "ana", &icons(vec![broken])).await;
    assert_error(&got, bad, "invalid_png");

    let got = upload(&w.app, OWNER_SESSION, "ana", &icons(vec![png(4, 4)])).await;
    assert_error(&got, bad, "icon_bad_dimensions");

    let got = upload(&w.app, OWNER_SESSION, "ana", &icons(vec![png(300, 16)])).await;
    assert_error(&got, bad, "icon_bad_dimensions");

    // One bad part refuses the whole request.
    let mixed = icons(vec![png(16, 16), b"nope".to_vec()]);
    let got = upload(&w.app, OWNER_SESSION, "ana", &mixed).await;
    assert_error(&got, media, "not_png");

    let renamed = [("avatar", png(16, 16))];
    let got = upload(&w.app, OWNER_SESSION, "ana", &renamed).await;
    assert_error(&got, bad, "unexpected_field");

    let got = upload(&w.app, OWNER_SESSION, "ana", &[]).await;
    assert_error(&got, bad, "no_icons");

    // A body that is not multipart at all gets the JSON error body too.
    let uri = "/api/hero-icons?hero=ana";
    let body = Some(json!({}));
    let got = call(&w.app, Method::POST, uri, Some(OWNER_SESSION), body).await;
    assert_error(&got, bad, "invalid_multipart");

    assert!(files_on_disk(w.dir()).is_empty());
    w.cleanup();
}

#[tokio::test]
async fn too_big_and_too_many_are_413() {
    let w = world().await;
    set_opt_in(&w.app, OWNER_SESSION, true).await;
    let too_large = StatusCode::PAYLOAD_TOO_LARGE;

    let mut big = png(16, 16);
    big.resize(HERO_ICON_MAX_BYTES + 1, 0);
    let got = upload(&w.app, OWNER_SESSION, "ana", &icons(vec![big])).await;
    assert_error(&got, too_large, "icon_too_large");

    let many = icons(vec![png(16, 16); HERO_ICON_MAX_FILES_PER_UPLOAD + 1]);
    let got = upload(&w.app, OWNER_SESSION, "ana", &many).await;
    assert_error(&got, too_large, "too_many_icons_in_request");
    assert!(files_on_disk(w.dir()).is_empty());

    // Exactly the cap is fine.
    let max = icons(vec![png(16, 16); HERO_ICON_MAX_FILES_PER_UPLOAD]);
    let (status, body) = upload(&w.app, OWNER_SESSION, "ana", &max).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let stored = files_on_disk(w.dir());
    assert_eq!(stored.len(), HERO_ICON_MAX_FILES_PER_UPLOAD);
    w.cleanup();
}

#[tokio::test]
async fn stored_icon_cap_is_409() {
    let w = world().await;
    set_opt_in(&w.app, OWNER_SESSION, true).await;
    // Rows only, no files: the cap counts rows.
    for i in 0..HERO_ICON_MAX_PER_MEMBER - 1 {
        let id = format!("{i:032x}");
        let file_name = format!("{id}.png");
        let row = NewHeroIcon {
            id,
            member_id: OWNER.to_string(),
            hero: "Ana".to_string(),
            file_name,
            size_bytes: 10,
            width: 16,
            height: 16,
            created_at: Utc::now(),
        };
        w.db().insert_hero_icon(&row).await.expect("seed icon row");
    }
    let limit = StatusCode::CONFLICT;

    // This request would pass the cap, so none of it is stored.
    let two = icons(vec![png(16, 16), png(16, 16)]);
    let got = upload(&w.app, OWNER_SESSION, "ana", &two).await;
    assert_error(&got, limit, "icon_limit_reached");
    assert!(files_on_disk(w.dir()).is_empty());

    // Filling up to the cap exactly is fine.
    let (status, body) = upload_one(&w.app, OWNER_SESSION, "ana").await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    // At the cap the request is refused before the body is read.
    let got = upload_one(&w.app, OWNER_SESSION, "ana").await;
    assert_error(&got, limit, "icon_limit_reached");
    assert_eq!(files_on_disk(w.dir()).len(), 1);

    let (_, mine) = get(&w.app, "/api/hero-icons/mine", OWNER_SESSION).await;
    assert_eq!(mine["count"], json!(HERO_ICON_MAX_PER_MEMBER));
    w.cleanup();
}

#[tokio::test]
async fn unknown_hero_is_400() {
    let w = world().await;
    set_opt_in(&w.app, OWNER_SESSION, true).await;
    let got = upload_one(&w.app, OWNER_SESSION, "notahero").await;
    assert_error(&got, StatusCode::BAD_REQUEST, "unknown_hero");

    let parts = icons(vec![png(16, 16)]);
    let resp = upload_raw(&w.app, Some(OWNER_SESSION), "/api/hero-icons", &parts).await;
    let got = read_json(resp).await;
    assert_error(&got, StatusCode::BAD_REQUEST, "unknown_hero");
    assert!(files_on_disk(w.dir()).is_empty());
    w.cleanup();
}

#[tokio::test]
async fn upload_rate_limit_is_json_429() {
    let w = world().await;
    set_opt_in(&w.app, OWNER_SESSION, true).await;
    for i in 0..HERO_ICON_RATE_BURST {
        let (status, body) = upload_one(&w.app, OWNER_SESSION, "notahero").await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "request {i}: {body}");
    }
    let parts = icons(vec![png(16, 16)]);
    let uri = "/api/hero-icons?hero=ana";
    let resp = upload_raw(&w.app, Some(OWNER_SESSION), uri, &parts).await;
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    let retry_header = resp.headers()[header::RETRY_AFTER].clone();
    let retry: u64 = retry_header.to_str().unwrap().parse().unwrap();
    assert!(retry >= 1);
    let (_, body) = read_json(resp).await;
    assert_eq!(body["error"], json!("rate_limited"));
    assert_eq!(body["retry_after"], json!(retry));
    assert!(files_on_disk(w.dir()).is_empty());

    // The limiter is per member.
    set_opt_in(&w.app, OTHER_SESSION, true).await;
    let (status, body) = upload_one(&w.app, OTHER_SESSION, "ana").await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    w.cleanup();
}

#[tokio::test]
async fn admin_lists_reads_and_deletes_visible_icons() {
    let w = world().await;
    set_opt_in(&w.app, OWNER_SESSION, true).await;
    set_opt_in(&w.app, OTHER_SESSION, true).await;
    let parts = icons(vec![png(16, 16), png(20, 20)]);
    let (status, body) = upload(&w.app, OWNER_SESSION, "ana", &parts).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let mut owner_ids = ids_of(&body["icons"]);
    owner_ids.sort();
    let other_id = first_id(&upload_one(&w.app, OTHER_SESSION, "mercy").await);
    // Opted out: kept, but not counted, listed, read or deletable by staff.
    set_opt_in(&w.app, OTHER_SESSION, false).await;

    let (status, list) = get(&w.app, "/api/hero-icons", ADMIN_SESSION).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let mut listed = ids_of(&list["data"]);
    listed.sort();
    assert_eq!(listed, owner_ids);
    let first = &list["data"][0];
    assert_eq!(first["member_id"], json!(OWNER));
    assert_eq!(first["display_name"], json!(OWNER));
    assert_eq!(first["hero"], json!("Ana"));
    assert!(first["size_bytes"].as_u64().unwrap() > 0);
    let when = first["uploaded_at"].as_str().unwrap();
    assert!(DateTime::parse_from_rfc3339(when).is_ok(), "{when}");
    let viewed = audits_of(w.db(), "viewed_hero_icons").await;
    let all = (
        "member".to_string(),
        "all".to_string(),
        Some("2 icons".to_string()),
    );
    assert_eq!(viewed, vec![all]);

    // Officers see opted-in members only. Admins also see the opted-out
    // member with a hidden icon (covered in
    // `admin_counts_include_opted_out_members_with_hidden_icons`).
    let (status, counts) = get(&w.app, "/api/hero-icons/members", OFFICER_SESSION).await;
    assert_eq!(status, StatusCode::OK, "{counts}");
    let (status, admin_counts) = get(&w.app, "/api/hero-icons/members", ADMIN_SESSION).await;
    assert_eq!(status, StatusCode::OK, "{admin_counts}");
    let both = vec![OTHER.to_string(), OWNER.to_string()];
    assert_eq!(member_ids_of(&admin_counts), both);
    assert_eq!(admin_counts["data"][1], counts["data"][0]);
    assert_eq!(member_ids_of(&counts), vec![OWNER.to_string()]);
    let row = &counts["data"][0];
    assert_eq!(row["icon_count"], json!(2));
    assert_eq!(row["opted_in"], json!(true));
    assert_eq!(row["icon_contribution_opt_in"], json!(true));
    assert_eq!(row["display_name"], json!(OWNER));
    assert!(counts.get("next_cursor").is_none(), "{counts}");

    let uri = format!("/api/hero-icons/members/{OWNER}");
    let (status, per_member) = get(&w.app, &uri, ADMIN_SESSION).await;
    assert_eq!(status, StatusCode::OK, "{per_member}");
    assert_eq!(ids_of(&per_member["icons"]).len(), 2);
    let viewed = audits_of(w.db(), "viewed_hero_icons").await;
    assert_eq!(viewed.len(), 2, "{viewed:?}");
    let owner_view = (
        "member".to_string(),
        OWNER.to_string(),
        Some("2 icons".to_string()),
    );
    assert!(viewed.contains(&owner_view), "{viewed:?}");

    // Thumbnail fetches are not audited.
    let target = owner_ids[0].clone();
    let uri = format!("/api/hero-icons/{target}/image");
    let (status, _, bytes) = get_raw(&w.app, &uri, ADMIN_SESSION).await;
    assert_eq!(status, StatusCode::OK);
    assert!(bytes.starts_with(&PNG_SIGNATURE));
    assert_eq!(audits_of(w.db(), "viewed_hero_icons").await.len(), 2);

    // Admin delete is a real removal of the row and the file.
    let uri = format!("/api/hero-icons/{target}");
    let (status, body) = delete(&w.app, &uri, ADMIN_SESSION).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["deleted"], json!(1));
    assert!(!w.dir().join(format!("{target}.png")).exists());
    let row_left = w.db().get_hero_icon(&target).await.unwrap();
    assert!(row_left.is_none());
    let deleted = audits_of(w.db(), "deleted_hero_icon").await;
    let expected = (
        "member".to_string(),
        OWNER.to_string(),
        Some(format!("icon {target}")),
    );
    assert_eq!(deleted, vec![expected]);

    // An opted-out member's icon is hidden from admin reads and stays stored.
    // Admins can still delete it; see
    // `admin_deletes_an_opted_out_members_hidden_icon`.
    let image = format!("/api/hero-icons/{other_id}/image");
    let got = get(&w.app, &image, ADMIN_SESSION).await;
    assert_error(&got, StatusCode::NOT_FOUND, "icon_not_found");
    assert!(w.dir().join(format!("{other_id}.png")).exists());
    assert!(w.db().get_hero_icon(&other_id).await.unwrap().is_some());

    let (_, mine) = get(&w.app, "/api/hero-icons/mine", OWNER_SESSION).await;
    assert_eq!(mine["count"], json!(1));
    assert_eq!(files_on_disk(w.dir()).len(), 2);
    w.cleanup();
}

#[tokio::test]
async fn member_counts_are_paged_by_name() {
    let w = world().await;
    set_opt_in(&w.app, OWNER_SESSION, true).await;
    set_opt_in(&w.app, OTHER_SESSION, true).await;
    let (status, body) = upload_one(&w.app, OWNER_SESSION, "ana").await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    let uri = "/api/hero-icons/members?limit=1";
    let (status, page) = get(&w.app, uri, OFFICER_SESSION).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    // "iconother" sorts before "iconowner".
    assert_eq!(member_ids_of(&page), vec![OTHER.to_string()]);
    assert_eq!(page["data"][0]["icon_count"], json!(0));
    let cursor = page["next_cursor"]
        .as_str()
        .expect("next_cursor")
        .to_string();

    let uri = format!("/api/hero-icons/members?limit=1&cursor={cursor}");
    let (status, page) = get(&w.app, &uri, OFFICER_SESSION).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(member_ids_of(&page), vec![OWNER.to_string()]);
    assert_eq!(page["data"][0]["icon_count"], json!(1));
    assert!(page.get("next_cursor").is_none(), "{page}");
    w.cleanup();
}

#[tokio::test]
async fn icons_of_one_request_list_last_part_first() {
    let w = world().await;
    set_opt_in(&w.app, OWNER_SESSION, true).await;
    let three = icons(vec![png(16, 16), png(17, 17), png(18, 18)]);
    let (status, body) = upload(&w.app, OWNER_SESSION, "ana", &three).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    // The upload answer is in part order.
    let uploaded = ids_of(&body["icons"]);
    let widths: Vec<Value> = (0..3usize)
        .map(|i| body["icons"][i]["width"].clone())
        .collect();
    assert_eq!(widths, vec![json!(16), json!(17), json!(18)]);

    // Lists are newest first, and the last part is the newest.
    let (_, mine) = get(&w.app, "/api/hero-icons/mine", OWNER_SESSION).await;
    let mut expected = uploaded.clone();
    expected.reverse();
    assert_eq!(ids_of(&mine["icons"]), expected);
    let (_, list) = get(&w.app, "/api/hero-icons", ADMIN_SESSION).await;
    assert_eq!(ids_of(&list["data"]), expected);
    w.cleanup();
}

#[tokio::test]
async fn members_and_officers_are_refused_on_staff_routes() {
    let w = world().await;
    set_opt_in(&w.app, OWNER_SESSION, true).await;
    let forbidden = StatusCode::FORBIDDEN;
    let got = get(&w.app, "/api/hero-icons/members", OTHER_SESSION).await;
    assert_error(&got, forbidden, "Officer access required");

    let owner_icons = format!("/api/hero-icons/members/{OWNER}");
    for token in [OTHER_SESSION, OFFICER_SESSION] {
        for uri in ["/api/hero-icons", owner_icons.as_str()] {
            let got = get(&w.app, uri, token).await;
            assert_error(&got, forbidden, "Admin access required");
        }
        let got = delete(&w.app, &owner_icons, token).await;
        assert_error(&got, forbidden, "Admin access required");
    }
    w.cleanup();
}

/// Robert's rule: officers get per-member counts and nothing else of another
/// member's icons. Their own icons work like any member's.
#[tokio::test]
async fn officers_get_counts_only() {
    let w = world().await;
    set_opt_in(&w.app, OWNER_SESSION, true).await;
    set_opt_in(&w.app, OFFICER_SESSION, true).await;
    let id = first_id(&upload_one(&w.app, OWNER_SESSION, "ana").await);
    let forbidden = StatusCode::FORBIDDEN;
    let admin_required = "Admin access required";

    // Count: 200.
    let (status, counts) = get(&w.app, "/api/hero-icons/members", OFFICER_SESSION).await;
    assert_eq!(status, StatusCode::OK, "{counts}");
    let mut counted = member_ids_of(&counts);
    counted.sort();
    assert_eq!(counted, vec![OFFICER.to_string(), OWNER.to_string()]);
    let owner_row = counts["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["member_id"] == json!(OWNER))
        .expect("owner row");
    assert_eq!(owner_row["icon_count"], json!(1));

    // Image read, icon lists, single delete and delete all: 403.
    let image = format!("/api/hero-icons/{id}/image");
    let got = get(&w.app, &image, OFFICER_SESSION).await;
    assert_error(&got, forbidden, admin_required);
    let got = get(&w.app, "/api/hero-icons", OFFICER_SESSION).await;
    assert_error(&got, forbidden, admin_required);
    let owner_icons = format!("/api/hero-icons/members/{OWNER}");
    let got = get(&w.app, &owner_icons, OFFICER_SESSION).await;
    assert_error(&got, forbidden, admin_required);
    let one = format!("/api/hero-icons/{id}");
    let got = delete(&w.app, &one, OFFICER_SESSION).await;
    assert_error(&got, forbidden, admin_required);
    let got = delete(&w.app, &owner_icons, OFFICER_SESSION).await;
    assert_error(&got, forbidden, admin_required);

    // An unknown id gets the same 403, so it tells the officer nothing.
    let missing = "/api/hero-icons/0123456789abcdef0123456789abcdef";
    let got = get(&w.app, &format!("{missing}/image"), OFFICER_SESSION).await;
    assert_error(&got, forbidden, admin_required);
    let got = delete(&w.app, missing, OFFICER_SESSION).await;
    assert_error(&got, forbidden, admin_required);

    // Nothing was deleted and nothing was audited.
    assert!(w.dir().join(format!("{id}.png")).exists());
    assert!(w.db().get_hero_icon(&id).await.unwrap().is_some());
    for action in [
        "viewed_hero_icons",
        "deleted_hero_icon",
        "deleted_all_hero_icons",
    ] {
        assert!(audits_of(w.db(), action).await.is_empty(), "{action}");
    }

    // The officer's own icons: listed, read and deleted like any member's.
    let own = first_id(&upload_one(&w.app, OFFICER_SESSION, "mercy").await);
    let (_, mine) = get(&w.app, "/api/hero-icons/mine", OFFICER_SESSION).await;
    assert_eq!(ids_of(&mine["icons"]), vec![own.clone()]);
    let own_image = format!("/api/hero-icons/{own}/image");
    let (status, _, bytes) = get_raw(&w.app, &own_image, OFFICER_SESSION).await;
    assert_eq!(status, StatusCode::OK);
    assert!(bytes.starts_with(&PNG_SIGNATURE));
    let own_one = format!("/api/hero-icons/{own}");
    let (status, body) = delete(&w.app, &own_one, OFFICER_SESSION).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["deleted"], json!(1));
    assert_eq!(files_on_disk(w.dir()), vec![format!("{id}.png")]);
    w.cleanup();
}

/// Admin deletes of another member's icons, one and all, and a member's own
/// delete, side by side.
#[tokio::test]
async fn admin_delete_and_self_delete_work() {
    let w = world().await;
    set_opt_in(&w.app, OWNER_SESSION, true).await;
    set_opt_in(&w.app, OTHER_SESSION, true).await;
    let two = icons(vec![png(16, 16), png(17, 17)]);
    let (status, body) = upload(&w.app, OWNER_SESSION, "ana", &two).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let owner_ids = ids_of(&body["icons"]);
    let other_id = first_id(&upload_one(&w.app, OTHER_SESSION, "mercy").await);

    // Admin single delete: 200.
    let one = format!("/api/hero-icons/{}", owner_ids[0]);
    let (status, body) = delete(&w.app, &one, ADMIN_SESSION).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["deleted"], json!(1));
    // Admin delete all: 200.
    let owner_icons = format!("/api/hero-icons/members/{OWNER}");
    let (status, body) = delete(&w.app, &owner_icons, ADMIN_SESSION).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["deleted"], json!(1));
    assert_eq!(w.db().count_hero_icons_for_member(OWNER).await.unwrap(), 0);
    let audited = audits_of(w.db(), "deleted_hero_icon").await;
    assert_eq!(audited.len(), 1, "{audited:?}");
    assert_eq!(audited[0].1, OWNER);
    assert_eq!(audits_of(w.db(), "deleted_all_hero_icons").await.len(), 1);

    // Self delete: 200, not audited.
    let mine = format!("/api/hero-icons/{other_id}");
    let (status, body) = delete(&w.app, &mine, OTHER_SESSION).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["deleted"], json!(1));
    assert!(files_on_disk(w.dir()).is_empty());
    assert_eq!(audits_of(w.db(), "deleted_hero_icon").await.len(), 1);
    w.cleanup();
}

#[tokio::test]
async fn another_members_icon_is_404_for_read_and_delete() {
    let w = world().await;
    set_opt_in(&w.app, OWNER_SESSION, true).await;
    let id = first_id(&upload_one(&w.app, OWNER_SESSION, "ana").await);
    let not_found = StatusCode::NOT_FOUND;

    let image = format!("/api/hero-icons/{id}/image");
    let got = get(&w.app, &image, OTHER_SESSION).await;
    assert_error(&got, not_found, "icon_not_found");
    let one = format!("/api/hero-icons/{id}");
    let got = delete(&w.app, &one, OTHER_SESSION).await;
    assert_error(&got, not_found, "icon_not_found");

    // Same body as an id that does not exist.
    let missing = "/api/hero-icons/0123456789abcdef0123456789abcdef/image";
    let got = get(&w.app, missing, OWNER_SESSION).await;
    assert_error(&got, not_found, "icon_not_found");

    assert!(w.dir().join(format!("{id}.png")).exists());
    let (status, _, _) = get_raw(&w.app, &image, OWNER_SESSION).await;
    assert_eq!(status, StatusCode::OK);

    // The owner can delete their own icon.
    let (status, body) = delete(&w.app, &one, OWNER_SESSION).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["deleted"], json!(1));
    assert!(files_on_disk(w.dir()).is_empty());
    w.cleanup();
}

#[tokio::test]
async fn opting_out_hides_icons_from_staff_but_keeps_them() {
    let w = world().await;
    set_opt_in(&w.app, OWNER_SESSION, true).await;
    let two = icons(vec![png(16, 16), png(17, 17)]);
    let (status, body) = upload(&w.app, OWNER_SESSION, "ana", &two).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let uploaded = ids_of(&body["icons"]);
    let id = uploaded[0].clone();
    set_opt_in(&w.app, OWNER_SESSION, false).await;

    let image = format!("/api/hero-icons/{id}/image");
    let got = get(&w.app, &image, ADMIN_SESSION).await;
    assert_error(&got, StatusCode::NOT_FOUND, "icon_not_found");

    let uri = format!("/api/hero-icons/members/{OWNER}");
    let got = get(&w.app, &uri, ADMIN_SESSION).await;
    assert_error(&got, StatusCode::FORBIDDEN, "icon_opt_in_off");

    let (status, list) = get(&w.app, "/api/hero-icons", ADMIN_SESSION).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert!(ids_of(&list["data"]).is_empty());

    let (status, counts) = get(&w.app, "/api/hero-icons/members", OFFICER_SESSION).await;
    assert_eq!(status, StatusCode::OK, "{counts}");
    assert!(member_ids_of(&counts).is_empty(), "{counts}");

    // Admins can still delete hidden icons; that is covered in
    // `admin_deletes_an_opted_out_members_hidden_icons`. Nothing here deletes.
    let one = format!("/api/hero-icons/{id}");
    assert_eq!(files_on_disk(w.dir()).len(), 2);
    assert_eq!(w.db().count_hero_icons_for_member(OWNER).await.unwrap(), 2);
    assert!(audits_of(w.db(), "deleted_hero_icon").await.is_empty());
    assert!(audits_of(w.db(), "deleted_all_hero_icons").await.is_empty());
    assert_eq!(audits_of(w.db(), "viewed_hero_icons").await.len(), 1);

    // Not deleted: the owner still lists and reads them.
    let (_, mine) = get(&w.app, "/api/hero-icons/mine", OWNER_SESSION).await;
    assert_eq!(mine["count"], json!(2));
    assert_eq!(mine["icon_contribution_opt_in"], json!(false));
    let (status, _, _) = get_raw(&w.app, &image, OWNER_SESSION).await;
    assert_eq!(status, StatusCode::OK);

    // New uploads stop.
    let got = upload_one(&w.app, OWNER_SESSION, "ana").await;
    assert_error(&got, StatusCode::FORBIDDEN, "icon_opt_in_required");

    // The owner deletes one, then the rest, while still opted out.
    let (status, body) = delete(&w.app, &one, OWNER_SESSION).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["deleted"], json!(1));
    assert_eq!(files_on_disk(w.dir()), vec![format!("{}.png", uploaded[1])]);
    let (status, body) = delete(&w.app, "/api/hero-icons/mine", OWNER_SESSION).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["deleted"], json!(1));
    assert!(files_on_disk(w.dir()).is_empty());
    // Owner deletes are not audited.
    assert!(audits_of(w.db(), "deleted_hero_icon").await.is_empty());
    w.cleanup();
}

#[tokio::test]
async fn delete_all_removes_rows_and_files() {
    let w = world().await;
    set_opt_in(&w.app, OWNER_SESSION, true).await;
    set_opt_in(&w.app, OTHER_SESSION, true).await;
    let three = icons(vec![png(16, 16), png(17, 17), png(18, 18)]);
    let (status, body) = upload(&w.app, OWNER_SESSION, "ana", &three).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let other_id = first_id(&upload_one(&w.app, OTHER_SESSION, "ana").await);
    assert_eq!(files_on_disk(w.dir()).len(), 4);

    let (status, body) = delete(&w.app, "/api/hero-icons/mine", OWNER_SESSION).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["deleted"], json!(3));
    assert_eq!(files_on_disk(w.dir()), vec![format!("{other_id}.png")]);

    let (_, mine) = get(&w.app, "/api/hero-icons/mine", OWNER_SESSION).await;
    assert_eq!(mine["count"], json!(0));
    assert_eq!(mine["icon_contribution_opt_in"], json!(true));
    w.cleanup();
}

#[tokio::test]
async fn admin_delete_all_works_opted_in_or_out() {
    let w = world().await;
    set_opt_in(&w.app, OWNER_SESSION, true).await;
    set_opt_in(&w.app, OTHER_SESSION, true).await;
    let two = icons(vec![png(16, 16), png(17, 17)]);
    let (status, body) = upload(&w.app, OWNER_SESSION, "ana", &two).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let other_id = first_id(&upload_one(&w.app, OTHER_SESSION, "ana").await);
    set_opt_in(&w.app, OWNER_SESSION, false).await;

    // Members cannot use the admin route, not even for their own icons.
    let owner_uri = format!("/api/hero-icons/members/{OWNER}");
    let got = delete(&w.app, &owner_uri, OWNER_SESSION).await;
    assert_error(&got, StatusCode::FORBIDDEN, "Admin access required");
    assert_eq!(files_on_disk(w.dir()).len(), 3);

    // Opted in: every icon of that member goes, and the delete is audited.
    let other_uri = format!("/api/hero-icons/members/{OTHER}");
    let (status, body) = delete(&w.app, &other_uri, ADMIN_SESSION).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["deleted"], json!(1));
    assert!(!w.dir().join(format!("{other_id}.png")).exists());
    assert_eq!(files_on_disk(w.dir()).len(), 2);
    let audited = audits_of(w.db(), "deleted_all_hero_icons").await;
    let expected = (
        "member".to_string(),
        OTHER.to_string(),
        Some("1 icons".to_string()),
    );
    assert_eq!(audited, vec![expected]);

    // Nothing left: 0, and no second audit entry.
    let (status, body) = delete(&w.app, &other_uri, ADMIN_SESSION).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["deleted"], json!(0));
    assert_eq!(audits_of(w.db(), "deleted_all_hero_icons").await.len(), 1);

    // Opted out: the hidden icons go too, and the delete is audited.
    let (status, body) = delete(&w.app, &owner_uri, ADMIN_SESSION).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["deleted"], json!(2));
    assert!(files_on_disk(w.dir()).is_empty());
    assert_eq!(w.db().count_hero_icons_for_member(OWNER).await.unwrap(), 0);
    let audited = audits_of(w.db(), "deleted_all_hero_icons").await;
    let expected = (
        "member".to_string(),
        OWNER.to_string(),
        Some("2 icons".to_string()),
    );
    assert!(audited.contains(&expected), "{audited:?}");
    // The opt-in is left as it was.
    assert!(!settings_of(w.db(), OWNER).await.hero_icon_opt_in);
    w.cleanup();
}

/// Site needs this: an admin's counts list opted-out members who still have
/// hidden icons, with `opted_in` false. An officer's counts never do.
#[tokio::test]
async fn admin_counts_include_opted_out_members_with_hidden_icons() {
    let w = world().await;
    set_opt_in(&w.app, OWNER_SESSION, true).await;
    set_opt_in(&w.app, OTHER_SESSION, true).await;
    let two = icons(vec![png(16, 16), png(17, 17)]);
    let (status, body) = upload(&w.app, OWNER_SESSION, "ana", &two).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    set_opt_in(&w.app, OWNER_SESSION, false).await;
    // OTHER is opted in with no icons. The officer is opted out with none, so
    // is not listed for anyone.
    let counts_uri = "/api/hero-icons/members";

    let (status, admin) = get(&w.app, counts_uri, ADMIN_SESSION).await;
    assert_eq!(status, StatusCode::OK, "{admin}");
    let both = vec![OTHER.to_string(), OWNER.to_string()];
    assert_eq!(member_ids_of(&admin), both);
    let hidden = &admin["data"][1];
    assert_eq!(hidden["member_id"], json!(OWNER));
    assert_eq!(hidden["icon_count"], json!(2));
    assert_eq!(hidden["opted_in"], json!(false));
    assert_eq!(hidden["icon_contribution_opt_in"], json!(false));
    let shown = &admin["data"][0];
    assert_eq!(shown["icon_count"], json!(0));
    assert_eq!(shown["opted_in"], json!(true));
    assert!(admin.get("next_cursor").is_none(), "{admin}");

    // Same paged shape for admins.
    let (status, page) = get(&w.app, &format!("{counts_uri}?limit=1"), ADMIN_SESSION).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(member_ids_of(&page), vec![OTHER.to_string()]);
    let cursor = page["next_cursor"]
        .as_str()
        .expect("next_cursor")
        .to_string();
    let next = format!("{counts_uri}?limit=1&cursor={cursor}");
    let (status, page) = get(&w.app, &next, ADMIN_SESSION).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(member_ids_of(&page), vec![OWNER.to_string()]);

    // The officer's list leaves the opted-out member out.
    let (status, officer) = get(&w.app, counts_uri, OFFICER_SESSION).await;
    assert_eq!(status, StatusCode::OK, "{officer}");
    assert_eq!(member_ids_of(&officer), vec![OTHER.to_string()]);
    assert_eq!(officer["data"][0]["opted_in"], json!(true));

    // Once the admin deletes the hidden icons, the row goes away.
    let owner_icons = format!("/api/hero-icons/members/{OWNER}");
    let (status, body) = delete(&w.app, &owner_icons, ADMIN_SESSION).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["deleted"], json!(2));
    let (_, admin) = get(&w.app, counts_uri, ADMIN_SESSION).await;
    assert_eq!(member_ids_of(&admin), vec![OTHER.to_string()]);
    w.cleanup();
}

/// Robert's rule: admins delete an opted-out member's hidden icons. They still
/// cannot list or read them, and officers still cannot delete them.
#[tokio::test]
async fn admin_deletes_an_opted_out_members_hidden_icon() {
    let w = world().await;
    set_opt_in(&w.app, OWNER_SESSION, true).await;
    let two = icons(vec![png(16, 16), png(17, 17)]);
    let (status, body) = upload(&w.app, OWNER_SESSION, "ana", &two).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let uploaded = ids_of(&body["icons"]);
    let id = uploaded[0].clone();
    set_opt_in(&w.app, OWNER_SESSION, false).await;

    // Hidden from admin reads.
    let image = format!("/api/hero-icons/{id}/image");
    let got = get(&w.app, &image, ADMIN_SESSION).await;
    assert_error(&got, StatusCode::NOT_FOUND, "icon_not_found");
    let owner_icons = format!("/api/hero-icons/members/{OWNER}");
    let got = get(&w.app, &owner_icons, ADMIN_SESSION).await;
    assert_error(&got, StatusCode::FORBIDDEN, "icon_opt_in_off");

    // Officers still cannot delete it.
    let one = format!("/api/hero-icons/{id}");
    let got = delete(&w.app, &one, OFFICER_SESSION).await;
    assert_error(&got, StatusCode::FORBIDDEN, "Admin access required");
    assert_eq!(files_on_disk(w.dir()).len(), 2);

    // The admin deletes it: 200, row and file gone, audited.
    let (status, body) = delete(&w.app, &one, ADMIN_SESSION).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["deleted"], json!(1));
    assert!(!w.dir().join(format!("{id}.png")).exists());
    assert!(w.db().get_hero_icon(&id).await.unwrap().is_none());
    let audited = audits_of(w.db(), "deleted_hero_icon").await;
    let expected = (
        "member".to_string(),
        OWNER.to_string(),
        Some(format!("icon {id}")),
    );
    assert_eq!(audited, vec![expected]);

    // The other icon stays, and the owner still sees it, opted out.
    let (_, mine) = get(&w.app, "/api/hero-icons/mine", OWNER_SESSION).await;
    assert_eq!(ids_of(&mine["icons"]), vec![uploaded[1].clone()]);
    assert_eq!(mine["icon_contribution_opt_in"], json!(false));
    w.cleanup();
}

#[tokio::test]
async fn path_traversal_and_malformed_ids_are_404() {
    let w = world().await;
    set_opt_in(&w.app, OWNER_SESSION, true).await;
    let id = first_id(&upload_one(&w.app, OWNER_SESSION, "ana").await);
    let before = files_on_disk(w.dir());

    let upper = id.to_uppercase();
    let bad_ids = [
        "..%2F..%2Fetc%2Fpasswd",
        "..%2F..%2F..%2Fetc%2Fpasswd%00",
        "abc.def",
        "..",
        upper.as_str(),
    ];
    let refusals = [
        (OWNER_SESSION, StatusCode::NOT_FOUND, "icon_not_found"),
        (ADMIN_SESSION, StatusCode::NOT_FOUND, "icon_not_found"),
        (
            OFFICER_SESSION,
            StatusCode::FORBIDDEN,
            "Admin access required",
        ),
    ];
    for (token, status, code) in refusals {
        for bad in bad_ids {
            let image = format!("/api/hero-icons/{bad}/image");
            let got = get(&w.app, &image, token).await;
            assert_error(&got, status, code);
            let one = format!("/api/hero-icons/{bad}");
            let got = delete(&w.app, &one, token).await;
            assert_error(&got, status, code);
        }
    }

    assert_eq!(files_on_disk(w.dir()), before);
    let row = w.db().get_hero_icon(&id).await.unwrap();
    assert!(row.is_some());
    w.cleanup();
}

#[tokio::test]
async fn opt_in_defaults_off_round_trips_and_keeps_stats_settings() {
    let w = world().await;
    let db = w.db();
    let vis = Some(StatsVisibility::Private);
    let saved = db.upsert_member_settings(OWNER, Some("Tag"), vis).await;
    assert!(!saved.expect("stats settings").hero_icon_opt_in);

    let (status, body) = get(&w.app, "/api/hero-icons/opt-in", OWNER_SESSION).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!({ "icon_contribution_opt_in": false }));

    set_opt_in(&w.app, OWNER_SESSION, true).await;
    let (_, body) = get(&w.app, "/api/hero-icons/opt-in", OWNER_SESSION).await;
    assert_eq!(body, json!({ "icon_contribution_opt_in": true }));
    let settings = settings_of(db, OWNER).await;
    assert_eq!(settings.player_name.as_deref(), Some("Tag"));
    assert_eq!(settings.stats_visibility, StatsVisibility::Private);

    // A stats settings write keeps the opt-in.
    let saved = db.upsert_member_settings(OWNER, Some("Tag"), None).await;
    assert!(saved.expect("stats settings").hero_icon_opt_in);
    assert!(settings_of(db, OWNER).await.hero_icon_opt_in);

    set_opt_in(&w.app, OWNER_SESSION, false).await;
    let (_, body) = get(&w.app, "/api/hero-icons/opt-in", OWNER_SESSION).await;
    assert_eq!(body, json!({ "icon_contribution_opt_in": false }));

    // A member with no settings row gets one with public stats.
    set_opt_in(&w.app, OTHER_SESSION, true).await;
    let settings = settings_of(db, OTHER).await;
    assert_eq!(settings.stats_visibility, StatsVisibility::Public);
    assert!(settings.hero_icon_opt_in);
    w.cleanup();
}

#[tokio::test]
async fn disabled_store_answers_503() {
    let name = format!("scuffed-hero-icons-off-{}", uuid::Uuid::new_v4().simple());
    let dir = std::env::temp_dir().join(name);
    let w = world_with(HeroIconStore::new(dir, false)).await;
    let disabled = StatusCode::SERVICE_UNAVAILABLE;

    let got = get(&w.app, "/api/hero-icons/mine", OWNER_SESSION).await;
    assert_error(&got, disabled, "hero_icons_disabled");
    let got = upload_one(&w.app, OWNER_SESSION, "ana").await;
    assert_error(&got, disabled, "hero_icons_disabled");
    let got = get(&w.app, "/api/hero-icons", ADMIN_SESSION).await;
    assert_error(&got, disabled, "hero_icons_disabled");
    let got = delete(&w.app, "/api/hero-icons/mine", OWNER_SESSION).await;
    assert_error(&got, disabled, "hero_icons_disabled");

    // The opt-in only touches the flag, so it works with the store off.
    let (status, body) = get(&w.app, "/api/hero-icons/opt-in", OWNER_SESSION).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!({ "icon_contribution_opt_in": false }));
    set_opt_in(&w.app, OWNER_SESSION, true).await;
    set_opt_in(&w.app, OWNER_SESSION, false).await;
    assert!(!settings_of(w.db(), OWNER).await.hero_icon_opt_in);
    assert!(!w.dir().exists());
}

#[tokio::test]
async fn daemon_sync_token_is_not_a_session() {
    let w = world().await;
    let daemon = "icons-owner-daemon-token";
    w.db()
        .create_daemon_token(OWNER, daemon, "tracker")
        .await
        .expect("daemon token");
    set_opt_in(&w.app, OWNER_SESSION, true).await;
    let unauthorized = StatusCode::UNAUTHORIZED;

    let got = get(&w.app, "/api/hero-icons/mine", daemon).await;
    assert_error(&got, unauthorized, "Session expired or invalid");
    let parts = icons(vec![png(16, 16)]);
    let resp = upload_raw(&w.app, Some(daemon), "/api/hero-icons?hero=ana", &parts).await;
    let got = read_json(resp).await;
    assert_error(&got, unauthorized, "Session expired or invalid");
    assert!(files_on_disk(w.dir()).is_empty());
    w.cleanup();
}

#[tokio::test]
async fn inactive_member_is_refused_and_hidden_from_staff() {
    let w = world().await;
    set_opt_in(&w.app, OWNER_SESSION, true).await;
    let id = first_id(&upload_one(&w.app, OWNER_SESSION, "ana").await);
    deactivate(w.db(), OWNER).await;
    let forbidden = StatusCode::FORBIDDEN;

    let got = upload_one(&w.app, OWNER_SESSION, "ana").await;
    assert_error(&got, forbidden, "Membership inactive");
    let got = get(&w.app, "/api/hero-icons/mine", OWNER_SESSION).await;
    assert_error(&got, forbidden, "Membership inactive");

    // Still opted in, but inactive: not listed, counted, read or deletable.
    let (status, list) = get(&w.app, "/api/hero-icons", ADMIN_SESSION).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert!(ids_of(&list["data"]).is_empty());
    let (status, counts) = get(&w.app, "/api/hero-icons/members", OFFICER_SESSION).await;
    assert_eq!(status, StatusCode::OK, "{counts}");
    assert!(member_ids_of(&counts).is_empty(), "{counts}");

    let not_found = StatusCode::NOT_FOUND;
    let uri = format!("/api/hero-icons/members/{OWNER}");
    let got = get(&w.app, &uri, ADMIN_SESSION).await;
    assert_error(&got, not_found, "member_not_found");
    let got = delete(&w.app, &uri, ADMIN_SESSION).await;
    assert_error(&got, not_found, "member_not_found");
    let image = format!("/api/hero-icons/{id}/image");
    let got = get(&w.app, &image, ADMIN_SESSION).await;
    assert_error(&got, not_found, "icon_not_found");
    let one = format!("/api/hero-icons/{id}");
    let got = delete(&w.app, &one, ADMIN_SESSION).await;
    assert_error(&got, not_found, "icon_not_found");
    assert_eq!(files_on_disk(w.dir()), vec![format!("{id}.png")]);

    // An unknown member id gets the same answer.
    let got = get(
        &w.app,
        "/api/hero-icons/members/nosuchmember",
        ADMIN_SESSION,
    )
    .await;
    assert_error(&got, not_found, "member_not_found");
    let got = delete(
        &w.app,
        "/api/hero-icons/members/nosuchmember",
        ADMIN_SESSION,
    )
    .await;
    assert_error(&got, not_found, "member_not_found");
    w.cleanup();
}
