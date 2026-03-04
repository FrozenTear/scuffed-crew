use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiteSettings {
    pub id: String,
    pub org_name: String,
    pub site_description: String,
    pub recruitment_open: bool,
    pub recruitment_message: String,
    pub min_age: u32,
    pub updated_at: DateTime<Utc>,
}
