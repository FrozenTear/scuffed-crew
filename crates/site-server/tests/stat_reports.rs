//! HTTP tests for tracker bug-report bundles.
//!
//! Fixtures are synthetic 1x1 PNGs and short text. Nothing here is a game capture.

use std::io::{Cursor, Read, Write};
use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use chrono::{Duration, Utc};
use flate2::Compression;
use flate2::write::ZlibEncoder;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tower::ServiceExt;
use zip::CompressionMethod;
use zip::write::{SimpleFileOptions, ZipWriter};

use scuffed_auth::SessionConfig;
use scuffed_auth::crypto::hash_session_token;
use scuffed_db::Database;
use scuffed_db::migrations::run_migrations;
use scuffed_site_server::create_router;
use scuffed_site_server::stat_reports::sweep_stat_reports;
use scuffed_site_server::state::{AppState, OAuthConfig};
use scuffed_types::{DAILY_REPORT_CAP, MAX_BUNDLE_BYTES, RETENTION_DAYS};

const MEMBER_TOKEN: &str = "report-member-token";
const OTHER_TOKEN: &str = "report-other-token";
const OFFICER_TOKEN: &str = "report-officer-token";

struct Harness {
    state: AppState,
    app: axum::Router,
    reports_dir: PathBuf,
}

async fn harness() -> Harness {
    let db = Database::connect_memory().await.expect("mem db");
    run_migrations(&db.client).await.expect("migrations");
    seed_user(
        &db,
        "memberuser",
        "membermember",
        "MemberOne",
        "member",
        MEMBER_TOKEN,
    )
    .await;
    seed_user(
        &db,
        "otheruser",
        "othermember",
        "MemberTwo",
        "member",
        OTHER_TOKEN,
    )
    .await;
    seed_user(
        &db,
        "officeruser",
        "officermember",
        "OfficerOne",
        "officer",
        OFFICER_TOKEN,
    )
    .await;

    let reports_dir =
        std::env::temp_dir().join(format!("scuffed-reports-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&reports_dir).unwrap();
    let state = AppState {
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
        reports_dir: reports_dir.clone(),
        notifier: None,
        nostr_challenge_key: [0u8; 32],
        consumed_challenges: scuffed_site_server::challenge_store::ConsumedChallengeStore::new(),
        nostr_rate_limiter: scuffed_site_server::nostr_rate_limit::NostrRateLimiter::new(),
        login_lockout: scuffed_site_server::login_lockout::LoginLockout::new(),
        crypto: None,
        relay_url: None,
        dm_events: None,
        nip05_domain: None,
        nip05_republish_enabled: false,
        public_settings: scuffed_site_server::state::PublicSettingsCache::new(),
        leaderboard_cache: scuffed_site_server::leaderboard_cache::LeaderboardCache::from_env(),
    };
    let app = create_router(state.clone());
    Harness {
        state,
        app,
        reports_dir,
    }
}

async fn seed_user(db: &Database, user: &str, member: &str, name: &str, role: &str, token: &str) {
    let token_hash = hash_session_token(token);
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
                is_active = true"#
        ))
        .await
        .expect("seed member");
    db.client
        .query(format!(
            r#"CREATE session:sess_{member} SET
                user_id = '{user}',
                token = $tok,
                expires_at = time::now() + 365d,
                created_at = time::now()"#
        ))
        .bind(("tok", token_hash))
        .await
        .expect("seed session");
}

fn sha256_hex(bytes: &[u8]) -> String {
    let dig = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in dig {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

fn tiny_png() -> Vec<u8> {
    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&1u32.to_be_bytes());
    ihdr[4..8].copy_from_slice(&1u32.to_be_bytes());
    ihdr[8] = 8;
    ihdr[9] = 2;
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
    enc.write_all(&[0, 255, 0, 0]).unwrap();
    let idat = enc.finish().unwrap();
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend(png_chunk(b"IHDR", &ihdr));
    png.extend(png_chunk(b"IDAT", &idat));
    png.extend(png_chunk(b"IEND", &[]));
    png
}

fn png_with_secret_text() -> Vec<u8> {
    let base = tiny_png();
    let text = png_chunk(b"tEXt", b"Comment\0SECRETMETA");
    let iend_at = base.len() - 12;
    let mut out = base[..iend_at].to_vec();
    out.extend(text);
    out.extend_from_slice(&base[iend_at..]);
    out
}

fn png_chunk(ctype: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(ctype);
    out.extend_from_slice(data);
    let crc = crc32(&out[4..]);
    out.extend_from_slice(&crc.to_be_bytes());
    out
}

struct Built {
    bytes: Vec<u8>,
}

fn valid_bundle(training: bool) -> Built {
    bundle_with(training, tiny_png(), false, None, None)
}

fn bundle_with(
    training: bool,
    png: Vec<u8>,
    bad_hash: bool,
    extra: Option<(&str, Vec<u8>)>,
    version: Option<u64>,
) -> Built {
    let log = b"synthetic log line\n".to_vec();
    let mut png_hash = sha256_hex(&png);
    if bad_hash {
        png_hash = "f".repeat(64);
    }
    let manifest = json!({
        "bundle_version": version.unwrap_or(1),
        "app_version": "0.0.0",
        "recognizers": { "matcher": "cv-v3", "ocr": "ocr-v1" },
        "resolution": { "width": 1920, "height": 1080 },
        "ui_scale": null,
        "reason": { "category": "wrong_stats", "text": "elims looked high" },
        "session_id": "sess-example",
        "game": {
            "map": "Busan",
            "mode": "Control",
            "result": "Defeat",
            "team_size": 5,
            "captured_at": "2026-10-09T12:00:00Z"
        },
        "reads": read_block(),
        "corrections": {},
        "consent": {
            "training": training,
            "own_name_included": false,
            "glyphs_included": false
        },
        "files": [
            {
                "path": "log.txt",
                "sha256": sha256_hex(&log),
                "bytes": log.len(),
                "role": "log",
                "screen_class": null
            },
            {
                "path": "crops/scoreboard.png",
                "sha256": png_hash,
                "bytes": png.len(),
                "role": "crop",
                "screen_class": "scoreboard"
            }
        ]
    });
    let mut files = vec![
        (
            "manifest.json".to_string(),
            serde_json::to_vec(&manifest).unwrap(),
        ),
        ("log.txt".to_string(), log),
        ("crops/scoreboard.png".to_string(), png),
    ];
    if let Some((name, bytes)) = extra {
        files.push((name.to_string(), bytes));
    }
    Built {
        bytes: zip_stored(&files),
    }
}

fn read_block() -> Value {
    let cell = |value: &str, confidence: Option<f64>, suspect: bool| {
        json!({
            "value": value,
            "confidence": confidence,
            "suspect": suspect,
            "ocr_v1": value
        })
    };
    json!({
        "mode": cell("Control", None, false),
        "result": cell("Defeat", None, false),
        "hero": cell("Ana", None, false),
        "e": { "value": "21", "confidence": 0.22, "suspect": true, "ocr_v1": "20" },
        "a": cell("8", Some(0.9), false),
        "d": cell("4", Some(0.88), false),
        "dmg": cell("8432", Some(0.8), false),
        "h": cell("2100", Some(0.77), false),
        "mit": cell("0", Some(0.7), false)
    })
}

fn zip_stored(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut locals = Vec::new();
    let mut central = Vec::new();
    let mut offset = 0u32;
    for (name, data) in files {
        let name_bytes = name.as_bytes();
        let crc = crc32(data);
        let mut local = Vec::new();
        local.extend_from_slice(&0x04034b50u32.to_le_bytes());
        local.extend_from_slice(&20u16.to_le_bytes());
        local.extend_from_slice(&0u16.to_le_bytes());
        local.extend_from_slice(&0u16.to_le_bytes());
        local.extend_from_slice(&0u16.to_le_bytes());
        local.extend_from_slice(&0u16.to_le_bytes());
        local.extend_from_slice(&crc.to_le_bytes());
        local.extend_from_slice(&(data.len() as u32).to_le_bytes());
        local.extend_from_slice(&(data.len() as u32).to_le_bytes());
        local.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        local.extend_from_slice(&0u16.to_le_bytes());
        local.extend_from_slice(name_bytes);
        local.extend_from_slice(data);
        let mut cd = Vec::new();
        cd.extend_from_slice(&0x02014b50u32.to_le_bytes());
        cd.extend_from_slice(&20u16.to_le_bytes());
        cd.extend_from_slice(&20u16.to_le_bytes());
        cd.extend_from_slice(&0u16.to_le_bytes());
        cd.extend_from_slice(&0u16.to_le_bytes());
        cd.extend_from_slice(&0u16.to_le_bytes());
        cd.extend_from_slice(&0u16.to_le_bytes());
        cd.extend_from_slice(&crc.to_le_bytes());
        cd.extend_from_slice(&(data.len() as u32).to_le_bytes());
        cd.extend_from_slice(&(data.len() as u32).to_le_bytes());
        cd.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        cd.extend_from_slice(&0u16.to_le_bytes());
        cd.extend_from_slice(&0u16.to_le_bytes());
        cd.extend_from_slice(&0u16.to_le_bytes());
        cd.extend_from_slice(&0u16.to_le_bytes());
        cd.extend_from_slice(&0u32.to_le_bytes());
        cd.extend_from_slice(&offset.to_le_bytes());
        cd.extend_from_slice(name_bytes);
        offset += local.len() as u32;
        locals.extend(local);
        central.extend(cd);
    }
    let mut out = locals;
    let cd_offset = out.len() as u32;
    out.extend(&central);
    let cd_size = central.len() as u32;
    out.extend_from_slice(&0x06054b50u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&cd_size.to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

fn too_many_entries() -> Vec<u8> {
    let png = tiny_png();
    let log = b"synthetic log line\n".to_vec();
    let mut listed = vec![json!({
        "path": "log.txt",
        "sha256": sha256_hex(&log),
        "bytes": log.len(),
        "role": "log",
        "screen_class": null
    })];
    let mut files = vec![("log.txt".to_string(), log)];
    for i in 0..30 {
        let path = format!("crops/c{i}.png");
        listed.push(json!({
            "path": path,
            "sha256": sha256_hex(&png),
            "bytes": png.len(),
            "role": "crop",
            "screen_class": "scoreboard"
        }));
        files.push((path, png.clone()));
    }
    let manifest = json!({
        "bundle_version": 1,
        "app_version": "0.0.0",
        "recognizers": { "matcher": "cv-v3", "ocr": "ocr-v1" },
        "resolution": { "width": 1920, "height": 1080 },
        "ui_scale": null,
        "reason": { "category": "other", "text": "" },
        "session_id": "sess-example",
        "game": {
            "map": null, "mode": null, "result": null, "team_size": 5,
            "captured_at": "2026-10-09T12:00:00Z"
        },
        "reads": read_block(),
        "corrections": {},
        "consent": { "training": false, "own_name_included": false, "glyphs_included": false },
        "files": listed
    });
    files.insert(
        0,
        (
            "manifest.json".to_string(),
            serde_json::to_vec(&manifest).unwrap(),
        ),
    );
    zip_stored(&files)
}

fn deflated_log_bomb() -> Vec<u8> {
    let log = vec![b'a'; (MAX_BUNDLE_BYTES as usize) + 1];
    let manifest = json!({
        "bundle_version": 1,
        "app_version": "0.0.0",
        "recognizers": { "matcher": "cv-v3", "ocr": "ocr-v1" },
        "resolution": { "width": 1920, "height": 1080 },
        "ui_scale": null,
        "reason": { "category": "other", "text": "" },
        "session_id": "sess-example",
        "game": {
            "map": null, "mode": null, "result": null, "team_size": 5,
            "captured_at": "2026-10-09T12:00:00Z"
        },
        "reads": read_block(),
        "corrections": {},
        "consent": { "training": false, "own_name_included": false, "glyphs_included": false },
        "files": [{
            "path": "log.txt",
            "sha256": sha256_hex(&log),
            "bytes": log.len(),
            "role": "log",
            "screen_class": null
        }]
    });
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    writer.start_file("manifest.json", opts).unwrap();
    writer
        .write_all(&serde_json::to_vec(&manifest).unwrap())
        .unwrap();
    writer
        .start_file(
            "log.txt",
            SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
        )
        .unwrap();
    writer.write_all(&log).unwrap();
    writer.finish().unwrap().into_inner()
}

async fn send(
    app: &axum::Router,
    method: Method,
    uri: &str,
    token: Option<&str>,
    body: Option<Vec<u8>>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let req = if let Some(body) = body {
        builder
            .header(header::CONTENT_TYPE, "application/zip")
            .body(Body::from(body))
            .unwrap()
    } else {
        builder.body(Body::empty()).unwrap()
    };
    let response = app.clone().oneshot(req).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or(Value::String(String::from_utf8_lossy(&bytes).into_owned()))
    };
    (status, value)
}

async fn send_raw(
    app: &axum::Router,
    method: Method,
    uri: &str,
    token: &str,
) -> (StatusCode, Vec<u8>) {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    (status, bytes)
}

fn zip_count(dir: &PathBuf) -> usize {
    std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".zip"))
        .count()
}

#[tokio::test]
async fn auth_required() {
    let h = harness().await;
    let zip = valid_bundle(false).bytes;
    let (status, _) = send(&h.app, Method::POST, "/api/stat-reports", None, Some(zip)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = send(&h.app, Method::GET, "/api/stat-reports", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = send(
        &h.app,
        Method::DELETE,
        "/api/stat-reports/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn rejects_oversize_body_and_uncompressed_bomb() {
    let h = harness().await;
    let too_big = vec![0u8; (MAX_BUNDLE_BYTES as usize) + 1];
    let (status, _) = send(
        &h.app,
        Method::POST,
        "/api/stat-reports",
        Some(MEMBER_TOKEN),
        Some(too_big),
    )
    .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);

    let bomb = deflated_log_bomb();
    assert!(
        bomb.len() as u64 <= MAX_BUNDLE_BYTES,
        "fixture should compress under the cap"
    );
    let (status, _) = send(
        &h.app,
        Method::POST,
        "/api/stat-reports",
        Some(MEMBER_TOKEN),
        Some(bomb),
    )
    .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(zip_count(&h.reports_dir), 0);
}

#[tokio::test]
async fn daily_cap_is_five_successful_stores() {
    let h = harness().await;
    for _ in 0..DAILY_REPORT_CAP {
        let (status, _) = send(
            &h.app,
            Method::POST,
            "/api/stat-reports",
            Some(MEMBER_TOKEN),
            Some(valid_bundle(false).bytes),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
    }
    let (status, _) = send(
        &h.app,
        Method::POST,
        "/api/stat-reports",
        Some(MEMBER_TOKEN),
        Some(valid_bundle(false).bytes),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    let (status, body) = send(
        &h.app,
        Method::GET,
        "/api/stat-reports",
        Some(MEMBER_TOKEN),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["reports"].as_array().unwrap().len(),
        DAILY_REPORT_CAP as usize
    );
    assert_eq!(zip_count(&h.reports_dir), DAILY_REPORT_CAP as usize);
}

#[tokio::test]
async fn rejects_bad_manifest_extra_file_traversal_and_non_png() {
    let h = harness().await;

    let bad = bundle_with(false, tiny_png(), false, None, Some(2));
    let (status, _) = send(
        &h.app,
        Method::POST,
        "/api/stat-reports",
        Some(MEMBER_TOKEN),
        Some(bad.bytes),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let extra = bundle_with(
        false,
        tiny_png(),
        false,
        Some(("crops/extra.png", tiny_png())),
        None,
    );
    let (status, _) = send(
        &h.app,
        Method::POST,
        "/api/stat-reports",
        Some(MEMBER_TOKEN),
        Some(extra.bytes),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let traversal = bundle_with(
        false,
        tiny_png(),
        false,
        Some(("../secret.txt", b"nope".to_vec())),
        None,
    );
    let (status, _) = send(
        &h.app,
        Method::POST,
        "/api/stat-reports",
        Some(MEMBER_TOKEN),
        Some(traversal.bytes),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(!h.reports_dir.join("../secret.txt").exists());

    let (status, _) = send(
        &h.app,
        Method::POST,
        "/api/stat-reports",
        Some(MEMBER_TOKEN),
        Some(too_many_entries()),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let not_png = bundle_with(false, b"not a png".to_vec(), false, None, None);
    let (status, _) = send(
        &h.app,
        Method::POST,
        "/api/stat-reports",
        Some(MEMBER_TOKEN),
        Some(not_png.bytes),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let mismatch = bundle_with(false, tiny_png(), true, None, None);
    let (status, _) = send(
        &h.app,
        Method::POST,
        "/api/stat-reports",
        Some(MEMBER_TOKEN),
        Some(mismatch.bytes),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(zip_count(&h.reports_dir), 0);
}

#[tokio::test]
async fn member_lists_own_rows_officer_reads_and_member_is_forbidden() {
    let h = harness().await;
    let (status, created) = send(
        &h.app,
        Method::POST,
        "/api/stat-reports",
        Some(MEMBER_TOKEN),
        Some(bundle_with(false, png_with_secret_text(), false, None, None).bytes),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created["training"], false);
    let id = created["id"].as_str().unwrap();
    let expires = chrono::DateTime::parse_from_rfc3339(created["expires_at"].as_str().unwrap())
        .unwrap()
        .with_timezone(&Utc);
    let delta = expires - Utc::now();
    assert!(delta > Duration::days(RETENTION_DAYS - 1));
    assert!(delta < Duration::days(RETENTION_DAYS) + Duration::minutes(5));

    let (status, mine) = send(
        &h.app,
        Method::GET,
        "/api/stat-reports",
        Some(MEMBER_TOKEN),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(mine["reports"].as_array().unwrap().len(), 1);
    assert!(mine["reports"][0].get("reason_text").is_none());
    assert_eq!(mine["reports"][0]["training_consent"], false);

    let (status, _) = send(
        &h.app,
        Method::POST,
        "/api/stat-reports",
        Some(OTHER_TOKEN),
        Some(valid_bundle(true).bytes),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, mine) = send(
        &h.app,
        Method::GET,
        "/api/stat-reports",
        Some(MEMBER_TOKEN),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(mine["reports"].as_array().unwrap().len(), 1);

    let (status, all) = send(
        &h.app,
        Method::GET,
        "/api/stat-reports",
        Some(OFFICER_TOKEN),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(all["reports"].as_array().unwrap().len(), 2);
    let officer_row = all["reports"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == id)
        .unwrap();
    assert_eq!(officer_row["reason_text"], "elims looked high");
    assert_eq!(officer_row["app_version"], "0.0.0");
    assert_eq!(officer_row["recognizer_matcher"], "cv-v3");

    let (status, _) = send_raw(
        &h.app,
        Method::GET,
        &format!("/api/stat-reports/{id}"),
        MEMBER_TOKEN,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = send(
        &h.app,
        Method::GET,
        &format!("/api/stat-reports/{id}/manifest"),
        Some(MEMBER_TOKEN),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, zip_bytes) = send_raw(
        &h.app,
        Method::GET,
        &format!("/api/stat-reports/{id}"),
        OFFICER_TOKEN,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let mut archive = zip::ZipArchive::new(Cursor::new(zip_bytes)).unwrap();
    let mut png = Vec::new();
    archive
        .by_name("crops/scoreboard.png")
        .unwrap()
        .read_to_end(&mut png)
        .unwrap();
    assert!(!png.windows(10).any(|w| w == b"SECRETMETA"));
    assert!(!png.windows(4).any(|w| w == b"tEXt"));
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");

    let (status, manifest) = send(
        &h.app,
        Method::GET,
        &format!("/api/stat-reports/{id}/manifest"),
        Some(OFFICER_TOKEN),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(manifest["bundle_version"], 1);
    assert_eq!(manifest["files"][1]["path"], "crops/scoreboard.png");
}

#[tokio::test]
async fn owner_delete_and_withdraw_are_refused_for_other_members() {
    let h = harness().await;
    let (status, created) = send(
        &h.app,
        Method::POST,
        "/api/stat-reports",
        Some(MEMBER_TOKEN),
        Some(valid_bundle(true).bytes),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(created["expires_at"].is_null());
    let id = created["id"].as_str().unwrap().to_string();

    let (status, _) = send(
        &h.app,
        Method::DELETE,
        &format!("/api/stat-reports/{id}"),
        Some(OTHER_TOKEN),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = send(
        &h.app,
        Method::POST,
        &format!("/api/stat-reports/{id}/withdraw"),
        Some(OTHER_TOKEN),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let created_at = Utc::now() - Duration::days(10);
    h.state
        .db
        .set_stat_report_clock(&id, created_at, None)
        .await
        .unwrap();
    let (status, body) = send(
        &h.app,
        Method::POST,
        &format!("/api/stat-reports/{id}/withdraw"),
        Some(MEMBER_TOKEN),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["deleted"], false);
    assert_eq!(body["training"], false);
    let expires = chrono::DateTime::parse_from_rfc3339(body["expires_at"].as_str().unwrap())
        .unwrap()
        .with_timezone(&Utc);
    let expected = created_at + Duration::days(RETENTION_DAYS);
    assert!((expires - expected).num_seconds().abs() < 2);
    assert!(h.reports_dir.join(format!("{id}.zip")).exists());

    let (status, patched) = send(
        &h.app,
        Method::PATCH,
        &format!("/api/stat-reports/{id}"),
        Some(MEMBER_TOKEN),
        Some(br#"{"training":false}"#.to_vec()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(patched["deleted"], false);

    let (status, _) = send(
        &h.app,
        Method::DELETE,
        &format!("/api/stat-reports/{id}"),
        Some(MEMBER_TOKEN),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(!h.reports_dir.join(format!("{id}.zip")).exists());
    let (status, zip_body) = send_raw(
        &h.app,
        Method::GET,
        &format!("/api/stat-reports/{id}"),
        OFFICER_TOKEN,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let _ = zip_body;
}

#[tokio::test]
async fn withdraw_deletes_when_the_original_window_has_passed() {
    let h = harness().await;
    let (status, created) = send(
        &h.app,
        Method::POST,
        "/api/stat-reports",
        Some(MEMBER_TOKEN),
        Some(valid_bundle(true).bytes),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = created["id"].as_str().unwrap().to_string();
    let created_at = Utc::now() - Duration::days(40);
    h.state
        .db
        .set_stat_report_clock(&id, created_at, None)
        .await
        .unwrap();
    let (status, body) = send(
        &h.app,
        Method::POST,
        &format!("/api/stat-reports/{id}/withdraw"),
        Some(MEMBER_TOKEN),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["deleted"], true);
    assert!(!h.reports_dir.join(format!("{id}.zip")).exists());
    let (status, list) = send(
        &h.app,
        Method::GET,
        "/api/stat-reports",
        Some(MEMBER_TOKEN),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(list["reports"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn sweep_removes_expired_rows_orphan_files_and_missing_files() {
    let h = harness().await;
    let (status, created) = send(
        &h.app,
        Method::POST,
        "/api/stat-reports",
        Some(MEMBER_TOKEN),
        Some(valid_bundle(false).bytes),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = created["id"].as_str().unwrap().to_string();
    h.state
        .db
        .set_stat_report_clock(
            &id,
            Utc::now() - Duration::days(31),
            Some(Utc::now() - Duration::hours(1)),
        )
        .await
        .unwrap();

    let orphan = "abcdef0123456789abcdef0123456789";
    std::fs::write(h.reports_dir.join(format!("{orphan}.zip")), b"orphan").unwrap();

    let outcome = sweep_stat_reports(&h.state.db, &h.reports_dir)
        .await
        .unwrap();
    assert!(outcome.expired >= 1);
    assert!(outcome.orphan_files >= 1);
    assert!(!h.reports_dir.join(format!("{id}.zip")).exists());
    assert!(!h.reports_dir.join(format!("{orphan}.zip")).exists());

    let (status, created) = send(
        &h.app,
        Method::POST,
        "/api/stat-reports",
        Some(MEMBER_TOKEN),
        Some(valid_bundle(false).bytes),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = created["id"].as_str().unwrap().to_string();
    std::fs::remove_file(h.reports_dir.join(format!("{id}.zip"))).unwrap();
    let outcome = sweep_stat_reports(&h.state.db, &h.reports_dir)
        .await
        .unwrap();
    assert!(outcome.missing_rows >= 1);
    let (status, list) = send(
        &h.app,
        Method::GET,
        "/api/stat-reports",
        Some(MEMBER_TOKEN),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(list["reports"].as_array().unwrap().is_empty());
}
