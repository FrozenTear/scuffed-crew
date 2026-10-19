use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatsUploadEntry {
    /// Client-generated game session id. All capture snapshots of one game
    /// share it; the server upserts per (member, session) so re-uploads and
    /// corrections update the same row instead of inserting duplicates.
    /// Empty for uploads from pre-session daemons (server assigns a legacy id).
    #[serde(default)]
    pub session_id: String,
    pub hero: String,
    pub map_name: String,
    pub game_mode: String,
    pub role: String,
    pub outcome: String,
    #[serde(default)]
    pub elims: u32,
    #[serde(default)]
    pub deaths: u32,
    #[serde(default)]
    pub assists: u32,
    #[serde(default)]
    pub damage: u32,
    #[serde(default)]
    pub healing: u32,
    #[serde(default)]
    pub mitigation: u32,
    pub played_at: DateTime<Utc>,
    /// True when the uploaded values include at least one manual correction
    /// (see the tracker's edit overlay). The numeric/label fields above already
    /// carry the effective (corrected-if-present, else OCR) values, so server
    /// aggregates count the corrected numbers; this flag drives the "edited"
    /// badge on the site. Defaulted so older daemons keep uploading.
    #[serde(default)]
    pub edited: bool,
    /// Recognizer stored on the local row. The tracker sends it. Older
    /// clients omit it, and that deserializes as `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recognizer: Option<String>,
    /// Flat names this row was not sure about. Omitted when empty so a
    /// confident or ocr-v1 row does not add the key.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suspect_fields: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatsUploadRequest {
    pub matches: Vec<StatsUploadEntry>,
    /// Sessions the user deleted locally — the server removes its rows for
    /// them (tombstones). Defaulted for older daemons that don't send it.
    #[serde(default)]
    pub deleted_sessions: Vec<String>,
}

/// Recognizer id stored when an upload omits the field.
///
/// Every tracker through 0.4.x reads digits with Tesseract and does not send
/// a recognizer. Those uploads, and rows written before the column existed,
/// are `ocr-v1`.
pub const RECOGNIZER_OCR_V1: &str = "ocr-v1";

/// Max length of a recognizer id (`ocr-v1`, `cv-v1`, and later ids).
pub const RECOGNIZER_ID_MAX_LEN: usize = 32;

/// Returned with HTTP 400 when `recognizer` is present but not a usable id.
pub const RECOGNIZER_ID_ERROR: &str =
    "recognizer must be a string of 1-32 characters in [a-z0-9.-] including a letter or digit";

/// `true` for a short lowercase id such as `ocr-v1` or `cv-v1`.
///
/// This is a format check, not a closed allowlist: a later id (`cv-v2`, a
/// renamed model) can be stored without a server change. A string of only
/// `.` and `-` is rejected so the column cannot hold a nameless token.
pub fn is_recognizer_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    (1..=RECOGNIZER_ID_MAX_LEN).contains(&bytes.len())
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'.' || *b == b'-')
        && bytes.iter().any(u8::is_ascii_alphanumeric)
}

/// Value to persist for a stored recognizer column.
///
/// Empty (the column missing on a row written before it existed) is `ocr-v1`.
/// Any other stored string is returned unchanged so a bad historical value
/// stays visible instead of being relabeled.
pub fn effective_recognizer(stored: &str) -> &str {
    if stored.is_empty() {
        RECOGNIZER_OCR_V1
    } else {
        stored
    }
}

/// How `recognizer` arrived on one match object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecognizerInput {
    /// JSON field omitted or `null`. Persist [`RECOGNIZER_OCR_V1`].
    Absent,
    /// A JSON string. The upload route accepts it only when [`is_recognizer_id`].
    Value(String),
    /// Present, but not a string (number, bool, object, array).
    NotAString,
}

/// Turn a decoded recognizer field into the id to store.
pub fn resolve_recognizer(input: &RecognizerInput) -> Result<String, &'static str> {
    match input {
        RecognizerInput::Absent => Ok(RECOGNIZER_OCR_V1.to_string()),
        RecognizerInput::Value(id) if is_recognizer_id(id) => Ok(id.clone()),
        RecognizerInput::Value(_) | RecognizerInput::NotAString => Err(RECOGNIZER_ID_ERROR),
    }
}

/// Names a tracker may mark as unsure on one player's own `personal_match`.
///
/// Each upload row is that player's row, so these are flat names. Row-indexed
/// names such as `r3.dmg` are not in the set.
pub const SUSPECT_FIELD_NAMES: &[&str] = &[
    "map", "mode", "result", "hero", "e", "a", "d", "dmg", "h", "mit",
];

/// Largest elims count the tracker accepts as a real scoreboard cell.
///
/// A larger value is a damage or timer digit in the elims column. The tracker
/// gate (`parse::MAX_ELIMS`) is this constant, and hero/role averages use it
/// so a row stored before that check existed does not move the mean. The
/// value itself is included. Damage and healing have no ceiling here: a clean
/// wide column is not capped on upload.
pub const PLAUSIBLE_ELIMS_MAX: u32 = 99;
/// Largest assists count the tracker accepts as a real scoreboard cell.
///
/// Same rule as [`PLAUSIBLE_ELIMS_MAX`]. Assists are not part of the hero or
/// role averages.
pub const PLAUSIBLE_ASSISTS_MAX: u32 = 99;
/// Largest deaths count the tracker accepts as a real scoreboard cell.
///
/// Same rule as [`PLAUSIBLE_ELIMS_MAX`]. Deaths above this are left out of
/// hero and role averages. The value itself is included.
pub const PLAUSIBLE_DEATHS_MAX: u32 = 50;

/// Max entries accepted in one match's `suspect_fields` array.
pub const SUSPECT_FIELDS_MAX_LEN: usize = 10;

/// Returned with HTTP 400 when `suspect_fields` is present but not a usable list.
pub const SUSPECT_FIELDS_ERROR: &str = "suspect_fields must be an array of at most 10 unique names from map, mode, result, hero, e, a, d, dmg, h, mit";

const _: () = assert!(SUSPECT_FIELD_NAMES.len() == SUSPECT_FIELDS_MAX_LEN);

/// How `suspect_fields` arrived on one match object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SuspectFieldsInput {
    /// JSON field omitted or `null`. Persist an empty list.
    Absent,
    /// A JSON array of strings. The upload route accepts it only when every
    /// name is in [`SUSPECT_FIELD_NAMES`], with no duplicates, and length is
    /// at most [`SUSPECT_FIELDS_MAX_LEN`].
    Value(Vec<String>),
    /// Present, but not an array (string, number, bool, object).
    NotAnArray,
    /// An array that contains a non-string element.
    NonStringEntry,
}

fn valid_suspect_fields(names: &[String]) -> bool {
    if names.len() > SUSPECT_FIELDS_MAX_LEN {
        return false;
    }
    let mut seen = [false; SUSPECT_FIELDS_MAX_LEN];
    for name in names {
        let Some(idx) = SUSPECT_FIELD_NAMES
            .iter()
            .position(|allowed| *allowed == name.as_str())
        else {
            return false;
        };
        if seen[idx] {
            return false;
        }
        seen[idx] = true;
    }
    true
}

/// Turn a decoded `suspect_fields` value into the list to store.
///
/// Omitted and `null` become `[]`. Any other shape, unknown name, duplicate,
/// or over-long list is an error.
pub fn resolve_suspect_fields(input: &SuspectFieldsInput) -> Result<Vec<String>, &'static str> {
    match input {
        SuspectFieldsInput::Absent => Ok(Vec::new()),
        SuspectFieldsInput::Value(names) if valid_suspect_fields(names) => Ok(names.clone()),
        SuspectFieldsInput::Value(_)
        | SuspectFieldsInput::NotAnArray
        | SuspectFieldsInput::NonStringEntry => Err(SUSPECT_FIELDS_ERROR),
    }
}

/// Rank-screen reads for one personal match.
///
/// All fields are optional. An upload that omits a field, or sends null,
/// stores none, so a client from before rank-screen capture keeps working.
/// A later upload of the same session replaces the whole set: leaving a
/// field out clears a value stored earlier.
///
/// `role` on the match stays the scoreboard hero role (`Tank`, `Damage`,
/// `Support`, or empty). The queue on the rank screen is [`Self::rank_role`]
/// (`tank`, `damage`, `support`). That screen's season-progression number
/// is not stored. [`RANK_TIER_TOP500`] is a tier and does not use a division.
///
/// These fields are returned on the member match list
/// (`GET /api/stats/me/matches`). Public and anonymous responses do not
/// include them.
///
/// A present value whose JSON type matches the field, but which is out of
/// range or not allowed, is stored as none. The rest of that match is still
/// stored. `rank_modifiers` keeps the valid names, at most
/// [`RANK_MODIFIERS_MAX_LEN`]. `rank_confidence` keeps the valid entries.
/// A wrong JSON type rejects the upload. Names of dropped fields are listed
/// on [`Self::rank_dropped`]. The client does not send that list.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RankScreenFields {
    /// Queue on the rank screen: `tank`, `damage`, or `support`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rank_role: Option<String>,
    /// `bronze`, `silver`, `gold`, `platinum`, `diamond`, `master`,
    /// `grandmaster`, `champion`, or `top500`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rank_tier: Option<String>,
    /// Division inside the tier, `1..=5`. Omitted for [`RANK_TIER_TOP500`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rank_division: Option<i32>,
    /// Progress through the current division, `0..=100`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rank_progress_pct: Option<i32>,
    /// Change in that progress for this game, `-100..=100`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rank_delta_pct: Option<i32>,
    /// Modifier chips, normalized to lowercase snake_case. Unknown names
    /// are stored. At most [`RANK_MODIFIERS_MAX_LEN`] entries, each at most
    /// [`RANK_MODIFIER_MAX_LEN`] characters after normalization.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rank_modifiers: Option<Vec<String>>,
    /// Hero skill rating shown beside the role rank, `0..=6000`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hero_sr: Option<i32>,
    /// Change in hero skill rating for this game, `-1000..=1000`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hero_sr_delta: Option<i32>,
    /// Recognizer id that read the rank screen. Same shape as `recognizer`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rank_recognizer: Option<String>,
    /// Per-field confidence, `0.0..=1.0`, keyed by the rank-screen field names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rank_confidence: Option<BTreeMap<String, f64>>,
    /// Names the server dropped on this upload, in first-seen order, without
    /// duplicates. Absent when nothing was dropped. A confidence entry is
    /// `rank_confidence.<field>`. Modifier drops are `rank_modifiers`.
    /// Upload JSON is not a source for this list: a client value is ignored.
    /// Returned on `GET /api/stats/me/matches` with the other rank fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rank_dropped: Option<Vec<String>>,
}

/// Rank-screen queue values accepted on upload.
pub const RANK_ROLES: &[&str] = &["tank", "damage", "support"];

/// `rank_tier` value for the top 500 leaderboard. It has no division.
pub const RANK_TIER_TOP500: &str = "top500";

/// Rank tiers accepted on upload. Includes [`RANK_TIER_TOP500`].
pub const RANK_TIERS: &[&str] = &[
    "bronze",
    "silver",
    "gold",
    "platinum",
    "diamond",
    "master",
    "grandmaster",
    "champion",
    RANK_TIER_TOP500,
];

pub const RANK_DIVISION_MIN: i32 = 1;
pub const RANK_DIVISION_MAX: i32 = 5;
pub const RANK_PROGRESS_PCT_MIN: i32 = 0;
pub const RANK_PROGRESS_PCT_MAX: i32 = 100;
pub const RANK_DELTA_PCT_MIN: i32 = -100;
pub const RANK_DELTA_PCT_MAX: i32 = 100;
pub const HERO_SR_MIN: i32 = 0;
pub const HERO_SR_MAX: i32 = 6000;
pub const HERO_SR_DELTA_MIN: i32 = -1000;
pub const HERO_SR_DELTA_MAX: i32 = 1000;

/// Max modifier chips stored on one match.
pub const RANK_MODIFIERS_MAX_LEN: usize = 8;
/// Max length of one modifier after lowercase snake_case normalization.
pub const RANK_MODIFIER_MAX_LEN: usize = 32;
/// Max raw length of one modifier string before normalization.
pub const RANK_MODIFIER_RAW_MAX_LEN: usize = 64;

/// Keys allowed in `rank_confidence`. The season-progression number is not one.
pub const RANK_CONFIDENCE_FIELDS: &[&str] = &[
    "rank_role",
    "rank_tier",
    "rank_division",
    "rank_progress_pct",
    "rank_delta_pct",
    "rank_modifiers",
    "hero_sr",
    "hero_sr_delta",
    "rank_recognizer",
];

/// HTTP 400 when `rank_role` is present but not a JSON string.
/// A string outside [`RANK_ROLES`] is dropped, not rejected.
pub const RANK_ROLE_ERROR: &str = "rank_role must be a string";
/// HTTP 400 when `rank_tier` is present but not a JSON string.
/// A string outside [`RANK_TIERS`] is dropped, not rejected.
pub const RANK_TIER_ERROR: &str = "rank_tier must be a string";
/// HTTP 400 when `rank_division` is present but not a JSON integer.
/// An integer outside [`RANK_DIVISION_MIN`]..=[`RANK_DIVISION_MAX`] is dropped.
pub const RANK_DIVISION_ERROR: &str = "rank_division must be an integer";
/// HTTP 400 when `rank_progress_pct` is present but not a JSON integer.
/// An integer outside 0..=100 is dropped.
pub const RANK_PROGRESS_PCT_ERROR: &str = "rank_progress_pct must be an integer";
/// HTTP 400 when `rank_delta_pct` is present but not a JSON integer.
/// An integer outside -100..=100 is dropped.
pub const RANK_DELTA_PCT_ERROR: &str = "rank_delta_pct must be an integer";
/// HTTP 400 when `rank_modifiers` is present but not a JSON array.
/// Invalid entries are dropped. More than [`RANK_MODIFIERS_MAX_LEN`] valid
/// names keeps the first [`RANK_MODIFIERS_MAX_LEN`].
pub const RANK_MODIFIERS_ERROR: &str = "rank_modifiers must be an array";
/// HTTP 400 when `hero_sr` is present but not a JSON integer.
/// An integer outside 0..=6000 is dropped.
pub const HERO_SR_ERROR: &str = "hero_sr must be an integer";
/// HTTP 400 when `hero_sr_delta` is present but not a JSON integer.
/// An integer outside -1000..=1000 is dropped.
pub const HERO_SR_DELTA_ERROR: &str = "hero_sr_delta must be an integer";
/// HTTP 400 when `rank_recognizer` is present but not a JSON string.
/// A string that is not a recognizer id is dropped.
pub const RANK_RECOGNIZER_ERROR: &str = "rank_recognizer must be a string";
/// HTTP 400 when `rank_confidence` is present but not a JSON object.
/// Entries that are not a rank field mapped to a finite number in 0.0..=1.0
/// are dropped one at a time.
pub const RANK_CONFIDENCE_ERROR: &str = "rank_confidence must be an object";

const RANK_DROP_NOT_ALLOWED: &str = "not allowed";
const RANK_DROP_OUT_OF_RANGE: &str = "out of range";
const RANK_DROP_BAD_RECOGNIZER: &str = "not a usable recognizer id";
const RANK_DROP_INVALID_MODIFIER: &str = "invalid entry dropped";
const RANK_DROP_MODIFIERS_TRUNCATED: &str = "truncated to the first 8 valid entries";
const RANK_DROP_CONFIDENCE_ENTRY: &str = "confidence entry dropped";
const RANK_DROP_DIVISION_TOP500: &str = "not used for top500";

/// One rank-screen value that was present with the right JSON type, but not stored.
///
/// `field` is the JSON name. `reason` is a fixed phrase. The raw OCR text is
/// not included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RankScreenDrop {
    pub field: &'static str,
    pub reason: &'static str,
}

/// Rank-screen fields kept from one upload object, plus the values dropped.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedRankScreen {
    pub fields: RankScreenFields,
    pub drops: Vec<RankScreenDrop>,
}

fn json_i64(number: &serde_json::Number) -> Option<i64> {
    if let Some(value) = number.as_i64() {
        return Some(value);
    }
    if let Some(value) = number.as_u64() {
        return i64::try_from(value).ok();
    }
    let value = number.as_f64()?;
    if value.is_finite()
        && value.fract() == 0.0
        && (i64::MIN as f64..=i64::MAX as f64).contains(&value)
    {
        Some(value as i64)
    } else {
        None
    }
}

fn note_drop(drops: &mut Vec<RankScreenDrop>, field: &'static str, reason: &'static str) {
    drops.push(RankScreenDrop { field, reason });
}

fn parse_bounded_int(
    value: Option<serde_json::Value>,
    min: i64,
    max: i64,
    field: &'static str,
    err: &'static str,
    drops: &mut Vec<RankScreenDrop>,
) -> Result<Option<i32>, &'static str> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let serde_json::Value::Number(number) = value else {
        return Err(err);
    };
    // A non-integer, or an integer that does not fit i32, is a type serde
    // would reject for this field. That still fails the batch.
    let Some(parsed) = json_i64(&number).and_then(|n| i32::try_from(n).ok()) else {
        return Err(err);
    };
    if i64::from(parsed) < min || i64::from(parsed) > max {
        note_drop(drops, field, RANK_DROP_OUT_OF_RANGE);
        return Ok(None);
    }
    Ok(Some(parsed))
}

fn parse_rank_token(
    value: Option<serde_json::Value>,
    allowed: &[&str],
    field: &'static str,
    err: &'static str,
    drops: &mut Vec<RankScreenDrop>,
) -> Result<Option<String>, &'static str> {
    match value {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(token)) if allowed.contains(&token.as_str()) => {
            Ok(Some(token))
        }
        Some(serde_json::Value::String(_)) => {
            note_drop(drops, field, RANK_DROP_NOT_ALLOWED);
            Ok(None)
        }
        Some(_) => Err(err),
    }
}

/// Lowercase snake_case. Spaces and hyphens become underscores.
/// Unknown names are kept. Empty, oversized, or punctuated names are rejected.
fn normalize_rank_modifier(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > RANK_MODIFIER_RAW_MAX_LEN {
        return None;
    }
    let mut out = String::new();
    let mut underscore = false;
    for c in trimmed.chars() {
        if c.is_ascii_alphabetic() {
            out.push(c.to_ascii_lowercase());
            underscore = false;
        } else if c.is_ascii_digit() {
            out.push(c);
            underscore = false;
        } else if matches!(c, ' ' | '-' | '_') {
            if !underscore && !out.is_empty() {
                out.push('_');
                underscore = true;
            }
        } else {
            return None;
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    if out.is_empty()
        || out.len() > RANK_MODIFIER_MAX_LEN
        || !out.as_bytes()[0].is_ascii_lowercase()
    {
        return None;
    }
    Some(out)
}

fn parse_rank_modifiers(
    value: Option<serde_json::Value>,
    drops: &mut Vec<RankScreenDrop>,
) -> Result<Option<Vec<String>>, &'static str> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let serde_json::Value::Array(items) = value else {
        return Err(RANK_MODIFIERS_ERROR);
    };
    let mut names = Vec::new();
    let mut truncated = false;
    for item in items {
        let serde_json::Value::String(raw) = item else {
            note_drop(drops, "rank_modifiers", RANK_DROP_INVALID_MODIFIER);
            continue;
        };
        let Some(name) = normalize_rank_modifier(&raw) else {
            note_drop(drops, "rank_modifiers", RANK_DROP_INVALID_MODIFIER);
            continue;
        };
        if names.iter().any(|existing| existing == &name) {
            continue;
        }
        if names.len() >= RANK_MODIFIERS_MAX_LEN {
            truncated = true;
            continue;
        }
        names.push(name);
    }
    if truncated {
        note_drop(drops, "rank_modifiers", RANK_DROP_MODIFIERS_TRUNCATED);
    }
    Ok(Some(names))
}

fn parse_rank_recognizer(
    value: Option<serde_json::Value>,
    drops: &mut Vec<RankScreenDrop>,
) -> Result<Option<String>, &'static str> {
    match value {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(id)) if is_recognizer_id(&id) => Ok(Some(id)),
        Some(serde_json::Value::String(_)) => {
            note_drop(drops, "rank_recognizer", RANK_DROP_BAD_RECOGNIZER);
            Ok(None)
        }
        Some(_) => Err(RANK_RECOGNIZER_ERROR),
    }
}

/// Name stored in `rank_dropped` for one bad confidence entry.
///
/// A known rank field keeps its key (`rank_confidence.rank_tier`). Any other
/// key is recorded as `rank_confidence` so an arbitrary client string is not
/// stored.
fn confidence_drop_field(key: &str) -> &'static str {
    match key {
        "rank_role" => "rank_confidence.rank_role",
        "rank_tier" => "rank_confidence.rank_tier",
        "rank_division" => "rank_confidence.rank_division",
        "rank_progress_pct" => "rank_confidence.rank_progress_pct",
        "rank_delta_pct" => "rank_confidence.rank_delta_pct",
        "rank_modifiers" => "rank_confidence.rank_modifiers",
        "hero_sr" => "rank_confidence.hero_sr",
        "hero_sr_delta" => "rank_confidence.hero_sr_delta",
        "rank_recognizer" => "rank_confidence.rank_recognizer",
        _ => "rank_confidence",
    }
}

fn dropped_field_names(drops: &[RankScreenDrop]) -> Option<Vec<String>> {
    let mut names = Vec::new();
    for drop in drops {
        if names.iter().any(|name| name == drop.field) {
            continue;
        }
        names.push(drop.field.to_string());
    }
    if names.is_empty() { None } else { Some(names) }
}

fn parse_confidence_number(value: &serde_json::Value) -> Option<f64> {
    let serde_json::Value::Number(number) = value else {
        return None;
    };
    let parsed = number.as_f64()?;
    (parsed.is_finite() && (0.0..=1.0).contains(&parsed)).then_some(parsed)
}

fn parse_rank_confidence(
    value: Option<serde_json::Value>,
    drops: &mut Vec<RankScreenDrop>,
) -> Result<Option<BTreeMap<String, f64>>, &'static str> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let serde_json::Value::Object(map) = value else {
        return Err(RANK_CONFIDENCE_ERROR);
    };
    if map.is_empty() {
        return Ok(None);
    }
    let mut out = BTreeMap::new();
    for (key, raw) in map {
        let known = RANK_CONFIDENCE_FIELDS.contains(&key.as_str());
        let score = known.then(|| parse_confidence_number(&raw)).flatten();
        let Some(score) = score else {
            note_drop(
                drops,
                confidence_drop_field(&key),
                RANK_DROP_CONFIDENCE_ENTRY,
            );
            continue;
        };
        out.insert(key, score);
    }
    if out.is_empty() {
        Ok(None)
    } else {
        Ok(Some(out))
    }
}

/// Read rank-screen fields off one upload object and remove them.
///
/// Omitted and null fields become `None`. A value of the right JSON type that
/// is out of range or not allowed is stored as none and recorded in
/// [`ParsedRankScreen::drops`]. A wrong JSON type is [`Err`] and rejects the
/// batch. Called from [`StatsUploadMatch`] before the rest of the object is
/// decoded, so an old client that does not send these keys still decodes.
pub fn parse_rank_screen(
    obj: &mut serde_json::Map<String, serde_json::Value>,
) -> Result<ParsedRankScreen, &'static str> {
    let mut drops = Vec::new();
    // The server fills `rank_dropped`. A client value is not a rank read.
    obj.remove("rank_dropped");
    let rank_role = parse_rank_token(
        obj.remove("rank_role"),
        RANK_ROLES,
        "rank_role",
        RANK_ROLE_ERROR,
        &mut drops,
    )?;
    let rank_tier = parse_rank_token(
        obj.remove("rank_tier"),
        RANK_TIERS,
        "rank_tier",
        RANK_TIER_ERROR,
        &mut drops,
    )?;
    let division_raw = obj.remove("rank_division");
    let rank_division = if rank_tier.as_deref() == Some(RANK_TIER_TOP500) {
        match division_raw {
            None | Some(serde_json::Value::Null) => None,
            Some(_) => {
                note_drop(&mut drops, "rank_division", RANK_DROP_DIVISION_TOP500);
                None
            }
        }
    } else {
        parse_bounded_int(
            division_raw,
            i64::from(RANK_DIVISION_MIN),
            i64::from(RANK_DIVISION_MAX),
            "rank_division",
            RANK_DIVISION_ERROR,
            &mut drops,
        )?
    };
    let rank_progress_pct = parse_bounded_int(
        obj.remove("rank_progress_pct"),
        i64::from(RANK_PROGRESS_PCT_MIN),
        i64::from(RANK_PROGRESS_PCT_MAX),
        "rank_progress_pct",
        RANK_PROGRESS_PCT_ERROR,
        &mut drops,
    )?;
    let rank_delta_pct = parse_bounded_int(
        obj.remove("rank_delta_pct"),
        i64::from(RANK_DELTA_PCT_MIN),
        i64::from(RANK_DELTA_PCT_MAX),
        "rank_delta_pct",
        RANK_DELTA_PCT_ERROR,
        &mut drops,
    )?;
    let rank_modifiers = parse_rank_modifiers(obj.remove("rank_modifiers"), &mut drops)?;
    let hero_sr = parse_bounded_int(
        obj.remove("hero_sr"),
        i64::from(HERO_SR_MIN),
        i64::from(HERO_SR_MAX),
        "hero_sr",
        HERO_SR_ERROR,
        &mut drops,
    )?;
    let hero_sr_delta = parse_bounded_int(
        obj.remove("hero_sr_delta"),
        i64::from(HERO_SR_DELTA_MIN),
        i64::from(HERO_SR_DELTA_MAX),
        "hero_sr_delta",
        HERO_SR_DELTA_ERROR,
        &mut drops,
    )?;
    let rank_recognizer = parse_rank_recognizer(obj.remove("rank_recognizer"), &mut drops)?;
    let rank_confidence = parse_rank_confidence(obj.remove("rank_confidence"), &mut drops)?;
    Ok(ParsedRankScreen {
        fields: RankScreenFields {
            rank_role,
            rank_tier,
            rank_division,
            rank_progress_pct,
            rank_delta_pct,
            rank_modifiers,
            hero_sr,
            hero_sr_delta,
            rank_recognizer,
            rank_confidence,
            rank_dropped: dropped_field_names(&drops),
        },
        drops,
    })
}

/// `POST /api/stats/upload` as the server reads it.
///
/// [`StatsUploadRequest`] is the body the desktop tracker builds. It sends
/// `recognizer` from the stored row and omits `suspect_fields` when that list
/// is empty. 0.4.x clients omit both. This type accepts that JSON plus an
/// optional `recognizer` and `suspect_fields` on each object in `matches`. The values
/// are per match because one sync batch can carry games captured under
/// different readers.
///
/// A later upload of the same session replaces both fields. Omitting
/// `suspect_fields`, or sending null, stores `[]`, the same way omitting
/// `recognizer` stores `ocr-v1`. A correction that leaves the list out clears
/// names stored by an earlier upload of that session.
///
/// Each match may also carry the optional rank-screen fields on
/// [`RankScreenFields`]. A value of the right JSON type that is out of range
/// or not allowed is dropped and stored as none. The game and the rest of the
/// batch are still written. Each drop is logged with the field name, a fixed
/// reason, and the match id. Dropped names are stored on `rank_dropped`.
/// `top500` is a `rank_tier` and does not use `rank_division`: any division
/// sent with that tier is dropped. A client `rank_dropped` value is ignored.
/// A wrong JSON type, malformed JSON, or a missing required core field is
/// HTTP 400 and the batch is not stored.
/// Omitting a rank-screen field, including from a client that has never heard
/// of it, stores none. A later upload that leaves a field out clears it.
#[derive(Debug, Clone, Deserialize)]
pub struct StatsUploadBody {
    pub matches: Vec<StatsUploadMatch>,
    /// Same tombstone list as [`StatsUploadRequest::deleted_sessions`].
    #[serde(default)]
    pub deleted_sessions: Vec<String>,
}

/// One match from [`StatsUploadBody`], plus optional recognizer, suspect, and rank-screen fields.
#[derive(Debug, Clone, Deserialize)]
#[serde(try_from = "serde_json::Value")]
pub struct StatsUploadMatch {
    pub entry: StatsUploadEntry,
    pub recognizer: RecognizerInput,
    pub suspect_fields: SuspectFieldsInput,
    /// `Ok` when the rank-screen JSON types decode. Out-of-range and other
    /// disallowed values are listed on [`ParsedRankScreen::drops`] and stored
    /// as none. `Err` is the 400 message for a wrong JSON type. Absent fields
    /// are an empty [`RankScreenFields`], not an error.
    pub rank_screen: Result<ParsedRankScreen, &'static str>,
}

impl TryFrom<serde_json::Value> for StatsUploadMatch {
    type Error = String;

    fn try_from(mut value: serde_json::Value) -> Result<Self, Self::Error> {
        let (recognizer, suspect_fields, rank_screen) = {
            let Some(obj) = value.as_object_mut() else {
                return Err("match entry must be a JSON object".into());
            };
            let rank_screen = parse_rank_screen(obj);
            let recognizer = match obj.remove("recognizer") {
                None | Some(serde_json::Value::Null) => RecognizerInput::Absent,
                Some(serde_json::Value::String(id)) => RecognizerInput::Value(id),
                Some(_) => RecognizerInput::NotAString,
            };
            let suspect_fields = match obj.remove("suspect_fields") {
                None | Some(serde_json::Value::Null) => SuspectFieldsInput::Absent,
                Some(serde_json::Value::Array(items)) => {
                    let mut names = Vec::with_capacity(items.len());
                    let mut non_string = false;
                    for item in items {
                        match item {
                            serde_json::Value::String(name) => names.push(name),
                            _ => {
                                non_string = true;
                                break;
                            }
                        }
                    }
                    if non_string {
                        SuspectFieldsInput::NonStringEntry
                    } else {
                        SuspectFieldsInput::Value(names)
                    }
                }
                Some(_) => SuspectFieldsInput::NotAnArray,
            };
            (recognizer, suspect_fields, rank_screen)
        };
        let entry = serde_json::from_value(value).map_err(|err| err.to_string())?;
        Ok(Self {
            entry,
            recognizer,
            suspect_fields,
            rank_screen,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatsUploadResponse {
    pub inserted: u32,
    pub skipped: u32,
    /// Server rows removed via `deleted_sessions`. Defaulted so new daemons
    /// tolerate older servers that don't report it.
    #[serde(default)]
    pub deleted: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateDaemonTokenRequest {
    #[serde(default = "default_label")]
    pub label: String,
}

fn default_label() -> String {
    "default".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateDaemonTokenResponse {
    pub id: String,
    pub token: String,
    pub label: String,
}

/// Per-member settings returned by GET /api/stats/settings (session auth).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberSettingsResponse {
    pub player_name: Option<String>,
}

/// Body for PUT /api/stats/settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateMemberSettingsRequest {
    pub player_name: Option<String>,
}

/// Daemon configuration returned by GET /api/stats/daemon-config (token auth).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfigResponse {
    pub player_name: Option<String>,
}

/// `GET /api/stats/token-check` (daemon token auth).
///
/// Display name only. No ids, emails, or roles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenCheckResponse {
    pub display_name: String,
}

/// Per-role aggregate for `GET /api/stats/me/roles` and
/// `GET /api/stats/member/{id}/roles`.
///
/// Same counters as the hero aggregate, with `role` instead of `hero`.
/// Match and win/loss/draw counts include every stored game. The averages
/// leave out a counter past [`PLAUSIBLE_ELIMS_MAX`] / [`PLAUSIBLE_DEATHS_MAX`],
/// or named in that row's `suspect_fields` (`e`, `d`, `dmg`, `h`). Damage and
/// healing have no numeric ceiling. A group with no included value for a
/// counter averages to 0.
/// Rows are grouped by the role the tracker uploaded: its manual correction
/// if made, else the detected role.
/// `""` is its own row. Order is matches descending, then role ascending.
/// Defined here so the WASM app can deserialize the response; `scuffed_db`
/// re-exports it next to `HeroStats`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoleStats {
    pub role: String,
    pub matches: u32,
    pub wins: u32,
    pub losses: u32,
    pub draws: u32,
    pub avg_elims: f64,
    pub avg_deaths: f64,
    pub avg_damage: f64,
    pub avg_healing: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> StatsUploadRequest {
        StatsUploadRequest {
            matches: vec![StatsUploadEntry {
                session_id: "sess-1".into(),
                hero: "Ana".into(),
                map_name: "Oasis".into(),
                game_mode: "control".into(),
                role: "Support".into(),
                outcome: "victory".into(),
                elims: 4,
                deaths: 1,
                assists: 2,
                damage: 1000,
                healing: 4000,
                mitigation: 0,
                played_at: chrono::DateTime::parse_from_rfc3339("2026-07-01T20:00:00Z")
                    .unwrap()
                    .with_timezone(&chrono::Utc),
                edited: false,
                recognizer: Some(RECOGNIZER_OCR_V1.into()),
                suspect_fields: Vec::new(),
            }],
            deleted_sessions: vec![],
        }
    }

    #[test]
    fn recognizer_id_accepts_known_readers_and_rejects_garbage() {
        assert!(is_recognizer_id("ocr-v1"));
        assert!(is_recognizer_id("cv-v1"));
        assert!(is_recognizer_id("a"));
        assert!(is_recognizer_id(&"a".repeat(RECOGNIZER_ID_MAX_LEN)));
        assert!(is_recognizer_id("cv.v2"));

        assert!(!is_recognizer_id(""));
        assert!(!is_recognizer_id("OCR-V1"));
        assert!(!is_recognizer_id("cv-v1 "));
        assert!(!is_recognizer_id("cv v1"));
        assert!(!is_recognizer_id("cv/v1"));
        assert!(!is_recognizer_id("..."));
        assert!(!is_recognizer_id("---"));
        assert!(!is_recognizer_id(&"a".repeat(RECOGNIZER_ID_MAX_LEN + 1)));
    }

    #[test]
    fn resolve_recognizer_defaults_absent_and_keeps_valid_ids() {
        assert_eq!(
            resolve_recognizer(&RecognizerInput::Absent).unwrap(),
            RECOGNIZER_OCR_V1
        );
        assert_eq!(
            resolve_recognizer(&RecognizerInput::Value("cv-v1".into())).unwrap(),
            "cv-v1"
        );
        assert!(resolve_recognizer(&RecognizerInput::Value(String::new())).is_err());
        assert!(resolve_recognizer(&RecognizerInput::Value("NOPE".into())).is_err());
        assert!(resolve_recognizer(&RecognizerInput::NotAString).is_err());
        assert_eq!(effective_recognizer(""), RECOGNIZER_OCR_V1);
        assert_eq!(effective_recognizer("cv-v1"), "cv-v1");
    }

    #[test]
    fn missing_hero_is_rejected_when_null_or_omitted() {
        let omitted = r#"{
            "matches": [{
                "map_name": "Oasis",
                "game_mode": "Control",
                "role": "Support",
                "outcome": "victory",
                "played_at": "2026-07-01T20:00:00Z"
            }]
        }"#;
        assert!(
            serde_json::from_str::<StatsUploadBody>(omitted).is_err(),
            "an omitted hero field is not accepted"
        );
        let null_hero = r#"{
            "matches": [{
                "hero": null,
                "map_name": "Oasis",
                "game_mode": "Control",
                "role": "Support",
                "outcome": "victory",
                "played_at": "2026-07-01T20:00:00Z"
            }]
        }"#;
        assert!(
            serde_json::from_str::<StatsUploadBody>(null_hero).is_err(),
            "a null hero is not accepted"
        );
    }

    #[test]
    fn old_shape_upload_body_deserializes_without_recognizer() {
        let current = serde_json::to_string(&sample_request()).unwrap();
        assert!(
            current.contains("\"recognizer\":\"ocr-v1\""),
            "the tracker request type sends the stored recognizer: {current}"
        );
        assert!(
            !current.contains("suspect_fields"),
            "an empty suspect list is omitted: {current}"
        );
        let body: StatsUploadBody = serde_json::from_str(&current).unwrap();
        assert_eq!(
            body.matches[0].recognizer,
            RecognizerInput::Value(RECOGNIZER_OCR_V1.into())
        );
        assert!(matches!(
            body.matches[0].suspect_fields,
            SuspectFieldsInput::Absent
        ));
        assert_eq!(body.matches[0].entry.hero, "Ana");
        assert_eq!(body.matches[0].entry.elims, 4);
        assert!(!body.matches[0].entry.edited);

        // 0.4.x bodies also omit fields that later gained defaults.
        let historical = r#"{
            "matches": [{
                "hero": "Ana",
                "map_name": "Oasis",
                "game_mode": "control",
                "role": "Support",
                "outcome": "victory",
                "played_at": "2026-07-01T20:00:00Z"
            }]
        }"#;
        let old: StatsUploadBody = serde_json::from_str(historical).unwrap();
        assert!(matches!(old.matches[0].recognizer, RecognizerInput::Absent));
        assert!(matches!(
            old.matches[0].suspect_fields,
            SuspectFieldsInput::Absent
        ));
        assert_eq!(old.matches[0].entry.session_id, "");
        assert_eq!(old.matches[0].entry.elims, 0);
        assert!(!old.matches[0].entry.edited);
        assert_eq!(old.matches[0].entry.recognizer, None);
        let bare: StatsUploadEntry = serde_json::from_str(
            r#"{"hero":"Ana","map_name":"Oasis","game_mode":"control","role":"Support","outcome":"victory","played_at":"2026-07-01T20:00:00Z"}"#,
        )
        .unwrap();
        assert_eq!(bare.recognizer, None);
        assert!(old.deleted_sessions.is_empty());

        // A body that also carries recognizer and suspect_fields still
        // decodes the rest of the match. Unknown keys stay ignored.
        let mut with_id: serde_json::Value = serde_json::from_str(&current).unwrap();
        with_id["matches"][0]["recognizer"] = serde_json::json!("cv-v1");
        with_id["matches"][0]["suspect_fields"] = serde_json::json!(["hero", "dmg"]);
        with_id["matches"][0]["future_field"] = serde_json::json!(true);
        let as_old: StatsUploadRequest =
            serde_json::from_value(with_id.clone()).expect("old type ignores unknown fields");
        assert_eq!(as_old.matches[0].hero, "Ana");
        let as_new: StatsUploadBody = serde_json::from_value(with_id).unwrap();
        assert_eq!(
            as_new.matches[0].recognizer,
            RecognizerInput::Value("cv-v1".into())
        );
        assert_eq!(
            as_new.matches[0].suspect_fields,
            SuspectFieldsInput::Value(vec!["hero".into(), "dmg".into()])
        );
    }

    #[test]
    fn recognizer_null_is_absent_and_non_string_is_flagged() {
        let null_body: StatsUploadBody = serde_json::from_str(
            r#"{
                "matches": [{
                    "hero": "Ana",
                    "map_name": "Oasis",
                    "game_mode": "control",
                    "role": "Support",
                    "outcome": "victory",
                    "played_at": "2026-07-01T20:00:00Z",
                    "recognizer": null
                }]
            }"#,
        )
        .unwrap();
        assert_eq!(null_body.matches[0].recognizer, RecognizerInput::Absent);

        let number_body: StatsUploadBody = serde_json::from_str(
            r#"{
                "matches": [{
                    "hero": "Ana",
                    "map_name": "Oasis",
                    "game_mode": "control",
                    "role": "Support",
                    "outcome": "victory",
                    "played_at": "2026-07-01T20:00:00Z",
                    "recognizer": 1
                }]
            }"#,
        )
        .unwrap();
        assert_eq!(
            number_body.matches[0].recognizer,
            RecognizerInput::NotAString
        );
    }

    #[test]
    fn resolve_suspect_fields_defaults_absent_and_keeps_valid_names() {
        assert_eq!(
            resolve_suspect_fields(&SuspectFieldsInput::Absent).unwrap(),
            Vec::<String>::new()
        );
        let all: Vec<String> = SUSPECT_FIELD_NAMES
            .iter()
            .map(|n| (*n).to_string())
            .collect();
        assert_eq!(
            resolve_suspect_fields(&SuspectFieldsInput::Value(all.clone())).unwrap(),
            all
        );
        assert_eq!(
            resolve_suspect_fields(&SuspectFieldsInput::Value(vec!["mit".into(), "map".into()]))
                .unwrap(),
            vec!["mit".to_string(), "map".to_string()]
        );
        assert_eq!(
            resolve_suspect_fields(&SuspectFieldsInput::Value(vec![])).unwrap(),
            Vec::<String>::new()
        );
    }

    #[test]
    fn resolve_suspect_fields_rejects_unknown_indexed_duplicate_and_overlong() {
        assert!(resolve_suspect_fields(&SuspectFieldsInput::Value(vec!["nope".into()])).is_err());
        assert!(resolve_suspect_fields(&SuspectFieldsInput::Value(vec!["r3.dmg".into()])).is_err());
        assert!(resolve_suspect_fields(&SuspectFieldsInput::Value(vec!["r01.e".into()])).is_err());
        assert!(
            resolve_suspect_fields(&SuspectFieldsInput::Value(vec!["e".into(), "e".into()]))
                .is_err()
        );
        let mut too_many = vec!["map".to_string(); SUSPECT_FIELDS_MAX_LEN + 1];
        too_many[0] = "hero".into();
        assert!(resolve_suspect_fields(&SuspectFieldsInput::Value(too_many)).is_err());
        assert!(resolve_suspect_fields(&SuspectFieldsInput::NotAnArray).is_err());
        assert!(resolve_suspect_fields(&SuspectFieldsInput::NonStringEntry).is_err());
    }

    #[test]
    fn suspect_fields_null_is_absent_and_wrong_types_are_flagged() {
        let null_body: StatsUploadBody = serde_json::from_str(
            r#"{
                "matches": [{
                    "hero": "Ana",
                    "map_name": "Oasis",
                    "game_mode": "control",
                    "role": "Support",
                    "outcome": "victory",
                    "played_at": "2026-07-01T20:00:00Z",
                    "suspect_fields": null
                }]
            }"#,
        )
        .unwrap();
        assert_eq!(
            null_body.matches[0].suspect_fields,
            SuspectFieldsInput::Absent
        );

        let number_body: StatsUploadBody = serde_json::from_str(
            r#"{
                "matches": [{
                    "hero": "Ana",
                    "map_name": "Oasis",
                    "game_mode": "control",
                    "role": "Support",
                    "outcome": "victory",
                    "played_at": "2026-07-01T20:00:00Z",
                    "suspect_fields": "map"
                }]
            }"#,
        )
        .unwrap();
        assert_eq!(
            number_body.matches[0].suspect_fields,
            SuspectFieldsInput::NotAnArray
        );

        let mixed_body: StatsUploadBody = serde_json::from_str(
            r#"{
                "matches": [{
                    "hero": "Ana",
                    "map_name": "Oasis",
                    "game_mode": "control",
                    "role": "Support",
                    "outcome": "victory",
                    "played_at": "2026-07-01T20:00:00Z",
                    "suspect_fields": ["hero", 1]
                }]
            }"#,
        )
        .unwrap();
        assert_eq!(
            mixed_body.matches[0].suspect_fields,
            SuspectFieldsInput::NonStringEntry
        );
    }

    fn rank_object() -> serde_json::Map<String, serde_json::Value> {
        serde_json::json!({
            "rank_role": "support",
            "rank_tier": "diamond",
            "rank_division": 2,
            "rank_progress_pct": 40,
            "rank_delta_pct": -15,
            "rank_modifiers": ["Victory", "win streak", "future_mod"],
            "hero_sr": 3200,
            "hero_sr_delta": -25,
            "rank_recognizer": "cv-v1",
            "rank_confidence": {
                "rank_role": 1,
                "rank_tier": 0.5,
                "rank_division": 0.0,
                "hero_sr": 1.0
            }
        })
        .as_object()
        .unwrap()
        .clone()
    }

    #[test]
    fn rank_screen_absent_and_null_are_empty() {
        let mut empty = serde_json::Map::new();
        let empty = parse_rank_screen(&mut empty).unwrap();
        assert_eq!(empty.fields, RankScreenFields::default());
        assert!(empty.drops.is_empty());

        let mut nulls = serde_json::json!({
            "rank_role": null,
            "rank_tier": null,
            "rank_division": null,
            "rank_progress_pct": null,
            "rank_delta_pct": null,
            "rank_modifiers": null,
            "hero_sr": null,
            "hero_sr_delta": null,
            "rank_recognizer": null,
            "rank_confidence": null
        })
        .as_object()
        .unwrap()
        .clone();
        let parsed = parse_rank_screen(&mut nulls).unwrap();
        assert_eq!(parsed.fields, RankScreenFields::default());
        assert!(parsed.drops.is_empty());
        assert!(nulls.is_empty(), "rank keys are consumed: {nulls:?}");
    }

    #[test]
    fn rank_screen_accepts_bounds_and_normalizes_modifiers() {
        let mut low = serde_json::json!({
            "rank_role": "tank",
            "rank_tier": "bronze",
            "rank_division": 1,
            "rank_progress_pct": 0,
            "rank_delta_pct": -100,
            "rank_modifiers": [],
            "hero_sr": 0,
            "hero_sr_delta": -1000,
            "rank_recognizer": "a",
            "rank_confidence": {}
        })
        .as_object()
        .unwrap()
        .clone();
        let low = parse_rank_screen(&mut low).unwrap();
        assert!(low.drops.is_empty());
        let low = low.fields;
        assert_eq!(low.rank_role.as_deref(), Some("tank"));
        assert_eq!(low.rank_tier.as_deref(), Some("bronze"));
        assert_eq!(low.rank_division, Some(1));
        assert_eq!(low.rank_progress_pct, Some(0));
        assert_eq!(low.rank_delta_pct, Some(-100));
        assert_eq!(low.rank_modifiers, Some(vec![]));
        assert_eq!(low.hero_sr, Some(0));
        assert_eq!(low.hero_sr_delta, Some(-1000));
        assert_eq!(low.rank_recognizer.as_deref(), Some("a"));
        assert_eq!(low.rank_confidence, None);

        let mut high = serde_json::json!({
            "rank_role": "damage",
            "rank_tier": "champion",
            "rank_division": 5,
            "rank_progress_pct": 100,
            "rank_delta_pct": 100,
            "hero_sr": 6000,
            "hero_sr_delta": 1000,
            "rank_recognizer": "a".repeat(RECOGNIZER_ID_MAX_LEN),
            "rank_confidence": { "rank_tier": 0.0, "hero_sr_delta": 1.0 }
        })
        .as_object()
        .unwrap()
        .clone();
        let high = parse_rank_screen(&mut high).unwrap();
        assert!(high.drops.is_empty());
        let high = high.fields;
        assert_eq!(high.rank_division, Some(5));
        assert_eq!(high.rank_progress_pct, Some(100));
        assert_eq!(high.rank_delta_pct, Some(100));
        assert_eq!(high.hero_sr, Some(6000));
        assert_eq!(high.hero_sr_delta, Some(1000));
        assert_eq!(
            high.rank_recognizer.as_deref().map(str::len),
            Some(RECOGNIZER_ID_MAX_LEN)
        );

        let mut mid = rank_object();
        let mid = parse_rank_screen(&mut mid).unwrap();
        assert!(mid.drops.is_empty());
        let mid = mid.fields;
        assert_eq!(
            mid.rank_modifiers,
            Some(vec![
                "victory".into(),
                "win_streak".into(),
                "future_mod".into()
            ])
        );
        let scores = mid.rank_confidence.unwrap();
        assert_eq!(scores["rank_role"], 1.0);
        assert_eq!(scores["rank_tier"], 0.5);
        assert_eq!(scores["rank_division"], 0.0);
        assert_eq!(scores["hero_sr"], 1.0);

        let mut dupes = serde_json::json!({
            "rank_modifiers": ["Victory", "victory", "uphill-battle", "Uphill Battle"]
        })
        .as_object()
        .unwrap()
        .clone();
        let dupes = parse_rank_screen(&mut dupes).unwrap();
        assert!(dupes.drops.is_empty());
        let dupes = dupes.fields;
        assert_eq!(
            dupes.rank_modifiers,
            Some(vec!["victory".into(), "uphill_battle".into()])
        );

        let eight: Vec<_> = (0..8).map(|i| format!("mod_{i}")).collect();
        let mut eight_obj = serde_json::Map::new();
        eight_obj.insert("rank_modifiers".into(), serde_json::json!(eight));
        let eight = parse_rank_screen(&mut eight_obj).unwrap();
        assert!(eight.drops.is_empty());
        assert_eq!(eight.fields.rank_modifiers.unwrap().len(), 8);
    }

    fn assert_field_dropped(key: &str, bad: serde_json::Value, reason: &str) {
        let mut obj = rank_object();
        obj.insert(key.to_string(), bad.clone());
        let parsed = parse_rank_screen(&mut obj)
            .unwrap_or_else(|err| panic!("{key} = {bad} should be dropped, not rejected ({err})"));
        let stored = serde_json::to_value(&parsed.fields).unwrap();
        assert!(
            stored.get(key).is_none(),
            "{key} = {bad} should store none, got {stored}"
        );
        assert!(
            parsed
                .drops
                .iter()
                .any(|drop| drop.field == key && drop.reason == reason),
            "{key} = {bad} drops: {:?}",
            parsed.drops
        );
        if key == "rank_tier" {
            assert_eq!(parsed.fields.rank_role.as_deref(), Some("support"));
        } else {
            assert_eq!(parsed.fields.rank_tier.as_deref(), Some("diamond"));
        }
        let rendered = format!("{:?}", parsed.drops);
        let raw = bad.to_string();
        assert!(
            !rendered.contains(&raw),
            "drop record must not include the raw value {raw}"
        );
        assert_eq!(parsed.fields.rank_dropped, Some(vec![key.to_string()]));
    }

    #[test]
    fn rank_screen_drops_type_compatible_bad_values() {
        let dropped = [
            (
                "rank_role",
                serde_json::json!("Tank"),
                RANK_DROP_NOT_ALLOWED,
            ),
            ("rank_role", serde_json::json!("dps"), RANK_DROP_NOT_ALLOWED),
            ("rank_role", serde_json::json!(""), RANK_DROP_NOT_ALLOWED),
            (
                "rank_tier",
                serde_json::json!("Bronze"),
                RANK_DROP_NOT_ALLOWED,
            ),
            (
                "rank_tier",
                serde_json::json!("radiant"),
                RANK_DROP_NOT_ALLOWED,
            ),
            (
                "rank_division",
                serde_json::json!(0),
                RANK_DROP_OUT_OF_RANGE,
            ),
            (
                "rank_division",
                serde_json::json!(6),
                RANK_DROP_OUT_OF_RANGE,
            ),
            (
                "rank_division",
                serde_json::json!(-1),
                RANK_DROP_OUT_OF_RANGE,
            ),
            (
                "rank_progress_pct",
                serde_json::json!(-1),
                RANK_DROP_OUT_OF_RANGE,
            ),
            (
                "rank_progress_pct",
                serde_json::json!(101),
                RANK_DROP_OUT_OF_RANGE,
            ),
            (
                "rank_delta_pct",
                serde_json::json!(-101),
                RANK_DROP_OUT_OF_RANGE,
            ),
            (
                "rank_delta_pct",
                serde_json::json!(150),
                RANK_DROP_OUT_OF_RANGE,
            ),
            ("hero_sr", serde_json::json!(-1), RANK_DROP_OUT_OF_RANGE),
            ("hero_sr", serde_json::json!(6001), RANK_DROP_OUT_OF_RANGE),
            (
                "hero_sr_delta",
                serde_json::json!(-1001),
                RANK_DROP_OUT_OF_RANGE,
            ),
            (
                "hero_sr_delta",
                serde_json::json!(1001),
                RANK_DROP_OUT_OF_RANGE,
            ),
            (
                "rank_recognizer",
                serde_json::json!(""),
                RANK_DROP_BAD_RECOGNIZER,
            ),
            (
                "rank_recognizer",
                serde_json::json!("NOPE"),
                RANK_DROP_BAD_RECOGNIZER,
            ),
            (
                "rank_recognizer",
                serde_json::json!("a".repeat(RECOGNIZER_ID_MAX_LEN + 1)),
                RANK_DROP_BAD_RECOGNIZER,
            ),
        ];
        for (key, bad, reason) in dropped {
            assert_field_dropped(key, bad, reason);
        }
    }

    #[test]
    fn rank_screen_rejects_wrong_json_types() {
        let cases = [
            ("rank_role", serde_json::json!(1), RANK_ROLE_ERROR),
            ("rank_tier", serde_json::json!(true), RANK_TIER_ERROR),
            ("rank_division", serde_json::json!("3"), RANK_DIVISION_ERROR),
            ("rank_division", serde_json::json!(1.5), RANK_DIVISION_ERROR),
            (
                "rank_division",
                serde_json::json!(i64::from(i32::MAX) + 1),
                RANK_DIVISION_ERROR,
            ),
            (
                "rank_progress_pct",
                serde_json::json!(50.5),
                RANK_PROGRESS_PCT_ERROR,
            ),
            (
                "rank_delta_pct",
                serde_json::json!(0.5),
                RANK_DELTA_PCT_ERROR,
            ),
            (
                "rank_delta_pct",
                serde_json::json!(true),
                RANK_DELTA_PCT_ERROR,
            ),
            (
                "rank_delta_pct",
                serde_json::json!("150"),
                RANK_DELTA_PCT_ERROR,
            ),
            ("hero_sr", serde_json::json!(1.2), HERO_SR_ERROR),
            ("hero_sr", serde_json::json!("6000"), HERO_SR_ERROR),
            ("hero_sr_delta", serde_json::json!("1"), HERO_SR_DELTA_ERROR),
            (
                "rank_recognizer",
                serde_json::json!(1),
                RANK_RECOGNIZER_ERROR,
            ),
            (
                "rank_modifiers",
                serde_json::json!("victory"),
                RANK_MODIFIERS_ERROR,
            ),
            (
                "rank_confidence",
                serde_json::json!(["rank_tier"]),
                RANK_CONFIDENCE_ERROR,
            ),
            (
                "rank_confidence",
                serde_json::json!("high"),
                RANK_CONFIDENCE_ERROR,
            ),
        ];
        for (key, bad, err) in cases {
            let mut obj = serde_json::Map::new();
            obj.insert(key.to_string(), bad.clone());
            assert_eq!(
                parse_rank_screen(&mut obj),
                Err(err),
                "{key} = {bad} should be a structural error"
            );
        }
    }

    #[test]
    fn rank_modifiers_drop_invalid_entries_and_keep_eight() {
        let mut mixed = serde_json::json!({
            "rank_tier": "gold",
            "rank_modifiers": ["ok", "!!!", 1, "Victory", "a".repeat(RANK_MODIFIER_MAX_LEN + 1)]
        })
        .as_object()
        .unwrap()
        .clone();
        let mixed = parse_rank_screen(&mut mixed).unwrap();
        assert_eq!(
            mixed.fields.rank_modifiers,
            Some(vec!["ok".into(), "victory".into()])
        );
        assert_eq!(mixed.fields.rank_tier.as_deref(), Some("gold"));
        assert_eq!(
            mixed
                .drops
                .iter()
                .filter(|drop| drop.field == "rank_modifiers"
                    && drop.reason == RANK_DROP_INVALID_MODIFIER)
                .count(),
            3
        );
        assert_eq!(
            mixed.fields.rank_dropped,
            Some(vec!["rank_modifiers".into()])
        );

        let nine: Vec<_> = (0..9).map(|i| format!("mod_{i}")).collect();
        let mut too_many = serde_json::Map::new();
        too_many.insert("rank_modifiers".into(), serde_json::json!(nine));
        let too_many = parse_rank_screen(&mut too_many).unwrap();
        let kept = too_many.fields.rank_modifiers.unwrap();
        assert_eq!(kept.len(), 8);
        assert_eq!(kept[0], "mod_0");
        assert_eq!(kept[7], "mod_7");
        assert_eq!(
            too_many.drops,
            vec![RankScreenDrop {
                field: "rank_modifiers",
                reason: RANK_DROP_MODIFIERS_TRUNCATED,
            }]
        );
        assert_eq!(
            too_many.fields.rank_dropped,
            Some(vec!["rank_modifiers".into()])
        );
    }

    #[test]
    fn top500_saves_and_drops_any_division() {
        let mut bare = serde_json::json!({ "rank_tier": "top500", "hero_sr": 4800 })
            .as_object()
            .unwrap()
            .clone();
        let bare = parse_rank_screen(&mut bare).unwrap();
        assert_eq!(bare.fields.rank_tier.as_deref(), Some("top500"));
        assert_eq!(bare.fields.rank_division, None);
        assert_eq!(bare.fields.hero_sr, Some(4800));
        assert!(bare.drops.is_empty());
        assert_eq!(bare.fields.rank_dropped, None);

        for division in [
            serde_json::json!(4),
            serde_json::json!(9),
            serde_json::json!("3"),
            serde_json::json!(true),
        ] {
            let raw = division.to_string();
            let mut obj = serde_json::json!({
                "rank_tier": "top500",
                "rank_role": "tank",
                "rank_division": division
            })
            .as_object()
            .unwrap()
            .clone();
            let parsed = parse_rank_screen(&mut obj).unwrap();
            assert_eq!(parsed.fields.rank_tier.as_deref(), Some("top500"));
            assert_eq!(parsed.fields.rank_role.as_deref(), Some("tank"));
            assert_eq!(parsed.fields.rank_division, None);
            assert_eq!(
                parsed.drops,
                vec![RankScreenDrop {
                    field: "rank_division",
                    reason: RANK_DROP_DIVISION_TOP500,
                }]
            );
            assert_eq!(
                parsed.fields.rank_dropped,
                Some(vec!["rank_division".into()])
            );
            let rendered = format!("{:?}", parsed.drops);
            assert!(!rendered.contains(&raw), "{raw} leaked into {rendered}");
        }

        let mut null_division = serde_json::json!({
            "rank_tier": "top500",
            "rank_division": null
        })
        .as_object()
        .unwrap()
        .clone();
        let null_division = parse_rank_screen(&mut null_division).unwrap();
        assert!(null_division.drops.is_empty());
        assert_eq!(null_division.fields.rank_dropped, None);
    }

    #[test]
    fn client_rank_dropped_is_ignored() {
        let mut obj = serde_json::json!({
            "rank_tier": "gold",
            "rank_dropped": ["rank_role", "secret_name"]
        })
        .as_object()
        .unwrap()
        .clone();
        let parsed = parse_rank_screen(&mut obj).unwrap();
        assert_eq!(parsed.fields.rank_tier.as_deref(), Some("gold"));
        assert_eq!(parsed.fields.rank_dropped, None);
        assert!(parsed.drops.is_empty());
        assert!(!obj.contains_key("rank_dropped"));
        let rendered = format!("{parsed:?}");
        assert!(!rendered.contains("secret_name"));
    }

    #[test]
    fn rank_confidence_drops_bad_entries_individually() {
        let mut obj = serde_json::json!({
            "rank_role": "tank",
            "hero_sr": 2500,
            "rank_confidence": {
                "rank_role": 0.5,
                "rank_tier": 1.1,
                "ocr_junk_token": 0.2,
                "hero_sr": "high"
            }
        })
        .as_object()
        .unwrap()
        .clone();
        let parsed = parse_rank_screen(&mut obj).unwrap();
        assert_eq!(parsed.fields.rank_role.as_deref(), Some("tank"));
        assert_eq!(parsed.fields.hero_sr, Some(2500));
        let scores = parsed.fields.rank_confidence.unwrap();
        assert_eq!(scores.len(), 1);
        assert_eq!(scores["rank_role"], 0.5);
        assert_eq!(
            parsed
                .drops
                .iter()
                .map(|drop| drop.field)
                .collect::<Vec<_>>(),
            vec![
                "rank_confidence.hero_sr",
                "rank_confidence",
                "rank_confidence.rank_tier",
            ]
        );
        assert!(
            parsed
                .drops
                .iter()
                .all(|drop| drop.reason == RANK_DROP_CONFIDENCE_ENTRY)
        );
        assert_eq!(
            parsed.fields.rank_dropped,
            Some(vec![
                "rank_confidence.hero_sr".into(),
                "rank_confidence".into(),
                "rank_confidence.rank_tier".into(),
            ])
        );
        let rendered = format!("{:?}", parsed.drops);
        assert!(!rendered.contains("ocr_junk_token"));
        assert!(!rendered.contains("1.1"));
        assert!(!rendered.contains("high"));
    }

    #[test]
    fn old_upload_body_decodes_with_empty_rank_screen() {
        let body: StatsUploadBody = serde_json::from_str(
            r#"{"matches":[{"hero":"Ana","map_name":"Oasis","game_mode":"control","role":"Support","outcome":"victory","played_at":"2026-07-01T20:00:00Z"}]}"#,
        )
        .unwrap();
        let parsed = body.matches[0].rank_screen.as_ref().unwrap();
        assert_eq!(parsed.fields, RankScreenFields::default());
        assert!(parsed.drops.is_empty());
        assert_eq!(body.matches[0].entry.role, "Support");
    }
}
