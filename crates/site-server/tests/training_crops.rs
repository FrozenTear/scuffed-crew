//! Opt-in training crops from the tracker (`/api/training-crops`, alpha.3
//! item 14): sync token auth on exactly upload, own consent read and own
//! deletes (setting consent is session only), per-kind consent, `full_screen`
//! refusal (upload and consent), metadata, per-kind PNG bounds, caps, the
//! per-kind and per-IP limiters, officer counts, admin reads, and the hero
//! icon routes still seeing only hero icons.
//!
//! Members: an owner who contributes, another member, an officer and an
//! admin, each with a session and a tracker sync token. Crops are written to
//! a fresh directory under the system temp dir, which each test removes.

use std::io::Cursor;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Method, Request, StatusCode, header};
use axum::response::Response;
use chrono::Utc;
use http_body_util::BodyExt;
use image::{ImageFormat, Rgba, RgbaImage};
use serde_json::{Value, json};
use tower::ServiceExt;

use scuffed_auth::SessionConfig;
use scuffed_auth::crypto::hash_session_token;
use scuffed_db::migrations::run_migrations;
use scuffed_db::{Database, NewTrainingCrop, TrainingCrop};
use scuffed_site_server::create_router;
use scuffed_site_server::hero_icons::HeroIconStore;
use scuffed_site_server::state::{AppState, OAuthConfig};
use scuffed_types::{
    CropKind, HERO_ICON_MAX_FILES_PER_UPLOAD, HERO_ICON_MAX_REQUEST_BYTES, HERO_ICON_RATE_BURST,
    MAP_NAME_MAX_BYTES, MAP_NAME_MAX_PER_MEMBER, STATS_UPLOAD_IP_BURST, TrainingCropMeta,
};

const OWNER: &str = "cropowner";
const OTHER: &str = "cropother";
const OFFICER: &str = "cropofficer";
const ADMIN: &str = "cropadmin";

const OWNER_SESSION: &str = "crops-owner-session";
const OTHER_SESSION: &str = "crops-other-session";
const OFFICER_SESSION: &str = "crops-officer-session";
const ADMIN_SESSION: &str = "crops-admin-session";

const OWNER_TOKEN: &str = "crops-owner-sync-token";
const OTHER_TOKEN: &str = "crops-other-sync-token";
const OFFICER_TOKEN: &str = "crops-officer-sync-token";
const ADMIN_TOKEN: &str = "crops-admin-sync-token";

const BOUNDARY: &str = "training-crop-test-boundary";
const TRACKER_UA: &str = "scuffed-stat-tracker/0.6.0-alpha.3";

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

/// Seed a user, an active member, a session and a sync token. The display
/// name is the member id.
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

    async fn rows_of(&self, member: &str) -> Vec<TrainingCrop> {
        self.db()
            .list_training_crops_for_member(member, None, None)
            .await
            .expect("rows")
    }
}

async fn world_with(hero_icons: HeroIconStore) -> World {
    let state = test_state(hero_icons).await;
    seed_member(&state.db, OWNER, "member", OWNER_SESSION, OWNER_TOKEN).await;
    seed_member(&state.db, OTHER, "member", OTHER_SESSION, OTHER_TOKEN).await;
    seed_member(
        &state.db,
        OFFICER,
        "officer",
        OFFICER_SESSION,
        OFFICER_TOKEN,
    )
    .await;
    seed_member(&state.db, ADMIN, "admin", ADMIN_SESSION, ADMIN_TOKEN).await;
    let app = create_router(state.clone());
    World { app, state }
}

async fn world() -> World {
    world_with(HeroIconStore::in_temp_dir()).await
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let canvas = RgbaImage::from_pixel(width, height, Rgba([30, 140, 220, 255]));
    let mut out = Cursor::new(Vec::new());
    canvas.write_to(&mut out, ImageFormat::Png).unwrap();
    out.into_inner()
}

/// A PNG of random pixels, so it does not compress: about `width * height * 4`
/// bytes.
fn noise_png(width: u32, height: u32, seed: u32) -> Vec<u8> {
    let mut state = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
    let canvas = RgbaImage::from_fn(width, height, |_, _| {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        let [a, b, c, d] = state.to_le_bytes();
        Rgba([a, b, c, d | 1])
    });
    let mut out = Cursor::new(Vec::new());
    canvas.write_to(&mut out, ImageFormat::Png).unwrap();
    out.into_inner()
}

/// One multipart part: name and bytes.
type Part = (&'static str, Vec<u8>);

fn meta(value: Value) -> Part {
    ("meta", serde_json::to_vec(&value).unwrap())
}

fn crop(width: u32, height: u32) -> Part {
    ("crop", png(width, height))
}

/// `pairs` meta and crop pairs with the same metadata.
fn pairs(count: usize, value: Value) -> Vec<Part> {
    let mut parts = Vec::new();
    for _ in 0..count {
        parts.push(meta(value.clone()));
        parts.push(crop(40, 16));
    }
    parts
}

fn multipart_body(parts: &[Part]) -> Vec<u8> {
    let mut body = Vec::new();
    for (name, bytes) in parts {
        body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
        let head = if *name == "crop" {
            format!(
                "Content-Disposition: form-data; name=\"{name}\"; filename=\"crop.png\"\r\n\
                 Content-Type: image/png\r\n\r\n"
            )
        } else {
            format!(
                "Content-Disposition: form-data; name=\"{name}\"\r\n\
                 Content-Type: application/json\r\n\r\n"
            )
        };
        body.extend_from_slice(head.as_bytes());
        body.extend_from_slice(bytes);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
    body
}

/// A fresh client address per request, so the per-IP training crop limit
/// only trips in the test that is about it.
fn next_ip() -> IpAddr {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // 198.18.0.0/15, the benchmarking range.
    let n = NEXT.fetch_add(1, Ordering::Relaxed) % (1 << 17);
    IpAddr::V4(Ipv4Addr::from(0xC612_0000u32 + n))
}

fn base_request(method: Method, uri: &str, token: Option<&str>) -> axum::http::request::Builder {
    base_request_from(next_ip(), method, uri, token)
}

fn base_request_from(
    ip: IpAddr,
    method: Method,
    uri: &str,
    token: Option<&str>,
) -> axum::http::request::Builder {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .extension(ConnectInfo(SocketAddr::new(ip, 9)))
        .header(header::USER_AGENT, TRACKER_UA);
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
    let resp = app
        .clone()
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    read_json(resp).await
}

async fn get(app: &axum::Router, uri: &str, token: &str) -> Got {
    call(app, Method::GET, uri, Some(token), None).await
}

async fn delete(app: &axum::Router, uri: &str, token: &str) -> Got {
    call(app, Method::DELETE, uri, Some(token), None).await
}

async fn upload(app: &axum::Router, token: &str, kind: &str, parts: &[Part]) -> Got {
    let uri = format!("/api/training-crops?kind={kind}");
    let request = base_request(Method::POST, &uri, Some(token))
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .body(Body::from(multipart_body(parts)))
        .unwrap();
    read_json(app.clone().oneshot(request).await.unwrap()).await
}

async fn set_consent(app: &axum::Router, token: &str, body: Value) -> Value {
    let uri = "/api/training-crops/consent";
    let (status, got) = call(app, Method::PUT, uri, Some(token), Some(body)).await;
    assert_eq!(status, StatusCode::OK, "consent put: {got}");
    got
}

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

fn crop_ids(got: &Got) -> Vec<String> {
    assert_eq!(got.0, StatusCode::CREATED, "{}", got.1);
    got.1["crops"]
        .as_array()
        .expect("crops")
        .iter()
        .map(|c| c["id"].as_str().expect("id").to_string())
        .collect()
}

fn assert_error(got: &Got, status: StatusCode, code: &str) {
    assert_eq!(got.0, status, "{}", got.1);
    assert_eq!(got.1["error"], json!(code), "{}", got.1);
}

fn full_meta() -> Value {
    json!({
        "screen": "scoreboard",
        "slot_type": "ally",
        "slot_index": 3,
        "game_token": "4f2a9c1e7b0d3e5f6a7b8c9d0e1f2a3b",
        "score": 0.93,
        "runner_up": 0.41,
        "margin": 0.52,
        "icon_set_version": "2026.11.1",
        "spot_version": "v3",
        "vote_card_filled": true,
        "locked_in": false,
        "screen_width": 2560,
        "screen_height": 1440,
        "ui_scale": 100.0,
        "colorblind_filter": "deuteranopia",
        "found_spot": { "x": 10, "y": 20, "w": 40, "h": 16 },
        "found_confidence": 0.8
    })
}

/// Seed `count` crop rows (no files) of `kind` for `member`.
async fn seed_rows(db: &Database, member: &str, kind: CropKind, count: u32, prefix: u32) {
    for i in 0..count {
        let id = format!("{prefix:08x}{i:024x}");
        let row = NewTrainingCrop {
            id: id.clone(),
            member_id: member.to_string(),
            kind,
            hero: (kind == CropKind::HeroIcon).then(|| "Ana".to_string()),
            file_name: format!("{id}.png"),
            size_bytes: 10,
            width: 40,
            height: 16,
            created_at: Utc::now(),
            tracker_version: None,
            meta: TrainingCropMeta::default(),
        };
        db.insert_training_crop(&row).await.expect("seed crop row");
    }
}

#[tokio::test]
async fn sync_token_upload_with_consent_stores_every_field() {
    let w = world().await;
    let consent = set_consent(&w.app, OWNER_SESSION, json!({ "stat_digits": true })).await;
    assert_eq!(
        consent,
        json!({ "hero_icon": false, "stat_digits": true, "map_name": false })
    );
    let (status, read) = get(&w.app, "/api/training-crops/consent", OWNER_TOKEN).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(read, consent);

    let parts = vec![
        meta(full_meta()),
        crop(40, 16),
        meta(json!({})),
        crop(24, 12),
    ];
    let got = upload(&w.app, OWNER_TOKEN, "stat_digits", &parts).await;
    let ids = crop_ids(&got);
    assert_eq!(ids.len(), 2);
    assert_eq!(got.1["kind"], json!("stat_digits"));
    let first = &got.1["crops"][0];
    assert_eq!(first["kind"], json!("stat_digits"));
    assert_eq!(first["tracker_version"], json!("0.6.0-alpha.3"));
    assert_eq!(first["meta"], full_meta());
    assert_eq!(first["width"], json!(40));
    assert_eq!(first["height"], json!(16));
    assert_eq!(
        first["thumbnail_url"],
        json!(format!("/api/training-crops/{}/image", ids[0]))
    );
    assert_eq!(got.1["crops"][1]["meta"], json!({}));
    assert!(got.1["crops"][1].get("hero").is_none());

    let stored = w.rows_of(OWNER).await;
    assert_eq!(stored.len(), 2);
    let row = stored.iter().find(|c| c.id == ids[0]).expect("row");
    assert_eq!(row.kind, CropKind::StatDigits);
    assert_eq!(
        row.meta.game_token.as_deref(),
        Some("4f2a9c1e7b0d3e5f6a7b8c9d0e1f2a3b")
    );
    assert_eq!(row.meta.found_spot.map(|s| s.w), Some(40));
    assert_eq!(row.meta.screen_width, Some(2560));
    let mut expected: Vec<String> = ids.iter().map(|id| format!("{id}.png")).collect();
    expected.sort();
    assert_eq!(files_on_disk(w.dir()), expected);

    // The owner reads them back with a session, newest (last part) first.
    let (status, mine) = get(&w.app, "/api/training-crops/mine", OWNER_SESSION).await;
    assert_eq!(status, StatusCode::OK, "{mine}");
    assert_eq!(mine["consent"]["stat_digits"], json!(true));
    let listed: Vec<&str> = mine["crops"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    assert_eq!(listed, vec![ids[1].as_str(), ids[0].as_str()]);
    let kinds = mine["kinds"].as_array().unwrap();
    assert_eq!(kinds.len(), 3, "full_screen is never stored");
    assert_eq!(kinds[1]["kind"], json!("stat_digits"));
    assert_eq!(kinds[1]["count"], json!(2));
    assert_eq!(kinds[1]["max"], json!(1000));

    let uri = format!("/api/training-crops/{}/image", ids[0]);
    let request = base_request(Method::GET, &uri, Some(OWNER_SESSION))
        .body(Body::empty())
        .unwrap();
    let resp = w.app.clone().oneshot(request).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers()[header::CONTENT_TYPE], "image/png");
    w.cleanup();
}

#[tokio::test]
async fn no_tracker_user_agent_stores_no_version() {
    let w = world().await;
    set_consent(&w.app, OWNER_SESSION, json!({ "map_name": true })).await;
    let request = Request::builder()
        .method(Method::POST)
        .uri("/api/training-crops?kind=map_name")
        .extension(ConnectInfo(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(203, 0, 113, 60)),
            9,
        )))
        .header(header::AUTHORIZATION, format!("Bearer {OWNER_TOKEN}"))
        .header(header::USER_AGENT, "curl/8.0")
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .body(Body::from(multipart_body(&pairs(1, json!({})))))
        .unwrap();
    let got = read_json(w.app.clone().oneshot(request).await.unwrap()).await;
    assert_eq!(got.0, StatusCode::CREATED, "{}", got.1);
    assert!(got.1["crops"][0].get("tracker_version").is_none());
    assert_eq!(w.rows_of(OWNER).await[0].tracker_version, None);
    w.cleanup();
}

#[tokio::test]
async fn sync_token_is_refused_on_session_only_routes() {
    let w = world().await;
    set_consent(&w.app, OWNER_SESSION, json!({ "stat_digits": true })).await;
    let ids = crop_ids(&upload(&w.app, OWNER_SESSION, "stat_digits", &pairs(1, json!({}))).await);
    let unauthorized = StatusCode::UNAUTHORIZED;
    let image = format!("/api/training-crops/{}/image", ids[0]);

    // Admin and officer routes, the own list and images need a session.
    for (uri, token) in [
        ("/api/training-crops", ADMIN_TOKEN),
        ("/api/training-crops/members", ADMIN_TOKEN),
        ("/api/training-crops/members", OFFICER_TOKEN),
        ("/api/training-crops/mine", OWNER_TOKEN),
        (image.as_str(), OWNER_TOKEN),
        (image.as_str(), ADMIN_TOKEN),
        ("/api/hero-icons", ADMIN_TOKEN),
        ("/api/hero-icons/members", OFFICER_TOKEN),
        ("/api/hero-icons/mine", OWNER_TOKEN),
        ("/api/hero-icons/opt-in", OWNER_TOKEN),
    ] {
        let got = get(&w.app, uri, token).await;
        assert_eq!(got.0, unauthorized, "{uri}: {}", got.1);
    }
    let got = delete(
        &w.app,
        &format!("/api/hero-icons/members/{OWNER}"),
        ADMIN_TOKEN,
    )
    .await;
    assert_eq!(got.0, unauthorized, "{}", got.1);

    // On the single delete a sync token never acts as staff: an admin's
    // token gets the same 404 as anyone for another member's crop.
    let uri = format!("/api/training-crops/{}", ids[0]);
    let got = delete(&w.app, &uri, ADMIN_TOKEN).await;
    assert_error(&got, StatusCode::NOT_FOUND, "crop_not_found");
    let got = delete(&w.app, &uri, OFFICER_TOKEN).await;
    assert_error(&got, StatusCode::NOT_FOUND, "crop_not_found");
    let got = delete(&w.app, &uri, OTHER_TOKEN).await;
    assert_error(&got, StatusCode::NOT_FOUND, "crop_not_found");
    assert_eq!(w.rows_of(OWNER).await.len(), 1);
    assert_eq!(files_on_disk(w.dir()).len(), 1);

    // A bad token is the opaque 401, like stat reports.
    let got = get(&w.app, "/api/training-crops/consent", "not-a-token").await;
    assert_error(&got, unauthorized, "Unauthorized");
    w.cleanup();
}

#[tokio::test]
async fn upload_without_consent_is_403_and_stores_nothing() {
    let w = world().await;
    // Consent to stat digits only, then forge map name and hero icon uploads.
    set_consent(&w.app, OWNER_SESSION, json!({ "stat_digits": true })).await;
    let got = upload(&w.app, OWNER_TOKEN, "map_name", &pairs(2, json!({}))).await;
    assert_error(&got, StatusCode::FORBIDDEN, "crop_consent_required");
    let hero = pairs(1, json!({ "hero": "ana" }));
    let got = upload(&w.app, OWNER_TOKEN, "hero_icon", &hero).await;
    assert_error(&got, StatusCode::FORBIDDEN, "crop_consent_required");
    // No consent row at all.
    let got = upload(&w.app, OTHER_SESSION, "stat_digits", &pairs(1, json!({}))).await;
    assert_error(&got, StatusCode::FORBIDDEN, "crop_consent_required");
    assert!(w.rows_of(OWNER).await.is_empty());
    assert!(w.rows_of(OTHER).await.is_empty());
    assert!(files_on_disk(w.dir()).is_empty());
    w.cleanup();
}

#[tokio::test]
async fn full_screen_is_refused_and_stored_nowhere() {
    let w = world().await;
    // Its consent cannot be stored: the field is unknown, so the body is
    // refused like any other unknown kind and nothing changes.
    for body in [
        json!({ "full_screen": true }),
        json!({ "full_screen": false }),
        json!({ "map_name": true, "full_screen": true }),
    ] {
        let got = call(
            &w.app,
            Method::PUT,
            "/api/training-crops/consent",
            Some(OWNER_SESSION),
            Some(body),
        )
        .await;
        assert_error(&got, StatusCode::BAD_REQUEST, "invalid_consent");
    }
    let (_, consent) = get(&w.app, "/api/training-crops/consent", OWNER_SESSION).await;
    assert_eq!(
        consent,
        json!({ "hero_icon": false, "stat_digits": false, "map_name": false })
    );
    assert!(consent.get("full_screen").is_none());
    let (_, mine) = get(&w.app, "/api/training-crops/mine", OWNER_SESSION).await;
    assert!(mine["consent"].get("full_screen").is_none());
    for token in [OWNER_TOKEN, OWNER_SESSION, OTHER_SESSION] {
        let got = upload(&w.app, token, "full_screen", &pairs(1, json!({}))).await;
        assert_error(
            &got,
            StatusCode::UNPROCESSABLE_ENTITY,
            "full_screen_not_accepted",
        );
    }
    assert!(w.rows_of(OWNER).await.is_empty());
    assert!(files_on_disk(w.dir()).is_empty());
    assert!(!w.dir().exists(), "the directory is not even created");
    w.cleanup();
}

#[tokio::test]
async fn own_deletes_via_sync_token_remove_rows_and_files() {
    let w = world().await;
    set_consent(
        &w.app,
        OWNER_SESSION,
        json!({ "stat_digits": true, "map_name": true }),
    )
    .await;
    let digits = crop_ids(&upload(&w.app, OWNER_TOKEN, "stat_digits", &pairs(3, json!({}))).await);
    let maps = crop_ids(&upload(&w.app, OWNER_TOKEN, "map_name", &pairs(2, json!({}))).await);
    set_consent(&w.app, OTHER_SESSION, json!({ "map_name": true })).await;
    let theirs = crop_ids(&upload(&w.app, OTHER_TOKEN, "map_name", &pairs(1, json!({}))).await);
    assert_eq!(files_on_disk(w.dir()).len(), 6);

    // Single delete of an own crop.
    let got = delete(
        &w.app,
        &format!("/api/training-crops/{}", digits[0]),
        OWNER_TOKEN,
    )
    .await;
    assert_eq!(got.0, StatusCode::OK, "{}", got.1);
    assert_eq!(got.1, json!({ "deleted": 1 }));
    assert!(!w.dir().join(format!("{}.png", digits[0])).exists());
    // Gone is 404 the second time.
    let got = delete(
        &w.app,
        &format!("/api/training-crops/{}", digits[0]),
        OWNER_TOKEN,
    )
    .await;
    assert_error(&got, StatusCode::NOT_FOUND, "crop_not_found");
    // Another member's crop is 404.
    let got = delete(
        &w.app,
        &format!("/api/training-crops/{}", theirs[0]),
        OWNER_TOKEN,
    )
    .await;
    assert_error(&got, StatusCode::NOT_FOUND, "crop_not_found");

    // Delete-all of one kind, then of everything.
    let got = delete(
        &w.app,
        "/api/training-crops/mine?kind=map_name",
        OWNER_TOKEN,
    )
    .await;
    assert_eq!(got.1, json!({ "deleted": 2 }));
    for id in &maps {
        assert!(!w.dir().join(format!("{id}.png")).exists());
    }
    let got = delete(&w.app, "/api/training-crops/mine?kind=bogus", OWNER_TOKEN).await;
    assert_error(&got, StatusCode::BAD_REQUEST, "unknown_crop_kind");
    let got = delete(&w.app, "/api/training-crops/mine", OWNER_TOKEN).await;
    assert_eq!(got.1, json!({ "deleted": 2 }));
    assert!(w.rows_of(OWNER).await.is_empty());
    assert_eq!(files_on_disk(w.dir()), vec![format!("{}.png", theirs[0])]);
    // Consent is kept.
    let (_, consent) = get(&w.app, "/api/training-crops/consent", OWNER_TOKEN).await;
    assert_eq!(consent["map_name"], json!(true));
    w.cleanup();
}

#[tokio::test]
async fn caps_are_per_kind() {
    let w = world().await;
    set_consent(
        &w.app,
        OWNER_SESSION,
        json!({ "stat_digits": true, "map_name": true }),
    )
    .await;
    seed_rows(
        w.db(),
        OWNER,
        CropKind::MapName,
        MAP_NAME_MAX_PER_MEMBER - 1,
        1,
    )
    .await;
    let limit = StatusCode::CONFLICT;

    // Two would pass the cap: none of it is stored.
    let got = upload(&w.app, OWNER_TOKEN, "map_name", &pairs(2, json!({}))).await;
    assert_error(&got, limit, "crop_limit_reached");
    assert!(files_on_disk(w.dir()).is_empty());
    // Exactly to the cap is fine.
    let got = upload(&w.app, OWNER_TOKEN, "map_name", &pairs(1, json!({}))).await;
    assert_eq!(got.0, StatusCode::CREATED, "{}", got.1);
    // At the cap the request is refused before the body is read.
    let got = upload(&w.app, OWNER_TOKEN, "map_name", &pairs(1, json!({}))).await;
    assert_error(&got, limit, "crop_limit_reached");
    // Another kind has its own cap.
    let got = upload(&w.app, OWNER_TOKEN, "stat_digits", &pairs(1, json!({}))).await;
    assert_eq!(got.0, StatusCode::CREATED, "{}", got.1);
    assert_eq!(files_on_disk(w.dir()).len(), 2);

    // Too many crops in one request is 413.
    let got = upload(
        &w.app,
        OWNER_TOKEN,
        "stat_digits",
        &pairs(HERO_ICON_MAX_FILES_PER_UPLOAD + 1, json!({})),
    )
    .await;
    assert_error(
        &got,
        StatusCode::PAYLOAD_TOO_LARGE,
        "too_many_crops_in_request",
    );
    assert_eq!(files_on_disk(w.dir()).len(), 2);
    w.cleanup();
}

#[tokio::test]
async fn rate_limit_is_per_kind_and_hero_icons_share_the_old_bucket() {
    let w = world().await;
    // Without consent every request is 403, but each still spends a token.
    for i in 0..HERO_ICON_RATE_BURST {
        let got = upload(&w.app, OWNER_TOKEN, "stat_digits", &pairs(1, json!({}))).await;
        assert_eq!(got.0, StatusCode::FORBIDDEN, "request {i}: {}", got.1);
    }
    let got = upload(&w.app, OWNER_TOKEN, "stat_digits", &pairs(1, json!({}))).await;
    assert_eq!(got.0, StatusCode::TOO_MANY_REQUESTS, "{}", got.1);
    assert_eq!(got.1["error"], json!("rate_limited"));
    // Another kind has its own bucket.
    let got = upload(&w.app, OWNER_TOKEN, "map_name", &pairs(1, json!({}))).await;
    assert_error(&got, StatusCode::FORBIDDEN, "crop_consent_required");

    // hero_icon via the new route spends the same bucket as /api/hero-icons.
    for _ in 0..HERO_ICON_RATE_BURST {
        let got = upload(&w.app, OTHER_TOKEN, "hero_icon", &pairs(1, json!({}))).await;
        assert_eq!(got.0, StatusCode::FORBIDDEN, "{}", got.1);
    }
    let request = base_request(
        Method::POST,
        "/api/hero-icons?hero=ana",
        Some(OTHER_SESSION),
    )
    .header(
        header::CONTENT_TYPE,
        format!("multipart/form-data; boundary={BOUNDARY}"),
    )
    .body(Body::from(multipart_body(&[("icon", png(16, 16))])))
    .unwrap();
    let resp = w.app.clone().oneshot(request).await.unwrap();
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    w.cleanup();
}

#[tokio::test]
async fn body_shape_and_meta_errors() {
    let w = world().await;
    // Cases rotate over four members so the per-kind limiter is not hit.
    let tokens = [OWNER_TOKEN, OTHER_TOKEN, OFFICER_TOKEN, ADMIN_TOKEN];
    for session in [OWNER_SESSION, OTHER_SESSION, OFFICER_SESSION, ADMIN_SESSION] {
        let both = json!({ "stat_digits": true, "hero_icon": true });
        set_consent(&w.app, session, both).await;
    }
    let bad = StatusCode::BAD_REQUEST;
    let cases: Vec<(&str, Vec<Part>, StatusCode, &str)> = vec![
        ("stat_digits", vec![crop(16, 16)], bad, "unexpected_field"),
        (
            "stat_digits",
            vec![meta(json!({}))],
            bad,
            "unexpected_field",
        ),
        (
            "stat_digits",
            vec![meta(json!({})), meta(json!({})), crop(16, 16)],
            bad,
            "unexpected_field",
        ),
        (
            "stat_digits",
            vec![("icon", png(16, 16))],
            bad,
            "unexpected_field",
        ),
        ("stat_digits", vec![], bad, "no_crops"),
        (
            "stat_digits",
            vec![meta(json!({ "kind": "map_name" })), crop(16, 16)],
            bad,
            "invalid_crop_meta",
        ),
        (
            "stat_digits",
            vec![
                meta(json!({ "found_confidence": 1.5, "found_spot": {"x":0,"y":0,"w":1,"h":1} })),
                crop(16, 16),
            ],
            bad,
            "invalid_crop_meta",
        ),
        (
            "stat_digits",
            vec![meta(json!({ "game_token": "x".repeat(65) })), crop(16, 16)],
            bad,
            "invalid_crop_meta",
        ),
        (
            "stat_digits",
            vec![("meta", vec![b' '; 5000]), crop(16, 16)],
            bad,
            "invalid_crop_meta",
        ),
        (
            "hero_icon",
            vec![meta(json!({})), crop(16, 16)],
            bad,
            "unknown_hero",
        ),
        (
            "stat_digits",
            vec![meta(json!({})), ("crop", b"GIF89a-not-a-png".to_vec())],
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "not_png",
        ),
        (
            "stat_digits",
            vec![meta(json!({})), crop(300, 16)],
            bad,
            "icon_bad_dimensions",
        ),
        (
            "bogus",
            vec![meta(json!({})), crop(16, 16)],
            bad,
            "unknown_crop_kind",
        ),
        (
            "",
            vec![meta(json!({})), crop(16, 16)],
            bad,
            "unknown_crop_kind",
        ),
    ];
    for (index, (kind, parts, status, code)) in cases.into_iter().enumerate() {
        let got = upload(&w.app, tokens[index % tokens.len()], kind, &parts).await;
        assert_eq!(got.0, status, "{kind} {code}: {}", got.1);
        assert_eq!(got.1["error"], json!(code), "{kind}: {}", got.1);
    }
    assert!(files_on_disk(w.dir()).is_empty());
    for member in [OWNER, OTHER, OFFICER, ADMIN] {
        assert!(w.rows_of(member).await.is_empty());
    }

    // A bad consent body is 400 and changes nothing.
    let uri = "/api/training-crops/consent";
    let got = call(
        &w.app,
        Method::PUT,
        uri,
        Some(OWNER_SESSION),
        Some(json!({ "replays": true })),
    )
    .await;
    assert_error(&got, bad, "invalid_consent");
    let got = call(
        &w.app,
        Method::PUT,
        uri,
        Some(OWNER_SESSION),
        Some(json!({ "map_name": "yes" })),
    )
    .await;
    assert_error(&got, bad, "invalid_consent");
    let (_, consent) = get(&w.app, uri, OWNER_TOKEN).await;
    assert_eq!(consent["map_name"], json!(false));
    w.cleanup();
}

#[tokio::test]
async fn hero_icon_kind_is_the_hero_icon_store() {
    let w = world().await;
    // The old opt-in route turns on hero_icon consent, and only that.
    let body = json!({ "icon_contribution_opt_in": true });
    let got = call(
        &w.app,
        Method::PUT,
        "/api/hero-icons/opt-in",
        Some(OWNER_SESSION),
        Some(body),
    )
    .await;
    assert_eq!(got.0, StatusCode::OK, "{}", got.1);
    let (_, consent) = get(&w.app, "/api/training-crops/consent", OWNER_TOKEN).await;
    assert_eq!(
        consent,
        json!({ "hero_icon": true, "stat_digits": false, "map_name": false })
    );
    set_consent(&w.app, OWNER_SESSION, json!({ "stat_digits": true })).await;

    let hero = upload(
        &w.app,
        OWNER_TOKEN,
        "hero_icon",
        &pairs(1, json!({ "hero": "lucio" })),
    )
    .await;
    let hero_ids = crop_ids(&hero);
    // Stored and answered as the hero key; the hero icon routes still give
    // the display name.
    assert_eq!(hero.1["crops"][0]["hero"], json!("lucio"));
    let digits = crop_ids(&upload(&w.app, OWNER_TOKEN, "stat_digits", &pairs(1, json!({}))).await);

    // The hero icon routes see the hero icon and nothing else.
    let (_, mine) = get(&w.app, "/api/hero-icons/mine", OWNER_SESSION).await;
    assert_eq!(mine["count"], json!(1));
    assert_eq!(mine["icons"][0]["id"], json!(hero_ids[0]));
    assert_eq!(mine["icons"][0]["hero"], json!("Lúcio"));
    let uri = format!("/api/hero-icons/{}/image", digits[0]);
    let got = get(&w.app, &uri, OWNER_SESSION).await;
    assert_error(&got, StatusCode::NOT_FOUND, "icon_not_found");
    let got = delete(
        &w.app,
        &format!("/api/hero-icons/{}", digits[0]),
        OWNER_SESSION,
    )
    .await;
    assert_error(&got, StatusCode::NOT_FOUND, "icon_not_found");
    let (_, counts) = get(&w.app, "/api/hero-icons/members", ADMIN_SESSION).await;
    assert_eq!(counts["data"][0]["icon_count"], json!(1));
    // Old delete-all removes hero icons only.
    let got = delete(&w.app, "/api/hero-icons/mine", OWNER_SESSION).await;
    assert_eq!(got.1, json!({ "deleted": 1 }));
    let rows = w.rows_of(OWNER).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, digits[0]);
    assert_eq!(files_on_disk(w.dir()), vec![format!("{}.png", digits[0])]);

    // Withdrawing hero_icon consent is the old opt-out.
    set_consent(&w.app, OWNER_SESSION, json!({ "hero_icon": false })).await;
    let (_, old) = get(&w.app, "/api/hero-icons/opt-in", OWNER_SESSION).await;
    assert_eq!(old, json!({ "icon_contribution_opt_in": false }));
    w.cleanup();
}

#[tokio::test]
async fn legacy_opt_in_rows_become_hero_icon_consent_only() {
    let w = world().await;
    // A member_settings row from before the per-kind fields existed.
    w.db()
        .client
        .query(format!(
            r#"
            REMOVE FIELD IF EXISTS stat_digits_opt_in ON member_settings;
            REMOVE FIELD IF EXISTS map_name_opt_in ON member_settings;
            CREATE member_settings SET member_id = '{OWNER}', player_name = 'Old',
                stats_visibility = 'public', hero_icon_opt_in = true;
            CREATE member_settings SET member_id = '{OTHER}', player_name = 'Old',
                stats_visibility = 'public', hero_icon_opt_in = false;
            "#
        ))
        .await
        .expect("legacy rows")
        .check()
        .expect("legacy rows");
    run_migrations(&w.db().client).await.expect("migrate");

    let (_, owner) = get(&w.app, "/api/training-crops/consent", OWNER_TOKEN).await;
    assert_eq!(
        owner,
        json!({ "hero_icon": true, "stat_digits": false, "map_name": false })
    );
    let (_, other) = get(&w.app, "/api/training-crops/consent", OTHER_TOKEN).await;
    assert_eq!(
        other,
        json!({ "hero_icon": false, "stat_digits": false, "map_name": false })
    );
    // The migrated opt-in allows hero icons and nothing else.
    let got = upload(
        &w.app,
        OWNER_TOKEN,
        "hero_icon",
        &pairs(1, json!({ "hero": "ana" })),
    )
    .await;
    assert_eq!(got.0, StatusCode::CREATED, "{}", got.1);
    let got = upload(&w.app, OWNER_TOKEN, "stat_digits", &pairs(1, json!({}))).await;
    assert_error(&got, StatusCode::FORBIDDEN, "crop_consent_required");
    w.cleanup();
}

#[tokio::test]
async fn officer_counts_by_kind_and_resolution_admin_sees_hidden_kinds() {
    let w = world().await;
    set_consent(
        &w.app,
        OWNER_SESSION,
        json!({ "stat_digits": true, "map_name": true }),
    )
    .await;
    let hd = json!({ "screen_width": 1920, "screen_height": 1080 });
    let qhd = json!({ "screen_width": 2560, "screen_height": 1440 });
    upload(&w.app, OWNER_TOKEN, "stat_digits", &pairs(2, hd.clone())).await;
    upload(&w.app, OWNER_TOKEN, "stat_digits", &pairs(1, qhd)).await;
    upload(&w.app, OWNER_TOKEN, "map_name", &pairs(1, json!({}))).await;
    // Then map_name consent goes off: its crop is hidden from officers.
    set_consent(&w.app, OWNER_SESSION, json!({ "map_name": false })).await;
    // OTHER consents to map_name only: listed with nothing stored.
    set_consent(&w.app, OTHER_SESSION, json!({ "map_name": true })).await;

    let (status, page) = get(&w.app, "/api/training-crops/members", OFFICER_SESSION).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    let rows = page["data"].as_array().unwrap();
    // Sorted by display name: cropother before cropowner.
    assert_eq!(rows.len(), 2);
    let other = &rows[0];
    let owner = &rows[1];
    assert_eq!(owner["member_id"], json!(OWNER));
    assert_eq!(owner["total"], json!(3));
    assert_eq!(
        owner["kinds"],
        json!([{ "kind": "stat_digits", "count": 3, "consented": true, "max": 1000 }])
    );
    assert_eq!(
        owner["resolutions"],
        json!([
            { "kind": "stat_digits", "screen_width": 1920, "screen_height": 1080, "count": 2 },
            { "kind": "stat_digits", "screen_width": 2560, "screen_height": 1440, "count": 1 }
        ])
    );
    assert_eq!(other["member_id"], json!(OTHER));
    assert_eq!(
        other["kinds"],
        json!([{ "kind": "map_name", "count": 0, "consented": true, "max": 200 }])
    );
    assert_eq!(other["resolutions"], json!([]));

    let (_, page) = get(&w.app, "/api/training-crops/members", ADMIN_SESSION).await;
    let owner = &page["data"][1];
    assert_eq!(owner["member_id"], json!(OWNER));
    assert_eq!(owner["total"], json!(4));
    assert_eq!(owner["kinds"][1]["kind"], json!("map_name"));
    assert_eq!(owner["kinds"][1]["consented"], json!(false));
    assert_eq!(owner["kinds"][1]["count"], json!(1));
    let resolutions = owner["resolutions"].as_array().unwrap();
    assert_eq!(resolutions.len(), 3);
    assert_eq!(
        resolutions[2],
        json!({ "kind": "map_name", "count": 1 }),
        "no resolution sent"
    );

    // Plain members get 403.
    let got = get(&w.app, "/api/training-crops/members", OTHER_SESSION).await;
    assert_eq!(got.0, StatusCode::FORBIDDEN, "{}", got.1);
    w.cleanup();
}

#[tokio::test]
async fn admin_reads_follow_consent_and_officers_get_no_images() {
    let w = world().await;
    set_consent(&w.app, OWNER_SESSION, json!({ "stat_digits": true })).await;
    let ids = crop_ids(&upload(&w.app, OWNER_TOKEN, "stat_digits", &pairs(1, full_meta())).await);
    let image = format!("/api/training-crops/{}/image", ids[0]);

    let (status, page) = get(&w.app, "/api/training-crops", ADMIN_SESSION).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["data"][0]["id"], json!(ids[0]));
    assert_eq!(page["data"][0]["member_id"], json!(OWNER));
    assert_eq!(page["data"][0]["meta"]["slot_index"], json!(3));
    let (status, filtered) = get(&w.app, "/api/training-crops?kind=map_name", ADMIN_SESSION).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(filtered["data"], json!([]));
    let request = base_request(Method::GET, &image, Some(ADMIN_SESSION))
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        w.app.clone().oneshot(request).await.unwrap().status(),
        StatusCode::OK
    );
    let got = get(&w.app, &image, OFFICER_SESSION).await;
    assert_error(&got, StatusCode::FORBIDDEN, "Admin access required");
    let got = get(&w.app, &image, OTHER_SESSION).await;
    assert_error(&got, StatusCode::NOT_FOUND, "crop_not_found");
    let got = get(&w.app, "/api/training-crops", OFFICER_SESSION).await;
    assert_eq!(got.0, StatusCode::FORBIDDEN, "{}", got.1);

    // Consent off: the admin list still gives the id (so it can be deleted)
    // but marks it hidden and drops the metadata; the image is 404.
    set_consent(&w.app, OWNER_SESSION, json!({ "stat_digits": false })).await;
    let (_, page) = get(&w.app, "/api/training-crops", ADMIN_SESSION).await;
    let items = page["data"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], json!(ids[0]));
    assert_eq!(items[0]["hidden"], json!(true));
    assert_eq!(items[0]["meta"], json!({}));
    assert!(items[0].get("tracker_version").is_none());
    let got = get(&w.app, &image, ADMIN_SESSION).await;
    assert_error(&got, StatusCode::NOT_FOUND, "crop_not_found");
    let request = base_request(Method::GET, &image, Some(OWNER_SESSION))
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        w.app.clone().oneshot(request).await.unwrap().status(),
        StatusCode::OK
    );
    // An admin session may still delete it, and that is audited.
    let got = delete(
        &w.app,
        &format!("/api/training-crops/{}", ids[0]),
        ADMIN_SESSION,
    )
    .await;
    assert_eq!(got.1, json!({ "deleted": 1 }));
    assert!(files_on_disk(w.dir()).is_empty());
    let audits = w.db().list_audit_log(50, 0).await.expect("audit");
    assert!(
        audits
            .iter()
            .any(|e| e.action == "deleted_training_crop" && e.target_id == OWNER)
    );
    assert!(!audits.iter().any(|e| e.action == "deleted_hero_icon"));
    w.cleanup();
}

#[tokio::test]
async fn disabled_store_keeps_consent_working() {
    let dir = std::env::temp_dir().join("scuffed-crops-disabled-never-created");
    let w = world_with(HeroIconStore::new(dir, false)).await;
    let consent = set_consent(&w.app, OWNER_SESSION, json!({ "map_name": true })).await;
    assert_eq!(consent["map_name"], json!(true));
    let got = upload(&w.app, OWNER_TOKEN, "map_name", &pairs(1, json!({}))).await;
    assert_error(&got, StatusCode::SERVICE_UNAVAILABLE, "hero_icons_disabled");
    let got = delete(&w.app, "/api/training-crops/mine", OWNER_TOKEN).await;
    assert_error(&got, StatusCode::SERVICE_UNAVAILABLE, "hero_icons_disabled");
    assert!(!w.dir().exists());
}

async fn call_from(
    app: &axum::Router,
    ip: IpAddr,
    method: Method,
    uri: &str,
    token: &str,
    body: Option<Value>,
) -> Got {
    let mut builder = base_request_from(ip, method, uri, Some(token));
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

async fn upload_from(app: &axum::Router, ip: IpAddr, token: &str, kind: &str) -> Got {
    let uri = format!("/api/training-crops?kind={kind}");
    let request = base_request_from(ip, Method::POST, &uri, Some(token))
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .body(Body::from(multipart_body(&pairs(1, json!({})))))
        .unwrap();
    read_json(app.clone().oneshot(request).await.unwrap()).await
}

#[tokio::test]
async fn sync_token_cannot_set_consent() {
    let w = world().await;
    let uri = "/api/training-crops/consent";
    for token in [OWNER_TOKEN, ADMIN_TOKEN] {
        let got = call(
            &w.app,
            Method::PUT,
            uri,
            Some(token),
            Some(json!({ "stat_digits": true, "map_name": true })),
        )
        .await;
        assert_eq!(got.0, StatusCode::UNAUTHORIZED, "{token}: {}", got.1);
    }
    // Nothing changed, and the token may still read its own consent.
    let (status, consent) = get(&w.app, uri, OWNER_TOKEN).await;
    assert_eq!(status, StatusCode::OK, "{consent}");
    assert_eq!(
        consent,
        json!({ "hero_icon": false, "stat_digits": false, "map_name": false })
    );
    // A session sets it, and the token sees the change.
    set_consent(&w.app, OWNER_SESSION, json!({ "stat_digits": true })).await;
    let (_, consent) = get(&w.app, uri, OWNER_TOKEN).await;
    assert_eq!(consent["stat_digits"], json!(true));
    // A token still turning consent off is refused too.
    let got = call(
        &w.app,
        Method::PUT,
        uri,
        Some(OWNER_TOKEN),
        Some(json!({ "stat_digits": false })),
    )
    .await;
    assert_eq!(got.0, StatusCode::UNAUTHORIZED, "{}", got.1);
    let (_, consent) = get(&w.app, uri, OWNER_TOKEN).await;
    assert_eq!(consent["stat_digits"], json!(true));
    w.cleanup();
}

#[tokio::test]
async fn overlong_tracker_version_is_not_stored() {
    let w = world().await;
    set_consent(&w.app, OWNER_SESSION, json!({ "stat_digits": true })).await;
    let long = format!("scuffed-stat-tracker/1.0.0-{}", "a".repeat(80));
    let mut request = base_request(
        Method::POST,
        "/api/training-crops?kind=stat_digits",
        Some(OWNER_TOKEN),
    )
    .header(
        header::CONTENT_TYPE,
        format!("multipart/form-data; boundary={BOUNDARY}"),
    )
    .body(Body::from(multipart_body(&pairs(1, json!({})))))
    .unwrap();
    // Replace the default tracker User-Agent rather than adding a second one.
    request
        .headers_mut()
        .insert(header::USER_AGENT, long.parse().unwrap());
    let got = read_json(w.app.clone().oneshot(request).await.unwrap()).await;
    assert_eq!(got.0, StatusCode::CREATED, "{}", got.1);
    assert!(got.1["crops"][0].get("tracker_version").is_none());
    assert_eq!(w.rows_of(OWNER).await[0].tracker_version, None);
    w.cleanup();
}

#[tokio::test]
async fn crop_bounds_are_per_kind_and_map_names_may_be_wide() {
    let w = world().await;
    let all = json!({ "stat_digits": true, "map_name": true, "hero_icon": true });
    for session in [OWNER_SESSION, OTHER_SESSION, OFFICER_SESSION, ADMIN_SESSION] {
        set_consent(&w.app, session, all.clone()).await;
    }
    let bad = StatusCode::BAD_REQUEST;
    let too_large = StatusCode::PAYLOAD_TOO_LARGE;
    // A noise crop just over the map_name byte limit, inside its edges.
    let big_map = noise_png(300, 256, 1);
    assert!(big_map.len() > MAP_NAME_MAX_BYTES, "{}", big_map.len());
    // A noise crop over the 64 KiB limit of the other kinds.
    let big_digits = noise_png(130, 130, 2);
    assert!(big_digits.len() > 64 * 1024, "{}", big_digits.len());
    let hero = || meta(json!({ "hero": "ana" }));
    let cases: Vec<(&str, Vec<Part>, StatusCode, &str)> = vec![
        (
            "map_name",
            vec![meta(json!({})), crop(1025, 16)],
            bad,
            "icon_bad_dimensions",
        ),
        (
            "map_name",
            vec![meta(json!({})), crop(64, 257)],
            bad,
            "icon_bad_dimensions",
        ),
        (
            "map_name",
            vec![meta(json!({})), crop(7, 16)],
            bad,
            "icon_bad_dimensions",
        ),
        (
            "map_name",
            vec![meta(json!({})), ("crop", big_map)],
            too_large,
            "icon_too_large",
        ),
        (
            "stat_digits",
            vec![meta(json!({})), crop(257, 16)],
            bad,
            "icon_bad_dimensions",
        ),
        (
            "stat_digits",
            vec![meta(json!({})), ("crop", big_digits)],
            too_large,
            "icon_too_large",
        ),
        (
            "hero_icon",
            vec![hero(), crop(300, 64)],
            bad,
            "icon_bad_dimensions",
        ),
    ];
    let tokens = [OWNER_TOKEN, OTHER_TOKEN, OFFICER_TOKEN, ADMIN_TOKEN];
    for (index, (kind, parts, status, code)) in cases.into_iter().enumerate() {
        let got = upload(&w.app, tokens[index % tokens.len()], kind, &parts).await;
        assert_eq!(got.0, status, "case {index} {kind} {code}: {}", got.1);
        assert_eq!(got.1["error"], json!(code), "case {index}: {}", got.1);
    }
    assert!(files_on_disk(w.dir()).is_empty());

    // The largest map name banner and a noise one just under its byte limit
    // are both accepted.
    let wide = vec![
        meta(json!({})),
        crop(1024, 256),
        meta(json!({})),
        ("crop", noise_png(250, 256, 3)),
    ];
    let got = upload(&w.app, OWNER_TOKEN, "map_name", &wide).await;
    assert_eq!(got.0, StatusCode::CREATED, "{}", got.1);
    assert_eq!(got.1["crops"][0]["width"], json!(1024));
    assert_eq!(got.1["crops"][0]["height"], json!(256));
    assert_eq!(files_on_disk(w.dir()).len(), 2);
    w.cleanup();
}

#[tokio::test]
async fn bad_png_errors_on_the_training_crop_route() {
    let w = world().await;
    let all = json!({ "stat_digits": true, "map_name": true });
    for session in [OWNER_SESSION, OTHER_SESSION, OFFICER_SESSION, ADMIN_SESSION] {
        set_consent(&w.app, session, all.clone()).await;
    }
    let signature = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut garbage = signature.clone();
    garbage.extend_from_slice(&[0u8; 200]);
    let truncated = png(40, 16)[..40].to_vec();
    let cases: Vec<(&str, Vec<u8>, StatusCode, &str)> = vec![
        (
            "stat_digits",
            b"GIF89a-not-a-png".to_vec(),
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "not_png",
        ),
        (
            "map_name",
            b"\xff\xd8\xff\xe0 jpeg".to_vec(),
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "not_png",
        ),
        (
            "map_name",
            b"PNG".to_vec(),
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "not_png",
        ),
        (
            "stat_digits",
            signature,
            StatusCode::BAD_REQUEST,
            "invalid_png",
        ),
        ("map_name", garbage, StatusCode::BAD_REQUEST, "invalid_png"),
        (
            "stat_digits",
            truncated,
            StatusCode::BAD_REQUEST,
            "invalid_png",
        ),
    ];
    let tokens = [OWNER_TOKEN, OTHER_TOKEN, OFFICER_TOKEN, ADMIN_TOKEN];
    for (index, (kind, bytes, status, code)) in cases.into_iter().enumerate() {
        // A good pair first: a later bad crop still stores nothing.
        let parts = vec![
            meta(json!({})),
            crop(40, 16),
            meta(json!({})),
            ("crop", bytes),
        ];
        let got = upload(&w.app, tokens[index % tokens.len()], kind, &parts).await;
        assert_eq!(got.0, status, "case {index} {kind} {code}: {}", got.1);
        assert_eq!(got.1["error"], json!(code), "case {index}: {}", got.1);
    }
    assert!(files_on_disk(w.dir()).is_empty());
    for member in [OWNER, OTHER, OFFICER, ADMIN] {
        assert!(w.rows_of(member).await.is_empty());
    }
    w.cleanup();
}

#[tokio::test]
async fn request_over_2_mib_is_413_and_stores_nothing() {
    let w = world().await;
    set_consent(&w.app, OWNER_SESSION, json!({ "map_name": true })).await;
    set_consent(&w.app, OTHER_SESSION, json!({ "map_name": true })).await;
    // Each banner fits the map_name crop limit; eleven of them pass 2 MiB.
    let banners: Vec<Vec<u8>> = (0..11).map(|i| noise_png(200, 256, 10 + i)).collect();
    let mut parts = Vec::new();
    for banner in &banners {
        assert!(banner.len() <= MAP_NAME_MAX_BYTES);
        parts.push(meta(json!({})));
        parts.push(("crop", banner.clone()));
    }
    assert!(multipart_body(&parts).len() > HERO_ICON_MAX_REQUEST_BYTES);
    let got = upload(&w.app, OWNER_TOKEN, "map_name", &parts).await;
    assert_error(&got, StatusCode::PAYLOAD_TOO_LARGE, "icon_too_large");
    assert!(w.rows_of(OWNER).await.is_empty());
    assert!(files_on_disk(w.dir()).is_empty());

    // Nine of the same banners stay under 2 MiB and are stored.
    let under: Vec<Part> = parts[..18].to_vec();
    assert!(multipart_body(&under).len() < HERO_ICON_MAX_REQUEST_BYTES);
    let got = upload(&w.app, OTHER_TOKEN, "map_name", &under).await;
    assert_eq!(crop_ids(&got).len(), 9);
    assert_eq!(files_on_disk(w.dir()).len(), 9);
    w.cleanup();
}

#[tokio::test]
async fn per_ip_limit_covers_the_sync_token_routes() {
    let w = world().await;
    set_consent(&w.app, OWNER_SESSION, json!({ "stat_digits": true })).await;
    let ids = crop_ids(&upload(&w.app, OWNER_TOKEN, "stat_digits", &pairs(1, json!({}))).await);
    let hot = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 77));
    let consent = "/api/training-crops/consent";
    for i in 0..STATS_UPLOAD_IP_BURST {
        let got = call_from(&w.app, hot, Method::GET, consent, OWNER_TOKEN, None).await;
        assert_eq!(got.0, StatusCode::OK, "request {i}: {}", got.1);
    }
    let limited = StatusCode::TOO_MANY_REQUESTS;
    let got = call_from(&w.app, hot, Method::GET, consent, OWNER_TOKEN, None).await;
    assert_eq!(got.0, limited, "{}", got.1);
    // Upload and both deletes share the bucket, and are refused before the
    // credential is looked at.
    let got = upload_from(&w.app, hot, OWNER_TOKEN, "stat_digits").await;
    assert_eq!(got.0, limited, "{}", got.1);
    let got = call_from(
        &w.app,
        hot,
        Method::DELETE,
        "/api/training-crops/mine",
        OWNER_TOKEN,
        None,
    )
    .await;
    assert_eq!(got.0, limited, "{}", got.1);
    let one = format!("/api/training-crops/{}", ids[0]);
    let got = call_from(&w.app, hot, Method::DELETE, &one, "not-a-token", None).await;
    assert_eq!(got.0, limited, "{}", got.1);
    assert_eq!(w.rows_of(OWNER).await.len(), 1);
    // Session-only routes on the same IP are not in this bucket.
    let got = call_from(
        &w.app,
        hot,
        Method::GET,
        "/api/training-crops/mine",
        OWNER_SESSION,
        None,
    )
    .await;
    assert_eq!(got.0, StatusCode::OK, "{}", got.1);
    let body = Some(json!({ "map_name": true }));
    let got = call_from(&w.app, hot, Method::PUT, consent, OWNER_SESSION, body).await;
    assert_eq!(got.0, StatusCode::OK, "{}", got.1);
    // Another address has its own bucket.
    let got = call_from(&w.app, next_ip(), Method::GET, consent, OWNER_TOKEN, None).await;
    assert_eq!(got.0, StatusCode::OK, "{}", got.1);
    w.cleanup();
}

#[tokio::test]
async fn admin_list_gives_ids_of_every_kind_to_delete() {
    let w = world().await;
    let all = json!({ "stat_digits": true, "map_name": true, "hero_icon": true });
    set_consent(&w.app, OWNER_SESSION, all).await;
    let mut hero_meta = full_meta();
    hero_meta["hero"] = json!("Soldier: 76");
    let hero = crop_ids(&upload(&w.app, OWNER_TOKEN, "hero_icon", &pairs(1, hero_meta)).await);
    let digits =
        crop_ids(&upload(&w.app, OWNER_TOKEN, "stat_digits", &pairs(1, full_meta())).await);
    let map = crop_ids(&upload(&w.app, OWNER_TOKEN, "map_name", &pairs(1, full_meta())).await);
    // map_name consent off: still listed for the admin, hidden.
    set_consent(&w.app, OWNER_SESSION, json!({ "map_name": false })).await;

    let (status, page) = get(&w.app, "/api/training-crops", ADMIN_SESSION).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    let items = page["data"].as_array().unwrap();
    let listed: Vec<&str> = items.iter().map(|c| c["id"].as_str().unwrap()).collect();
    assert_eq!(
        listed,
        vec![map[0].as_str(), digits[0].as_str(), hero[0].as_str()]
    );
    for item in items {
        assert_eq!(item["member_id"], json!(OWNER));
        // Upload times are cut to the hour, so they cannot be matched to a game.
        let at = item["uploaded_at"].as_str().unwrap();
        let parsed = chrono::DateTime::parse_from_rfc3339(at).expect("rfc3339");
        assert_eq!(parsed.timestamp() % 3600, 0, "{at}");
        assert_eq!(parsed.timestamp_subsec_nanos(), 0, "{at}");
    }
    let by_kind = |kind: &str| {
        items
            .iter()
            .find(|c| c["kind"] == json!(kind))
            .cloned()
            .unwrap()
    };
    let map_item = by_kind("map_name");
    assert_eq!(map_item["hidden"], json!(true));
    assert_eq!(map_item["meta"], json!({}));
    let digit_item = by_kind("stat_digits");
    assert_eq!(digit_item["hidden"], json!(false));
    assert_eq!(digit_item["meta"]["slot_index"], json!(3));
    let hero_item = by_kind("hero_icon");
    assert_eq!(hero_item["hero"], json!("soldier-76"));

    // The kind filter works for every kind, hidden ones too.
    for (kind, id) in [
        ("hero_icon", &hero[0]),
        ("stat_digits", &digits[0]),
        ("map_name", &map[0]),
    ] {
        let uri = format!("/api/training-crops?kind={kind}");
        let (status, page) = get(&w.app, &uri, ADMIN_SESSION).await;
        assert_eq!(status, StatusCode::OK, "{page}");
        assert_eq!(page["data"].as_array().unwrap().len(), 1, "{kind}");
        assert_eq!(page["data"][0]["id"], json!(id));
    }
    let got = get(
        &w.app,
        "/api/training-crops?kind=full_screen",
        ADMIN_SESSION,
    )
    .await;
    assert_eq!(got.1["data"], json!([]));
    let got = get(&w.app, "/api/training-crops?kind=bogus", ADMIN_SESSION).await;
    assert_error(&got, StatusCode::BAD_REQUEST, "unknown_crop_kind");

    // Each listed id can be deleted by the admin, whatever its kind.
    for id in [&hero[0], &digits[0], &map[0]] {
        let got = delete(&w.app, &format!("/api/training-crops/{id}"), ADMIN_SESSION).await;
        assert_eq!(got.1, json!({ "deleted": 1 }), "{id}");
    }
    assert!(w.rows_of(OWNER).await.is_empty());
    assert!(files_on_disk(w.dir()).is_empty());

    // Audit names: hero icons keep theirs, the other kinds get their own.
    let audits = w.db().list_audit_log(100, 0).await.expect("audit");
    let count = |action: &str| audits.iter().filter(|e| e.action == action).count();
    assert_eq!(count("deleted_hero_icon"), 1);
    assert_eq!(count("deleted_training_crop"), 2);
    assert_eq!(count("viewed_hero_icons"), 1, "the hero_icon filtered read");
    assert_eq!(
        count("viewed_training_crops"),
        4,
        "unfiltered plus three filters"
    );
    for kind in ["stat_digits", "map_name"] {
        assert!(
            audits.iter().any(|e| e.action == "deleted_training_crop"
                && e.details.as_deref().is_some_and(|d| d.contains(kind))),
            "{kind}"
        );
    }
    w.cleanup();
}
