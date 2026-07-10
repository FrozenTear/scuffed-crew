use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{HomepageContent, PublicLayout};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiteSettings {
    pub id: String,
    pub org_name: String,
    pub site_description: String,
    pub recruitment_open: bool,
    pub recruitment_message: String,
    pub min_age: u32,
    pub forum_backend: String,
    pub extra_relay_urls: String,
    /// Public homepage layout: "hub" | "landing"
    #[serde(default)]
    pub public_layout: PublicLayout,
    /// Editable homepage copy (with defaults applied server-side).
    #[serde(default)]
    pub homepage: HomepageContent,
    pub updated_at: DateTime<Utc>,
}
