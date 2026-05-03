use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Article {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub content_markdown: String,
    pub summary: Option<String>,
    pub cover_image_url: Option<String>,
    pub author_member_id: String,
    pub published: bool,
    pub nostr_event_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub published_at: Option<DateTime<Utc>>,
}
