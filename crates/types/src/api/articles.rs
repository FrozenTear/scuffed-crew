use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateArticleRequest {
    pub title: String,
    pub slug: String,
    pub content_markdown: String,
    pub summary: Option<String>,
    pub cover_image_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateArticleRequest {
    pub title: Option<String>,
    pub slug: Option<String>,
    pub content_markdown: Option<String>,
    pub summary: Option<String>,
    pub cover_image_url: Option<String>,
}
