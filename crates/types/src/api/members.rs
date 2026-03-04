use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ChangeRoleRequest {
    pub role: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToggleActiveRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_active: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateGameAccountRequest {
    pub game_id: String,
    pub account_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
}
