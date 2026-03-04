use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct CreateAnnouncementRequest {
    pub title: String,
    pub content: String,
    pub is_pinned: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateAnnouncementRequest {
    pub title: String,
    pub content: String,
    pub is_pinned: bool,
}
