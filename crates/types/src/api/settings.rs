use serde::{Deserialize, Serialize};

use crate::org::{HomepageContent, PublicLayout};

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_relay_urls: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_layout: Option<PublicLayout>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub homepage: Option<HomepageContent>,
    /// Solid page background color. Empty string clears to theme default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_bg_color: Option<String>,
    /// Background image URL. Empty string clears.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_bg_image_url: Option<String>,
}
