use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct MatchPayload {
    pub team_id: String,
    pub opponent: String,
    /// Always serialized so admin edit can send `null` to clear (omit = leave).
    pub score_us: Option<u32>,
    pub score_them: Option<u32>,
    pub map_name: Option<String>,
    pub game_mode: Option<String>,
    pub match_type: String,
    /// RFC3339 when played; `null` clears on update / omits meaning for create.
    pub played_at: Option<String>,
    pub scheduled_at: Option<String>,
    pub notes: Option<String>,
    /// When true, appears on public team pages / lists.
    /// Always serialized so update can unpublish (`false` must not be omitted).
    #[serde(default)]
    pub is_public: bool,
    pub vod_url: Option<String>,
    pub replay_code: Option<String>,
}
