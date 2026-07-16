use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct MatchPayload {
    pub team_id: String,
    pub opponent: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score_us: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score_them: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub map_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_mode: Option<String>,
    pub match_type: String,
    /// RFC3339 when played; omit for scheduled-only fixtures.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub played_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scheduled_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// When true, appears on public team pages / lists.
    /// Always serialized so update can unpublish (`false` must not be omitted).
    #[serde(default)]
    pub is_public: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vod_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replay_code: Option<String>,
}
