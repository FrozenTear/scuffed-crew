//! Brand-configurable accent colors.
//!
//! Product default is a neutral purple. Live sites override via SiteSettings
//! (`brand_accent_dark` / `brand_accent_light`). Homepage presets may suggest
//! accents when applied in admin.
//!
//! The derivation and the contrast lock live in [`scuffed_types::brand`] so
//! the site server enforces the same lock on `PUT /api/settings`. This module
//! re-exports them and keeps the app-only favicon helpers.

pub use scuffed_types::brand::*;

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

/// An accent the contrast lock blocks: in dark neither white nor the dark
/// ink reaches 4.5:1 on it.
#[cfg(test)]
pub const TEST_BLOCKED_ACCENT: &str = "#d35400";
/// Text that is not a hex colour the server accepts, for the hex field.
#[cfg(test)]
pub const TEST_INVALID_ACCENTS: [&str; 4] = ["mint", "#12", "#46d8a4ff", "url(x)"];
/// A short hex in upper case with stray spaces: accepted and normalized.
#[cfg(test)]
pub const TEST_SHORT_ACCENT: &str = " #ABC ";
/// The live mint brand, for tests that need a passing accent.
#[cfg(test)]
pub const TEST_MINT_ACCENT: &str = MINT_DARK;
/// A second passing accent that is not the mint and not in any token file.
#[cfg(test)]
pub const TEST_OTHER_ACCENT: &str = "#ff5fa2";
/// An accent as dark as the dark page: every text pair passes, but a
/// primary button in it has no edge, so the fill pair blocks it.
#[cfg(test)]
pub const TEST_EDGELESS_ACCENT: &str = "#000000";
/// Accents for contrast sweeps: the extremes, the page colours, and the
/// accents QA 694 measured (pale yellow, sky, lilac, mint, the defaults).
#[cfg(test)]
pub const TEST_SWEEP_ACCENTS: [&str; 11] = [
    "#000000", "#ffffff", "#ffe680", "#46d8a4", "#38bdf8", "#c084fc", "#8f73ff", "#6d4aff",
    "#ff0000", "#17171d", "#f7f7f9",
];

#[cfg(test)]
mod tests {
    use super::*;

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
        // `--border` is the field outline a focused input sits next to.
        let dark = DARK_SURFACES.iter().copied().chain(["#353541"]);
        let light = LIGHT_SURFACES.iter().copied().chain(["#e3e3e9"]);
        for (roles, surfaces) in [
            (&pending.dark, dark.collect::<Vec<_>>()),
            (&pending.light, light.collect::<Vec<_>>()),
        ] {
            for bg in &surfaces {
                for fg in [&roles.accent, &roles.solid] {
                    let ratio = contrast_ratio(fg, bg);
                    assert!(ratio >= 3.0, "{fg} on {bg} = {ratio:.2}");
                }
            }
            // `--border` is not a text surface; text clears AA on the other four.
            for bg in &surfaces[..4] {
                let ratio = contrast_ratio(&roles.text, bg);
                assert!(ratio >= 4.5, "text {} on {bg} = {ratio:.2}", roles.text);
            }
        }
        // The favicon draws white initials on the raw pending gray.
        let accent = pending.accent_dark.as_str();
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
}
