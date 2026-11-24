//! Error UI for public loaders. Loading and empty stay on the page; this is only the failure path.

use dioxus::prelude::*;

/// Failed fetch with a retry that bumps the resource's refresh counter.
pub fn fetch_error(message: &str, refresh: Signal<u32>) -> Element {
    fetch_error_state(message, Some(refresh), false)
}

/// Retry button text. While a refetch runs the old error stays on screen, so
/// the button says so instead of inviting a second click.
pub fn retry_label(retrying: bool) -> &'static str {
    if retrying { "Retrying..." } else { "Retry" }
}

/// [`fetch_error`] with the in-flight state. `refresh: None` hides Retry for
/// failures a retry cannot fix; `retrying` marks the block busy until the
/// refetch resolves. The busy button is `aria-disabled` and ignores clicks: a
/// natively disabled button is blurred, which drops focus to the page body.
/// Always an alert, with or without Retry, the same rule as
/// [`crate::components::ui::LoadError`]: the box appears with its text already
/// in it, and a status that appears already filled is not reliably
/// announced. Callers keep any status line of their own empty while it
/// shows, so it is said once.
pub fn fetch_error_state(message: &str, refresh: Option<Signal<u32>>, retrying: bool) -> Element {
    let message = message.to_string();
    let label = retry_label(retrying);
    rsx! {
        div {
            class: "fetch-error-wrap",
            role: "alert",
            aria_busy: if retrying { "true" },
            p { class: "fetch-error", "{message}" }
            if let Some(mut refresh) = refresh {
                button {
                    r#type: "button",
                    class: "fetch-error__retry",
                    aria_disabled: if retrying { "true" },
                    onclick: move |_| {
                        if !retrying {
                            refresh += 1;
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

    #[test]
    fn retry_label_reflects_the_refetch() {
        assert_eq!(retry_label(false), "Retry");
        assert_eq!(retry_label(true), "Retrying...");
    }

    fn render(app: fn() -> Element) -> String {
        let mut dom = VirtualDom::new(app);
        dom.rebuild_in_place();
        dom.render_immediate(&mut dioxus::dioxus_core::NoOpMutations);
        dioxus_ssr::render(&dom)
    }

    #[component]
    fn Idle() -> Element {
        fetch_error_state("Couldn't load.", Some(use_signal(|| 0u32)), false)
    }

    #[component]
    fn Busy() -> Element {
        fetch_error_state("Couldn't load.", Some(use_signal(|| 0u32)), true)
    }

    #[component]
    fn NoRetry() -> Element {
        fetch_error_state("Not available.", None, false)
    }

    #[test]
    fn retry_is_disabled_and_busy_while_in_flight() {
        let idle = render(Idle);
        assert!(idle.contains(">Retry</button>"), "{idle}");
        assert!(!idle.contains("disabled"), "{idle}");
        assert!(!idle.contains("aria-busy"), "{idle}");

        let busy = render(Busy);
        assert!(busy.contains("Retrying..."), "{busy}");
        // Busy, but still focusable: never the native attribute.
        assert!(busy.contains(r#"aria-disabled="true""#), "{busy}");
        assert!(!busy.contains(" disabled"), "{busy}");
        assert!(busy.contains(r#"aria-busy="true""#), "{busy}");
    }

    /// Vision nit 2 on round 7: a no-Retry box is an alert too, the same
    /// rule as `LoadError`, so it is announced when it appears filled.
    #[test]
    fn no_refresh_means_no_retry_button() {
        let html = render(NoRetry);
        assert!(html.contains("Not available."), "{html}");
        assert!(!html.contains("<button"), "{html}");
        assert_eq!(html.matches(r#"role="alert""#).count(), 1, "{html}");
        assert!(!html.contains(r#"role="status""#), "{html}");
    }

    #[test]
    fn every_state_is_one_alert() {
        for html in [render(Idle), render(Busy), render(NoRetry)] {
            assert_eq!(html.matches(r#"role="alert""#).count(), 1, "{html}");
            assert!(!html.contains(r#"role="status""#), "{html}");
        }
    }
}
