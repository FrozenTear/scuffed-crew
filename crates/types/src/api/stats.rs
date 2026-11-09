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
    /// Which reader found this player's own scoreboard row.
    ///
    /// `names-v1` when the name came from the names pack. `ocr-v1` when it
    /// fell back to Tesseract. Omitted when this capture did not read a
    /// name, so an older server that ignores the field is unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name_recognizer: Option<String>,
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

/// How a recognizer id arrived on one match object.
///
/// [`resolve_recognizer`] stores [`RECOGNIZER_OCR_V1`] for [`Self::Absent`].
/// [`resolve_name_recognizer`] stores none for that same variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecognizerInput {
    /// JSON field omitted or `null`.
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

/// Returned with HTTP 400 when `name_recognizer` is present but not a usable id.
///
/// Same format rule as [`RECOGNIZER_ID_ERROR`]. The field name differs so a
/// client can tell which key failed.
pub const NAME_RECOGNIZER_ID_ERROR: &str =
    "name_recognizer must be a string of 1-32 characters in [a-z0-9.-] including a letter or digit";

/// Turn a decoded `name_recognizer` field into the id to store.
///
/// Omitted and `null` are unknown (`None`, stored as NONE). A string is kept
/// only when [`is_recognizer_id`] accepts it. There is no allowlist and no
/// default reader id. The check is format only (`[a-z0-9.-]`, 1 to 32
/// characters). Capitals and spaces are rejected, but a short lowercase token
/// such as `player1` passes, so clients must send only reader ids such as
/// `names-v1` or `ocr-v1`.
pub fn resolve_name_recognizer(input: &RecognizerInput) -> Result<Option<String>, &'static str> {
    match input {
        RecognizerInput::Absent => Ok(None),
        RecognizerInput::Value(id) if is_recognizer_id(id) => Ok(Some(id.clone())),
        RecognizerInput::Value(_) | RecognizerInput::NotAString => Err(NAME_RECOGNIZER_ID_ERROR),
    }
}

fn take_recognizer_input(
    obj: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> RecognizerInput {
    match obj.remove(key) {
        None | Some(serde_json::Value::Null) => RecognizerInput::Absent,
        Some(serde_json::Value::String(id)) => RecognizerInput::Value(id),
        Some(_) => RecognizerInput::NotAString,
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
/// value itself is included. The upload route has its own, wider ceiling
/// ([`STATS_UPLOAD_MAX_ELIMS`], see [`validate_upload_entry`]), so a row past
/// this is still stored and only left out of the averages.
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

/// Max entries accepted in `bans_picks.bans`.
pub const BANS_PICKS_MAX_BANS: usize = 4;
/// Max entries accepted in `bans_picks.picks`.
pub const BANS_PICKS_MAX_PICKS: usize = 6;
/// Max entries accepted in `bans_picks.ban_vote`.
pub const BANS_PICKS_MAX_BAN_VOTE: usize = 6;
/// Literal a `picks` or `ban_vote` entry may carry when a teammate chose no
/// hero. Stored and returned as sent. It is not a hero key, and it is not
/// null (null means the entry was unreadable).
pub const BANS_PICKS_NO_PREFERENCE: &str = "no_preference";

/// Which side placed one hero ban.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BanTeam {
    Own,
    Enemy,
}

/// One hero ban read from the bans screen.
///
/// `hero` is a hero pack key (`wrecking-ball`). `None` means the icon was
/// not readable or the key was not a known hero. `team` is `None` when the
/// side was not read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeroBan {
    #[serde(default)]
    pub hero: Option<String>,
    #[serde(default)]
    pub team: Option<BanTeam>,
}

/// Bans and picks the tracker read for one game.
///
/// Optional `bans_picks` object on each uploaded match and on match-list
/// JSON. Hero values are pack keys. Every field is always serialized (null
/// or `[]` when empty) so clients see one fixed shape.
///
/// - `bans`: at most [`BANS_PICKS_MAX_BANS`] entries.
/// - `lobby_ban`: pack key or null.
/// - `picks`: at most [`BANS_PICKS_MAX_PICKS`] entries, own team, local
///   player first. Each entry is a pack key, [`BANS_PICKS_NO_PREFERENCE`]
///   (the teammate chose no hero) or null (unreadable).
/// - `ban_vote`: at most [`BANS_PICKS_MAX_BAN_VOTE`] entries, same entry
///   values as `picks`. From stat trackers newer than 0.6.0-alpha.2.1:
///   the uploading player's own three ban votes in slot order (a hero key,
///   or null for a skipped, empty or unread slot). Older trackers sent the
///   six own team rows of the ban vote screen, which show each player's
///   preferred hero, not a vote; treat those as unreliable.
/// - Newer trackers may also send `ban_votes` (each vote slot's state and
///   hero) and `ban_score` (the TEAM BAN SCORE strip: rank, hero, vote
///   count). They are optional and not part of this contract yet:
///   [`resolve_bans_picks`] ignores them, like any key it does not know.
/// - `recognizer`: reader id, same format rule as `name_recognizer`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BansPicks {
    #[serde(default)]
    pub bans: Vec<HeroBan>,
    #[serde(default)]
    pub lobby_ban: Option<String>,
    #[serde(default)]
    pub picks: Vec<Option<String>>,
    #[serde(default)]
    pub ban_vote: Vec<Option<String>>,
    #[serde(default)]
    pub recognizer: Option<String>,
}

/// Returned with HTTP 422 when `bans_picks.recognizer` is present but not a usable id.
pub const BANS_PICKS_RECOGNIZER_ERROR: &str = "bans_picks.recognizer must be a string of 1-32 characters in [a-z0-9.-] including a letter or digit";

/// Canonical pack key for a hero value, or `None` when it is not a known hero.
///
/// Accepts what [`crate::hero_key_to_name`] accepts (pack key, portrait
/// stem, display name) and returns the pack key (`wrecking-ball`).
pub fn canonical_hero_pack_key(raw: &str) -> Option<String> {
    let display = crate::hero_key_to_name(raw)?;
    crate::heroes::HERO_NAMES
        .iter()
        .find(|hero| hero.display == display)
        .map(|hero| crate::heroes::pack_file_stem(hero.file).to_string())
}

fn bans_picks_hero(
    value: Option<&serde_json::Value>,
    field: &str,
) -> Result<Option<String>, String> {
    match value {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(raw)) => Ok(canonical_hero_pack_key(raw)),
        Some(_) => Err(format!("{field} must be a hero key string or null")),
    }
}

fn bans_picks_array<'a>(
    obj: &'a serde_json::Map<String, serde_json::Value>,
    key: &str,
    max: usize,
) -> Result<&'a [serde_json::Value], String> {
    match obj.get(key) {
        None | Some(serde_json::Value::Null) => Ok(&[]),
        Some(serde_json::Value::Array(items)) if items.len() > max => Err(format!(
            "bans_picks.{key} must have at most {max} entries, got {}",
            items.len()
        )),
        Some(serde_json::Value::Array(items)) => Ok(items),
        Some(_) => Err(format!("bans_picks.{key} must be an array or null")),
    }
}

/// A `picks` or `ban_vote` entry: a pack key, the literal
/// [`BANS_PICKS_NO_PREFERENCE`] (kept as is) or null. Any other string that is
/// not a known hero becomes null.
fn bans_picks_choice(value: &serde_json::Value, field: &str) -> Result<Option<String>, String> {
    match value {
        serde_json::Value::String(raw) if raw == BANS_PICKS_NO_PREFERENCE => Ok(Some(raw.clone())),
        other => bans_picks_hero(Some(other), field),
    }
}

/// Turn a raw `bans_picks` upload value into what is stored.
///
/// Omitted or null is `Ok(None)`. An unknown hero key becomes null and does
/// not reject the game. A wrong shape, an over-long `bans`, `picks` or
/// `ban_vote`, a
/// `team` other than `own` / `enemy` / null, or a bad `recognizer` is an
/// error (HTTP 422, message without the `matches[n]: ` prefix).
pub fn resolve_bans_picks(value: Option<&serde_json::Value>) -> Result<Option<BansPicks>, String> {
    let obj = match value {
        None | Some(serde_json::Value::Null) => return Ok(None),
        Some(serde_json::Value::Object(obj)) => obj,
        Some(_) => return Err("bans_picks must be an object or null".into()),
    };
    let mut bans = Vec::new();
    for (i, item) in bans_picks_array(obj, "bans", BANS_PICKS_MAX_BANS)?
        .iter()
        .enumerate()
    {
        let ban = match item {
            serde_json::Value::Null => HeroBan {
                hero: None,
                team: None,
            },
            serde_json::Value::Object(ban) => {
                let hero = bans_picks_hero(ban.get("hero"), &format!("bans_picks.bans[{i}].hero"))?;
                let team = match ban.get("team") {
                    None | Some(serde_json::Value::Null) => None,
                    Some(serde_json::Value::String(t)) if t == "own" => Some(BanTeam::Own),
                    Some(serde_json::Value::String(t)) if t == "enemy" => Some(BanTeam::Enemy),
                    Some(_) => {
                        return Err(format!(
                            "bans_picks.bans[{i}].team must be \"own\", \"enemy\" or null"
                        ));
                    }
                };
                HeroBan { hero, team }
            }
            _ => return Err(format!("bans_picks.bans[{i}] must be an object")),
        };
        bans.push(ban);
    }
    let mut picks = Vec::new();
    for (i, item) in bans_picks_array(obj, "picks", BANS_PICKS_MAX_PICKS)?
        .iter()
        .enumerate()
    {
        picks.push(bans_picks_choice(item, &format!("bans_picks.picks[{i}]"))?);
    }
    let mut ban_vote = Vec::new();
    for (i, item) in bans_picks_array(obj, "ban_vote", BANS_PICKS_MAX_BAN_VOTE)?
        .iter()
        .enumerate()
    {
        ban_vote.push(bans_picks_choice(
            item,
            &format!("bans_picks.ban_vote[{i}]"),
        )?);
    }
    let lobby_ban = bans_picks_hero(obj.get("lobby_ban"), "bans_picks.lobby_ban")?;
    let recognizer = match obj.get("recognizer") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(id)) if is_recognizer_id(id) => Some(id.clone()),
        Some(_) => return Err(BANS_PICKS_RECOGNIZER_ERROR.to_string()),
    };
    Ok(Some(BansPicks {
        bans,
        lobby_ban,
        picks,
        ban_vote,
        recognizer,
    }))
}

/// Read a stored `bans_picks` object back.
///
/// Tolerant: a field of the wrong type reads as null / empty instead of
/// failing the whole match row. Only values written by [`resolve_bans_picks`]
/// are expected here.
pub fn bans_picks_from_stored(value: &serde_json::Value) -> Option<BansPicks> {
    let obj = value.as_object()?;
    let key = |v: Option<&serde_json::Value>| v.and_then(|v| v.as_str()).map(str::to_string);
    let bans = obj
        .get("bans")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .map(|item| HeroBan {
                    hero: key(item.get("hero")),
                    team: match item.get("team").and_then(|t| t.as_str()) {
                        Some("own") => Some(BanTeam::Own),
                        Some("enemy") => Some(BanTeam::Enemy),
                        _ => None,
                    },
                })
                .collect()
        })
        .unwrap_or_default();
    let picks = obj
        .get("picks")
        .and_then(|v| v.as_array())
        .map(|items| items.iter().map(|item| key(Some(item))).collect())
        .unwrap_or_default();
    let ban_vote = obj
        .get("ban_vote")
        .and_then(|v| v.as_array())
        .map(|items| items.iter().map(|item| key(Some(item))).collect())
        .unwrap_or_default();
    Some(BansPicks {
        bans,
        lobby_ban: key(obj.get("lobby_ban")),
        picks,
        ban_vote,
        recognizer: key(obj.get("recognizer")),
    })
}

/// Max characters in each `rank` string value (`role_rank`,
/// `next_mode_hint`, `queue_title`, each `change_tags` entry).
pub const RANK_STRING_MAX_CHARS: usize = 64;
/// Max entries in `rank.change_tags.value`.
pub const RANK_MAX_CHANGE_TAGS: usize = 8;

/// Queue role on a rank update screen. `Open` is open queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RankRole {
    Tank,
    Damage,
    Support,
    Open,
}

/// One rank-v1 field: the value read and how sure the reader was.
///
/// Either part is `None` (JSON null) when it was omitted or failed its
/// check. The tracker's `bbox` is accepted on upload and dropped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RankField<T> {
    pub value: Option<T>,
    /// 0..=1.
    pub confidence: Option<f32>,
}

impl<T> Default for RankField<T> {
    fn default() -> Self {
        Self {
            value: None,
            confidence: None,
        }
    }
}

/// Rank update screen read for one game (`rank` on uploads and match JSON).
///
/// Every field is always serialized as `{value, confidence}` so clients see
/// one fixed shape. `rank_recognizer` is the reader id (`rank-v1`), which
/// also covers `queue_title`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MatchRank {
    #[serde(default)]
    pub role: RankField<RankRole>,
    #[serde(default)]
    pub role_rank: RankField<String>,
    /// 1..=5.
    #[serde(default)]
    pub division: RankField<u8>,
    #[serde(default)]
    pub progress_pct: RankField<i32>,
    #[serde(default)]
    pub progress_change: RankField<i32>,
    /// At most [`RANK_MAX_CHANGE_TAGS`] entries.
    #[serde(default)]
    pub change_tags: RankField<Vec<String>>,
    /// 0..=100.
    #[serde(default)]
    pub higher_than_pct: RankField<u8>,
    #[serde(default)]
    pub hero_sr: RankField<u32>,
    #[serde(default)]
    pub hero_sr_change: RankField<i32>,
    #[serde(default)]
    pub next_mode_hint: RankField<String>,
    #[serde(default)]
    pub queue_title: RankField<String>,
    #[serde(default)]
    pub rank_recognizer: Option<String>,
}

fn rank_string(value: &serde_json::Value) -> Option<String> {
    value
        .as_str()
        .filter(|s| s.chars().count() <= RANK_STRING_MAX_CHARS)
        .map(str::to_string)
}

fn rank_int<T: TryFrom<i64>>(value: &serde_json::Value) -> Option<T> {
    value.as_i64().and_then(|n| T::try_from(n).ok())
}

fn rank_field<T>(
    obj: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    parse: impl Fn(&serde_json::Value) -> Option<T>,
) -> RankField<T> {
    let Some(field) = obj.get(key).and_then(serde_json::Value::as_object) else {
        return RankField::default();
    };
    RankField {
        value: field.get("value").and_then(&parse),
        confidence: field
            .get("confidence")
            .and_then(serde_json::Value::as_f64)
            .filter(|c| c.is_finite() && (0.0..=1.0).contains(c))
            .map(|c| c as f32),
    }
}

/// Turn a raw `rank` value (upload or stored) into what is stored and returned.
///
/// Never an error: rank data must not cost the game. Omitted, null or a
/// non-object `rank` is `None`. Inside it, a value or confidence that is the
/// wrong type or out of range becomes null on its own, and unknown keys
/// (including each field's `bbox`) are dropped. Checks: `division` 1..=5,
/// `higher_than_pct` at most 100, `confidence` finite in 0..=1, integers in
/// their type's range, strings at most [`RANK_STRING_MAX_CHARS`] characters,
/// `change_tags` at most [`RANK_MAX_CHANGE_TAGS`] strings (otherwise the whole
/// list is null), `role` one of `Tank`, `Damage`, `Support`, `Open`, and
/// `rank_recognizer` the same id format as `recognizer`.
pub fn resolve_rank(value: Option<&serde_json::Value>) -> Option<MatchRank> {
    let obj = value?.as_object()?;
    Some(MatchRank {
        role: rank_field(obj, "role", |v| {
            serde_json::from_value::<RankRole>(v.clone()).ok()
        }),
        role_rank: rank_field(obj, "role_rank", rank_string),
        division: rank_field(obj, "division", |v| {
            rank_int::<u8>(v).filter(|d| (1..=5).contains(d))
        }),
        progress_pct: rank_field(obj, "progress_pct", rank_int::<i32>),
        progress_change: rank_field(obj, "progress_change", rank_int::<i32>),
        change_tags: rank_field(obj, "change_tags", |v| {
            let items = v.as_array().filter(|a| a.len() <= RANK_MAX_CHANGE_TAGS)?;
            items.iter().map(rank_string).collect()
        }),
        higher_than_pct: rank_field(obj, "higher_than_pct", |v| {
            rank_int::<u8>(v).filter(|p| *p <= 100)
        }),
        hero_sr: rank_field(obj, "hero_sr", rank_int::<u32>),
        hero_sr_change: rank_field(obj, "hero_sr_change", rank_int::<i32>),
        next_mode_hint: rank_field(obj, "next_mode_hint", rank_string),
        queue_title: rank_field(obj, "queue_title", rank_string),
        rank_recognizer: obj
            .get("rank_recognizer")
            .and_then(serde_json::Value::as_str)
            .filter(|id| is_recognizer_id(id))
            .map(str::to_string),
    })
}

/// Max characters (not bytes) in each string field of one uploaded match
/// (`session_id`, `hero`, `map_name`, `game_mode`, `role`, `outcome`) and in
/// each `deleted_sessions` entry. Tracker session ids are 16 hex characters
/// and server legacy ids are 23; hero, map, mode and role names are well
/// under this.
pub const STATS_UPLOAD_STRING_MAX_CHARS: usize = 64;

/// Earliest `played_at` an upload may carry: 2022-10-04T00:00:00Z, the day
/// Overwatch 2 launched. The tracker reads the Overwatch 2 scoreboard, so an
/// earlier date is a bad clock or a made-up row.
pub const STATS_UPLOAD_PLAYED_AT_MIN_UNIX: i64 = 1_664_841_600;

/// How far past the server clock `played_at` may be. Covers a client clock
/// that is a few hours fast (or a local time sent as UTC) without letting a
/// row sit in a later season or pin itself to the top of a recent list.
pub const STATS_UPLOAD_PLAYED_AT_MAX_FUTURE_SECS: i64 = 24 * 60 * 60;

/// Largest per-match damage, healing, or mitigation an upload may carry.
///
/// Long overtime and Stadium games, tank mitigation included, stay under
/// this. It exists so a row cannot carry `u32::MAX` and top a damage or
/// healing board.
pub const STATS_UPLOAD_MAX_DAMAGE: u32 = 200_000;
/// Largest per-match elims an upload may carry.
///
/// Wider than [`PLAUSIBLE_ELIMS_MAX`] (the tracker's read gate and the
/// averages filter) so a long game or an older tracker's row is stored, not
/// rejected. It still stops a `u32::MAX` row from topping an elims board.
pub const STATS_UPLOAD_MAX_ELIMS: u32 = 999;
/// Same ceiling as [`STATS_UPLOAD_MAX_ELIMS`], for assists.
pub const STATS_UPLOAD_MAX_ASSISTS: u32 = 999;
/// Same ceiling as [`STATS_UPLOAD_MAX_DAMAGE`], for healing.
pub const STATS_UPLOAD_MAX_HEALING: u32 = 200_000;
/// Same ceiling as [`STATS_UPLOAD_MAX_DAMAGE`], for mitigation.
pub const STATS_UPLOAD_MAX_MITIGATION: u32 = 200_000;

/// Hero name 0.4.x and 0.5.0 trackers upload when no hero was read. Live
/// data has rows with it, so the upload check accepts it in any case.
pub const UPLOAD_HERO_UNKNOWN: &str = "Unknown";

/// True when `hero` is [`UPLOAD_HERO_UNKNOWN`], ignoring ASCII case.
pub fn is_upload_hero_unknown(hero: &str) -> bool {
    hero.eq_ignore_ascii_case(UPLOAD_HERO_UNKNOWN)
}

fn string_too_long(value: &str) -> bool {
    value.chars().count() > STATS_UPLOAD_STRING_MAX_CHARS
}

/// Check one uploaded match beyond the reader ids and suspect fields.
///
/// - every string field is at most [`STATS_UPLOAD_STRING_MAX_CHARS`]
///   characters;
/// - `hero` is a known hero (display name or pack key, see
///   [`crate::hero_key_to_name`]) or [`UPLOAD_HERO_UNKNOWN`] in any case,
///   which 0.4.x and 0.5.0 trackers send when no hero was read;
/// - `elims` and `assists` are at most [`STATS_UPLOAD_MAX_ELIMS`] and
///   [`STATS_UPLOAD_MAX_ASSISTS`], `deaths` at most [`PLAUSIBLE_DEATHS_MAX`];
/// - `damage`, `healing`, `mitigation` are at most
///   [`STATS_UPLOAD_MAX_DAMAGE`], [`STATS_UPLOAD_MAX_HEALING`],
///   [`STATS_UPLOAD_MAX_MITIGATION`];
/// - `played_at` is no earlier than [`STATS_UPLOAD_PLAYED_AT_MIN_UNIX`] and
///   no later than `now` plus [`STATS_UPLOAD_PLAYED_AT_MAX_FUTURE_SECS`].
///
/// `outcome` keeps its existing rule (the handler skips a value the schema
/// cannot store); only its length is checked here. Returns the message for
/// HTTP 400 without the `matches[n]: ` prefix.
pub fn validate_upload_entry(entry: &StatsUploadEntry, now: DateTime<Utc>) -> Result<(), String> {
    for (name, value) in [
        ("session_id", &entry.session_id),
        ("hero", &entry.hero),
        ("map_name", &entry.map_name),
        ("game_mode", &entry.game_mode),
        ("role", &entry.role),
        ("outcome", &entry.outcome),
    ] {
        if string_too_long(value) {
            return Err(format!(
                "{name} must be at most {STATS_UPLOAD_STRING_MAX_CHARS} characters"
            ));
        }
    }
    if !is_upload_hero_unknown(&entry.hero) && crate::hero_key_to_name(&entry.hero).is_none() {
        return Err("hero must be a known hero or Unknown".to_string());
    }
    for (name, value, max) in [
        ("elims", entry.elims, STATS_UPLOAD_MAX_ELIMS),
        ("assists", entry.assists, STATS_UPLOAD_MAX_ASSISTS),
        ("deaths", entry.deaths, PLAUSIBLE_DEATHS_MAX),
        ("damage", entry.damage, STATS_UPLOAD_MAX_DAMAGE),
        ("healing", entry.healing, STATS_UPLOAD_MAX_HEALING),
        ("mitigation", entry.mitigation, STATS_UPLOAD_MAX_MITIGATION),
    ] {
        if value > max {
            return Err(format!("{name} must be at most {max}"));
        }
    }
    let earliest = DateTime::<Utc>::from_timestamp(STATS_UPLOAD_PLAYED_AT_MIN_UNIX, 0)
        .expect("constant timestamp is in range");
    let latest = now + chrono::Duration::seconds(STATS_UPLOAD_PLAYED_AT_MAX_FUTURE_SECS);
    if entry.played_at < earliest || entry.played_at > latest {
        return Err(
            "played_at must be on or after 2022-10-04T00:00:00Z and no more than 24 hours after server time"
                .to_string(),
        );
    }
    Ok(())
}

/// Check one `deleted_sessions` entry. Returns the message for HTTP 400
/// without the `deleted_sessions[n]: ` prefix.
pub fn validate_deleted_session(session_id: &str) -> Result<(), String> {
    if string_too_long(session_id) {
        return Err(format!(
            "session id must be at most {STATS_UPLOAD_STRING_MAX_CHARS} characters"
        ));
    }
    Ok(())
}

/// `POST /api/stats/upload` as the server reads it.
///
/// [`StatsUploadRequest`] is the body the desktop tracker builds. It sends
/// `recognizer` from the stored row and omits `suspect_fields` when that list
/// is empty. 0.4.x clients omit both. This type accepts that JSON plus an
/// optional `recognizer`, `name_recognizer`, and `suspect_fields` on each
/// object in `matches`. The values are per match because one sync batch can
/// carry games captured under different readers.
///
/// A later upload of the same session replaces these fields. Omitting
/// `suspect_fields`, or sending null, stores `[]`, the same way omitting
/// `recognizer` stores `ocr-v1`. A correction that leaves the list out clears
/// names stored by an earlier upload of that session. Omitting
/// `name_recognizer`, or sending null, stores NONE. It is not filled with a
/// reader id.
#[derive(Debug, Clone, Deserialize)]
pub struct StatsUploadBody {
    pub matches: Vec<StatsUploadMatch>,
    /// Same tombstone list as [`StatsUploadRequest::deleted_sessions`].
    #[serde(default)]
    pub deleted_sessions: Vec<String>,
}

/// One match from [`StatsUploadBody`], plus optional reader ids and suspect fields.
#[derive(Debug, Clone, Deserialize)]
#[serde(try_from = "serde_json::Value")]
pub struct StatsUploadMatch {
    pub entry: StatsUploadEntry,
    pub recognizer: RecognizerInput,
    /// Which reader found this player's own scoreboard row.
    ///
    /// Omitted or null means the reader is unknown and is stored as NONE.
    /// There is no default. The check is format only (`[a-z0-9.-]`, 1 to 32
    /// characters). Capitals and spaces are rejected, but a short lowercase
    /// token such as `player1` passes, so clients must send only reader ids
    /// such as `names-v1` or `ocr-v1`. The field identifies only which reader
    /// found the row.
    pub name_recognizer: RecognizerInput,
    pub suspect_fields: SuspectFieldsInput,
    /// Raw `bans_picks` value, `None` when omitted. Checked by
    /// [`resolve_bans_picks`] in the upload route.
    pub bans_picks: Option<serde_json::Value>,
    /// Raw `rank` value, `None` when omitted. Read by [`resolve_rank`] in
    /// the upload route. It never rejects the upload.
    pub rank: Option<serde_json::Value>,
}

impl TryFrom<serde_json::Value> for StatsUploadMatch {
    type Error = String;

    fn try_from(mut value: serde_json::Value) -> Result<Self, Self::Error> {
        let (recognizer, name_recognizer, suspect_fields, bans_picks, rank) = {
            let Some(obj) = value.as_object_mut() else {
                return Err("match entry must be a JSON object".into());
            };
            let recognizer = take_recognizer_input(obj, "recognizer");
            let name_recognizer = take_recognizer_input(obj, "name_recognizer");
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
            let bans_picks = obj.remove("bans_picks");
            let rank = obj.remove("rank");
            (
                recognizer,
                name_recognizer,
                suspect_fields,
                bans_picks,
                rank,
            )
        };
        let entry = serde_json::from_value(value).map_err(|err| err.to_string())?;
        Ok(Self {
            entry,
            recognizer,
            name_recognizer,
            suspect_fields,
            bans_picks,
            rank,
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

/// Who can see a member's tracker stats.
///
/// Stored on `member_settings.stats_visibility`. Missing rows and legacy rows
/// read as [`StatsVisibility::Public`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatsVisibility {
    /// Signed-out visitors and every crew member.
    #[default]
    Public,
    /// Active signed-in crew members. Signed-out callers do not see the numbers.
    Members,
    /// The member, and officers on the aggregate stat routes (not on the officer
    /// match routes, see [`StatsVisibility::officer_match_denial`]). Off every
    /// leaderboard, including for the owner.
    Private,
}

impl StatsVisibility {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Members => "members",
            Self::Private => "private",
        }
    }

    /// Leaderboards and other multi-member aggregates.
    ///
    /// `crew` is true for an active signed-in member. `private` is never
    /// included. `members` is included only when `crew` is true.
    pub fn on_leaderboard(self, crew: bool) -> bool {
        match self {
            Self::Public => true,
            Self::Members => crew,
            Self::Private => false,
        }
    }

    /// `None` when this viewer may see the member's own stat routes.
    ///
    /// `active_member` means a signed-in crew member who is not suspended.
    /// `owner` and `officer` are that same viewer. Signed-out and inactive
    /// callers pass false for all three flags.
    pub fn denial(self, active_member: bool, owner: bool, officer: bool) -> Option<&'static str> {
        match self {
            Self::Public => None,
            Self::Members if active_member => None,
            Self::Members => Some("stats_members_only"),
            Self::Private if owner || officer => None,
            Self::Private => Some("stats_private"),
        }
    }

    /// `None` when an officer may read this member's individual matches on
    /// `GET /api/stats/member/{id}/matches` and `.../matches/{match_id}`.
    ///
    /// `owner` is true when the officer reads their own matches, which is
    /// always allowed. Otherwise `public` and `members` allow officers and
    /// `private` answers [`STATS_HIDDEN_BY_MEMBER`]. Decided 2026-11-03:
    /// officers see a private member's totals on the aggregate routes
    /// ([`StatsVisibility::denial`]) but not their individual matches.
    pub fn officer_match_denial(self, owner: bool) -> Option<&'static str> {
        match self {
            _ if owner => None,
            Self::Public | Self::Members => None,
            Self::Private => Some(STATS_HIDDEN_BY_MEMBER),
        }
    }
}

/// `error` value of [`StatsHiddenError`].
pub const STATS_HIDDEN_BY_MEMBER: &str = "hidden_by_member";

/// 403 body when a member's `stats_visibility` hides their matches from
/// officers.
///
/// Exactly `{"error":"hidden_by_member","message":"..."}`. It carries no
/// counts or dates, so a blocked read says nothing about the member's games.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatsHiddenError {
    pub error: String,
    pub message: String,
}

impl StatsHiddenError {
    pub fn hidden_by_member() -> Self {
        Self {
            error: STATS_HIDDEN_BY_MEMBER.to_string(),
            message: "This member keeps their stats private.".to_string(),
        }
    }
}

/// Per-member settings returned by GET /api/stats/settings (session auth).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberSettingsResponse {
    pub player_name: Option<String>,
    pub stats_visibility: StatsVisibility,
}

/// Body for PUT /api/stats/settings.
///
/// Omitting `player_name`, or sending null, clears the stored name. That is
/// the same as before `stats_visibility` existed. Omitting `stats_visibility`,
/// or sending null, keeps the stored value. The handler answers 400
/// `invalid_stats_visibility` for any other JSON value.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateMemberSettingsRequest {
    pub player_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stats_visibility: Option<StatsVisibility>,
}

/// Daemon configuration returned by GET /api/stats/daemon-config (token auth).
///
/// `stats_visibility` uses the same `public` | `members` | `private` strings as
/// GET and PUT /api/stats/settings. The field is optional so a server that
/// omits it, and a client that ignores it, both keep working.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfigResponse {
    pub player_name: Option<String>,
    /// Stored visibility, or public when the member has no settings row.
    /// Absent when an older server omits the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stats_visibility: Option<StatsVisibility>,
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
/// Rows are grouped by the role the tracker uploaded (its manual correction
/// if made, else the detected role), filled at read time by
/// [`crate::filled_role_label`]: a recognized stored role is kept, an empty or
/// unrecognized one takes the hero's current role. Only games whose hero is
/// not on the list stay in the `""` row. Stored rows are never changed.
/// Order is matches descending, then role ascending.
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

    fn valid_entry() -> StatsUploadEntry {
        StatsUploadEntry {
            session_id: "0123456789abcdef".into(),
            hero: "Ana".into(),
            map_name: "Oasis".into(),
            game_mode: "Control".into(),
            role: "Support".into(),
            outcome: "victory".into(),
            elims: 4,
            deaths: 1,
            assists: 2,
            damage: 1000,
            healing: 4000,
            mitigation: 0,
            played_at: DateTime::parse_from_rfc3339("2026-07-01T20:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            edited: false,
            recognizer: None,
            suspect_fields: Vec::new(),
            name_recognizer: None,
        }
    }

    fn fixed_now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-26T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn upload_entry_valid_row_passes() {
        assert_eq!(validate_upload_entry(&valid_entry(), fixed_now()), Ok(()));
        let mut key = valid_entry();
        key.hero = "wrecking-ball".into();
        assert_eq!(validate_upload_entry(&key, fixed_now()), Ok(()));
    }

    #[test]
    fn upload_entry_strings_capped_at_64_characters() {
        let at_cap = "\u{e9}".repeat(STATS_UPLOAD_STRING_MAX_CHARS);
        let over = "a".repeat(STATS_UPLOAD_STRING_MAX_CHARS + 1);
        type SetStr = fn(&mut StatsUploadEntry, String);
        let setters: [(&str, SetStr); 5] = [
            ("session_id", |e, v| e.session_id = v),
            ("map_name", |e, v| e.map_name = v),
            ("game_mode", |e, v| e.game_mode = v),
            ("role", |e, v| e.role = v),
            ("outcome", |e, v| e.outcome = v),
        ];
        for (name, set) in setters {
            let mut ok = valid_entry();
            set(&mut ok, at_cap.clone());
            assert_eq!(validate_upload_entry(&ok, fixed_now()), Ok(()), "{name}");
            let mut bad = valid_entry();
            set(&mut bad, over.clone());
            assert_eq!(
                validate_upload_entry(&bad, fixed_now()),
                Err(format!("{name} must be at most 64 characters"))
            );
        }
        let mut hero = valid_entry();
        hero.hero = over;
        assert_eq!(
            validate_upload_entry(&hero, fixed_now()),
            Err("hero must be at most 64 characters".to_string())
        );
    }

    #[test]
    fn upload_entry_non_hero_rejected() {
        for hero in ["", "Not A Hero", "Unknown Hero", "Unknownn", "unknown "] {
            let mut e = valid_entry();
            e.hero = hero.into();
            assert_eq!(
                validate_upload_entry(&e, fixed_now()),
                Err("hero must be a known hero or Unknown".to_string()),
                "{hero:?}"
            );
        }
    }

    /// Regression: 0.4.x and 0.5.0 trackers send `Unknown` when no hero was
    /// read, and live data has such rows. It must pass in any case.
    #[test]
    fn upload_entry_unknown_hero_accepted_any_case() {
        for hero in ["Unknown", "unknown", "UNKNOWN", "uNkNoWn"] {
            let mut e = valid_entry();
            e.hero = hero.into();
            assert_eq!(validate_upload_entry(&e, fixed_now()), Ok(()), "{hero:?}");
        }
    }

    #[test]
    fn upload_entry_doctrine_accepted() {
        for hero in ["Doctrine", "doctrine", "DOCTRINE"] {
            let mut e = valid_entry();
            e.hero = hero.into();
            assert_eq!(validate_upload_entry(&e, fixed_now()), Ok(()), "{hero:?}");
        }
    }

    /// Every hero in the shared list must pass the upload check, so a new
    /// hero added to `heroes.rs` cannot be rejected by the server.
    #[test]
    fn upload_entry_every_listed_hero_accepted() {
        for hero in crate::HEROES {
            let mut e = valid_entry();
            e.hero = (*hero).into();
            assert_eq!(validate_upload_entry(&e, fixed_now()), Ok(()), "{hero:?}");
        }
        for hero in crate::HERO_NAMES {
            let mut e = valid_entry();
            e.hero = hero.display.into();
            assert_eq!(
                validate_upload_entry(&e, fixed_now()),
                Ok(()),
                "{:?}",
                hero.display
            );
        }
    }

    #[test]
    fn upload_entry_numeric_ceilings() {
        type Set = fn(&mut StatsUploadEntry, u32);
        let cases: [(&str, u32, Set); 6] = [
            ("elims", 999, |e, v| e.elims = v),
            ("assists", 999, |e, v| e.assists = v),
            ("deaths", 50, |e, v| e.deaths = v),
            ("damage", 200_000, |e, v| e.damage = v),
            ("healing", 200_000, |e, v| e.healing = v),
            ("mitigation", 200_000, |e, v| e.mitigation = v),
        ];
        for (name, max, set) in cases {
            let mut ok = valid_entry();
            set(&mut ok, max);
            assert_eq!(validate_upload_entry(&ok, fixed_now()), Ok(()), "{name}");
            for bad_value in [max + 1, u32::MAX] {
                let mut bad = valid_entry();
                set(&mut bad, bad_value);
                assert_eq!(
                    validate_upload_entry(&bad, fixed_now()),
                    Err(format!("{name} must be at most {max}"))
                );
            }
        }
    }

    #[test]
    fn upload_entry_played_at_window() {
        let now = fixed_now();
        let err = Err(
            "played_at must be on or after 2022-10-04T00:00:00Z and no more than 24 hours after server time"
                .to_string(),
        );
        let floor = DateTime::parse_from_rfc3339("2022-10-04T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let cases = [
            (floor, Ok(())),
            (floor - chrono::Duration::seconds(1), err.clone()),
            (now + chrono::Duration::hours(24), Ok(())),
            (
                now + chrono::Duration::hours(24) + chrono::Duration::seconds(1),
                err.clone(),
            ),
            (
                DateTime::parse_from_rfc3339("2099-01-01T00:00:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
                err.clone(),
            ),
            (DateTime::<Utc>::UNIX_EPOCH, err),
        ];
        for (played_at, expected) in cases {
            let mut e = valid_entry();
            e.played_at = played_at;
            assert_eq!(validate_upload_entry(&e, now), expected, "{played_at}");
        }
    }

    #[test]
    fn deleted_session_capped_at_64_characters() {
        assert_eq!(validate_deleted_session(&"a".repeat(64)), Ok(()));
        assert_eq!(
            validate_deleted_session(&"a".repeat(65)),
            Err("session id must be at most 64 characters".to_string())
        );
    }

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
                name_recognizer: None,
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
    fn resolve_name_recognizer_keeps_absent_unknown_and_reuses_the_id_check() {
        assert_eq!(
            resolve_name_recognizer(&RecognizerInput::Absent).unwrap(),
            None
        );
        assert_eq!(
            resolve_name_recognizer(&RecognizerInput::Value("names-v1".into())).unwrap(),
            Some("names-v1".to_string())
        );
        assert_eq!(
            resolve_name_recognizer(&RecognizerInput::Value("ocr-v1".into())).unwrap(),
            Some("ocr-v1".to_string())
        );
        // Not an allowlist: a later id of the same shape is stored.
        assert_eq!(
            resolve_name_recognizer(&RecognizerInput::Value("reader-v2".into())).unwrap(),
            Some("reader-v2".to_string())
        );
        // Format only. Capitals and spaces are rejected. A short lowercase
        // token such as player1 passes.
        assert_eq!(
            resolve_name_recognizer(&RecognizerInput::Value("player1".into())).unwrap(),
            Some("player1".to_string())
        );
        assert!(is_recognizer_id("names-v1"));
        assert_eq!(
            resolve_name_recognizer(&RecognizerInput::Value("NOPE".into())).unwrap_err(),
            NAME_RECOGNIZER_ID_ERROR
        );
        assert!(resolve_name_recognizer(&RecognizerInput::Value(String::new())).is_err());
        assert!(resolve_name_recognizer(&RecognizerInput::Value("has space".into())).is_err());
        assert!(
            resolve_name_recognizer(&RecognizerInput::Value(
                "a".repeat(RECOGNIZER_ID_MAX_LEN + 1)
            ))
            .is_err()
        );
        assert!(resolve_name_recognizer(&RecognizerInput::NotAString).is_err());
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
            old.matches[0].name_recognizer,
            RecognizerInput::Absent
        ));
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
        assert_eq!(
            null_body.matches[0].name_recognizer,
            RecognizerInput::Absent
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
    fn name_recognizer_null_is_absent_and_non_string_is_flagged() {
        let null_body: StatsUploadBody = serde_json::from_str(
            r#"{
                "matches": [{
                    "hero": "Ana",
                    "map_name": "Oasis",
                    "game_mode": "control",
                    "role": "Support",
                    "outcome": "victory",
                    "played_at": "2026-07-01T20:00:00Z",
                    "name_recognizer": null
                }]
            }"#,
        )
        .unwrap();
        assert_eq!(
            null_body.matches[0].name_recognizer,
            RecognizerInput::Absent
        );
        assert!(matches!(
            null_body.matches[0].recognizer,
            RecognizerInput::Absent
        ));

        let number_body: StatsUploadBody = serde_json::from_str(
            r#"{
                "matches": [{
                    "hero": "Ana",
                    "map_name": "Oasis",
                    "game_mode": "control",
                    "role": "Support",
                    "outcome": "victory",
                    "played_at": "2026-07-01T20:00:00Z",
                    "name_recognizer": 1
                }]
            }"#,
        )
        .unwrap();
        assert_eq!(
            number_body.matches[0].name_recognizer,
            RecognizerInput::NotAString
        );

        let named: StatsUploadBody = serde_json::from_str(
            r#"{
                "matches": [{
                    "hero": "Ana",
                    "map_name": "Oasis",
                    "game_mode": "control",
                    "role": "Support",
                    "outcome": "victory",
                    "played_at": "2026-07-01T20:00:00Z",
                    "recognizer": "ocr-v1",
                    "name_recognizer": "names-v1"
                }]
            }"#,
        )
        .unwrap();
        assert_eq!(
            named.matches[0].name_recognizer,
            RecognizerInput::Value("names-v1".into())
        );
        assert_eq!(named.matches[0].entry.recognizer, None);
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

    fn entry_with_name_recognizer(name_recognizer: Option<&str>) -> StatsUploadEntry {
        let mut entry = sample_request().matches.remove(0);
        entry.name_recognizer = name_recognizer.map(str::to_string);
        entry
    }

    fn name_recognizer_json(name_recognizer: Option<&str>) -> String {
        serde_json::to_string(&entry_with_name_recognizer(name_recognizer)).unwrap()
    }

    #[test]
    fn name_recognizer_serializes_names_v1() {
        let raw = name_recognizer_json(Some("names-v1"));
        assert!(raw.contains("\"name_recognizer\":\"names-v1\""), "{raw}");
        let back: StatsUploadEntry = serde_json::from_str(&raw).unwrap();
        assert_eq!(back.name_recognizer.as_deref(), Some("names-v1"));
    }

    #[test]
    fn name_recognizer_serializes_ocr_v1() {
        let raw = name_recognizer_json(Some("ocr-v1"));
        assert!(raw.contains("\"name_recognizer\":\"ocr-v1\""), "{raw}");
        let back: StatsUploadEntry = serde_json::from_str(&raw).unwrap();
        assert_eq!(back.name_recognizer.as_deref(), Some("ocr-v1"));
    }

    #[test]
    fn name_recognizer_is_omitted_when_no_name_was_read() {
        let raw = name_recognizer_json(None);
        assert!(!raw.contains("name_recognizer"), "{raw}");
        let back: StatsUploadEntry = serde_json::from_str(&raw).unwrap();
        assert_eq!(back.name_recognizer, None);
        let bare: StatsUploadEntry = serde_json::from_str(
            r#"{"hero":"Ana","map_name":"Oasis","game_mode":"control","role":"Support","outcome":"victory","played_at":"2026-07-01T20:00:00Z"}"#,
        )
        .unwrap();
        assert_eq!(bare.name_recognizer, None);
    }

    #[test]
    fn stats_visibility_serde_and_denial() {
        assert_eq!(
            serde_json::to_string(&StatsVisibility::Members).unwrap(),
            "\"members\""
        );
        assert_eq!(
            serde_json::from_str::<StatsVisibility>("\"private\"").unwrap(),
            StatsVisibility::Private
        );
        assert!(serde_json::from_str::<StatsVisibility>("\"nope\"").is_err());

        let omitted: UpdateMemberSettingsRequest =
            serde_json::from_str(r#"{"player_name":"Ana"}"#).unwrap();
        assert_eq!(omitted.stats_visibility, None);
        let missing_name =
            serde_json::from_str::<UpdateMemberSettingsRequest>(r#"{"stats_visibility":"public"}"#)
                .unwrap();
        assert_eq!(missing_name.player_name, None);
        assert_eq!(missing_name.stats_visibility, Some(StatsVisibility::Public));

        assert_eq!(
            StatsVisibility::Members.denial(false, false, false),
            Some("stats_members_only")
        );
        assert_eq!(StatsVisibility::Members.denial(true, false, false), None);
        assert_eq!(
            StatsVisibility::Private.denial(true, false, false),
            Some("stats_private")
        );
        assert_eq!(StatsVisibility::Private.denial(true, true, false), None);
        assert_eq!(StatsVisibility::Private.denial(true, false, true), None);
        assert!(!StatsVisibility::Private.on_leaderboard(true));
        assert!(!StatsVisibility::Members.on_leaderboard(false));
        assert!(StatsVisibility::Members.on_leaderboard(true));
        assert!(StatsVisibility::Public.on_leaderboard(false));
    }

    #[test]
    fn officer_match_denial_blocks_only_private_for_others() {
        assert_eq!(StatsVisibility::Public.officer_match_denial(false), None);
        assert_eq!(StatsVisibility::Members.officer_match_denial(false), None);
        assert_eq!(
            StatsVisibility::Private.officer_match_denial(false),
            Some(STATS_HIDDEN_BY_MEMBER)
        );
        for vis in [
            StatsVisibility::Public,
            StatsVisibility::Members,
            StatsVisibility::Private,
        ] {
            assert_eq!(vis.officer_match_denial(true), None, "{vis:?}");
        }
    }

    #[test]
    fn stats_hidden_error_body_is_stable() {
        let body = serde_json::to_value(StatsHiddenError::hidden_by_member()).unwrap();
        let expected = serde_json::json!({
            "error": "hidden_by_member",
            "message": "This member keeps their stats private.",
        });
        assert_eq!(body, expected);
    }

    #[test]
    fn daemon_config_stats_visibility_is_optional() {
        let bare: DaemonConfigResponse = serde_json::from_str(r#"{"player_name":"Ana"}"#).unwrap();
        assert_eq!(bare.player_name.as_deref(), Some("Ana"));
        assert_eq!(bare.stats_visibility, None);
        let raw = serde_json::to_string(&bare).unwrap();
        assert!(!raw.contains("stats_visibility"), "{raw}");

        let null_name: DaemonConfigResponse =
            serde_json::from_str(r#"{"player_name":null}"#).unwrap();
        assert_eq!(null_name.player_name, None);
        assert_eq!(null_name.stats_visibility, None);

        for (label, vis) in [
            ("public", StatsVisibility::Public),
            ("members", StatsVisibility::Members),
            ("private", StatsVisibility::Private),
        ] {
            let parsed: DaemonConfigResponse = serde_json::from_str(&format!(
                r#"{{"player_name":null,"stats_visibility":"{label}"}}"#
            ))
            .unwrap();
            assert_eq!(parsed.stats_visibility, Some(vis));
            let out = serde_json::to_string(&DaemonConfigResponse {
                player_name: None,
                stats_visibility: Some(vis),
            })
            .unwrap();
            assert!(
                out.contains(&format!("\"stats_visibility\":\"{label}\"")),
                "{out}"
            );
        }
    }

    #[test]
    fn bans_picks_full_object_resolves_to_pack_keys() {
        let raw = serde_json::json!({
            "bans": [
                { "hero": "mauga", "team": "own" },
                { "hero": "Wrecking Ball", "team": "enemy" },
                { "hero": null, "team": null },
                null
            ],
            "lobby_ban": "ana",
            "picks": ["wrecking-ball", null, "soldier-76", "dva", "lucio", "mercy"],
            "ban_vote": ["ana", "no_preference", null, "dva", "Mercy", "no_preference"],
            "recognizer": "cv-v1"
        });
        let got = resolve_bans_picks(Some(&raw)).unwrap().unwrap();
        assert_eq!(got.bans.len(), 4);
        assert_eq!(got.bans[0].hero.as_deref(), Some("mauga"));
        assert_eq!(got.bans[0].team, Some(BanTeam::Own));
        assert_eq!(got.bans[1].hero.as_deref(), Some("wrecking-ball"));
        assert_eq!(got.bans[1].team, Some(BanTeam::Enemy));
        assert_eq!(
            got.bans[2],
            HeroBan {
                hero: None,
                team: None
            }
        );
        assert_eq!(
            got.bans[3],
            HeroBan {
                hero: None,
                team: None
            }
        );
        assert_eq!(got.lobby_ban.as_deref(), Some("ana"));
        assert_eq!(got.picks[0].as_deref(), Some("wrecking-ball"));
        assert_eq!(got.picks[1], None);
        assert_eq!(got.picks.len(), 6);
        assert_eq!(
            got.ban_vote,
            vec![
                Some("ana".to_string()),
                Some("no_preference".to_string()),
                None,
                Some("dva".to_string()),
                Some("mercy".to_string()),
                Some("no_preference".to_string()),
            ]
        );
        assert_eq!(got.recognizer.as_deref(), Some("cv-v1"));
        let json = serde_json::to_value(&got).unwrap();
        assert_eq!(
            json["bans"][0],
            serde_json::json!({ "hero": "mauga", "team": "own" })
        );
        assert_eq!(bans_picks_from_stored(&json), Some(got));
    }

    #[test]
    fn stored_object_without_ban_vote_reads_as_empty() {
        let stored = serde_json::json!({
            "bans": [],
            "lobby_ban": null,
            "picks": ["no_preference", null],
            "recognizer": "cv-v1"
        });
        let got = bans_picks_from_stored(&stored).unwrap();
        assert!(got.ban_vote.is_empty());
        assert_eq!(got.picks, vec![Some("no_preference".to_string()), None]);
    }

    #[test]
    fn bans_picks_absent_or_null_is_none() {
        assert_eq!(resolve_bans_picks(None), Ok(None));
        assert_eq!(resolve_bans_picks(Some(&serde_json::Value::Null)), Ok(None));
        let empty = resolve_bans_picks(Some(&serde_json::json!({})))
            .unwrap()
            .unwrap();
        assert_eq!(empty, BansPicks::default());
    }

    #[test]
    fn bans_picks_unknown_hero_key_is_null() {
        let raw = serde_json::json!({
            "bans": [{ "hero": "not-a-hero", "team": "own" }],
            "lobby_ban": "Unknown",
            "picks": ["zzz", "placeholder-1"],
            "ban_vote": ["nobody", "No_Preference", "no-preference", "ana"]
        });
        let got = resolve_bans_picks(Some(&raw)).unwrap().unwrap();
        assert_eq!(got.bans[0].hero, None);
        assert_eq!(got.bans[0].team, Some(BanTeam::Own));
        assert_eq!(got.lobby_ban, None);
        assert_eq!(got.picks, vec![None, None]);
        // Only the exact literal is kept. Near misses are unknown keys.
        assert_eq!(got.ban_vote, vec![None, None, None, Some("ana".into())]);
    }

    #[test]
    fn bans_picks_no_preference_is_kept_in_picks_and_ban_vote_and_never_null() {
        let raw = serde_json::json!({
            "picks": ["no_preference", null, "ana"],
            "ban_vote": ["no_preference", null, "ana"],
            "bans": [{ "hero": "no_preference", "team": "own" }],
            "lobby_ban": "no_preference"
        });
        let got = resolve_bans_picks(Some(&raw)).unwrap().unwrap();
        let expected = vec![Some("no_preference".to_string()), None, Some("ana".into())];
        assert_eq!(got.picks, expected);
        assert_eq!(got.ban_vote, expected);
        // The literal is for teammate choices only. A ban or the lobby ban
        // cannot be "no preference", so those stay unknown keys (null).
        assert_eq!(got.bans[0].hero, None);
        assert_eq!(got.lobby_ban, None);
        let json = serde_json::to_value(&got).unwrap();
        assert_eq!(
            json["picks"],
            serde_json::json!(["no_preference", null, "ana"])
        );
        assert_eq!(
            json["ban_vote"],
            serde_json::json!(["no_preference", null, "ana"])
        );
        assert_eq!(bans_picks_from_stored(&json), Some(got));
    }

    #[test]
    fn bans_picks_rejects_long_lists_bad_shape_and_bad_recognizer() {
        let five = serde_json::json!({ "bans": [
            { "hero": "ana" }, { "hero": "ana" }, { "hero": "ana" }, { "hero": "ana" }, { "hero": "ana" }
        ]});
        let err = resolve_bans_picks(Some(&five)).unwrap_err();
        assert!(
            err.contains("bans_picks.bans must have at most 4 entries"),
            "{err}"
        );
        let seven =
            serde_json::json!({ "picks": ["ana", "ana", "ana", "ana", "ana", "ana", "ana"] });
        let err = resolve_bans_picks(Some(&seven)).unwrap_err();
        assert!(
            err.contains("bans_picks.picks must have at most 6 entries"),
            "{err}"
        );
        let vote =
            serde_json::json!({ "ban_vote": ["ana", "ana", "ana", "ana", "ana", "ana", "ana"] });
        assert_eq!(
            resolve_bans_picks(Some(&vote)),
            Err("bans_picks.ban_vote must have at most 6 entries, got 7".to_string())
        );
        let six = serde_json::json!({ "ban_vote": ["ana", "ana", "ana", "ana", "ana", "ana"] });
        assert!(resolve_bans_picks(Some(&six)).is_ok());
        for bad in [
            "CV-V1",
            "",
            "cv v1",
            "a234567890123456789012345678901234",
            "--",
        ] {
            let raw = serde_json::json!({ "recognizer": bad });
            assert_eq!(
                resolve_bans_picks(Some(&raw)),
                Err(BANS_PICKS_RECOGNIZER_ERROR.to_string()),
                "{bad:?}"
            );
        }
        for raw in [
            serde_json::json!({ "recognizer": 3 }),
            serde_json::json!("cv-v1"),
            serde_json::json!({ "bans": "ana" }),
            serde_json::json!({ "bans": [3] }),
            serde_json::json!({ "bans": [{ "team": "ally" }] }),
            serde_json::json!({ "bans": [{ "hero": 7 }] }),
            serde_json::json!({ "picks": [1] }),
            serde_json::json!({ "ban_vote": [1] }),
            serde_json::json!({ "ban_vote": "ana" }),
            serde_json::json!({ "ban_vote": [{ "hero": "ana" }] }),
            serde_json::json!({ "lobby_ban": true }),
        ] {
            assert!(resolve_bans_picks(Some(&raw)).is_err(), "{raw}");
        }
    }

    #[test]
    fn upload_match_takes_bans_picks_out_of_the_entry() {
        let mut raw = serde_json::to_value(valid_entry()).unwrap();
        raw["bans_picks"] = serde_json::json!({ "lobby_ban": "ana" });
        let m: StatsUploadMatch = serde_json::from_value(raw).unwrap();
        assert_eq!(
            m.bans_picks,
            Some(serde_json::json!({ "lobby_ban": "ana" }))
        );
        let m: StatsUploadMatch =
            serde_json::from_value(serde_json::to_value(valid_entry()).unwrap()).unwrap();
        assert_eq!(m.bans_picks, None);
    }
}
