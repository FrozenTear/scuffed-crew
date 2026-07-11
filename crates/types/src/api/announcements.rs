use serde::Serialize;

/// Server accepts `pinned` (see site-server CreateAnnouncementRequest).
#[derive(Debug, Clone, Serialize)]
pub struct CreateAnnouncementRequest {
    pub title: String,
    pub content: String,
    pub pinned: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateAnnouncementRequest {
    pub title: String,
    pub content: String,
    pub pinned: bool,
}
