//! Toggle buttons that tell assistive tech their state.
//!
//! `TabButton` is a tab in a tablist (`aria-selected`). `ChipButton` is a
//! filter chip (`aria-pressed`). Both keep the page's own look: the page passes
//! its base class (`stats-tab`, `filter-chip`) and gets ` active` added when on,
//! exactly like the hand written buttons they replace.
//!
//! The shared look is the base class `chip` ([`CHIP_CSS`], the `--chip-*`
//! tokens): pass `class: "chip"` to either button and put them in a
//! [`ChipRow`].

use dioxus::prelude::*;

/// The redesign chip. 44px tall at every width. The selected state is a soft
/// fill plus a selected border and a heavier weight, so it is not colour alone.
/// It keys on the state the buttons already expose (`aria-pressed`,
/// `aria-selected`) and on the ` active` class.
pub const CHIP_CSS: &str = r#"
.chip-row {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2);
  min-width: 0;
}
.chip {
  box-sizing: border-box;
  height: var(--chip-height);
  min-height: var(--hit-target-min);
  min-width: var(--hit-target-min);
  margin: 0;
  padding: 0 var(--chip-pad-x);
  border: 1px solid var(--chip-border);
  border-radius: var(--chip-radius);
  background: var(--chip-bg);
  color: var(--chip-fg);
  font-family: var(--chip-font);
  font-size: var(--chip-font-size);
  font-weight: var(--chip-font-weight);
  line-height: var(--leading-ui);
  white-space: nowrap;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: var(--space-1);
  cursor: pointer;
  text-box-trim: trim-both;
  text-box-edge: cap alphabetic;
}
.chip:hover { border-color: var(--border-strong); }
.chip.active,
.chip[aria-pressed="true"],
.chip[aria-selected="true"] {
  background: var(--chip-selected-bg);
  color: var(--chip-selected-fg);
  border-color: var(--chip-selected-border);
  border-width: var(--chip-selected-border-width);
  border-style: solid;
  font-weight: var(--chip-selected-weight);
}
/* A save is running: the chip stays focusable but does nothing. */
.chip[aria-disabled="true"] { cursor: progress; }
/* A chip with a shorter phone label shows one of the two. */
.chip-short { display: none; }
@supports not (text-box-trim: trim-both) {
  .chip { padding-top: 0.08em; }
}
/* Phone: a row that would wrap scrolls sideways instead. The page does not
   scroll. The focus ring is drawn inside the chip, because a scroller clips
   an outer ring. */
@media (max-width: 820px) {
  .chip-row.is-scroll {
    flex-wrap: nowrap;
    overflow-x: auto;
    scrollbar-width: thin;
    scrollbar-color: var(--border-strong) transparent;
    width: 100%;
    max-width: 100%;
    min-width: 0;
  }
  .chip-row.is-scroll .chip {
    flex: 0 0 auto;
  }
  .chip-row.is-scroll .chip:focus-visible {
    outline-offset: calc(-1 * var(--focus-ring-width));
  }
  .chip-full { display: none; }
  .chip-short { display: inline; }
}
"#;

/// A labelled group of chips. `scroll: true` makes the row scroll sideways on
/// phone instead of wrapping (patch note filters, application status).
#[component]
pub fn ChipRow(
    /// Accessible name of the group ("Filter by role").
    label: String,
    #[props(default = false)] scroll: bool,
    children: Element,
) -> Element {
    rsx! {
        div {
            class: if scroll { "chip-row is-scroll" } else { "chip-row" },
            role: "group",
            aria_label: "{label}",
            {children}
        }
    }
}

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
///
/// `id` comes from the page's data (record id plus field name). `short_label`
/// is a shorter text for phone width; the accessible name stays `label`.
/// `busy` marks the chip `aria-disabled` while a save runs: it keeps focus and
/// ignores clicks.
#[component]
pub fn ChipButton(
    class: String,
    label: String,
    pressed: bool,
    onclick: EventHandler<()>,
    #[props(default)] id: Option<String>,
    #[props(default)] short_label: Option<String>,
    #[props(default = false)] busy: bool,
) -> Element {
    let class = toggle_class(&class, pressed);
    let name = short_label.is_some().then(|| label.clone());
    rsx! {
        button {
            r#type: "button",
            id,
            aria_pressed: if pressed { "true" } else { "false" },
            aria_disabled: if busy { "true" },
            aria_label: name,
            class: "{class}",
            onclick: move |_| {
                if !busy {
                    onclick.call(());
                }
            },
            if let Some(short) = short_label {
                span { class: "chip-full", "{label}" }
                span { class: "chip-short", "{short}" }
            } else {
                "{label}"
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::ui::test_support::{
        assert_ink_clears_aa, assert_sizes_from_tokens, css_rule, has_decl, render,
    };

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

    #[test]
    fn chip_takes_an_id_a_busy_state_and_a_short_phone_label() {
        fn app() -> Element {
            rsx! {
                ChipButton {
                    class: "chip".to_string(),
                    label: "Going".to_string(),
                    pressed: false,
                    id: "event-e1-rsvp-yes".to_string(),
                    busy: true,
                    onclick: |_| {},
                }
                ChipButton {
                    class: "chip".to_string(),
                    label: "Competitive".to_string(),
                    short_label: "Comp".to_string(),
                    pressed: true,
                    onclick: |_| {},
                }
            }
        }
        let html = render(app);
        // Busy is aria-disabled, never the native attribute, so focus stays.
        assert!(
            html.contains(
                "<button type=\"button\" id=\"event-e1-rsvp-yes\" aria-pressed=\"false\" aria-disabled=\"true\" class=\"chip\">Going</button>"
            ),
            "{html}"
        );
        assert!(!html.contains(" disabled"), "{html}");
        // The short label is what phones show; the name is the full word.
        assert!(
            html.contains(
                "<button type=\"button\" aria-pressed=\"true\" aria-label=\"Competitive\" class=\"chip active\"><span class=\"chip-full\">Competitive</span><span class=\"chip-short\">Comp</span></button>"
            ),
            "{html}"
        );
        let (base, phone) = CHIP_CSS
            .split_once("@media (max-width: 820px) {")
            .expect("phone rules");
        assert!(base.contains(".chip-short { display: none; }"), "{base}");
        assert!(phone.contains(".chip-full { display: none; }"), "{phone}");
        assert!(
            phone.contains(".chip-short { display: inline; }"),
            "{phone}"
        );
    }

    #[test]
    fn chip_row_is_a_named_group() {
        fn app() -> Element {
            rsx! {
                ChipRow { label: "Filter by role",
                    ChipButton {
                        class: "chip".to_string(),
                        label: "All roles".to_string(),
                        pressed: true,
                        onclick: |_| {},
                    }
                    ChipButton {
                        class: "chip".to_string(),
                        label: "Tank".to_string(),
                        pressed: false,
                        onclick: |_| {},
                    }
                }
                ChipRow { label: "Status", scroll: true, "x" }
            }
        }
        let html = render(app);
        assert!(
            html.contains("<div class=\"chip-row\" role=\"group\" aria-label=\"Filter by role\">"),
            "{html}"
        );
        assert!(
            html.contains(
                "<div class=\"chip-row is-scroll\" role=\"group\" aria-label=\"Status\">x</div>"
            ),
            "{html}"
        );
        assert!(
            html.contains(
                "<button type=\"button\" aria-pressed=\"true\" class=\"chip active\">All roles</button>"
            ),
            "{html}"
        );
        assert!(
            html.contains(
                "<button type=\"button\" aria-pressed=\"false\" class=\"chip\">Tank</button>"
            ),
            "{html}"
        );
    }

    #[test]
    fn chip_is_44px_and_built_on_the_chip_tokens() {
        assert_sizes_from_tokens(CHIP_CSS);
        let chip = css_rule(CHIP_CSS, ".chip {");
        for decl in [
            "height: var(--chip-height)",
            "min-height: var(--hit-target-min)",
            "min-width: var(--hit-target-min)",
            "padding: 0 var(--chip-pad-x)",
            "border: 1px solid var(--chip-border)",
            "border-radius: var(--chip-radius)",
            "background: var(--chip-bg)",
            "color: var(--chip-fg)",
            "font-family: var(--chip-font)",
            "font-size: var(--chip-font-size)",
            "font-weight: var(--chip-font-weight)",
        ] {
            assert!(has_decl(&chip, decl), "missing {decl}: {chip}");
        }
        // The 44px height is not behind a media query: it holds at 390 too.
        let base = CHIP_CSS.split("@media").next().expect("base rules");
        assert!(base.contains("height: var(--chip-height)"), "{base}");
    }

    /// Selected is a fill, a border and a weight, and it follows the state the
    /// buttons expose to assistive tech.
    #[test]
    fn selected_chip_is_not_colour_alone() {
        let marker = ".chip[aria-selected=\"true\"] {";
        let selected = css_rule(CHIP_CSS, marker);
        for decl in [
            "background: var(--chip-selected-bg)",
            "color: var(--chip-selected-fg)",
            "border-color: var(--chip-selected-border)",
            "border-width: var(--chip-selected-border-width)",
            "font-weight: var(--chip-selected-weight)",
        ] {
            assert!(has_decl(&selected, decl), "missing {decl}: {selected}");
        }
        let selectors = &CHIP_CSS[..CHIP_CSS.find(marker).expect("selected rule")];
        for selector in [".chip.active,", ".chip[aria-pressed=\"true\"],"] {
            assert!(selectors.contains(selector), "missing {selector}");
        }
        assert_ink_clears_aa("--chip-fg", "--chip-bg", 4.5);
        assert_ink_clears_aa("--chip-selected-fg", "--chip-selected-bg", 4.5);
    }

    #[test]
    fn scroll_row_scrolls_sideways_on_phone_only() {
        let row = css_rule(CHIP_CSS, ".chip-row {");
        assert!(has_decl(&row, "flex-wrap: wrap"), "{row}");
        assert!(has_decl(&row, "gap: var(--space-2)"), "{row}");
        let (base, phone) = CHIP_CSS
            .split_once("@media (max-width: 820px) {")
            .expect("phone rules");
        assert!(!base.contains("overflow-x"), "{base}");
        let scroll = css_rule(phone, ".chip-row.is-scroll {");
        for decl in ["flex-wrap: nowrap", "overflow-x: auto", "min-width: 0"] {
            assert!(has_decl(&scroll, decl), "missing {decl}: {scroll}");
        }
        let chip = css_rule(phone, ".chip-row.is-scroll .chip {");
        assert!(has_decl(&chip, "flex: 0 0 auto"), "{chip}");
    }
}
