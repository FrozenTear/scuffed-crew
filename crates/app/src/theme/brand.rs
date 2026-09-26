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
    /// Text on a solid dark-mode accent fill (derived for contrast).
    pub accent_fg_dark: String,
    /// Text on a solid light-mode accent fill (derived for contrast).
    pub accent_fg_light: String,
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
            accent_fg_dark: accent_fg(&dark).into(),
            accent_fg_light: accent_fg(&light).into(),
            accent_dark: dark,
            accent_light: light,
        }
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

/// Active brand when settings are not loaded yet.
pub fn current() -> BrandConfig {
    BrandConfig::product_default()
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

/// Dark ink for accent fills too light for white text.
pub const ACCENT_FG_INK: &str = "#17171d";

/// Foreground for text on a solid accent fill. White stays the brand pairing
/// while it clears the 3:1 UI/large-text floor (product purple is 3.46:1).
/// Lighter org accents (e.g. mint `#46d8a4`, 1.81:1) switch to dark ink.
fn accent_fg(hex: &str) -> &'static str {
    let Some((r, g, b)) = parse_rgb(hex) else {
        return "#ffffff";
    };
    let lin = |c: u8| {
        let c = f64::from(c) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let lum = 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
    let white_ratio = 1.05 / (lum + 0.05);
    if white_ratio >= 3.0 {
        "#ffffff"
    } else {
        ACCENT_FG_INK
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
    fn from_accents_derives_soft() {
        let b = BrandConfig::from_accents("#ff0000", "#cc0000");
        assert_eq!(b.accent_dark, "#ff0000");
        assert!(b.accent_soft_dark.contains("255,0,0"));
    }

    #[test]
    fn accent_fg_keeps_white_on_product_purple() {
        let b = BrandConfig::product_default();
        assert_eq!(b.accent_fg_dark, "#ffffff");
        assert_eq!(b.accent_fg_light, "#ffffff");
    }

    #[test]
    fn accent_fg_switches_to_ink_on_light_accents() {
        // Live org mint: white would be 1.81:1.
        let b = BrandConfig::from_settings("#46d8a4", "#0ea66e");
        assert_eq!(b.accent_fg_dark, ACCENT_FG_INK);
        assert_eq!(b.accent_fg_light, "#ffffff");
        let y = BrandConfig::from_accents("#fbbf24", "#fbbf24");
        assert_eq!(y.accent_fg_dark, ACCENT_FG_INK);
    }
}
