//! Section 9 fixtures for log-only bundles: the log fixture, the manifest
//! checked against section 2, and the zip rules.

use std::io::Read;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde_json::Value;

use super::bundle::{self, LogOnlyCategory, LogReportInput, Resolution, SessionType, SystemInfo};
use super::marker::LastExit;
use super::scrub::{LogWindow, ScrubContext};
use crate::capture::CaptureBackend;
use crate::config::ReaderSetting;

const TOKEN: &str = "sct_FAKEsyncTOKEN0123456789abcdef";
const BEARER_SECRET: &str = "qxBearerSecret42";
const TOKEN_RUN: &str = "QXrunTOKENshaped_abcdefghijklmnop0123";
const URL: &str = "https://crew.example.test/api/stats/sync";
const HOST: &str = "crew.example.test";
const HOME_PATH: &str = "/home/qxuser/.local/share/scuffed-stat-tracker";
const VAR_HOME_PATH: &str = "/var/home/qxsilver/.config";
const ACCOUNT: &str = "qxuser";
const HOSTNAME: &str = "qxhost-box";
const BATTLETAG: &str = "QXÄnnaß#12345";
const OTHER_1: &str = "QXOtherOne";
const OTHER_2: &str = "QXOtherTwo";
const MISREAD: &str = "QX0therOne";
const SHORT: &str = "Qy";
const SELF_NAME: &str = "QXSelfName";
const FIELD_NAME: &str = "QXFieldOnly";

fn t(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

fn ctx() -> ScrubContext {
    ScrubContext {
        sync_token: Some(TOKEN.into()),
        server_url: Some("https://crew.example.test".into()),
        home_dir: Some(PathBuf::from("/home/qxuser")),
        account_name: Some(ACCOUNT.into()),
        hostname: Some(HOSTNAME.into()),
        player_name: Some(SELF_NAME.into()),
        // FIELD_NAME is deliberately not on the list: only the field drop
        // can keep it out of log.txt.
        names: vec![OTHER_1.into(), OTHER_2.into(), MISREAD.into(), SHORT.into()],
    }
}

fn system() -> SystemInfo {
    SystemInfo {
        session_type: SessionType::X11,
        capture_backend: CaptureBackend::X11,
        cpu_threads: 16,
        ocr_threads: 4,
        reader: ReaderSetting::New,
        shadow_recognizer: false,
        prerelease: true,
        daemon_running: true,
        last_exit: LastExit::Crashed,
    }
}

fn line(ts: &str, target: &str, rest: &str) -> String {
    // Same shape as the daemon's tracing writer, colour codes included.
    format!(
        "\x1b[2m{ts}\x1b[0m \x1b[32m INFO\x1b[0m \x1b[2m{target}\x1b[0m\x1b[2m:\x1b[0m {rest}\n"
    )
}

/// Session 12:00:00 to 12:10:00, so the window is 11:58:00 to 12:12:00.
fn seed_logs(dir: &std::path::Path) {
    let mut old = String::new();
    old.push_str(&line(
        "2026-10-09T11:50:00.000001Z",
        "x",
        "QXBEFOREWINDOW line",
    ));
    old.push_str(&line(
        "2026-10-09T11:59:00.000001Z",
        "stat_tracker::sync",
        &format!("sync with token {TOKEN}"),
    ));
    std::fs::write(dir.join("daemon.log.1"), old).unwrap();

    let mut cur = String::new();
    let mid = "2026-10-09T12:05:00.000001Z";
    cur.push_str(&line(
        mid,
        "stat_tracker::sync",
        &format!("request header Authorization: Bearer {BEARER_SECRET}"),
    ));
    cur.push_str(&line(mid, "x", &format!("blob {TOKEN_RUN} end")));
    cur.push_str(&line(mid, "x", &format!("posting to {URL} failed")));
    cur.push_str(&line(mid, "x", &format!("opened {HOME_PATH}/x.db")));
    cur.push_str(&line(mid, "x", &format!("silverblue {VAR_HOME_PATH}/y")));
    cur.push_str(&line(
        mid,
        "x",
        &format!("account {ACCOUNT} on {HOSTNAME} ok"),
    ));
    cur.push_str(&line(mid, "x", &format!("tag {BATTLETAG} joined")));
    cur.push_str(&line(
        mid,
        "x",
        &format!("scoreboard names {OTHER_1} {OTHER_2} self {SELF_NAME}"),
    ));
    cur.push_str(&line(
        mid,
        "x",
        &format!("misread {MISREAD} and short {SHORT}"),
    ));
    cur.push_str(&line(
        mid,
        "stat_tracker::shadow",
        &format!("row read row=4 name={FIELD_NAME} conf=0.41"),
    ));
    cur.push_str(&format!("   continuation without a timestamp {OTHER_1}\n"));
    cur.push_str(&line(
        "2026-10-09T12:06:00.000001Z",
        "scuffed_stat_tracker",
        "capture timing grabs=60 grab_ms_p50=3",
    ));
    cur.push_str(&line(
        "2026-10-09T12:20:00.000001Z",
        "x",
        "QXAFTERWINDOW line",
    ));
    std::fs::write(dir.join("daemon.log"), cur).unwrap();
}

fn input(dir: &std::path::Path, window: LogWindow) -> LogReportInput {
    LogReportInput {
        category: LogOnlyCategory::Crash,
        text: format!("it crashed, {OTHER_1} was there, token {TOKEN}, see {URL}"),
        resolution: Resolution {
            width: 2560,
            height: 1440,
        },
        ui_scale: None,
        system: system(),
        training: false,
        scrub: ctx(),
        window,
        log_dir: dir.to_path_buf(),
    }
}

fn unzip(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut ar = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut out = Vec::new();
    for i in 0..ar.len() {
        let mut f = ar.by_index(i).unwrap();
        let mut data = Vec::new();
        f.read_to_end(&mut data).unwrap();
        out.push((f.name().to_string(), data));
    }
    out
}

const SECRETS: &[&str] = &[
    TOKEN,
    BEARER_SECRET,
    TOKEN_RUN,
    URL,
    HOST,
    "/home/qxuser",
    "/var/home/qxsilver",
    "qxsilver",
    ACCOUNT,
    HOSTNAME,
    BATTLETAG,
    "QXÄnnaß",
    OTHER_1,
    OTHER_2,
    MISREAD,
    SELF_NAME,
    FIELD_NAME,
    "QXBEFOREWINDOW",
    "QXAFTERWINDOW",
];

#[test]
fn section_9_log_fixture_scrubs_everything() {
    let dir = tempfile::tempdir().unwrap();
    seed_logs(dir.path());
    let window = LogWindow::session(t("2026-10-09T12:00:00Z"), t("2026-10-09T12:10:00Z"));
    let report = bundle::build_log_report(&input(dir.path(), window)).unwrap();

    let entries = unzip(&report.bytes);
    for (name, data) in &entries {
        let text = String::from_utf8(data.clone()).unwrap();
        for s in SECRETS {
            assert!(!name.contains(s), "{s} in entry name {name}");
            assert!(!text.contains(s), "{s} survived in {name}:\n{text}");
        }
        // The short name survives only inside other words, never alone.
        for tok in text.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
            assert_ne!(tok, SHORT, "short name survived in {name}:\n{text}");
        }
    }

    let log = &report.log_text;
    assert!(!log.contains('\x1b'), "colour codes are stripped");
    assert!(!log.contains("continuation"), "untimed lines are dropped");
    assert!(!log.contains("name="), "name field copied:\n{log}");
    assert!(log.contains("row read row=4 conf=0.41"), "{log}");
    for want in [
        "sync with token [sync-token]",
        "Authorization: Bearer [sync-token]",
        "blob [sync-token] end",
        "posting to [server-url] failed",
        // Only the home prefix is replaced; the folders under it stay.
        "opened [home]/.local/share/scuffed-stat-tracker/x.db",
        "silverblue [home]/.config/y",
        "account [user] on [host] ok",
        "tag [battletag] joined",
        "scoreboard names [player] [player] self [player]",
        "misread [player] and short [player]",
        "capture timing grabs=60 grab_ms_p50=3",
    ] {
        assert!(log.contains(want), "missing {want:?} in:\n{log}");
    }
    assert_eq!(log.lines().count(), 12, "{log}");

    let m: Value = serde_json::from_str(&report.manifest_json).unwrap();
    assert_eq!(
        m["reason"]["text"],
        "it crashed, [player] was there, token [sync-token], see [server-url]"
    );
}

#[test]
fn empty_window_writes_the_placeholder_line() {
    let dir = tempfile::tempdir().unwrap();
    seed_logs(dir.path());
    let window = LogWindow::log_only(t("2030-01-01T00:00:00Z"));
    let report = bundle::build_log_report(&input(dir.path(), window)).unwrap();
    assert_eq!(report.log_text, "[no log lines in window]\n");
    // No log files at all behaves the same.
    let empty = tempfile::tempdir().unwrap();
    let window = LogWindow::log_only(t("2026-10-09T12:05:00Z"));
    let report = bundle::build_log_report(&input(empty.path(), window)).unwrap();
    assert_eq!(report.log_text, "[no log lines in window]\n");
}

#[test]
fn crash_window_26_hours_after_last_seen() {
    // last_seen 2026-10-09T10:00Z, report built 26 hours later.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("last_crash.marker"),
        "pid=4242\nstarted_at=2026-10-09T08:00:00Z\nlast_seen=2026-10-09T10:00:00Z\n",
    )
    .unwrap();
    let mut raw = String::new();
    for (ts, tag) in [
        ("2026-10-09T08:00:00Z", "qxstart_out"), // start time is not an edge
        ("2026-10-09T09:29:59Z", "qxbefore_out"),
        ("2026-10-09T09:30:00Z", "qxcrash_first_in"),
        ("2026-10-09T09:59:00Z", "qxcrash_in"),
        ("2026-10-09T10:01:30Z", "qxafter_seen_in"),
        ("2026-10-09T10:02:01Z", "qxgap_out_1"),
        ("2026-10-09T22:00:00Z", "qxgap_out_2"),
        ("2026-10-10T11:29:59Z", "qxgap_out_3"),
        ("2026-10-10T11:30:00Z", "qxbuild_first_in"),
        ("2026-10-10T11:59:00Z", "qxbuild_in"),
    ] {
        raw.push_str(&line(ts, "x", tag));
    }
    std::fs::write(dir.path().join("daemon.log"), raw).unwrap();

    let built = t("2026-10-10T12:00:00Z");
    let seen = super::marker::last_crash_seen(dir.path());
    assert_eq!(seen, Some(t("2026-10-09T10:00:00Z")));
    let window = LogWindow::for_log_only(built, true, seen);
    let report = bundle::build_log_report(&input(dir.path(), window)).unwrap();
    let log = &report.log_text;
    for tag in [
        "qxcrash_first_in",
        "qxcrash_in",
        "qxafter_seen_in",
        "qxbuild_first_in",
        "qxbuild_in",
    ] {
        assert!(log.contains(tag), "{tag} missing:\n{log}");
    }
    for tag in [
        "qxstart_out",
        "qxbefore_out",
        "qxgap_out_1",
        "qxgap_out_2",
        "qxgap_out_3",
    ] {
        assert!(!log.contains(tag), "{tag} kept:\n{log}");
    }
    // A performance report ignores the marker.
    let perf = LogWindow::for_log_only(built, false, seen);
    let mut inp = input(dir.path(), perf);
    inp.category = LogOnlyCategory::Performance;
    let log = bundle::build_log_report(&inp).unwrap().log_text;
    assert!(
        !log.contains("qxcrash_in") && log.contains("qxbuild_in"),
        "{log}"
    );
}

const SYSTEM_KEYS: &[&str] = &[
    "session_type",
    "capture_backend",
    "cpu_threads",
    "ocr_threads",
    "reader",
    "shadow_recognizer",
    "update_channel",
    "daemon_running",
    "last_exit",
];
const READ_KEYS: &[&str] = &["mode", "result", "hero", "e", "a", "d", "dmg", "h", "mit"];

fn keys(v: &Value) -> Vec<String> {
    let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
    k.sort();
    k
}

fn sorted(list: &[&str]) -> Vec<String> {
    let mut k: Vec<String> = list.iter().map(|s| s.to_string()).collect();
    k.sort();
    k
}

fn is_sha256(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Section 2 rules for a bundle_version 2 log-only manifest, read from the
/// spec text (the v2 validator lands separately).
fn check_log_only_manifest(m: &Value) {
    assert_eq!(
        keys(m),
        sorted(&[
            "bundle_version",
            "app_version",
            "recognizers",
            "resolution",
            "ui_scale",
            "reason",
            "system",
            "session_id",
            "game",
            "reads",
            "corrections",
            "consent",
            "files",
        ])
    );
    assert_eq!(m["bundle_version"], 2);
    let app = m["app_version"].as_str().unwrap();
    assert!((1..=32).contains(&app.chars().count()));
    assert_eq!(keys(&m["recognizers"]), sorted(&["matcher", "ocr"]));
    assert_eq!(
        m["recognizers"]["matcher"],
        crate::shadow::digits::RECOGNIZER_ID
    );
    assert_eq!(m["recognizers"]["ocr"], "ocr-v1");
    assert_eq!(keys(&m["resolution"]), sorted(&["width", "height"]));
    assert!(m["resolution"]["width"].as_u64().unwrap() > 0);
    assert!(m["resolution"]["height"].as_u64().unwrap() > 0);
    assert!(
        m["ui_scale"].is_null()
            || m["ui_scale"]
                .as_f64()
                .is_some_and(|s| s > 0.0 && s <= 200.0)
    );
    assert_eq!(keys(&m["reason"]), sorted(&["category", "text"]));
    let cat = m["reason"]["category"].as_str().unwrap();
    assert!(["crash", "performance", "other"].contains(&cat));
    assert!(m["reason"]["text"].as_str().unwrap().chars().count() <= 500);

    let s = &m["system"];
    assert_eq!(keys(s), sorted(SYSTEM_KEYS));
    assert!(["x11", "wayland", "unknown"].contains(&s["session_type"].as_str().unwrap()));
    assert!(["x11", "wayshot", "portal", "none"].contains(&s["capture_backend"].as_str().unwrap()));
    assert!(s["cpu_threads"].as_u64().unwrap() >= 1);
    assert!(s["ocr_threads"].as_u64().unwrap() >= 1);
    assert!(["ocr-v1", "new"].contains(&s["reader"].as_str().unwrap()));
    assert!(s["shadow_recognizer"].is_boolean());
    assert!(["stable", "prerelease"].contains(&s["update_channel"].as_str().unwrap()));
    assert!(s["daemon_running"].is_boolean());
    assert!(["clean", "crashed", "unknown"].contains(&s["last_exit"].as_str().unwrap()));

    assert_eq!(m["session_id"], "none");
    assert!(m["game"].is_null());
    assert_eq!(keys(&m["reads"]), sorted(READ_KEYS));
    for k in READ_KEYS {
        assert_eq!(
            m["reads"][k],
            serde_json::json!({"value": null, "confidence": null, "suspect": true, "ocr_v1": null}),
            "reads.{k}"
        );
    }
    assert_eq!(m["corrections"], serde_json::json!({}));
    assert_eq!(
        keys(&m["consent"]),
        sorted(&["training", "own_name_included", "glyphs_included"])
    );
    assert!(m["consent"]["training"].is_boolean());
    assert_eq!(m["consent"]["own_name_included"], false);
    assert_eq!(m["consent"]["glyphs_included"], false);

    let files = m["files"].as_array().unwrap();
    assert_eq!(files.len(), 1, "log-only: log.txt only");
    let f = &files[0];
    assert_eq!(
        keys(f),
        sorted(&["path", "sha256", "bytes", "role", "screen_class"])
    );
    assert_eq!(f["path"], "log.txt");
    assert!(is_sha256(f["sha256"].as_str().unwrap()));
    assert!(f["bytes"].as_u64().unwrap() > 0);
    assert_eq!(f["role"], "log");
    assert!(f["screen_class"].is_null());
}

#[test]
fn manifest_matches_section_2_for_every_log_only_category() {
    let dir = tempfile::tempdir().unwrap();
    seed_logs(dir.path());
    for category in [
        LogOnlyCategory::Crash,
        LogOnlyCategory::Performance,
        LogOnlyCategory::Other,
    ] {
        let mut inp = input(
            dir.path(),
            LogWindow::session(t("2026-10-09T12:00:00Z"), t("2026-10-09T12:10:00Z")),
        );
        inp.category = category;
        let report = bundle::build_log_report(&inp).unwrap();
        let m: Value = serde_json::from_str(&report.manifest_json).unwrap();
        check_log_only_manifest(&m);
        assert_eq!(m["reason"]["category"], category.as_str());
        assert_eq!(
            m["consent"]["training"], false,
            "training is off by default"
        );
        assert_eq!(m["system"]["last_exit"], "crashed");
        assert_eq!(m["system"]["update_channel"], "prerelease");
        assert_eq!(m["app_version"], bundle::APP_VERSION);
    }
    let mut inp = input(dir.path(), LogWindow::log_only(t("2026-10-09T12:06:00Z")));
    inp.training = true;
    let m: Value =
        serde_json::from_str(&bundle::build_log_report(&inp).unwrap().manifest_json).unwrap();
    assert_eq!(m["consent"]["training"], true);
}

#[test]
fn bad_input_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let mut inp = input(dir.path(), LogWindow::log_only(t("2026-10-09T12:06:00Z")));
    inp.resolution.width = 0;
    assert!(bundle::build_log_report(&inp).is_err());
    let mut inp = input(dir.path(), LogWindow::log_only(t("2026-10-09T12:06:00Z")));
    inp.ui_scale = Some(250.0);
    assert!(bundle::build_log_report(&inp).is_err());
}

#[test]
fn reason_text_is_capped_at_500_characters_after_scrub() {
    let text = "é".repeat(600);
    assert_eq!(
        bundle::scrub_reason_text(&ScrubContext::default(), &text)
            .chars()
            .count(),
        500
    );
}

fn le16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// Walk the central directory by hand and check each header field the
/// spec fixes. Returns the entry names in zip order.
fn raw_zip_entries(b: &[u8]) -> Vec<String> {
    let eocd = (0..=b.len() - 22)
        .rev()
        .find(|&i| le32(b, i) == 0x0605_4b50)
        .expect("end of central directory");
    assert_eq!(le16(b, eocd + 20), 0, "zip comment");
    let count = le16(b, eocd + 10) as usize;
    let mut at = le32(b, eocd + 16) as usize;
    let mut names = Vec::new();
    for _ in 0..count {
        assert_eq!(le32(b, at), 0x0201_4b50, "central header");
        assert_eq!(le16(b, at + 12), 0, "central mod time is 00:00:00");
        assert_eq!(le16(b, at + 14), 0x0021, "central mod date is 1980-01-01");
        let name_len = le16(b, at + 28) as usize;
        assert_eq!(le16(b, at + 30), 0, "central extra field");
        assert_eq!(le16(b, at + 32), 0, "entry comment");
        let local = le32(b, at + 42) as usize;
        let name = String::from_utf8(b[at + 46..at + 46 + name_len].to_vec()).unwrap();
        assert_eq!(le32(b, local), 0x0403_4b50, "local header");
        assert_eq!(le16(b, local + 10), 0, "local mod time is 00:00:00");
        assert_eq!(le16(b, local + 12), 0x0021, "local mod date is 1980-01-01");
        assert_eq!(le16(b, local + 28), 0, "local extra field");
        names.push(name);
        at += 46 + name_len;
    }
    names
}

#[test]
fn zip_fixture_dates_entries_hashes_and_bom() {
    let dir = tempfile::tempdir().unwrap();
    seed_logs(dir.path());
    let report = bundle::build_log_report(&input(
        dir.path(),
        LogWindow::session(t("2026-10-09T12:00:00Z"), t("2026-10-09T12:10:00Z")),
    ))
    .unwrap();

    assert_eq!(raw_zip_entries(&report.bytes), ["manifest.json", "log.txt"]);

    let mut ar = zip::ZipArchive::new(std::io::Cursor::new(&report.bytes)).unwrap();
    assert_eq!(ar.comment(), b"");
    for i in 0..ar.len() {
        let f = ar.by_index(i).unwrap();
        let dt = f.last_modified().unwrap();
        assert_eq!(
            (
                dt.year(),
                dt.month(),
                dt.day(),
                dt.hour(),
                dt.minute(),
                dt.second()
            ),
            (1980, 1, 1, 0, 0, 0)
        );
        assert!(f.extra_data().is_none_or(|e| e.is_empty()));
        assert!(!f.name().contains('\\') && !f.name().starts_with('/'));
    }
    drop(ar);

    let entries = unzip(&report.bytes);
    let manifest = &entries[0].1;
    assert!(
        !manifest.starts_with(&[0xEF, 0xBB, 0xBF]),
        "manifest has a BOM"
    );
    assert_eq!(manifest, report.manifest_json.as_bytes());
    let log = &entries[1].1;
    assert!(!log.starts_with(&[0xEF, 0xBB, 0xBF]));
    assert!(std::str::from_utf8(log).is_ok());
    assert_eq!(log, report.log_text.as_bytes());

    let m: Value = serde_json::from_slice(manifest).unwrap();
    let listed = m["files"].as_array().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["sha256"], bundle::sha256_hex(log));
    assert_eq!(listed[0]["bytes"], log.len() as u64);
    assert_eq!(report.files[0].sha256, bundle::sha256_hex(log));
}

/// The real server validator (`scuffed_types`, #229) on this build's output.
#[test]
fn server_validator_accepts_log_only_manifest() {
    let dir = tempfile::tempdir().unwrap();
    seed_logs(dir.path());
    for category in [
        LogOnlyCategory::Crash,
        LogOnlyCategory::Performance,
        LogOnlyCategory::Other,
    ] {
        let mut inp = input(dir.path(), LogWindow::log_only(t("2026-10-09T12:06:00Z")));
        inp.category = category;
        let report = bundle::build_log_report(&inp).unwrap();
        let checked =
            scuffed_types::stat_report::parse_bundle_manifest(report.manifest_json.as_bytes())
                .unwrap_or_else(|e| panic!("{}: {e}", category.as_str()));
        assert_eq!(checked.reason_category, category.as_str());
        assert_eq!(checked.files.len(), 1);
    }
}

/// The daemon's own OCR log lines carry other players' names in fields that
/// are not called `name`. A log-only report has no name list, so the fields
/// themselves must go. Shapes copied from main.rs:4435 and match_end.rs.
#[test]
fn real_ocr_text_fields_are_dropped_without_a_name_list() {
    use super::scrub::{ScrubContext, scrub_log};
    let unknown = ["QXSTRANGER", "QXOTHERTEAM", "QXPOTGHERO", "QXACCOLADE"];
    let raw = [
        line(
            "2026-10-09T12:05:00.000000Z",
            "scuffed_stat_tracker",
            "OCR result confidence=81 text_preview=QXSTRANGER 12 4 3 QXOTHERTEAM 9 2",
        ),
        line(
            "2026-10-09T12:05:01.000000Z",
            "stat_tracker::detect::match_end",
            "end-reel title OCR matched text=PLAY OF THE GAME QXPOTGHERO context=\"potg\"",
        ),
        line(
            "2026-10-09T12:05:02.000000Z",
            "stat_tracker::detect::match_end",
            "map read from accolade screen map=Busan raw=QXACCOLADE BUSAN",
        ),
        line(
            "2026-10-09T12:05:03.000000Z",
            "stat_tracker::detect::match_end",
            "no result word in region ocr_text=QXSTRANGER context=\"end title\"",
        ),
    ]
    .concat();
    let window = LogWindow::log_only(t("2026-10-09T12:06:00Z"));
    let out = scrub_log(&ScrubContext::default(), &window, &raw);
    for name in unknown {
        assert!(!out.contains(name), "{name} survived:\n{out}");
    }
    assert!(out.contains("OCR result"), "the message stays:\n{out}");
    assert!(out.contains("map=Busan"), "non-text fields stay:\n{out}");
    assert!(
        out.contains("confidence=81"),
        "non-text fields stay:\n{out}"
    );
}

// --- Game reports ---

mod game {
    use super::*;
    use crate::report::blank::TeamLayout;
    use crate::report::bundle::{GameCategory, GameReportInput, build_game_report};
    use crate::report::kit::{self, KitRead, KitReads};
    use image::{DynamicImage, Rgb, RgbImage};
    use std::collections::BTreeMap;

    fn reads(team_size: Option<u32>) -> KitReads {
        let mut r = BTreeMap::new();
        for key in ["mode", "result", "hero", "e", "a", "d", "dmg", "h", "mit"] {
            r.insert(
                key.to_string(),
                KitRead {
                    value: Some(if key == "hero" {
                        "Ana".into()
                    } else {
                        "7".into()
                    }),
                    confidence: None,
                    ocr_v1: Some("7".into()),
                    suspect: key == "mit",
                },
            );
        }
        KitReads {
            captured_at: t("2026-10-09T12:05:00Z"),
            width: 1920,
            height: 1080,
            team_size,
            map: Some("Ilios".into()),
            mode: Some("Control".into()),
            result: Some("Victory".into()),
            recognizer: "cv-v5".into(),
            blank_table: crate::report::blank::BLANK_TABLE_VERSION,
            scoreboard_sha256: String::new(),
            reads: r,
        }
    }

    /// A frame with a bright "name" in every name cell, so a blank miss
    /// shows as a non-fill pixel.
    fn frame() -> DynamicImage {
        DynamicImage::ImageRgb8(RgbImage::from_fn(1920, 1080, |x, y| {
            Rgb([
                ((x * 3 + y) % 200 + 50) as u8,
                230,
                ((x + y * 5) % 200 + 50) as u8,
            ])
        }))
    }

    fn input(dir: &std::path::Path, category: GameCategory) -> GameReportInput {
        GameReportInput {
            session_id: "sess-1".into(),
            category,
            text: format!("hero was wrong, {OTHER_1} said so"),
            resolution: Resolution {
                width: 1920,
                height: 1080,
            },
            ui_scale: None,
            system: system(),
            training: true,
            scrub: ctx(),
            window: LogWindow::session(t("2026-10-09T12:00:00Z"), t("2026-10-09T12:10:00Z")),
            log_dir: dir.to_path_buf(),
            kit: kit::load(dir, "sess-1"),
            corrections: BTreeMap::from([("hero".to_string(), "Kiriko".to_string())]),
            built_at: t("2026-10-09T13:00:00Z"),
        }
    }

    fn unzip(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut out = BTreeMap::new();
        for i in 0..z.len() {
            let mut f = z.by_index(i).unwrap();
            let mut buf = Vec::new();
            f.read_to_end(&mut buf).unwrap();
            out.insert(f.name().to_string(), buf);
        }
        out
    }

    #[test]
    fn game_report_passes_the_server_validator_with_both_crops() {
        let tmp = tempfile::tempdir().unwrap();
        seed_logs(tmp.path());
        kit::record_scoreboard(
            tmp.path(),
            "sess-1",
            &frame(),
            TeamLayout::Five,
            Some(0),
            &reads(Some(5)),
        )
        .unwrap();
        let rank = DynamicImage::ImageRgb8(RgbImage::from_pixel(1920, 1080, Rgb([24, 20, 18])));
        let png = kit::rank_outcome_png(
            Some(crate::detect::match_end::OutcomeSource::RankScreen),
            &rank,
        )
        .unwrap();
        kit::record_outcome_png(tmp.path(), "sess-1", &png).unwrap();

        let zip = build_game_report(&input(tmp.path(), GameCategory::WrongHero)).unwrap();
        let files = unzip(&zip.bytes);
        assert_eq!(
            files.keys().cloned().collect::<Vec<_>>(),
            vec![
                "crops/outcome.png",
                "crops/scoreboard.png",
                "log.txt",
                "manifest.json"
            ]
        );
        let checked = scuffed_types::stat_report::parse_bundle_manifest(&files["manifest.json"])
            .expect("server validator accepts the manifest");
        let _ = checked;
        let m: Value = serde_json::from_slice(&files["manifest.json"]).unwrap();
        assert_eq!(m["reason"]["category"], "wrong_hero");
        assert_eq!(m["session_id"], "sess-1");
        assert_eq!(m["game"]["team_size"], 5);
        assert_eq!(m["game"]["captured_at"], "2026-10-09T12:05:00Z");
        assert_eq!(m["reads"]["hero"]["value"], "Ana");
        assert_eq!(m["reads"]["mit"]["suspect"], true);
        assert_eq!(m["corrections"]["hero"], "Kiriko");
        assert_eq!(m["consent"]["training"], true);
        // The free text and the log are scrubbed like a log-only report.
        let text = m["reason"]["text"].as_str().unwrap();
        assert!(!text.contains(OTHER_1), "{text}");
        let log = String::from_utf8(files["log.txt"].clone()).unwrap();
        for secret in [TOKEN, OTHER_1, SELF_NAME, HOSTNAME, ACCOUNT] {
            assert!(!log.contains(secret), "{secret} leaked into log.txt");
        }
        // Crops in the zip are the preview's bytes, and every name cell of
        // the scoreboard is the fill colour.
        assert_eq!(zip.images.len(), 2);
        for img in &zip.images {
            assert_eq!(files[&img.path], img.png);
        }
        let board = image::load_from_memory(&files["crops/scoreboard.png"])
            .unwrap()
            .to_rgb8();
        for n in crate::report::blank::name_rects(board.width(), board.height(), TeamLayout::Five) {
            for y in n.y..n.y + n.h {
                for x in n.x..n.x + n.w {
                    assert_eq!(*board.get_pixel(x, y), crate::report::blank::FILL);
                }
            }
        }
    }

    #[test]
    fn unknown_team_size_reports_null_and_blanks_both_layouts() {
        let tmp = tempfile::tempdir().unwrap();
        kit::record_scoreboard(
            tmp.path(),
            "sess-1",
            &frame(),
            TeamLayout::Unknown,
            None,
            &reads(None),
        )
        .unwrap();
        let zip = build_game_report(&input(tmp.path(), GameCategory::WrongStats)).unwrap();
        let files = unzip(&zip.bytes);
        scuffed_types::stat_report::parse_bundle_manifest(&files["manifest.json"]).unwrap();
        let m: Value = serde_json::from_slice(&files["manifest.json"]).unwrap();
        assert!(m["game"]["team_size"].is_null());
        assert!(!files.contains_key("crops/outcome.png"));
    }

    #[test]
    fn a_game_without_a_kit_cannot_be_reported_except_as_missed() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(build_game_report(&input(tmp.path(), GameCategory::WrongStats)).is_err());
        let zip = build_game_report(&input(tmp.path(), GameCategory::MissedGame)).unwrap();
        let files = unzip(&zip.bytes);
        scuffed_types::stat_report::parse_bundle_manifest(&files["manifest.json"]).unwrap();
        let m: Value = serde_json::from_slice(&files["manifest.json"]).unwrap();
        assert!(m["game"]["map"].is_null());
        assert!(m["game"]["team_size"].is_null());
        assert_eq!(m["game"]["captured_at"], "2026-10-09T13:00:00Z");
        assert_eq!(m["reads"]["e"]["suspect"], true);
        assert!(m["corrections"].as_object().unwrap().is_empty());
        assert_eq!(m["session_id"], "none");
        assert!(zip.images.is_empty());
    }

    #[test]
    fn categories_match_the_validator_list() {
        let ids: Vec<_> = GameCategory::ALL.iter().map(|c| c.as_str()).collect();
        assert_eq!(
            ids,
            [
                "wrong_stats",
                "wrong_hero",
                "wrong_map",
                "wrong_mode",
                "wrong_result",
                "missed_game",
                "other"
            ]
        );
    }
}
