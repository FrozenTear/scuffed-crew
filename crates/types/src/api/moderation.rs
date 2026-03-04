use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct CreateModerationRequest {
    pub member_id: String,
    pub action_type: String,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}
