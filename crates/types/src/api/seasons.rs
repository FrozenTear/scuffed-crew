//! Competitive seasons — half-open `[starts_at, ends_at)` windows over
//! `personal_match.played_at`. Every stats endpoint accepts `?season=<id>`;
//! omitted or empty means all time. The `Season` record itself lives in
//! [`crate::org::Season`].

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSeasonRequest {
    pub name: String,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    #[serde(default)]
    pub is_current: bool,
}

/// Partial update — every field optional; the merged window must still be
/// non-empty (`ends_at > starts_at`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateSeasonRequest {
    pub name: Option<String>,
    pub starts_at: Option<DateTime<Utc>>,
    pub ends_at: Option<DateTime<Utc>>,
    pub is_current: Option<bool>,
}

/// `?season=<id>` on stats endpoints. Unknown id → 404, omitted/empty → all time.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SeasonQuery {
    pub season: Option<String>,
}
