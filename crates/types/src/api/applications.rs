use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct PatchApplicationRequest {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review_notes: Option<String>,
}
