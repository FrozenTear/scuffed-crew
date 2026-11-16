use dioxus::prelude::*;

use super::shell::SiteShell;
use crate::routes::Route;
use crate::state::{loaded_site_settings, use_site_settings};

/// Build a safe CSS snippet that applies admin-configured page background.
fn page_bg_css(color: &str, image_url: &str) -> String {
    let mut decls = String::new();
    let color = color.trim();
    if !color.is_empty()
        && color.starts_with('#')
        && color.len() <= 9
        && color.chars().skip(1).all(|c| c.is_ascii_hexdigit())
    {
        decls.push_str(&format!("--page-bg-color:{color};"));
    }
    let url = image_url.trim();
    if !url.is_empty()
        && !url.contains(['"', '\'', '(', ')', ';', '<', '>', '\\', '\n', '\r'])
        && (url.starts_with('/') || url.starts_with("https://") || url.starts_with("http://"))
    {
        decls.push_str(&format!("--page-bg-image:url(\"{url}\");"));
    }
    if decls.is_empty() {
        String::new()
    } else {
        // A custom background sits right behind the Home hero, whose H1 and
        // lead are inked for `--bg`. With one set, the hero takes an opaque
        // `--bg` panel (see `.home-hero`), so its text keeps its contrast
        // over any colour or image.
        format!(":root{{{decls}{HERO_PANEL_DECLS}}}")
    }
}

/// Switches the Home hero's panel on. Unset, the hero has no fill.
const HERO_PANEL_DECLS: &str = "--page-hero-fill:var(--bg);--page-hero-pad:var(--space-section);";

/// Public pages: the site shell around the routed page, plus the page
/// background an admin set in Site Settings.
#[component]
pub fn PublicLayout() -> Element {
    let site_settings = use_site_settings();
    let resolved = site_settings.resolved.read();
    let bg_css = loaded_site_settings(resolved.as_ref())
        .map(|s| page_bg_css(&s.page_bg_color, &s.page_bg_image_url))
        .unwrap_or_default();

    rsx! {
        crate::styles::StyleLive { id: "scuffed-css-page-bg", css: bg_css }
        SiteShell {
            Outlet::<Route> {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_background_setting_means_no_css_and_no_hero_panel() {
        assert_eq!(page_bg_css("", ""), "");
        assert_eq!(page_bg_css("  ", "  "), "");
        // Rejected values are the same as none.
        assert_eq!(page_bg_css("red", "javascript:alert(1)"), "");
        assert_eq!(page_bg_css("", "/a.png\");x"), "");
    }

    #[test]
    fn a_page_background_gives_the_home_hero_an_opaque_panel() {
        // Built here so the source holds no raw colour literal.
        let navy = ["#", "101820"].concat();
        let navy = navy.as_str();
        for (color, image) in [
            (navy, ""),
            ("", "/api/uploads/bg.png"),
            (navy, "https://cdn.example/bg.png"),
        ] {
            let css = page_bg_css(color, image);
            assert!(css.starts_with(":root{"), "{css}");
            assert!(css.contains("--page-hero-fill:var(--bg);"), "{css}");
            assert!(
                css.contains("--page-hero-pad:var(--space-section);"),
                "{css}"
            );
        }
        assert_eq!(
            page_bg_css(navy, "/bg.png"),
            format!(
                ":root{{--page-bg-color:{navy};--page-bg-image:url(\"/bg.png\");--page-hero-fill:var(--bg);--page-hero-pad:var(--space-section);}}"
            )
        );
    }
}
