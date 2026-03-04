use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct CreateGameRequest {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub abbreviation: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateGameRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub abbreviation: Option<Option<String>>,
}
