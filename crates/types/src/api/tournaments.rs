use serde::Serialize;

/// Mirrors site-server CreateTournamentRequest field names.
#[derive(Debug, Clone, Serialize)]
pub struct CreateTournamentRequest {
    pub name: String,
    pub format: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_id: Option<String>,
    /// Server field is `max_teams` (not max_participants).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_teams: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub starts_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusChangeRequest {
    pub status: String,
}

/// Mirrors site-server `AddParticipantRequest` (POST .../participants): an
/// org team (`team_id`) or an outside entrant (`external_name`). The server
/// answers 400 when both are missing.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AddParticipantRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u32>,
}

/// Mirrors site-server `ReportMatchRequest` (PATCH .../matches/{mid}/report).
#[derive(Debug, Clone, Serialize)]
pub struct MatchReportRequest {
    pub score_a: u32,
    pub score_b: u32,
    pub winner_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replay_codes: Option<Vec<String>>,
}
