//! Self-service deletion shapes.
//!
//! Website session:
//! - `DELETE /api/stats/me` body `{"confirm":"delete my stats"}`
//!   -> `200 {"deleted":{...counts},"tokens_revoked":N}` (immediate)
//! - `DELETE /api/auth/me` body `{"password":"...","confirm":"<account name>"}`
//!   Password is omitted for accounts with no password.
//!   -> `200 {"deleted":true}`
//! - `POST /api/stats/me/restore` -> `200 {"restored":true}`
//!   `404 {"code":"no_pending_delete"}` when nothing is waiting.
//!
//! Tracker token delete (typed here so either body decodes):
//! `{"scheduled":true,"stats_delete_scheduled_at":"<RFC3339>","tokens_revoked":N}`
//!
//! `GET /api/stats/me` adds `stats_delete_scheduled_at`: an RFC3339 string or null.
//! Missing fields stay absent. Unknown keys are ignored.

use serde::{Deserialize, Serialize};

/// Phrase the website session sends with `DELETE /api/stats/me`.
pub const DELETE_MY_STATS_CONFIRM: &str = "delete my stats";

pub const DELETE_MY_STATS_PATH: &str = "/api/stats/me";
pub const RESTORE_MY_STATS_PATH: &str = "/api/stats/me/restore";
pub const DELETE_MY_ACCOUNT_PATH: &str = "/api/auth/me";

pub const CODE_INVALID_PASSWORD: &str = "invalid_password";
pub const CODE_REAUTH_REQUIRED: &str = "reauth_required";
pub const CODE_CONFIRM_MISMATCH: &str = "confirm_mismatch";
pub const CODE_LAST_ADMIN: &str = "last_admin";
pub const CODE_NO_PENDING_DELETE: &str = "no_pending_delete";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteMyStatsRequest {
    pub confirm: String,
}

impl DeleteMyStatsRequest {
    pub fn website() -> Self {
        Self {
            confirm: DELETE_MY_STATS_CONFIRM.to_string(),
        }
    }
}

/// `DELETE /api/stats/me` success. Either the immediate website body or the
/// scheduled token body. Every field is optional.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DeleteMyStatsResponse {
    #[serde(default)]
    pub deleted: Option<serde_json::Map<String, serde_json::Value>>,
    #[serde(default)]
    pub scheduled: Option<bool>,
    #[serde(default)]
    pub stats_delete_scheduled_at: Option<String>,
    #[serde(default)]
    pub tokens_revoked: Option<u64>,
}

const COUNT_ORDER: &[&str] = &[
    "personal_match",
    "stat_report",
    "matches",
    "heroes",
    "roles",
    "maps",
];

impl DeleteMyStatsResponse {
    /// Token path, or any body that names a schedule time.
    pub fn is_scheduled(&self) -> bool {
        self.scheduled.unwrap_or(false)
            || self
                .stats_delete_scheduled_at
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
    }

    /// Numeric entries inside `deleted`, preferred keys first, then the rest
    /// in alphabetical order. Missing keys and non-numbers are skipped.
    pub fn deleted_counts(&self) -> Vec<(String, u64)> {
        let Some(map) = &self.deleted else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for key in COUNT_ORDER {
            if let Some(count) = map.get(*key).and_then(json_count) {
                out.push(((*key).to_string(), count));
                seen.insert((*key).to_string());
            }
        }
        let mut rest: Vec<String> = map
            .keys()
            .filter(|key| !seen.contains(key.as_str()))
            .cloned()
            .collect();
        rest.sort();
        for key in rest {
            if let Some(count) = map.get(&key).and_then(json_count) {
                out.push((key, count));
            }
        }
        out
    }
}

fn json_count(value: &serde_json::Value) -> Option<u64> {
    match value {
        serde_json::Value::Number(number) => number.as_u64().or_else(|| {
            let float = number.as_f64()?;
            if float >= 0.0 && float.fract() == 0.0 && float <= u64::MAX as f64 {
                Some(float as u64)
            } else {
                None
            }
        }),
        serde_json::Value::String(text) => text.trim().parse::<u64>().ok(),
        _ => None,
    }
}

/// `code` from an error JSON body. Other shapes are `None`.
pub fn response_code(body: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| value.get("code")?.as_str().map(str::to_owned))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteMyAccountRequest {
    /// Current password. Omitted when the account has none, or the field was left blank.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    pub confirm: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteMyAccountResponse {
    #[serde(default)]
    pub deleted: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestoreMyStatsRequest {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestoreMyStatsResponse {
    #[serde(default)]
    pub restored: bool,
}

/// The one new field on `GET /api/stats/me`. Other overview fields are ignored
/// so this still decodes a full stats payload.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatsDeleteSchedule {
    #[serde(default)]
    pub stats_delete_scheduled_at: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn website_stats_delete_body_is_the_confirm_phrase() {
        let body = DeleteMyStatsRequest::website();
        assert_eq!(DELETE_MY_STATS_PATH, "/api/stats/me");
        assert_eq!(
            serde_json::to_string(&body).unwrap(),
            r#"{"confirm":"delete my stats"}"#
        );
    }

    #[test]
    fn account_delete_omits_a_missing_password() {
        let omitted = DeleteMyAccountRequest {
            password: None,
            confirm: "Ada".into(),
        };
        assert_eq!(
            serde_json::to_string(&omitted).unwrap(),
            r#"{"confirm":"Ada"}"#
        );
        let with_password = DeleteMyAccountRequest {
            password: Some("secret".into()),
            confirm: "Ada".into(),
        };
        assert_eq!(
            serde_json::to_string(&with_password).unwrap(),
            r#"{"password":"secret","confirm":"Ada"}"#
        );
    }

    #[test]
    fn stats_delete_tolerates_either_shape_and_missing_fields() {
        let empty: DeleteMyStatsResponse = serde_json::from_str("{}").unwrap();
        assert!(empty.deleted_counts().is_empty());
        assert!(!empty.is_scheduled());
        assert_eq!(empty.tokens_revoked, None);

        let immediate: DeleteMyStatsResponse = serde_json::from_str(
            r#"{"deleted":{"heroes":1,"matches":4,"extra":"nope","maps":2},"tokens_revoked":3}"#,
        )
        .unwrap();
        assert_eq!(
            immediate.deleted_counts(),
            vec![
                ("matches".into(), 4),
                ("heroes".into(), 1),
                ("maps".into(), 2),
            ]
        );
        assert!(!immediate.is_scheduled());
        assert_eq!(immediate.tokens_revoked, Some(3));

        let scheduled: DeleteMyStatsResponse = serde_json::from_str(
            r#"{"scheduled":true,"stats_delete_scheduled_at":"2026-10-26T15:04:05Z","tokens_revoked":1}"#,
        )
        .unwrap();
        assert!(scheduled.deleted.is_none());
        assert!(scheduled.is_scheduled());
        assert_eq!(
            scheduled.stats_delete_scheduled_at.as_deref(),
            Some("2026-10-26T15:04:05Z")
        );
    }

    #[test]
    fn schedule_field_is_optional_on_the_stats_overview() {
        let missing: StatsDeleteSchedule =
            serde_json::from_str(r#"{"member_id":"m","total_matches":2,"wins":1}"#).unwrap();
        assert_eq!(missing.stats_delete_scheduled_at, None);
        let null: StatsDeleteSchedule =
            serde_json::from_str(r#"{"stats_delete_scheduled_at":null,"wins":0}"#).unwrap();
        assert_eq!(null.stats_delete_scheduled_at, None);
        let set: StatsDeleteSchedule = serde_json::from_str(
            r#"{"stats_delete_scheduled_at":"2026-10-26T15:04:05Z","total_matches":0}"#,
        )
        .unwrap();
        assert_eq!(
            set.stats_delete_scheduled_at.as_deref(),
            Some("2026-10-26T15:04:05Z")
        );
    }

    #[test]
    fn restore_and_account_flags_default_when_absent() {
        let restored: RestoreMyStatsResponse = serde_json::from_str("{}").unwrap();
        assert!(!restored.restored);
        let restored: RestoreMyStatsResponse =
            serde_json::from_str(r#"{"restored":true}"#).unwrap();
        assert!(restored.restored);
        let deleted: DeleteMyAccountResponse = serde_json::from_str(r#"{"deleted":true}"#).unwrap();
        assert!(deleted.deleted);
    }

    #[test]
    fn response_code_reads_only_the_code_field() {
        assert_eq!(
            response_code(r#"{"code":"last_admin"}"#).as_deref(),
            Some("last_admin")
        );
        assert_eq!(response_code(r#"{"error":"last_admin"}"#), None);
        assert_eq!(response_code("nope"), None);
    }
}
