use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct CreateTournamentRequest {
    pub name: String,
    pub format: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_participants: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub starts_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusChangeRequest {
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AddParticipantRequest {
    pub member_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MatchReportRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score_a: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score_b: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub winner_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replay_codes: Option<String>,
}
