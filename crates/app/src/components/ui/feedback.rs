use dioxus::prelude::*;
pub const FEEDBACK_CSS: &str = r#"
.ui-empty { text-align: center; color: var(--text-3); padding: var(--space-8) var(--space-4); }
.ui-empty h3 { font-family: var(--font-head); color: var(--text); font-size: var(--text-lg); margin-bottom: var(--space-2); }
.ui-empty p { font-size: var(--text-sm); }
.ui-spinner { width: 18px; height: 18px; border: 2px solid var(--border);
  border-top-color: var(--accent-solid); border-radius: 50%; animation: ui-spin .7s linear infinite; }
@keyframes ui-spin { to { transform: rotate(360deg); } }
.ui-empty__next { margin-top: var(--space-4); }
.ui-skel-list { display: flex; flex-direction: column; gap: var(--space-3); padding: var(--space-4) 0; }
.ui-skel-row { height: var(--density-row); border-radius: var(--radius-control);
  background: color-mix(in srgb, var(--text) 12%, var(--surface-sunken)); }
.ui-skel-status { position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px;
  overflow: hidden; clip: rect(0, 0, 0, 0); white-space: nowrap; border: 0; }
.ui-status { color: var(--text-subtle); font-size: var(--text-sm); text-align: center; margin: var(--space-2) 0; }
"#;
/// The empty-state card of the frames (`.empty`): a card with an H2, one
/// sentence and an optional row of next actions.
pub const EMPTY_CARD_CSS: &str = r#"
.empty-card {
  box-sizing: border-box;
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: var(--space-4);
  min-width: 0;
  padding: var(--space-section);
  background: var(--card-bg);
  color: var(--card-fg);
  border: 1px solid var(--card-border);
  border-radius: var(--card-radius);
  box-shadow: var(--card-shadow);
}
.empty-card h2 {
  margin: 0;
  font-family: var(--font-head);
  font-weight: var(--font-head-weight);
  letter-spacing: var(--font-head-tracking);
  text-transform: var(--font-head-transform);
  line-height: var(--leading-display);
  font-size: var(--text-2xl);
  color: var(--text);
  overflow-wrap: anywhere;
}
.empty-card p {
  margin: 0;
  max-width: var(--content-max-narrow);
  color: var(--text-3);
  font-family: var(--font-body);
  font-size: var(--text-md);
  line-height: var(--leading-body);
}
.empty-card-actions {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-2);
}
"#;

/// One empty state for every redesigned page: nothing to list yet, or nothing
/// matches. The title is an H2 under the page's H1. `actions` holds links or
/// buttons with the shared `btn` class.
#[component]
pub fn EmptyCard(
    title: String,
    message: String,
    #[props(default)] actions: Option<Element>,
) -> Element {
    rsx! {
        section { class: "empty-card",
            h2 { "{title}" }
            p { "{message}" }
            if let Some(actions) = actions {
                div { class: "empty-card-actions", {actions} }
            }
        }
    }
}

#[component]
pub fn EmptyState(
    title: String,
    #[props(default)] message: Option<String>,
    /// One next step (a link or a button). Shown under the message.
    #[props(default)]
    next: Option<Element>,
) -> Element {
    rsx! {
        div { class: "ui-empty",
            h3 { "{title}" }
            if let Some(msg) = message {
                p { "{msg}" }
            }
            if let Some(next) = next {
                div { class: "ui-empty__next", {next} }
            }
        }
    }
}

/// Placeholder rows while a list loads. The bars are hidden from assistive
/// tech; `status` is the one line they hear ("Loading the crew..."). Static on
/// purpose, so there is nothing to switch off for reduced motion.
#[component]
pub fn SkeletonRows(status: String, #[props(default = 5)] rows: usize) -> Element {
    rsx! {
        div { class: "ui-skel-list",
            span { class: "ui-skel-status", role: "status", "{status}" }
            for i in 0..rows {
                div { key: "{i}", class: "ui-skel-row", aria_hidden: "true" }
            }
        }
    }
}

/// The skeleton bars alone, hidden from assistive tech. For pages that keep
/// their own always-rendered status line and only change its text.
#[component]
pub fn SkeletonBars(#[props(default = 3)] rows: usize) -> Element {
    rsx! {
        div { class: "ui-skel-list", aria_hidden: "true",
            for i in 0..rows {
                div { key: "{i}", class: "ui-skel-row" }
            }
        }
    }
}

/// One visible status line for a refetch or a save ("Updating..."). A polite
/// live region.
#[component]
pub fn StatusLine(text: String) -> Element {
    rsx! { p { class: "ui-status", role: "status", "{text}" } }
}

/// Failed load. Plain sentence, no raw server error. `on_retry: None` hides
/// the button for failures a retry cannot fix. Always an alert, with or
/// without Retry: the box appears with its text already in it, and a
/// status that appears already filled is not reliably announced (Vision
/// round 7 on 748; the strategy editor 403 is an alert too). Pages keep
/// their own status line empty while it shows, so it is said once. Same markup and classes as
/// `fetch_error_state`, but the retry is a handler, so it also works for
/// `use_resource` (`resource.restart()`). While `retrying` the button is
/// `aria-disabled` and ignores clicks, so it keeps keyboard focus.
#[component]
pub fn LoadError(
    message: String,
    #[props(default)] on_retry: Option<EventHandler<()>>,
    #[props(default = false)] retrying: bool,
) -> Element {
    let label = crate::components::fetch_state::retry_label(retrying);
    rsx! {
        div {
            class: "fetch-error-wrap",
            role: "alert",
            aria_busy: if retrying { "true" },
            p { class: "fetch-error", "{message}" }
            if let Some(retry) = on_retry {
                button {
                    r#type: "button",
                    class: "fetch-error__retry",
                    aria_disabled: if retrying { "true" },
                    onclick: move |_| {
                        if !retrying {
                            retry.call(());
                        }
                    },
                    "{label}"
                }
            }
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
    fn skeleton_rows_say_one_thing_and_hide_the_bars() {
        fn app() -> Element {
            rsx! { SkeletonRows { status: "Loading the crew...".to_string(), rows: 3 } }
        }
        let html = render(app);
        assert_eq!(html.matches("role=\"status\"").count(), 1, "{html}");
        assert!(html.contains("Loading the crew..."), "{html}");
        assert_eq!(html.matches("ui-skel-row").count(), 3, "{html}");
        assert_eq!(html.matches("aria-hidden=\"true\"").count(), 3, "{html}");
    }

    #[test]
    fn skeleton_rows_default_to_five() {
        fn app() -> Element {
            rsx! { SkeletonRows { status: "Loading...".to_string() } }
        }
        assert_eq!(render(app).matches("ui-skel-row").count(), 5);
    }

    #[test]
    fn status_line_is_a_polite_live_region() {
        fn app() -> Element {
            rsx! { StatusLine { text: "Updating...".to_string() } }
        }
        let html = render(app);
        assert!(html.contains("role=\"status\""), "{html}");
        assert!(html.contains(">Updating...</p>"), "{html}");
    }

    #[test]
    fn empty_state_shows_its_next_step_only_when_given() {
        fn without() -> Element {
            rsx! { EmptyState { title: "No members yet.".to_string() } }
        }
        fn with() -> Element {
            rsx! {
                EmptyState {
                    title: "No members yet.".to_string(),
                    next: rsx! { a { href: "/apply", "Apply to join" } },
                }
            }
        }
        assert!(!render(without).contains("ui-empty__next"));
        let html = render(with);
        assert!(html.contains("ui-empty__next"), "{html}");
        assert!(html.contains(">Apply to join</a>"), "{html}");
    }

    #[test]
    fn skeleton_bars_are_hidden_and_carry_no_status() {
        fn app() -> Element {
            rsx! { SkeletonBars { rows: 2 } }
        }
        let html = render(app);
        assert_eq!(
            html,
            "<div class=\"ui-skel-list\" aria-hidden=\"true\">\
             <div class=\"ui-skel-row\"></div><div class=\"ui-skel-row\"></div></div>"
        );
        // The row height follows the density token.
        assert!(FEEDBACK_CSS.contains(".ui-skel-row { height: var(--density-row);"));
    }

    #[test]
    fn empty_card_is_an_h2_a_sentence_and_optional_actions() {
        fn bare() -> Element {
            rsx! { EmptyCard { title: "No posts yet", message: "Check back soon." } }
        }
        fn with_actions() -> Element {
            rsx! {
                EmptyCard {
                    title: "No events scheduled",
                    message: "Nothing yet.",
                    actions: rsx! { a { class: "btn is-primary", href: "/forum", "Forum" } },
                }
            }
        }
        assert_eq!(
            render(bare),
            "<section class=\"empty-card\"><h2>No posts yet</h2><p>Check back soon.</p></section>"
        );
        let html = render(with_actions);
        assert!(
            html.contains("<div class=\"empty-card-actions\"><a class=\"btn is-primary\""),
            "{html}"
        );
        assert_eq!(html.matches("<h2").count(), 1, "{html}");
    }

    #[test]
    fn empty_card_follows_the_frame_recipe() {
        use crate::components::ui::test_support::{assert_sizes_from_tokens, css_rule, has_decl};
        assert_sizes_from_tokens(EMPTY_CARD_CSS);
        let card = css_rule(EMPTY_CARD_CSS, ".empty-card {");
        for decl in [
            "padding: var(--space-section)",
            "gap: var(--space-4)",
            "border: 1px solid var(--card-border)",
            "border-radius: var(--card-radius)",
            "align-items: flex-start",
        ] {
            assert!(has_decl(&card, decl), "missing {decl}: {card}");
        }
        let title = css_rule(EMPTY_CARD_CSS, ".empty-card h2 {");
        assert!(has_decl(&title, "font-size: var(--text-2xl)"), "{title}");
    }

    #[test]
    fn load_error_offers_retry_only_with_a_handler() {
        fn retryable() -> Element {
            rsx! { LoadError { message: "Couldn't load.".to_string(), on_retry: move |_| {} } }
        }
        fn fixed() -> Element {
            rsx! { LoadError { message: "Not available.".to_string() } }
        }
        fn busy() -> Element {
            rsx! {
                LoadError {
                    message: "Couldn't load.".to_string(),
                    on_retry: move |_| {},
                    retrying: true,
                }
            }
        }
        let html = render(retryable);
        assert!(html.contains("role=\"alert\""), "{html}");
        assert!(html.contains(">Retry</button>"), "{html}");
        let html = render(fixed);
        // Vision round 7 on 748: no Retry is still an alert, exactly one
        // live region, and no status that would say it a second time.
        assert_eq!(html.matches("role=\"alert\"").count(), 1, "{html}");
        assert!(!html.contains("role=\"status\""), "{html}");
        assert!(!html.contains("<button"), "{html}");
        let html = render(busy);
        assert!(html.contains(">Retrying...</button>"), "{html}");
        // Busy, but still focusable: never the native attribute.
        assert!(html.contains("aria-disabled=\"true\""), "{html}");
        assert!(!html.contains(" disabled"), "{html}");
        assert!(html.contains("aria-busy=\"true\""), "{html}");
    }
}
#[component]
pub fn Spinner() -> Element {
    rsx! { div { class: "ui-spinner" } }
}
