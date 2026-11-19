use serde::{Deserialize, Serialize};

use crate::org::{HomeShell, HomeSkin, HomepageContent, NavConfig, PublicLayout};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateSettingsRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub org_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub site_description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recruitment_open: Option<bool>,
    /// When false, strategy CRUD/helpers 404. Patch Notes stay public.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strategies_enabled: Option<bool>,
    /// Admin-only. When true, officers may `PUT /api/teams/:id`. Default off.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub officers_can_edit_teams: Option<bool>,
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
    /// Display font key: `""` (default, Urbanist), `"urbanist"` or `"ibm-plex-sans"`.
    /// Absent keeps the stored value. See `BRAND_FONT_DISPLAY_ALLOWED`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brand_font_display: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brand_font_display_absent_is_none_and_not_serialized() {
        let req: UpdateSettingsRequest = serde_json::from_str("{}").expect("empty body");
        assert_eq!(req.brand_font_display, None);
        let v = serde_json::to_value(&req).expect("serialize");
        assert!(v.get("brand_font_display").is_none());
    }

    #[test]
    fn brand_font_display_empty_string_is_some_and_serialized() {
        let req: UpdateSettingsRequest =
            serde_json::from_str(r#"{"brand_font_display":""}"#).expect("body");
        assert_eq!(req.brand_font_display.as_deref(), Some(""));
        let v = serde_json::to_value(&req).expect("serialize");
        assert_eq!(v["brand_font_display"], "");
    }
}
