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
    /// stays at least 3:1 against both theme backgrounds. The boot mark in
    /// `index.html` uses the same hex.
    pub fn pending() -> Self {
        Self::from_accents("#7a7a88", "#7a7a88")
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
        "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 32 32'><rect width='32' height='32' rx='6' fill='%237a7a88'/><text x='16' y='22' text-anchor='middle' font-family='system-ui,sans-serif' font-size='14' font-weight='700' fill='%23ffffff'>{letters}</text></svg>"
    );
    format!("data:image/svg+xml,{svg}")
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
        let pending = BrandConfig::pending();
        let on_dark = contrast_ratio(&pending.accent_dark, crate::theme::tokens::BG_DARK);
        let on_light = contrast_ratio(&pending.accent_light, "#f7f7f9");
        assert!(
            on_dark >= 3.0,
            "{} on dark = {on_dark:.2}",
            pending.accent_dark
        );
        assert!(
            on_light >= 3.0,
            "{} on light = {on_light:.2}",
            pending.accent_light
        );
        let boot = include_str!("../../index.html");
        assert!(
            boot.contains(&pending.accent_dark),
            "boot mark must use the same gray as BrandConfig::pending"
        );
        let white_on_pending = contrast_ratio("#ffffff", &pending.accent_dark);
        assert!(
            white_on_pending >= 3.0,
            "white on {} = {white_on_pending:.2}",
            pending.accent_dark
        );
    }

    fn contrast_ratio(fg: &str, bg: &str) -> f64 {
        let l1 = relative_luminance(fg);
        let l2 = relative_luminance(bg);
        let (hi, lo) = if l1 > l2 { (l1, l2) } else { (l2, l1) };
        (hi + 0.05) / (lo + 0.05)
    }

    fn relative_luminance(hex: &str) -> f64 {
        let hex = hex.trim().trim_start_matches('#');
        let n = u32::from_str_radix(hex, 16).unwrap();
        let lin = |c: u8| {
            let c = f64::from(c) / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        let r = lin(((n >> 16) & 0xff) as u8);
        let g = lin(((n >> 8) & 0xff) as u8);
        let b = lin((n & 0xff) as u8);
        0.2126 * r + 0.7152 * g + 0.0722 * b
    }

    #[test]
    fn from_accents_derives_soft() {
        let b = BrandConfig::from_accents("#ff0000", "#cc0000");
        assert_eq!(b.accent_dark, "#ff0000");
        assert!(b.accent_soft_dark.contains("255,0,0"));
    }
}
