use serde::{Deserialize, Serialize};

use crate::org::{HomeShell, HomeSkin, HomepageContent, NavConfig, PublicLayout};

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
    /// Preferred: homepage composition shell.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub home_shell: Option<HomeShell>,
    /// Preferred: homepage visual skin.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub home_skin: Option<HomeSkin>,
    /// Deprecated dual-write mirror; if only this is sent, maps Hub→ops_hub, Landing→recruit_landing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_layout: Option<PublicLayout>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub homepage: Option<HomepageContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nav: Option<NavConfig>,
    /// Solid page background color. Empty string clears to theme default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_bg_color: Option<String>,
    /// Background image URL. Empty string clears.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_bg_image_url: Option<String>,
    /// Brand accent dark (`#rrggbb`). Empty string = product default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brand_accent_dark: Option<String>,
    /// Brand accent light. Empty string = product default / match dark.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brand_accent_light: Option<String>,
}
