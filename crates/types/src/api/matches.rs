use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct MatchPayload {
    pub team_id: String,
    pub opponent: String,
    pub score_us: u32,
    pub score_them: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub map_name: Option<String>,
    pub match_type: String,
    pub played_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// When true, appears on public team pages / lists.
    /// Always serialized so update can unpublish (`false` must not be omitted).
    #[serde(default)]
    pub is_public: bool,
}
