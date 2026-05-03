use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct UpdateSettingsRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub org_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub site_description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recruitment_open: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recruitment_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_age: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forum_backend: Option<String>,
}
