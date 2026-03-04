use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct CreateTeamRequest {
    pub name: String,
    pub game_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub division: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AddRosterMemberRequest {
    pub member_id: String,
    pub team_role: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateRosterRoleRequest {
    pub team_role: String,
}
