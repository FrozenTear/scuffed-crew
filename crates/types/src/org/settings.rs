use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{HomeShell, HomeSkin, HomepageContent, NavConfig, PublicLayout};

fn default_true() -> bool {
    true
}

fn default_false() -> bool {
    false
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiteSettings {
    pub id: String,
    pub org_name: String,
    pub site_description: String,
    pub recruitment_open: bool,
    /// When false, strategy CRUD/helpers are absent (404). Patch Notes stay public.
    /// Missing / empty JSON defaults true for existing orgs.
    #[serde(default = "default_true")]
    pub strategies_enabled: bool,
    /// When false (default), only admins may update team records.
    /// Missing JSON defaults false so older clients and rows stay admin-only.
    #[serde(default = "default_false")]
    pub officers_can_edit_teams: bool,
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn missing_strategies_enabled_defaults_true() {
        let settings: SiteSettings = serde_json::from_value(json!({
            "id": "site",
            "org_name": "Clan",
            "site_description": "desc",
            "recruitment_open": true,
            "recruitment_message": "msg",
            "min_age": 16,
            "forum_backend": "local",
            "extra_relay_urls": "",
            "updated_at": "2026-09-21T00:00:00Z"
        }))
        .expect("legacy settings JSON without strategies_enabled");
        assert!(settings.strategies_enabled);
    }

    #[test]
    fn strategies_enabled_false_round_trips() {
        let settings: SiteSettings = serde_json::from_value(json!({
            "id": "site",
            "org_name": "Clan",
            "site_description": "desc",
            "recruitment_open": true,
            "strategies_enabled": false,
            "recruitment_message": "msg",
            "min_age": 16,
            "forum_backend": "local",
            "extra_relay_urls": "",
            "updated_at": "2026-09-21T00:00:00Z"
        }))
        .expect("settings JSON with strategies_enabled=false");
        assert!(!settings.strategies_enabled);
        let v = serde_json::to_value(&settings).expect("serialize");
        assert_eq!(v["strategies_enabled"], false);
    }

    #[test]
    fn missing_officers_can_edit_teams_defaults_false() {
        let settings: SiteSettings = serde_json::from_value(json!({
            "id": "site",
            "org_name": "Clan",
            "site_description": "desc",
            "recruitment_open": true,
            "recruitment_message": "msg",
            "min_age": 16,
            "forum_backend": "local",
            "extra_relay_urls": "",
            "updated_at": "2026-09-21T00:00:00Z"
        }))
        .expect("legacy settings JSON without officers_can_edit_teams");
        assert!(!settings.officers_can_edit_teams);
    }

    #[test]
    fn officers_can_edit_teams_true_round_trips() {
        let settings: SiteSettings = serde_json::from_value(json!({
            "id": "site",
            "org_name": "Clan",
            "site_description": "desc",
            "recruitment_open": true,
            "officers_can_edit_teams": true,
            "recruitment_message": "msg",
            "min_age": 16,
            "forum_backend": "local",
            "extra_relay_urls": "",
            "updated_at": "2026-09-21T00:00:00Z"
        }))
        .expect("settings JSON with officers_can_edit_teams=true");
        assert!(settings.officers_can_edit_teams);
        let v = serde_json::to_value(&settings).expect("serialize");
        assert_eq!(v["officers_can_edit_teams"], true);
    }
}
