//! Brand-configurable accent colors.
//!
//! Product default is a neutral purple. Live sites override via SiteSettings
//! (`brand_accent_dark` / `brand_accent_light`). Homepage presets may suggest
//! accents when applied in admin.

#[derive(Clone, PartialEq, Eq)]
pub struct BrandConfig {
    /// Accent in dark mode (hex).
    pub accent_dark: String,
    /// Accent in light mode (hex).
    pub accent_light: String,
    /// Soft accent fill, dark mode (rgba).
    pub accent_soft_dark: String,
    /// Soft accent fill, light mode (rgba).
    pub accent_soft_light: String,
}

impl BrandConfig {
    /// Product default accents (not clan-specific).
    pub fn product_default() -> Self {
        Self::from_accents("#8f73ff", "#6d4aff")
    }

    /// Build from primary dark/light hex accents; soft fills are derived.
    pub fn from_accents(accent_dark: &str, accent_light: &str) -> Self {
        let dark = normalize_hex(accent_dark).unwrap_or_else(|| "#8f73ff".into());
        let light = normalize_hex(accent_light).unwrap_or_else(|| dark.clone());
        Self {
            accent_soft_dark: soft_rgba(&dark, 0.17),
            accent_soft_light: soft_rgba(&light, 0.10),
            accent_dark: dark,
            accent_light: light,
        }
    }

    /// Neutral accent while public settings are still unknown.
    ///
    /// The product purple is a real brand. Painting it before settings arrive
    /// flashes the wrong accent on orgs that chose something else. This gray
    /// stays at least 3:1 against both theme backgrounds and against the dark
    /// `--border` used by a focused field. The boot mark in `index.html`,
    /// `assets/favicon.svg`, and [`org_favicon_data_uri`] use the same hex.
    pub fn pending() -> Self {
        Self::from_accents("#808088", "#808088")
    }

    /// Resolve settings fields: empty → product default.
    pub fn from_settings(accent_dark: &str, accent_light: &str) -> Self {
        let d = accent_dark.trim();
        let l = accent_light.trim();
        if d.is_empty() && l.is_empty() {
            return Self::product_default();
        }
        let dark = if d.is_empty() { "#8f73ff" } else { d };
        let light = if l.is_empty() { dark } else { l };
        Self::from_accents(dark, light)
    }
}

/// Product-default purple for callers that are not the public boot path.
///
/// Unloaded public settings use [`BrandConfig::pending`], not this. This is
/// the installed accent, not a placeholder.
pub fn current() -> BrandConfig {
    BrandConfig::product_default()
}

/// SVG favicon for one org. Letters come from the org name; the fill is the
/// pending gray so every clan does not share the same mark color and initials.
pub fn org_favicon_data_uri(initials: &str) -> String {
    let letters: String = initials
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(2)
        .collect();
    let svg = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 32 32'><rect width='32' height='32' rx='6' fill='%23808088'/><text x='16' y='22' text-anchor='middle' font-family='system-ui,sans-serif' font-size='14' font-weight='700' fill='%23ffffff'>{letters}</text></svg>"
    );
    format!("data:image/svg+xml,{svg}")
}

/// `None` leaves the shell's static icon (settings still unknown).
/// A non-empty org name always gets a data-URI mark, even when the initials
/// are `CL`. A settled blank name keeps the neutral asset.
pub fn runtime_favicon_href(settled_org_name: Option<&str>) -> Option<String> {
    match settled_org_name
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        None if settled_org_name.is_none() => None,
        None => Some("/assets/favicon.svg".to_string()),
        Some(name) => Some(org_favicon_data_uri(&scuffed_types::org_initials(name))),
    }
}

/// Accept `#rgb` / `#rrggbb` / bare hex → lowercase `#rrggbb`.
fn normalize_hex(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    let hex = s.strip_prefix('#').unwrap_or(s);
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    match hex.len() {
        3 => {
            let mut out = String::from("#");
            for c in hex.chars() {
                out.push(c);
                out.push(c);
            }
            Some(out.to_ascii_lowercase())
        }
        6 => Some(format!("#{}", hex.to_ascii_lowercase())),
        _ => None,
    }
}

fn soft_rgba(hex: &str, alpha: f32) -> String {
    let (r, g, b) = parse_rgb(hex).unwrap_or((143, 115, 255));
    format!("rgba({r},{g},{b},{alpha})")
}

fn parse_rgb(hex: &str) -> Option<(u8, u8, u8)> {
    let h = hex.strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&h[0..2], 16).ok()?;
    let g = u8::from_str_radix(&h[2..4], 16).ok()?;
    let b = u8::from_str_radix(&h[4..6], 16).ok()?;
    Some((r, g, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_settings_empty_uses_product_default() {
        let b = BrandConfig::from_settings("", "");
        assert_eq!(b.accent_dark, "#8f73ff");
    }

    #[test]
    fn pending_accent_is_not_the_product_purple() {
        let pending = BrandConfig::pending();
        let product = BrandConfig::product_default();
        assert_ne!(pending.accent_dark, product.accent_dark);
        assert_ne!(pending.accent_light, product.accent_light);
        assert_ne!(pending.accent_dark, "#8f73ff");
        assert_ne!(pending.accent_light, "#6d4aff");
        assert_eq!(pending.accent_dark, pending.accent_light);
    }

    #[test]
    fn pending_accent_meets_ui_contrast_on_both_themes() {
        use crate::theme::tokens::{contrast_ratio, scope_decls};
        let pending = BrandConfig::pending();
        let css = crate::theme::theme_css(&pending);
        let dark = scope_decls(&css, "[data-theme=\"dark\"]");
        let light = scope_decls(&css, "[data-theme=\"light\"]");
        let accent = pending.accent_dark.as_str();
        for (scope, name) in [
            (&dark, "--bg"),
            (&dark, "--surface"),
            (&dark, "--surface-2"),
            (&dark, "--border"),
            (&light, "--bg"),
            (&light, "--surface"),
            (&light, "--surface-2"),
        ] {
            let bg = scope.get(name).unwrap_or_else(|| panic!("missing {name}"));
            let ratio = contrast_ratio(accent, bg);
            assert!(ratio >= 3.0, "{accent} on {name} {bg} = {ratio:.2}");
        }
        let white_on_pending = contrast_ratio("#ffffff", accent);
        assert!(
            white_on_pending >= 3.0,
            "white on {accent} = {white_on_pending:.2}"
        );
        let boot = include_str!("../../index.html");
        assert!(
            boot.contains(accent),
            "boot mark must use the same gray as BrandConfig::pending"
        );
    }

    #[test]
    fn favicon_assets_use_the_pending_gray_including_cl_initials() {
        let pending = BrandConfig::pending();
        let svg = include_str!("../../assets/favicon.svg");
        assert!(
            svg.contains(pending.accent_dark.as_str()),
            "favicon.svg fill must match BrandConfig::pending"
        );
        let encoded = pending.accent_dark.replacen('#', "%23", 1);
        let marked = org_favicon_data_uri("CL");
        assert!(marked.contains(&encoded), "{marked}");
        assert!(runtime_favicon_href(None).is_none());
        assert_eq!(
            runtime_favicon_href(Some("")),
            Some("/assets/favicon.svg".to_string())
        );
        assert_eq!(
            runtime_favicon_href(Some("   ")),
            Some("/assets/favicon.svg".to_string())
        );
        let clan = runtime_favicon_href(Some("Clan League")).expect("mark");
        assert!(clan.starts_with("data:image/svg+xml,"), "{clan}");
        assert!(clan.contains(&encoded), "{clan}");
        assert!(clan.contains("CL"), "{clan}");
        let bangs = runtime_favicon_href(Some("!!!")).expect("punctuation still has a name");
        assert!(bangs.starts_with("data:image/svg+xml,"), "{bangs}");
    }

    #[test]
    fn from_accents_derives_soft() {
        let b = BrandConfig::from_accents("#ff0000", "#cc0000");
        assert_eq!(b.accent_dark, "#ff0000");
        assert!(b.accent_soft_dark.contains("255,0,0"));
    }
}
