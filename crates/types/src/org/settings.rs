use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{
    HomeShell, HomeSkin, HomepageContent, NavConfig, PublicLayout,
};

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
    /// Homepage composition shell (canonical).
    #[serde(default)]
    pub home_shell: HomeShell,
    /// Homepage visual skin (canonical).
    #[serde(default)]
    pub home_skin: HomeSkin,
    /// Dual-write mirror of shell for one-release Hub/Landing compatibility.
    #[serde(default)]
    pub public_layout: PublicLayout,
    /// Editable homepage copy (with defaults applied server-side).
    #[serde(default)]
    pub homepage: HomepageContent,
    /// Public navbar: primary / more / hidden catalog items.
    #[serde(default)]
    pub nav: NavConfig,
    /// Optional solid page background (`#rgb` / `#rrggbb` / `#rrggbbaa`). Empty = theme default.
    #[serde(default)]
    pub page_bg_color: String,
    /// Optional full-page background image URL (https or site-relative `/…`). Empty = none.
    #[serde(default)]
    pub page_bg_image_url: String,
    /// Brand accent (dark theme), `#rrggbb`. Empty = product default.
    #[serde(default)]
    pub brand_accent_dark: String,
    /// Brand accent (light theme). Empty = same as dark or product default.
    #[serde(default)]
    pub brand_accent_light: String,
    pub updated_at: DateTime<Utc>,
}
