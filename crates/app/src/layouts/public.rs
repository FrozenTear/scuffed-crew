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
        format!(":root{{{decls}}}")
    }
}

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
