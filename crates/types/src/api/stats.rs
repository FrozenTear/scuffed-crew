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
    /// Recognizer stored on the local row. Always sent. Older clients that
    /// omit it still deserialize as `ocr-v1`.
    #[serde(default = "default_upload_recognizer")]
    pub recognizer: String,
    /// Flat names this row was not sure about. Omitted when empty so a
    /// confident or ocr-v1 row does not add the key.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suspect_fields: Vec<String>,
}

fn default_upload_recognizer() -> String {
    RECOGNIZER_OCR_V1.to_string()
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
#[derive(Debug, Clone, Deserialize)]
pub struct StatsUploadBody {
    pub matches: Vec<StatsUploadMatch>,
    /// Same tombstone list as [`StatsUploadRequest::deleted_sessions`].
    #[serde(default)]
    pub deleted_sessions: Vec<String>,
}

/// One match from [`StatsUploadBody`], plus optional recognizer and suspect fields.
#[derive(Debug, Clone, Deserialize)]
#[serde(try_from = "serde_json::Value")]
pub struct StatsUploadMatch {
    pub entry: StatsUploadEntry,
    pub recognizer: RecognizerInput,
    pub suspect_fields: SuspectFieldsInput,
}

impl TryFrom<serde_json::Value> for StatsUploadMatch {
    type Error = String;

    fn try_from(mut value: serde_json::Value) -> Result<Self, Self::Error> {
        let (recognizer, suspect_fields) = {
            let Some(obj) = value.as_object_mut() else {
                return Err("match entry must be a JSON object".into());
            };
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
            (recognizer, suspect_fields)
        };
        let entry = serde_json::from_value(value).map_err(|err| err.to_string())?;
        Ok(Self {
            entry,
            recognizer,
            suspect_fields,
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
                recognizer: RECOGNIZER_OCR_V1.into(),
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
}
