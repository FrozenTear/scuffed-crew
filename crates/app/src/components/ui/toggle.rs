//! Toggle buttons that tell assistive tech their state.
//!
//! `TabButton` is a tab in a tablist (`aria-selected`). `ChipButton` is a
//! filter chip (`aria-pressed`). Both keep the page's own look: the page passes
//! its base class (`stats-tab`, `filter-chip`) and gets ` active` added when on,
//! exactly like the hand written buttons they replace.

use dioxus::prelude::*;

/// `base`, plus ` active` when `on`.
pub fn toggle_class(base: &str, on: bool) -> String {
    if on {
        format!("{base} active")
    } else {
        base.to_string()
    }
}

/// One tab. Put these inside an element with `role="tablist"`. `id` names
/// this tab, `controls` is the id of the `role="tabpanel"` element it shows.
#[component]
pub fn TabButton(
    class: String,
    label: String,
    id: String,
    controls: String,
    selected: bool,
    onclick: EventHandler<()>,
) -> Element {
    let class = toggle_class(&class, selected);
    rsx! {
        button {
            r#type: "button",
            role: "tab",
            id: "{id}",
            aria_controls: "{controls}",
            aria_selected: if selected { "true" } else { "false" },
            class: "{class}",
            onclick: move |_| onclick.call(()),
            "{label}"
        }
    }
}

/// One filter chip that is either on or off.
#[component]
pub fn ChipButton(
    class: String,
    label: String,
    pressed: bool,
    onclick: EventHandler<()>,
) -> Element {
    let class = toggle_class(&class, pressed);
    rsx! {
        button {
            r#type: "button",
            aria_pressed: if pressed { "true" } else { "false" },
            class: "{class}",
            onclick: move |_| onclick.call(()),
            "{label}"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(root: fn() -> Element) -> String {
        let mut dom = VirtualDom::new(root);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    #[test]
    fn active_class_is_added_only_when_on() {
        assert_eq!(toggle_class("stats-tab", true), "stats-tab active");
        assert_eq!(toggle_class("stats-tab", false), "stats-tab");
    }

    #[test]
    fn tabs_expose_selected_state() {
        fn app() -> Element {
            rsx! {
                div { role: "tablist",
                    TabButton {
                        class: "stats-tab".to_string(),
                        label: "Overview".to_string(),
                        id: "t-overview".to_string(),
                        controls: "t-panel".to_string(),
                        selected: true,
                        onclick: |_| {},
                    }
                    TabButton {
                        class: "stats-tab".to_string(),
                        label: "Heroes".to_string(),
                        id: "t-heroes".to_string(),
                        controls: "t-panel".to_string(),
                        selected: false,
                        onclick: |_| {},
                    }
                }
            }
        }
        let html = render(app);
        assert_eq!(html.matches("role=\"tab\"").count(), 2, "{html}");
        assert_eq!(html.matches("aria-selected=\"true\"").count(), 1, "{html}");
        assert_eq!(html.matches("aria-selected=\"false\"").count(), 1, "{html}");
        assert!(html.contains("class=\"stats-tab active\""), "{html}");
        assert_eq!(
            html.matches("aria-controls=\"t-panel\"").count(),
            2,
            "{html}"
        );
        assert!(html.contains("id=\"t-overview\""), "{html}");
        assert!(html.contains("id=\"t-heroes\""), "{html}");
    }

    #[test]
    fn chips_expose_pressed_state() {
        fn app() -> Element {
            rsx! {
                ChipButton {
                    class: "filter-chip".to_string(),
                    label: "Win".to_string(),
                    pressed: true,
                    onclick: |_| {},
                }
                ChipButton {
                    class: "filter-chip".to_string(),
                    label: "Loss".to_string(),
                    pressed: false,
                    onclick: |_| {},
                }
            }
        }
        let html = render(app);
        assert!(html.contains("aria-pressed=\"true\""), "{html}");
        assert!(html.contains("aria-pressed=\"false\""), "{html}");
        assert!(html.contains("class=\"filter-chip active\""), "{html}");
        assert!(!html.contains("role=\"tab\""), "{html}");
    }
}
