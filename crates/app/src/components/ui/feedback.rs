use dioxus::prelude::*;
pub const FEEDBACK_CSS: &str = r#"
.ui-empty { text-align: center; color: var(--text-2); padding: var(--space-8) var(--space-4); }
.ui-empty h3 { font-family: var(--font-head); color: var(--text); font-size: var(--text-lg); margin-bottom: var(--space-2); }
.ui-empty p { font-size: var(--text-sm); }
.ui-spinner { width: 18px; height: 18px; border: 2px solid var(--border);
  border-top-color: var(--accent); border-radius: 50%; animation: ui-spin .7s linear infinite; }
@keyframes ui-spin { to { transform: rotate(360deg); } }
.ui-empty__next { margin-top: var(--space-4); }
.ui-skel-list { display: flex; flex-direction: column; gap: var(--space-3); padding: var(--space-4) 0; }
.ui-skel-row { height: 3.25rem; border-radius: var(--radius-md);
  background: color-mix(in srgb, var(--text) 12%, var(--surface-2)); }
.ui-skel-status { position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px;
  overflow: hidden; clip: rect(0, 0, 0, 0); white-space: nowrap; border: 0; }
.ui-status { color: var(--text-3); font-size: var(--text-sm); text-align: center; margin: var(--space-2) 0; }
"#;
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

/// One visible status line for a refetch or a save ("Updating..."). A polite
/// live region.
#[component]
pub fn StatusLine(text: String) -> Element {
    rsx! { p { class: "ui-status", role: "status", "{text}" } }
}

/// Failed load. Plain sentence, no raw server error. `on_retry: None` hides
/// the button for failures a retry cannot fix. Same markup and classes as
/// `fetch_error_state`, but the retry is a handler, so it also works for
/// `use_resource` (`resource.restart()`).
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
            role: if on_retry.is_some() { "alert" } else { "status" },
            aria_busy: if retrying { "true" },
            p { class: "fetch-error", "{message}" }
            if let Some(retry) = on_retry {
                button {
                    r#type: "button",
                    class: "fetch-error__retry",
                    disabled: retrying,
                    onclick: move |_| retry.call(()),
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
        assert!(html.contains("role=\"status\""), "{html}");
        assert!(!html.contains("<button"), "{html}");
        let html = render(busy);
        assert!(html.contains(">Retrying...</button>"), "{html}");
        assert!(html.contains("disabled"), "{html}");
        assert!(html.contains("aria-busy=\"true\""), "{html}");
    }
}
#[component]
pub fn Spinner() -> Element {
    rsx! { div { class: "ui-spinner" } }
}
