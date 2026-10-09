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

/// Spec bundle version. Any other value is rejected.
pub const BUNDLE_VERSION: u64 = 1;

/// Compressed request body and total uncompressed size, in bytes.
pub const MAX_BUNDLE_BYTES: u64 = 10 * 1024 * 1024;

/// Files listed in the manifest, not counting `manifest.json`.
pub const MAX_FILES_BESIDES_MANIFEST: usize = 30;

/// Longest allowed image edge, in pixels.
pub const MAX_IMAGE_EDGE: u32 = 7680;

/// Successful stores per member per UTC day. A rejected upload does not count.
pub const DAILY_REPORT_CAP: u64 = 5;

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
    "other",
];

const SCREEN_CLASSES: &[&str] = &[
    "scoreboard",
    "gameplay",
    "killcam",
    "potg",
    "accolade",
    "party",
    "lobby",
];

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatReportList {
    pub reports: Vec<StatReportListItem>,
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
    session_id: String,
    game: RawGame,
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
#[allow(dead_code)]
struct RawGame {
    map: Option<String>,
    mode: Option<String>,
    result: Option<String>,
    team_size: u32,
    captured_at: DateTime<Utc>,
}

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
    if raw.bundle_version != BUNDLE_VERSION {
        return Err("bundle_version must be 1".into());
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
    if !REASON_CATEGORIES.contains(&raw.reason.category.as_str()) {
        return Err("reason.category is not a known category".into());
    }
    if raw.reason.text.chars().count() > 500 {
        return Err("reason.text is longer than 500 characters".into());
    }
    if raw.reason.text.chars().any(|c| c == '\0') {
        return Err("reason.text contains a null".into());
    }
    check_short("session_id", &raw.session_id, 1, 64)?;
    if raw.game.team_size != 5 && raw.game.team_size != 6 {
        return Err("game.team_size must be 5 or 6".into());
    }
    check_opt_label("game.map", raw.game.map.as_deref())?;
    check_opt_label("game.mode", raw.game.mode.as_deref())?;
    check_opt_label("game.result", raw.game.result.as_deref())?;

    check_read("mode", &raw.reads.mode)?;
    check_read("result", &raw.reads.result)?;
    check_read("hero", &raw.reads.hero)?;
    check_read("e", &raw.reads.e)?;
    check_read("a", &raw.reads.a)?;
    check_read("d", &raw.reads.d)?;
    check_read("dmg", &raw.reads.dmg)?;
    check_read("h", &raw.reads.h)?;
    check_read("mit", &raw.reads.mit)?;

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
            match file.screen_class.as_deref() {
                Some(class) if SCREEN_CLASSES.contains(&class) => {}
                _ => return Err("crop screen_class is not a known class".into()),
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

fn check_read(name: &str, read: &RawRead) -> Result<(), String> {
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
const CROP_PATHS: &[&str] = &["crops/scoreboard.png", "crops/frame.png"];

fn is_plain_crop_path(path: &str) -> bool {
    CROP_PATHS.contains(&path)
}

fn is_glyph_id(id: &str) -> bool {
    id.len() == 8 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
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
        let mut v = sample();
        v["bundle_version"] = serde_json::json!(2);
        let err = parse_bundle_manifest(&serde_json::to_vec(&v).unwrap()).unwrap_err();
        assert!(err.contains("bundle_version"), "{err}");
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
}
