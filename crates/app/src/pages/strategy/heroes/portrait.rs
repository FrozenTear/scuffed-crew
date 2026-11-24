//! Hero portrait with a fallback.
//!
//! The shared hero list sends an empty `portrait_url` today, and a URL that
//! is set can still 404. Either way the portrait becomes the hero's initial
//! on a role tinted tile, so a missing image never shows a broken icon.

use dioxus::prelude::*;

use super::model::RoleGroup;

/// `url` is `None` when the list has no portrait. `class` is the tile class
/// (for example `hero-btn-portrait`) and `initial_class` styles the initial.
#[component]
pub(super) fn HeroPortrait(
    url: Option<String>,
    name: String,
    initial: String,
    group: RoleGroup,
    class: &'static str,
    initial_class: &'static str,
) -> Element {
    // The URL that failed to load. Keyed by URL so picking another hero with
    // a different portrait tries again.
    let mut failed_url: Signal<Option<String>> = use_signal(|| None);
    let shown_url = url.filter(|url| failed_url.read().as_deref() != Some(url.as_str()));
    let on_error_url = shown_url.clone();
    let tile_style = if shown_url.is_some() {
        String::new()
    } else {
        format!("background: {}; color: {};", group.tint(), group.color())
    };

    rsx! {
        div { class: "{class}", style: "{tile_style}",
            if let Some(src) = &shown_url {
                img {
                    src: "{src}",
                    alt: "{name}",
                    loading: "lazy",
                    onerror: move |_| failed_url.set(on_error_url.clone()),
                }
            } else {
                span { class: "{initial_class}", aria_hidden: "true", "{initial}" }
            }
        }
    }
}
