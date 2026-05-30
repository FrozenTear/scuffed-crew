use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatsUploadEntry {
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatsUploadRequest {
    pub matches: Vec<StatsUploadEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatsUploadResponse {
    pub inserted: u32,
    pub skipped: u32,
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
