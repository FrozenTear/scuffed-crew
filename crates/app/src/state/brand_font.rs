//! The saved display font, as the chrome paints it.
//!
//! API gap: `SiteSettings` has no display font field yet, and the server
//! neither stores nor returns one. The client reads the optional
//! `brand_font_display` key straight from the settings JSON, so the font
//! picker switches on by itself once `GET /api/settings` carries that key.
//! Until then [`BrandFontState::font`] is the default and nothing is sent.
//!
//! That holds only while `brand_font_display` stays out of the shared
//! `SiteSettings` type. Brand settings flattens `SiteSettings` and
//! [`BrandFontField`] into one view, and serde gives a flattened key to the
//! first struct that declares it. Adding the field to `SiteSettings` without
//! dropping [`BrandFontField`] from that view leaves the picker locked.

use dioxus::prelude::*;
use serde::Deserialize;

use crate::theme::fonts::DisplayFont;

/// The settings key the client expects for the display font. Its value is a
/// [`DisplayFont::id`]; an empty string means the default.
pub const BRAND_FONT_FIELD: &str = "brand_font_display";

/// The one optional key, read next to `SiteSettings`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct BrandFontField {
    #[serde(default)]
    pub brand_font_display: Option<String>,
}

impl BrandFontField {
    /// `None` when the server did not send the key (no support yet).
    pub fn font(&self) -> Option<DisplayFont> {
        self.brand_font_display
            .as_deref()
            .map(|id| DisplayFont::from_setting(Some(id)))
    }
}

/// The display font in a settings JSON document, when it has the key.
pub fn parse_brand_font(json: Option<&str>) -> Option<DisplayFont> {
    serde_json::from_str::<BrandFontField>(json?.trim())
        .ok()?
        .font()
}

#[derive(Clone, Copy)]
pub struct BrandFontState {
    saved: Signal<Option<DisplayFont>>,
}

impl BrandFontState {
    /// The font the whole site uses.
    pub fn font(self) -> DisplayFont {
        (self.saved)().unwrap_or_default()
    }

    /// Follow a settings response. `None` (key absent) keeps the default.
    pub fn apply_saved(mut self, font: Option<DisplayFont>) {
        self.saved.set(font);
    }
}

/// Call once from the root `App` component. Seeds from the embedded
/// settings JSON; brand settings updates it after a load or a save.
pub fn provide_brand_font() -> BrandFontState {
    let saved =
        use_signal(|| parse_brand_font(super::site_settings::embedded_settings_json().as_deref()));
    let state = BrandFontState { saved };
    use_context_provider(|| state);
    state
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_key_means_no_server_support() {
        assert_eq!(parse_brand_font(None), None);
        assert_eq!(parse_brand_font(Some("")), None);
        assert_eq!(parse_brand_font(Some("{")), None);
        assert_eq!(parse_brand_font(Some(r#"{"org_name":"Clan"}"#)), None);
    }

    #[test]
    fn a_present_key_is_read_as_a_listed_font() {
        let json = format!(r#"{{"{BRAND_FONT_FIELD}":"ibm-plex-sans"}}"#);
        assert_eq!(
            parse_brand_font(Some(&json)),
            Some(DisplayFont::IbmPlexSans)
        );
        // Empty and unknown values are the default, never a raw family name.
        for value in ["", "Comic Sans MS", "https://fonts.example/x.woff2"] {
            let json = format!(r#"{{"{BRAND_FONT_FIELD}":"{value}"}}"#);
            assert_eq!(parse_brand_font(Some(&json)), Some(DisplayFont::Urbanist));
        }
    }
}
