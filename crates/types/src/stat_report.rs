//! Stat-tracker bug-report bundle manifest and API shapes.
//!
//! The zip layout and field rules come from the report-bundle spec (section 2
//! and the API summary in section 8). This module checks the manifest only.
//! Zip bytes, PNG chunks, and storage live in the site server.

use chrono::{DateTime, Utc};
use serde::de::{self, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// Bundle version the tracker writes today (spec revision 2).
pub const BUNDLE_VERSION: u64 = 2;

/// Oldest bundle version a reader accepts. Version 1 is spec revision 1: no
/// `system` object and no `crash` or `performance` category. Readers accept
/// `MIN_BUNDLE_VERSION..=BUNDLE_VERSION` and reject anything else.
pub const MIN_BUNDLE_VERSION: u64 = 1;

/// Compressed request body and total uncompressed size, in bytes.
pub const MAX_BUNDLE_BYTES: u64 = 10 * 1024 * 1024;

/// Files listed in the manifest, not counting `manifest.json`.
pub const MAX_FILES_BESIDES_MANIFEST: usize = 30;

/// Longest allowed image edge, in pixels.
pub const MAX_IMAGE_EDGE: u32 = 7680;

/// Successful stores per member per UTC day. A rejected upload does not count.
pub const DAILY_REPORT_CAP: u64 = 10;

/// How long a report without training consent is kept.
pub const RETENTION_DAYS: i64 = 30;

const READ_KEYS: &[&str] = &["mode", "result", "hero", "e", "a", "d", "dmg", "h", "mit"];

const REASON_CATEGORIES: &[&str] = &[
    "wrong_stats",
    "wrong_hero",
    "wrong_map",
    "wrong_mode",
    "wrong_result",
    "missed_game",
    "crash",
    "performance",
    "other",
];

/// Categories that are not about one game. They need bundle version 2 and
/// carry `game: null`, null reads, `session_id` `none`, and only `log.txt`.
const LOG_ONLY_CATEGORIES: &[&str] = &["crash", "performance"];

/// `session_id` for a report that has no game session.
const NO_SESSION: &str = "none";

const SESSION_TYPES: &[&str] = &["x11", "wayland", "unknown"];
const CAPTURE_BACKENDS: &[&str] = &["x11", "wayshot", "portal", "none"];
const READERS: &[&str] = &["ocr-v1", "new"];
const OCR_SETTINGS: &[&str] = &["on", "off"];
/// Most call sites `system.ocr_blocked_calls` may name.
pub const OCR_BLOCKED_SITES_MAX: usize = 64;
/// Longest call-site key in `system.ocr_blocked_calls`, in bytes.
pub const OCR_BLOCKED_SITE_MAX_BYTES: usize = 96;
const UPDATE_CHANNELS: &[&str] = &["stable", "prerelease"];
const LAST_EXITS: &[&str] = &["clean", "crashed", "unknown"];

const SCREEN_CLASSES: &[&str] = &[
    "scoreboard",
    "gameplay",
    "killcam",
    "potg",
    "accolade",
    "party",
    "lobby",
    "outcome",
];

/// The only crop path that carries the `outcome` screen class, and the only
/// class that path may carry.
const OUTCOME_CROP_PATH: &str = "crops/outcome.png";
const OUTCOME_CLASS: &str = "outcome";

const OCR_ID: &str = "ocr-v1";

/// Manifest plus the JSON object it was parsed from, so the server can rewrite
/// file hashes after it strips PNG chunks.
#[derive(Debug, Clone)]
pub struct CheckedManifest {
    pub app_version: String,
    pub matcher: String,
    pub ocr: String,
    pub reason_category: String,
    pub reason_text: String,
    pub training: bool,
    pub own_name_included: bool,
    pub glyphs_included: bool,
    pub files: Vec<CheckedFile>,
    pub value: Value,
}

#[derive(Debug, Clone)]
pub struct CheckedFile {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
    pub role: FileRole,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileRole {
    Log,
    Crop,
    OwnName,
    Glyph,
}

/// `POST /api/stat-reports` response. The zip is not echoed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatReportCreated {
    pub id: String,
    pub training: bool,
    pub expires_at: Option<DateTime<Utc>>,
}

/// `POST` or `PATCH` withdraw response.
///
/// `deleted` is true when the original received time plus 30 days had already
/// passed, so the row and the file are gone. Withdraw does not move the
/// deadline forward.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatReportWithdrawn {
    pub id: String,
    pub deleted: bool,
    pub training: bool,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatReportDeleted {
    pub deleted: bool,
}

/// One row in `GET /api/stat-reports`.
///
/// Members receive id, time, size, and the three consent flags. Officer-only
/// fields are omitted for a member (`skip_serializing_if`). Free text is one
/// of those fields.
///
/// `display_name` sits beside `member_id` on an officer row. It is the member
/// record's display name, or JSON null when that member has no name or the
/// member record is gone. A member's own list omits it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatReportListItem {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub training_consent: bool,
    pub own_name_included: bool,
    pub glyphs_included: bool,
    pub size_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub member_id: Option<String>,
    /// Outer `None` omits the field (member list). `Some(None)` is JSON null.
    /// `Some(Some(name))` is the display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recognizer_matcher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recognizer_ocr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zip_sha256: Option<String>,
}

/// Most reports one list response carries. The DB query reads one more row so
/// the route can tell that older reports were left out.
pub const STAT_REPORT_LIST_LIMIT: usize = 1000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatReportList {
    pub reports: Vec<StatReportListItem>,
    /// True when older reports exist past [`STAT_REPORT_LIST_LIMIT`].
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    bundle_version: u64,
    app_version: String,
    recognizers: RawRecognizers,
    resolution: RawResolution,
    ui_scale: Option<f64>,
    reason: RawReason,
    /// Optional, version 2 only. Absent is the only way to leave it out;
    /// `null` is rejected.
    #[serde(default, deserialize_with = "present")]
    system: Option<RawSystem>,
    session_id: String,
    /// Required key. `null` only in a report without a game.
    #[serde(deserialize_with = "nullable")]
    game: Option<RawGame>,
    reads: RawReads,
    corrections: BTreeMap<String, String>,
    consent: RawConsent,
    files: Vec<RawFile>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRecognizers {
    matcher: String,
    ocr: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawResolution {
    width: u32,
    height: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawReason {
    category: String,
    text: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSystem {
    session_type: String,
    capture_backend: String,
    cpu_threads: u32,
    ocr_threads: u32,
    reader: String,
    /// Optional. `on` or `off`: whether the tracker ran classic OCR
    /// (Tesseract). A tracker from before the OCR switch leaves it out,
    /// which means `on`. Present, it is not `null`.
    #[serde(default, deserialize_with = "present")]
    ocr: Option<String>,
    /// Optional, and only with `ocr` `off`. OCR calls the tracker stopped
    /// while OCR was off, per call site: a place in the tracker's source,
    /// never text read from a screen. See [`is_ocr_call_site`] for the
    /// shape of a key and [`check_ocr_blocked_calls`] for the bounds.
    #[serde(default, deserialize_with = "present")]
    ocr_blocked_calls: Option<BTreeMap<String, u32>>,
    #[allow(dead_code)]
    shadow_recognizer: bool,
    update_channel: String,
    #[allow(dead_code)]
    daemon_running: bool,
    last_exit: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct RawGame {
    map: Option<String>,
    mode: Option<String>,
    result: Option<String>,
    /// Required key. `null` means the app could not tell 5v5 from 6v6.
    #[serde(deserialize_with = "nullable")]
    team_size: Option<u32>,
    captured_at: DateTime<Utc>,
}

/// All nine keys are required, and each is an object. A read is never JSON
/// `null` as a whole, not even in a report without a game.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawReads {
    mode: RawRead,
    result: RawRead,
    hero: RawRead,
    e: RawRead,
    a: RawRead,
    d: RawRead,
    dmg: RawRead,
    h: RawRead,
    mit: RawRead,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct RawRead {
    value: Option<String>,
    confidence: Option<f64>,
    suspect: bool,
    ocr_v1: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConsent {
    training: bool,
    own_name_included: bool,
    glyphs_included: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    path: String,
    sha256: String,
    bytes: u64,
    role: String,
    screen_class: Option<String>,
    #[serde(default, deserialize_with = "absent_or_string")]
    id: Option<String>,
    #[serde(default, deserialize_with = "absent_or_string")]
    reader_guess: Option<String>,
    #[serde(default, deserialize_with = "absent_or_f64")]
    confidence: Option<f64>,
    #[serde(default, deserialize_with = "absent_or_string")]
    correct_char: Option<String>,
}

/// Parse `manifest.json` bytes against the bundle schema.
pub fn parse_bundle_manifest(bytes: &[u8]) -> Result<CheckedManifest, String> {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return Err("manifest has a byte order mark".into());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "manifest is not utf-8".to_string())?;
    let value = parse_json_no_duplicate_keys(text)?;
    if !value.is_object() {
        return Err("manifest must be a json object".into());
    }
    let raw: RawManifest = serde_json::from_value(value.clone()).map_err(schema_err)?;
    check_manifest(raw, value)
}

fn schema_err(err: serde_json::Error) -> String {
    let msg: String = err.to_string().chars().take(180).collect();
    if msg.contains("duplicate json key") {
        return "manifest has a duplicate json key".into();
    }
    format!("manifest does not match the schema: {msg}")
}

/// Parse JSON and reject a repeated key in any object, including nested ones
/// and objects inside arrays. `serde_json::Value` would keep the last value.
fn parse_json_no_duplicate_keys(text: &str) -> Result<Value, String> {
    let mut de = serde_json::Deserializer::from_str(text);
    let value = JsonSeed.deserialize(&mut de).map_err(schema_err)?;
    de.end().map_err(schema_err)?;
    Ok(value)
}

struct JsonSeed;

impl<'de> DeserializeSeed<'de> for JsonSeed {
    type Value = Value;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Value, D::Error> {
        deserializer.deserialize_any(JsonVisitor)
    }
}

struct JsonVisitor;

impl<'de> Visitor<'de> for JsonVisitor {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("json")
    }

    fn visit_bool<E: de::Error>(self, value: bool) -> Result<Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Value, E> {
        Ok(Value::Number(value.into()))
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Value, E> {
        Ok(Value::Number(value.into()))
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Value, E> {
        let number = serde_json::Number::from_f64(value)
            .ok_or_else(|| de::Error::custom("manifest number is not finite"))?;
        Ok(Value::Number(number))
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Value, E> {
        Ok(Value::String(value.to_owned()))
    }

    fn visit_string<E: de::Error>(self, value: String) -> Result<Value, E> {
        Ok(Value::String(value))
    }

    fn visit_unit<E: de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut out = Vec::new();
        while let Some(item) = seq.next_element_seed(JsonSeed)? {
            out.push(item);
        }
        Ok(Value::Array(out))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut out = serde_json::Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if out.contains_key(&key) {
                return Err(de::Error::custom("manifest has a duplicate json key"));
            }
            let value = map.next_value_seed(JsonSeed)?;
            out.insert(key, value);
        }
        Ok(Value::Object(out))
    }
}

fn check_manifest(raw: RawManifest, value: Value) -> Result<CheckedManifest, String> {
    let version = raw.bundle_version;
    if !(MIN_BUNDLE_VERSION..=BUNDLE_VERSION).contains(&version) {
        return Err("bundle_version must be 1 or 2".into());
    }
    check_short("app_version", &raw.app_version, 1, 32)?;
    check_short("recognizers.matcher", &raw.recognizers.matcher, 1, 64)?;
    if raw.recognizers.ocr != OCR_ID {
        return Err("recognizers.ocr must be ocr-v1".into());
    }
    if raw.resolution.width == 0
        || raw.resolution.height == 0
        || raw.resolution.width > MAX_IMAGE_EDGE
        || raw.resolution.height > MAX_IMAGE_EDGE
    {
        return Err("resolution is out of range".into());
    }
    if let Some(scale) = raw.ui_scale
        && (!scale.is_finite() || scale <= 0.0 || scale > 200.0)
    {
        return Err("ui_scale is out of range".into());
    }
    let category = raw.reason.category.as_str();
    if !REASON_CATEGORIES.contains(&category) {
        return Err("reason.category is not a known category".into());
    }
    let log_only_category = LOG_ONLY_CATEGORIES.contains(&category);
    if version < 2 {
        if raw.system.is_some() {
            return Err("system needs bundle_version 2".into());
        }
        if log_only_category {
            return Err("reason.category crash and performance need bundle_version 2".into());
        }
    }
    if let Some(system) = &raw.system {
        check_system(system)?;
    }
    if raw.reason.text.chars().count() > 500 {
        return Err("reason.text is longer than 500 characters".into());
    }
    if raw.reason.text.chars().any(|c| c == '\0') {
        return Err("reason.text contains a null".into());
    }
    check_short("session_id", &raw.session_id, 1, 64)?;
    // A report without a game: crash and performance always, and `other`
    // from Report a problem (spec section 7) when it carries `game: null`.
    let log_only = match &raw.game {
        None => {
            if !(log_only_category || (version >= 2 && category == "other")) {
                return Err(
                    "game may be null only in a crash, performance, or other report".into(),
                );
            }
            true
        }
        Some(game) => {
            if log_only_category {
                return Err("a crash or performance report must have game null".into());
            }
            if let Some(size) = game.team_size
                && size != 5
                && size != 6
            {
                return Err("game.team_size must be 5, 6, or null".into());
            }
            check_opt_label("game.map", game.map.as_deref())?;
            check_opt_label("game.mode", game.mode.as_deref())?;
            check_opt_label("game.result", game.result.as_deref())?;
            false
        }
    };
    if log_only {
        if raw.session_id != NO_SESSION {
            return Err("a report without a game must use session_id none".into());
        }
        if raw.consent.own_name_included || raw.consent.glyphs_included {
            return Err("a report without a game cannot include names or glyphs".into());
        }
    }

    check_read("mode", &raw.reads.mode, log_only)?;
    check_read("result", &raw.reads.result, log_only)?;
    check_read("hero", &raw.reads.hero, log_only)?;
    check_read("e", &raw.reads.e, log_only)?;
    check_read("a", &raw.reads.a, log_only)?;
    check_read("d", &raw.reads.d, log_only)?;
    check_read("dmg", &raw.reads.dmg, log_only)?;
    check_read("h", &raw.reads.h, log_only)?;
    check_read("mit", &raw.reads.mit, log_only)?;

    if raw.corrections.len() > READ_KEYS.len() {
        return Err("corrections has too many keys".into());
    }
    for (key, val) in &raw.corrections {
        if !READ_KEYS.contains(&key.as_str()) {
            return Err("corrections has an unknown key".into());
        }
        let n = val.chars().count();
        if n == 0 || n > 64 {
            return Err("a correction must be 1 to 64 characters".into());
        }
    }

    if raw.files.len() > MAX_FILES_BESIDES_MANIFEST {
        return Err("too many files in the manifest".into());
    }
    if raw.files.is_empty() {
        return Err("manifest files must include log.txt".into());
    }

    let mut files = Vec::with_capacity(raw.files.len());
    let mut paths = BTreeSet::new();
    let mut log_count = 0usize;
    let mut glyph_ids = BTreeSet::new();
    let mut declared: u64 = 0;
    for file in &raw.files {
        let role = check_file(file, &raw.consent, &mut glyph_ids)?;
        if log_only && role != FileRole::Log {
            return Err("a report without a game lists only log.txt".into());
        }
        if !paths.insert(file.path.clone()) {
            return Err("duplicate file path".into());
        }
        if role == FileRole::Log {
            log_count += 1;
        }
        declared = declared.saturating_add(file.bytes);
        if declared > MAX_BUNDLE_BYTES {
            return Err("uncompressed size is over the limit".into());
        }
        files.push(CheckedFile {
            path: file.path.clone(),
            sha256: file.sha256.clone(),
            bytes: file.bytes,
            role,
        });
    }
    if log_count != 1 {
        return Err("manifest must list log.txt exactly once".into());
    }

    Ok(CheckedManifest {
        app_version: raw.app_version,
        matcher: raw.recognizers.matcher,
        ocr: raw.recognizers.ocr,
        reason_category: raw.reason.category,
        reason_text: raw.reason.text,
        training: raw.consent.training,
        own_name_included: raw.consent.own_name_included,
        glyphs_included: raw.consent.glyphs_included,
        files,
        value,
    })
}

fn check_file(
    file: &RawFile,
    consent: &RawConsent,
    glyph_ids: &mut BTreeSet<String>,
) -> Result<FileRole, String> {
    if !valid_rel_path(&file.path) {
        return Err("file path is not allowed".into());
    }
    // Local map-label crops are never a report file. Listing one is the
    // same outcome as a missing file: 400, nothing stored.
    if is_debug_mapcrop_path(&file.path) {
        return Err("file missing".into());
    }
    if !is_sha256_hex(&file.sha256) {
        return Err("file sha256 is not 64 lowercase hex characters".into());
    }
    if file.bytes == 0 {
        return Err("file bytes must be positive".into());
    }
    let role = match file.role.as_str() {
        "log" => FileRole::Log,
        "crop" => FileRole::Crop,
        "own_name" => FileRole::OwnName,
        "glyph" => FileRole::Glyph,
        _ => return Err("file role is not known".into()),
    };
    match role {
        FileRole::Log => {
            if file.path != "log.txt" {
                return Err("the log path must be log.txt".into());
            }
            if file.screen_class.is_some() {
                return Err("the log screen_class must be null".into());
            }
            reject_glyph_fields(file)?;
        }
        FileRole::Crop => {
            if !is_plain_crop_path(&file.path) {
                return Err("crop path is not allowed".into());
            }
            let class = match file.screen_class.as_deref() {
                Some(class) if SCREEN_CLASSES.contains(&class) => class,
                _ => return Err("crop screen_class is not a known class".into()),
            };
            if (file.path == OUTCOME_CROP_PATH) != (class == OUTCOME_CLASS) {
                return Err("crop screen_class does not match its path".into());
            }
            reject_glyph_fields(file)?;
        }
        FileRole::OwnName => {
            if !consent.own_name_included {
                return Err("own_name file without consent".into());
            }
            if file.path != "crops/own-name.png" && file.path != "crops/own-name-hud.png" {
                return Err("own_name path is not allowed".into());
            }
            if file.screen_class.is_some() {
                return Err("own_name screen_class must be null".into());
            }
            reject_glyph_fields(file)?;
        }
        FileRole::Glyph => {
            if !consent.glyphs_included {
                return Err("glyph file without consent".into());
            }
            if file.screen_class.is_some() {
                return Err("glyph screen_class must be null".into());
            }
            let id = file
                .id
                .as_deref()
                .filter(|id| is_glyph_id(id))
                .ok_or_else(|| "glyph id must be 8 lowercase hex characters".to_string())?;
            if file.path != format!("crops/glyph-{id}.png") {
                return Err("glyph path does not match its id".into());
            }
            if !glyph_ids.insert(id.to_string()) {
                return Err("duplicate glyph id".into());
            }
            let guess = file
                .reader_guess
                .as_deref()
                .ok_or_else(|| "glyph reader_guess is required".to_string())?;
            if guess.chars().count() != 1 {
                return Err("glyph reader_guess must be one character".into());
            }
            match file.confidence {
                Some(c) if c.is_finite() && (0.0..=1.0).contains(&c) => {}
                _ => return Err("glyph confidence must be from 0 to 1".into()),
            }
            if let Some(ch) = file.correct_char.as_deref()
                && ch.chars().count() != 1
            {
                return Err("glyph correct_char must be one character".into());
            }
        }
    }
    Ok(role)
}

fn reject_glyph_fields(file: &RawFile) -> Result<(), String> {
    if file.id.is_some()
        || file.reader_guess.is_some()
        || file.confidence.is_some()
        || file.correct_char.is_some()
    {
        return Err("only a glyph file may carry letter fields".into());
    }
    Ok(())
}

fn check_system(system: &RawSystem) -> Result<(), String> {
    let pick = |label: &str, value: &str, allowed: &[&str]| {
        if allowed.contains(&value) {
            Ok(())
        } else {
            Err(format!("system.{label} is not a known value"))
        }
    };
    pick("session_type", &system.session_type, SESSION_TYPES)?;
    pick("capture_backend", &system.capture_backend, CAPTURE_BACKENDS)?;
    pick("reader", &system.reader, READERS)?;
    if let Some(ocr) = &system.ocr {
        pick("ocr", ocr, OCR_SETTINGS)?;
    }
    if let Some(blocked) = &system.ocr_blocked_calls {
        // The counts are from a run with OCR off, so they come with it.
        if system.ocr.as_deref() != Some("off") {
            return Err("system.ocr_blocked_calls needs system.ocr off".into());
        }
        check_ocr_blocked_calls(blocked)?;
    }
    pick("update_channel", &system.update_channel, UPDATE_CHANNELS)?;
    pick("last_exit", &system.last_exit, LAST_EXITS)?;
    if system.cpu_threads == 0 {
        return Err("system.cpu_threads must be positive".into());
    }
    if system.ocr_threads == 0 {
        return Err("system.ocr_threads must be positive".into());
    }
    Ok(())
}

/// `system.ocr_blocked_calls`: at most [`OCR_BLOCKED_SITES_MAX`] entries,
/// every key a call site ([`is_ocr_call_site`]). A value is a count that
/// fits a `u32` (the type already says so).
fn check_ocr_blocked_calls(blocked: &BTreeMap<String, u32>) -> Result<(), String> {
    if blocked.len() > OCR_BLOCKED_SITES_MAX {
        return Err("system.ocr_blocked_calls names too many call sites".into());
    }
    if blocked.keys().any(|site| !is_ocr_call_site(site)) {
        return Err("system.ocr_blocked_calls has a key that is not a call site".into());
    }
    Ok(())
}

/// True when `key` has the shape of a place in the tracker's source:
/// `<path>.rs:<line>`, optionally followed by one space and the function
/// that was called there. For example `detect/match_end.rs:1133
/// recognize_prepared`.
///
/// - `<path>` is relative: segments of `a-z`, `0-9` and `_` joined by `/`.
///   So no leading `/`, no `..`, no `~`, no drive letter and no `-`.
/// - `<line>` is 1 to 6 digits and does not start with `0`.
/// - The function is identifiers (`A-Z`, `a-z`, `0-9`, `_`, not starting
///   with a digit) joined by `::`.
/// - The whole key is at most [`OCR_BLOCKED_SITE_MAX_BYTES`] bytes.
///
/// The shape keeps an absolute path, a sentence and most names out. It
/// cannot tell a folder name from a user name: `soot/main.rs:1` passes.
/// The tracker uses the same function, so it never sends a key the site
/// refuses.
pub fn is_ocr_call_site(key: &str) -> bool {
    if key.len() > OCR_BLOCKED_SITE_MAX_BYTES {
        return false;
    }
    let (place, entry) = match key.split_once(' ') {
        Some((place, entry)) => (place, Some(entry)),
        None => (key, None),
    };
    let Some((path, line)) = place.rsplit_once(':') else {
        return false;
    };
    let Some(stem) = path.strip_suffix(".rs") else {
        return false;
    };
    let segment = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    };
    let ident = |s: &str| {
        let mut bytes = s.bytes();
        bytes
            .next()
            .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
            && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
    };
    let line_ok = (1..=6).contains(&line.len())
        && line.bytes().all(|b| b.is_ascii_digit())
        && !line.starts_with('0');
    stem.split('/').all(segment)
        && line_ok
        && entry.is_none_or(|entry| entry.split("::").all(ident))
}

/// In a report without a game, every read is the empty-read object:
/// `value`, `confidence`, and `ocr_v1` null and `suspect` true (spec
/// section 2), so a game read cannot ride along in a crash report.
fn check_read(name: &str, read: &RawRead, log_only: bool) -> Result<(), String> {
    if log_only
        && (read.value.is_some()
            || read.confidence.is_some()
            || read.ocr_v1.is_some()
            || !read.suspect)
    {
        return Err(format!(
            "reads.{name} must be an empty suspect read in a report without a game"
        ));
    }
    if let Some(value) = &read.value {
        check_short(&format!("{name}.value"), value, 0, 64)?;
    }
    if let Some(ocr) = &read.ocr_v1 {
        check_short(&format!("{name}.ocr_v1"), ocr, 0, 64)?;
    }
    if let Some(c) = read.confidence
        && (!c.is_finite() || !(0.0..=1.0).contains(&c))
    {
        return Err(format!("{name}.confidence is out of range"));
    }
    Ok(())
}

fn check_short(label: &str, value: &str, min: usize, max: usize) -> Result<(), String> {
    let n = value.chars().count();
    if n < min || n > max || value.chars().any(|c| c.is_control()) {
        return Err(format!("{label} length is out of range"));
    }
    Ok(())
}

fn check_opt_label(label: &str, value: Option<&str>) -> Result<(), String> {
    if let Some(value) = value {
        check_short(label, value, 0, 64)?;
    }
    Ok(())
}

fn is_debug_mapcrop_path(path: &str) -> bool {
    let segs: Vec<&str> = path
        .split(['/', '\\'])
        .filter(|seg| !seg.is_empty() && *seg != ".")
        .collect();
    segs.windows(2).any(|pair| {
        pair[0].eq_ignore_ascii_case("debug") && pair[1].eq_ignore_ascii_case("mapcrops")
    })
}

fn valid_rel_path(path: &str) -> bool {
    if path.is_empty() || path.len() > 128 {
        return false;
    }
    if path.starts_with('/') || path.starts_with('.') || path.ends_with('/') {
        return false;
    }
    if path.contains('\\') || path.contains('\0') || path.contains(':') || path.contains("//") {
        return false;
    }
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        return false;
    }
    for seg in path.split('/') {
        if seg.is_empty() || seg == "." || seg == ".." || seg.starts_with('.') {
            return false;
        }
        if !seg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
        {
            return false;
        }
    }
    true
}

/// Crop paths the spec names. Suspect-cell files are not given names there,
/// so they are not accepted.
const CROP_PATHS: &[&str] = &["crops/scoreboard.png", "crops/frame.png", OUTCOME_CROP_PATH];

fn is_plain_crop_path(path: &str) -> bool {
    CROP_PATHS.contains(&path)
}

fn is_glyph_id(id: &str) -> bool {
    id.len() == 8 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Required key whose value may be `null`. Unlike a bare `Option` field, a
/// missing key is a schema error.
fn nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

/// Optional key that, when present, is not `null`. Use with
/// `#[serde(default)]`.
fn present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

fn absent_or_string<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct V;
    impl<'de> serde::de::Visitor<'de> for V {
        type Value = Option<String>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a string")
        }
        fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Option<String>, E> {
            Ok(Some(v.to_owned()))
        }
        fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Option<String>, E> {
            Ok(Some(v))
        }
        fn visit_unit<E: serde::de::Error>(self) -> Result<Option<String>, E> {
            Err(E::custom("null is not allowed"))
        }
        fn visit_none<E: serde::de::Error>(self) -> Result<Option<String>, E> {
            Err(E::custom("null is not allowed"))
        }
    }
    deserializer.deserialize_any(V)
}

fn absent_or_f64<'de, D>(deserializer: D) -> Result<Option<f64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct V;
    impl<'de> serde::de::Visitor<'de> for V {
        type Value = Option<f64>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a number")
        }
        fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Option<f64>, E> {
            Ok(Some(v))
        }
        fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Option<f64>, E> {
            Ok(Some(v as f64))
        }
        fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Option<f64>, E> {
            Ok(Some(v as f64))
        }
        fn visit_unit<E: serde::de::Error>(self) -> Result<Option<f64>, E> {
            Err(E::custom("null is not allowed"))
        }
        fn visit_none<E: serde::de::Error>(self) -> Result<Option<f64>, E> {
            Err(E::custom("null is not allowed"))
        }
    }
    deserializer.deserialize_any(V)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> serde_json::Value {
        serde_json::json!({
            "bundle_version": 1,
            "app_version": "0.0.0",
            "recognizers": { "matcher": "cv-v3", "ocr": "ocr-v1" },
            "resolution": { "width": 2560, "height": 1440 },
            "ui_scale": null,
            "reason": { "category": "wrong_stats", "text": "elims looked high" },
            "session_id": "sess-example",
            "game": {
                "map": "Busan",
                "mode": "Control",
                "result": "Defeat",
                "team_size": 6,
                "captured_at": "2026-10-09T12:00:00Z"
            },
            "reads": {
                "mode": { "value": "Control", "confidence": null, "suspect": false, "ocr_v1": "Control" },
                "result": { "value": "Defeat", "confidence": null, "suspect": false, "ocr_v1": "Defeat" },
                "hero": { "value": "Ana", "confidence": null, "suspect": false, "ocr_v1": "Ana" },
                "e": { "value": "21", "confidence": 0.22, "suspect": true, "ocr_v1": "20" },
                "a": { "value": "8", "confidence": 0.9, "suspect": false, "ocr_v1": "8" },
                "d": { "value": "4", "confidence": 0.88, "suspect": false, "ocr_v1": "4" },
                "dmg": { "value": "8432", "confidence": 0.8, "suspect": false, "ocr_v1": "8432" },
                "h": { "value": "2100", "confidence": 0.77, "suspect": false, "ocr_v1": "2100" },
                "mit": { "value": "0", "confidence": 0.7, "suspect": false, "ocr_v1": "0" }
            },
            "corrections": { "e": "20" },
            "consent": {
                "training": false,
                "own_name_included": false,
                "glyphs_included": false
            },
            "files": [
                {
                    "path": "log.txt",
                    "sha256": "0000000000000000000000000000000000000000000000000000000000000000",
                    "bytes": 24,
                    "role": "log",
                    "screen_class": null
                },
                {
                    "path": "crops/scoreboard.png",
                    "sha256": "1111111111111111111111111111111111111111111111111111111111111111",
                    "bytes": 1000,
                    "role": "crop",
                    "screen_class": "scoreboard"
                }
            ]
        })
    }

    #[test]
    fn spec_example_parses() {
        let raw = serde_json::to_vec(&sample()).unwrap();
        let checked = parse_bundle_manifest(&raw).unwrap();
        assert!(!checked.training);
        assert_eq!(checked.files.len(), 2);
        assert_eq!(checked.matcher, "cv-v3");
    }

    #[test]
    fn unknown_key_is_rejected() {
        let mut v = sample();
        v["extra"] = serde_json::json!(true);
        let err = parse_bundle_manifest(&serde_json::to_vec(&v).unwrap()).unwrap_err();
        assert!(err.contains("schema"), "{err}");
    }

    #[test]
    fn other_version_is_rejected() {
        for version in [0, 3] {
            let mut v = sample();
            v["bundle_version"] = serde_json::json!(version);
            let err = parse_bundle_manifest(&serde_json::to_vec(&v).unwrap()).unwrap_err();
            assert!(err.contains("bundle_version"), "{version}: {err}");
        }
    }

    fn parse(v: &serde_json::Value) -> Result<CheckedManifest, String> {
        parse_bundle_manifest(&serde_json::to_vec(v).unwrap())
    }

    fn system() -> serde_json::Value {
        serde_json::json!({
            "session_type": "x11",
            "capture_backend": "x11",
            "cpu_threads": 16,
            "ocr_threads": 4,
            "reader": "new",
            "shadow_recognizer": false,
            "update_channel": "prerelease",
            "daemon_running": true,
            "last_exit": "clean"
        })
    }

    /// The spec section 2 revision-2 example.
    fn sample_v2() -> serde_json::Value {
        let mut v = sample();
        v["bundle_version"] = serde_json::json!(2);
        v["recognizers"]["matcher"] = serde_json::json!("cv-v5");
        v["system"] = system();
        v
    }

    /// Spec section 9 log-only fixture: a crash report with no game.
    fn crash_v2(category: &str) -> serde_json::Value {
        let mut v = sample_v2();
        v["reason"] = serde_json::json!({ "category": category, "text": "it stopped" });
        v["system"]["last_exit"] = serde_json::json!("crashed");
        v["session_id"] = serde_json::json!("none");
        v["game"] = serde_json::Value::Null;
        for key in READ_KEYS {
            v["reads"][*key] = serde_json::json!({
                "value": null, "confidence": null, "suspect": true, "ocr_v1": null
            });
        }
        v["corrections"] = serde_json::json!({});
        v["files"] = serde_json::json!([v["files"][0].clone()]);
        v
    }

    #[test]
    fn v1_without_system_passes() {
        let checked = parse(&sample()).unwrap();
        assert_eq!(checked.reason_category, "wrong_stats");
    }

    #[test]
    fn v1_with_system_or_log_only_category_is_rejected() {
        let mut with_system = sample();
        with_system["system"] = system();
        let err = parse(&with_system).unwrap_err();
        assert!(err.contains("system"), "{err}");

        for category in ["crash", "performance"] {
            let mut v = crash_v2(category);
            v["bundle_version"] = serde_json::json!(1);
            v.as_object_mut().unwrap().remove("system");
            let err = parse(&v).unwrap_err();
            assert!(err.contains("bundle_version 2"), "{category}: {err}");
        }
    }

    #[test]
    fn v2_spec_example_passes() {
        let checked = parse(&sample_v2()).unwrap();
        assert_eq!(checked.matcher, "cv-v5");
        assert_eq!(checked.files.len(), 2);
    }

    #[test]
    fn v2_without_system_passes() {
        let mut v = sample_v2();
        v.as_object_mut().unwrap().remove("system");
        parse(&v).unwrap();
    }

    /// The OCR switch keys are optional: a tracker from before the switch
    /// leaves them out, and its manifest still validates.
    #[test]
    fn system_takes_the_optional_ocr_switch_keys() {
        parse(&sample_v2()).expect("no ocr keys, as every tracker sends today");
        let sites = serde_json::json!({
            "detect/match_end.rs:1133 recognize_prepared": 2396,
            "main.rs:3966 recognize_scoreboard_cells_pre_cropped": 273,
            "ocr/mod.rs:140 ocr_with": 0,
            "tests/ocr_switch.rs:7 Reader::read_cells": 1,
            "other.rs:1": u32::MAX
        });
        for ocr in OCR_SETTINGS {
            let mut v = sample_v2();
            v["system"]["ocr"] = serde_json::json!(ocr);
            parse(&v).unwrap_or_else(|e| panic!("ocr={ocr} alone: {e}"));
        }
        let mut off = sample_v2();
        off["system"]["ocr"] = serde_json::json!("off");
        off["system"]["ocr_blocked_calls"] = sites.clone();
        parse(&off).expect("off with call sites");
        off["system"]["ocr_blocked_calls"] = serde_json::json!({});
        parse(&off).expect("off with no call site");

        // The counts are from a run with OCR off: without `ocr`, or with
        // `on`, a map is refused, a good one and an empty one alike.
        for ocr in [None, Some("on")] {
            for map in [sites.clone(), serde_json::json!({})] {
                let mut v = sample_v2();
                if let Some(ocr) = ocr {
                    v["system"]["ocr"] = serde_json::json!(ocr);
                }
                v["system"]["ocr_blocked_calls"] = map.clone();
                let err = parse(&v).unwrap_err();
                assert!(err.contains("needs system.ocr off"), "{ocr:?} {map}: {err}");
            }
        }

        for bad in ["OFF", "no", ""] {
            let mut v = sample_v2();
            v["system"]["ocr"] = serde_json::json!(bad);
            let err = parse(&v).unwrap_err();
            assert!(err.contains("system.ocr"), "{bad:?}: {err}");
        }
        for (key, bad) in [
            ("ocr", serde_json::Value::Null),
            ("ocr", serde_json::json!(false)),
            ("ocr_blocked_calls", serde_json::Value::Null),
            ("ocr_blocked_calls", serde_json::json!(12)),
            (
                "ocr_blocked_calls",
                serde_json::json!(["main.rs:1 recognize"]),
            ),
            (
                "ocr_blocked_calls",
                serde_json::json!({ "main.rs:1 recognize": -1 }),
            ),
            (
                "ocr_blocked_calls",
                serde_json::json!({ "main.rs:1 recognize": 1.5 }),
            ),
            (
                "ocr_blocked_calls",
                serde_json::json!({ "main.rs:1 recognize": 1.0 }),
            ),
            (
                "ocr_blocked_calls",
                serde_json::json!({ "main.rs:1 recognize": "3" }),
            ),
            (
                "ocr_blocked_calls",
                serde_json::json!({ "main.rs:1 recognize": u64::from(u32::MAX) + 1 }),
            ),
        ] {
            let mut v = sample_v2();
            v["system"]["ocr"] = serde_json::json!("off");
            v["system"][key] = bad.clone();
            assert!(
                parse(&v).unwrap_err().contains("schema"),
                "{key}={bad} must not parse"
            );
        }
    }

    /// The call-site map is bounded: how many sites it names, and the shape
    /// of a key. A key is a place in the tracker's source, so a name, a
    /// sentence, an absolute path and a host are each refused.
    #[test]
    fn system_ocr_blocked_calls_is_bounded() {
        let with = |ocr: Option<&str>, sites: serde_json::Value| {
            let mut v = sample_v2();
            if let Some(ocr) = ocr {
                v["system"]["ocr"] = serde_json::json!(ocr);
            }
            v["system"]["ocr_blocked_calls"] = sites;
            parse(&v)
        };
        let many = |n: usize| {
            let map: serde_json::Map<String, serde_json::Value> = (1..=n)
                .map(|i| (format!("main.rs:{i} recognize"), serde_json::json!(1)))
                .collect();
            serde_json::Value::Object(map)
        };
        with(Some("off"), many(OCR_BLOCKED_SITES_MAX)).expect("the cap itself is allowed");
        let err = with(Some("off"), many(OCR_BLOCKED_SITES_MAX + 1)).unwrap_err();
        assert!(err.contains("too many call sites"), "{err}");

        let tail = ".rs:1 x";
        let longest = format!(
            "{}{tail}",
            "a".repeat(OCR_BLOCKED_SITE_MAX_BYTES - tail.len())
        );
        assert_eq!(longest.len(), OCR_BLOCKED_SITE_MAX_BYTES);
        for good in [
            longest.as_str(),
            "main.rs:1",
            "main.rs:999999 recognize",
            "a/b_2/c9.rs:10 Reader::read_cells",
            "detect/match_end.rs:1133 _x9",
            "tests/ocr_switch.rs:7 A::b::C",
        ] {
            assert!(is_ocr_call_site(good), "{good:?}");
            with(Some("off"), serde_json::json!({ good: 1 }))
                .unwrap_or_else(|e| panic!("{good:?}: {e}"));
        }

        let mut refused: Vec<String> = [
            // too long, empty
            format!("a{longest}").as_str(),
            "",
            // a name, a sentence, a host
            "xXFrozenTearXx",
            "player",
            "PLAYER said hi",
            "my battletag is soot-1234 on eu",
            "192.168.1.20:3030",
            "desktop-ab12cd.lan",
            "name=PLAYER#1234",
            // a path that is not relative to the tracker's source
            "/home/alice/x.rs:12",
            "/home/alice/github/scuffed-crew/crates/stat-tracker/src/main.rs:3966 recognize_prepared",
            "crates/stat-tracker/src/main.rs:3966 recognize_prepared",
            "C:/Users/Alice/main.rs:1",
            "c:/users/alice/main.rs:1",
            "C:\\Users\\x\\main.rs:1",
            "../../etc/passwd",
            "../main.rs:1 x",
            "./main.rs:1 x",
            "~/main.rs:1 x",
            "a//b.rs:1 x",
            "/main.rs:1 x",
            "main.rs/:1 x",
            "Main.rs:1 x",
            "main.txt:1 x",
            ".rs:1 x",
            "/home/m\u{00fc}ller/main.rs:1 recognize",
            "m\u{00fc}.rs:1 x",
            // the line
            "main.rs",
            "main.rs recognize",
            "main.rs:",
            "main.rs: x",
            "main.rs:0 x",
            "main.rs:01 x",
            "main.rs:1234567 x",
            "main.rs:1x x",
            "main.rs:-1 x",
            // the function, and the one space before it
            "main.rs:1 ",
            " main.rs:1 x",
            "main.rs:1  x",
            "main.rs:1 x ",
            "main.rs:1 x y",
            "main.rs:1\nx",
            "main.rs:1\tx",
            "main.rs:1 9x",
            "main.rs:1 a:b",
            "main.rs:1 a::",
            "main.rs:1 ::a",
            "main.rs:1 a.b",
            "main.rs:1 \u{00fc}",
            "a\"b",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        // 96 letters, and one character that is not in the shape, alone, in
        // the path and in the function
        refused.push("a".repeat(OCR_BLOCKED_SITE_MAX_BYTES));
        for c in "#=!@~-. %$\\'+?*,;()[]{}<>|&^`\"".chars() {
            refused.push(format!("a{c}b.rs:1 x"));
            refused.push(format!("main.rs:1 a{c}b"));
        }
        for bad in &refused {
            assert!(!is_ocr_call_site(bad), "{bad:?}");
            let err = with(Some("off"), serde_json::json!({ bad.as_str(): 1 })).unwrap_err();
            assert!(err.contains("not a call site"), "{bad:?}: {err}");
            // and never accepted because `ocr` is not `off`
            for ocr in [None, Some("on")] {
                with(ocr, serde_json::json!({ bad.as_str(): 1 }))
                    .expect_err("a bad key without ocr off");
            }
        }
    }

    #[test]
    fn system_rejects_missing_unknown_null_and_out_of_range_values() {
        let mut missing = sample_v2();
        missing["system"]
            .as_object_mut()
            .unwrap()
            .remove("last_exit");
        assert!(parse(&missing).unwrap_err().contains("schema"));

        let mut unknown = sample_v2();
        unknown["system"]["hostname"] = serde_json::json!("box");
        assert!(parse(&unknown).unwrap_err().contains("schema"));

        let mut null = sample_v2();
        null["system"] = serde_json::Value::Null;
        assert!(parse(&null).unwrap_err().contains("schema"));

        let mut zero = sample_v2();
        zero["system"]["cpu_threads"] = serde_json::json!(0);
        assert!(parse(&zero).unwrap_err().contains("cpu_threads"));

        let mut zero_ocr = sample_v2();
        zero_ocr["system"]["ocr_threads"] = serde_json::json!(0);
        assert!(parse(&zero_ocr).unwrap_err().contains("ocr_threads"));

        let mut negative = sample_v2();
        negative["system"]["cpu_threads"] = serde_json::json!(-1);
        assert!(parse(&negative).is_err());

        for (key, bad) in [
            ("session_type", "mir"),
            ("capture_backend", "pipewire"),
            ("reader", "cv-v5"),
            ("update_channel", "nightly"),
            ("last_exit", "killed"),
        ] {
            let mut v = sample_v2();
            v["system"][key] = serde_json::json!(bad);
            let err = parse(&v).unwrap_err();
            assert!(err.contains(key), "{key}: {err}");
        }

        let mut wrong_type = sample_v2();
        wrong_type["system"]["daemon_running"] = serde_json::json!("yes");
        assert!(parse(&wrong_type).unwrap_err().contains("schema"));

        // Every listed enum value passes.
        for (key, values) in [
            ("session_type", SESSION_TYPES),
            ("capture_backend", CAPTURE_BACKENDS),
            ("reader", READERS),
            ("update_channel", UPDATE_CHANNELS),
            ("last_exit", LAST_EXITS),
        ] {
            for value in values {
                let mut v = sample_v2();
                v["system"][key] = serde_json::json!(value);
                parse(&v).unwrap_or_else(|e| panic!("{key}={value}: {e}"));
            }
        }
    }

    #[test]
    fn crash_and_performance_log_only_reports_pass() {
        for category in ["crash", "performance"] {
            let checked = parse(&crash_v2(category)).unwrap();
            assert_eq!(checked.reason_category, category);
            assert_eq!(checked.files.len(), 1);
            assert_eq!(checked.files[0].role, FileRole::Log);
        }
        // A read that is JSON null as a whole is invalid, even here.
        let mut null_read = crash_v2("crash");
        null_read["reads"]["dmg"] = serde_json::Value::Null;
        assert!(parse(&null_read).unwrap_err().contains("schema"));
        // A null value is always suspect.
        let mut not_suspect = crash_v2("performance");
        not_suspect["reads"]["hero"]["suspect"] = serde_json::json!(false);
        assert!(parse(&not_suspect).unwrap_err().contains("reads.hero"));
    }

    #[test]
    fn crash_report_with_a_crop_or_name_consent_is_rejected() {
        let mut crop = crash_v2("crash");
        crop["files"] = sample()["files"].clone();
        let err = parse(&crop).unwrap_err();
        assert!(err.contains("only log.txt"), "{err}");

        let mut own = crash_v2("crash");
        own["consent"]["own_name_included"] = serde_json::json!(true);
        let err = parse(&own).unwrap_err();
        assert!(err.contains("names or glyphs"), "{err}");

        let mut glyphs = crash_v2("performance");
        glyphs["consent"]["glyphs_included"] = serde_json::json!(true);
        let err = parse(&glyphs).unwrap_err();
        assert!(err.contains("names or glyphs"), "{err}");
    }

    #[test]
    fn crash_report_needs_null_game_null_reads_and_session_none() {
        let mut game = crash_v2("crash");
        game["game"] = sample()["game"].clone();
        assert!(parse(&game).unwrap_err().contains("game null"));

        let mut read = crash_v2("crash");
        read["reads"]["e"] = sample()["reads"]["e"].clone();
        assert!(parse(&read).unwrap_err().contains("reads.e"));

        let mut session = crash_v2("crash");
        session["session_id"] = serde_json::json!("sess-example");
        assert!(parse(&session).unwrap_err().contains("session_id"));

        // The nine keys stay present even when null.
        let mut missing = crash_v2("crash");
        missing["reads"].as_object_mut().unwrap().remove("mit");
        assert!(parse(&missing).unwrap_err().contains("schema"));

        // `game` is a required key, null or not.
        let mut absent = crash_v2("crash");
        absent.as_object_mut().unwrap().remove("game");
        assert!(parse(&absent).unwrap_err().contains("schema"));
    }

    #[test]
    fn game_report_needs_a_game_and_read_objects() {
        let mut no_game = sample_v2();
        no_game["game"] = serde_json::Value::Null;
        assert!(parse(&no_game).unwrap_err().contains("game may be null"));

        let mut null_read = sample_v2();
        null_read["reads"]["hero"] = serde_json::Value::Null;
        assert!(parse(&null_read).unwrap_err().contains("schema"));
    }

    #[test]
    fn other_without_a_game_is_log_only_on_v2() {
        let v2 = crash_v2("other");
        parse(&v2).unwrap();

        let mut with_crop = crash_v2("other");
        with_crop["files"] = sample()["files"].clone();
        assert!(parse(&with_crop).unwrap_err().contains("only log.txt"));

        let mut v1 = crash_v2("other");
        v1["bundle_version"] = serde_json::json!(1);
        v1.as_object_mut().unwrap().remove("system");
        assert!(parse(&v1).unwrap_err().contains("game may be null"));
    }

    #[test]
    fn team_size_null_passes_on_both_versions() {
        for mut v in [sample(), sample_v2()] {
            v["game"]["team_size"] = serde_json::Value::Null;
            parse(&v).unwrap();
        }
        let mut bad = sample_v2();
        bad["game"]["team_size"] = serde_json::json!(4);
        assert!(parse(&bad).unwrap_err().contains("team_size"));

        let mut missing = sample_v2();
        missing["game"].as_object_mut().unwrap().remove("team_size");
        assert!(parse(&missing).unwrap_err().contains("schema"));
    }

    #[test]
    fn v2_crop_paths_are_exactly_scoreboard_and_frame() {
        for path in [
            "crops/cell-e-1.png",
            "crops/Scoreboard.png",
            "crops/frame2.png",
        ] {
            let mut v = sample_v2();
            v["files"][1]["path"] = serde_json::json!(path);
            let err = parse(&v).unwrap_err();
            assert!(err.contains("crop path"), "{path}: {err}");
        }
        let mut frame = sample_v2();
        frame["files"][1]["path"] = serde_json::json!("crops/frame.png");
        parse(&frame).unwrap();
    }

    fn with_outcome_crop(mut v: serde_json::Value) -> serde_json::Value {
        v["files"].as_array_mut().unwrap().push(serde_json::json!({
            "path": "crops/outcome.png",
            "sha256": "2222222222222222222222222222222222222222222222222222222222222222",
            "bytes": 500,
            "role": "crop",
            "screen_class": "outcome"
        }));
        v
    }

    #[test]
    fn outcome_crop_with_outcome_class_is_accepted() {
        let checked = parse(&with_outcome_crop(sample_v2())).unwrap();
        assert_eq!(checked.files.len(), 3);
        parse(&with_outcome_crop(sample())).unwrap();
    }

    #[test]
    fn outcome_crop_must_carry_the_outcome_class() {
        for class in ["scoreboard", "gameplay", "potg"] {
            let mut v = with_outcome_crop(sample_v2());
            v["files"][2]["screen_class"] = serde_json::json!(class);
            let err = parse(&v).unwrap_err();
            assert!(err.contains("screen_class"), "{class}: {err}");
        }
    }

    #[test]
    fn outcome_class_is_only_for_the_outcome_crop() {
        for path in ["crops/scoreboard.png", "crops/frame.png"] {
            let mut v = sample_v2();
            v["files"][1]["path"] = serde_json::json!(path);
            v["files"][1]["screen_class"] = serde_json::json!("outcome");
            let err = parse(&v).unwrap_err();
            assert!(err.contains("screen_class"), "{path}: {err}");
        }
    }

    #[test]
    fn v2_duplicate_key_inside_system_is_rejected() {
        let text = serde_json::to_string(&sample_v2()).unwrap();
        let text = text.replacen(
            "\"cpu_threads\":16",
            "\"cpu_threads\":16,\"cpu_threads\":8",
            1,
        );
        let err = parse_bundle_manifest(text.as_bytes()).unwrap_err();
        assert!(err.contains("duplicate"), "{err}");
    }

    #[test]
    fn a_listed_mapcrop_is_file_missing() {
        let mut v = sample();
        v["files"][1]["path"] = serde_json::json!("debug/mapcrops/sess-1920x1080-1.png");
        let err = parse_bundle_manifest(&serde_json::to_vec(&v).unwrap()).unwrap_err();
        assert_eq!(err, "file missing");
    }

    #[test]
    fn crop_names_are_limited_to_the_spec_allowlist() {
        let mut denied = sample();
        denied["files"][1]["path"] = serde_json::json!("crops/notes.png");
        let err = parse_bundle_manifest(&serde_json::to_vec(&denied).unwrap()).unwrap_err();
        assert!(err.contains("crop") || err.contains("path"), "{err}");

        let mut frame = sample();
        frame["files"][1]["path"] = serde_json::json!("crops/frame.png");
        frame["files"][1]["screen_class"] = serde_json::json!("potg");
        parse_bundle_manifest(&serde_json::to_vec(&frame).unwrap()).unwrap();
    }

    #[test]
    fn path_traversal_in_manifest_is_rejected() {
        let mut v = sample();
        v["files"][1]["path"] = serde_json::json!("crops/../../secret.png");
        let err = parse_bundle_manifest(&serde_json::to_vec(&v).unwrap()).unwrap_err();
        assert!(err.contains("path"), "{err}");
    }

    #[test]
    fn duplicate_key_is_rejected_at_every_level() {
        let top = br#"{"bundle_version":1,"bundle_version":1}"#;
        let err = parse_bundle_manifest(top).unwrap_err();
        assert!(err.contains("duplicate"), "{err}");

        let nested = br#"{"game":{"map":"Busan","map":"Ilios"}}"#;
        let err = parse_bundle_manifest(nested).unwrap_err();
        assert!(err.contains("duplicate"), "{err}");

        let in_array = br#"{"files":[{"path":"log.txt","path":"other.txt"}]}"#;
        let err = parse_bundle_manifest(in_array).unwrap_err();
        assert!(err.contains("duplicate"), "{err}");

        // Escapes decode before the key check, so these are the same key.
        let escaped = br#"{"a":1,"\u0061":2}"#;
        let err = parse_bundle_manifest(escaped).unwrap_err();
        assert!(err.contains("duplicate"), "{err}");
    }

    fn list_item(display_name: Option<Option<String>>) -> StatReportListItem {
        StatReportListItem {
            id: "0123456789abcdef0123456789abcdef".into(),
            created_at: DateTime::parse_from_rfc3339("2026-10-09T12:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            expires_at: None,
            training_consent: false,
            own_name_included: false,
            glyphs_included: false,
            size_bytes: 24,
            member_id: Some("membermember".into()),
            display_name,
            reason_category: None,
            reason_text: None,
            app_version: None,
            recognizer_matcher: None,
            recognizer_ocr: None,
            zip_sha256: None,
        }
    }

    #[test]
    fn display_name_is_a_string_or_null_and_omitted_when_absent() {
        let named = serde_json::to_value(list_item(Some(Some("MemberOne".into())))).unwrap();
        assert_eq!(named["display_name"], "MemberOne");
        assert_eq!(named["member_id"], "membermember");

        let missing = serde_json::to_value(list_item(Some(None))).unwrap();
        assert!(missing["display_name"].is_null());
        assert!(missing.get("display_name").is_some());

        let member = serde_json::to_value(list_item(None)).unwrap();
        assert!(member.get("display_name").is_none());

        let parsed: StatReportListItem = serde_json::from_value(named).unwrap();
        assert_eq!(parsed.display_name, Some(Some("MemberOne".into())));
        let parsed: StatReportListItem = serde_json::from_str(
            r#"{"id":"0123456789abcdef0123456789abcdef","created_at":"2026-10-09T12:00:00Z","expires_at":null,"training_consent":false,"own_name_included":false,"glyphs_included":false,"size_bytes":24}"#,
        )
        .unwrap();
        assert!(parsed.display_name.is_none());
        assert!(parsed.member_id.is_none());
    }
}
