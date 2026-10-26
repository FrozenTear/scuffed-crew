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
/// failures a retry cannot fix; `retrying` disables the button and marks the
/// block busy until the refetch resolves.
pub fn fetch_error_state(message: &str, refresh: Option<Signal<u32>>, retrying: bool) -> Element {
    let message = message.to_string();
    let label = retry_label(retrying);
    rsx! {
        div {
            class: "fetch-error-wrap",
            role: if refresh.is_some() { "alert" } else { "status" },
            aria_busy: if retrying { "true" },
            p { class: "fetch-error", "{message}" }
            if let Some(mut refresh) = refresh {
                button {
                    r#type: "button",
                    class: "fetch-error__retry",
                    disabled: retrying,
                    onclick: move |_| refresh += 1,
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
        assert!(busy.contains("disabled"), "{busy}");
        assert!(busy.contains(r#"aria-busy="true""#), "{busy}");
    }

    #[test]
    fn no_refresh_means_no_retry_button() {
        let html = render(NoRetry);
        assert!(html.contains("Not available."), "{html}");
        assert!(!html.contains("<button"), "{html}");
        assert!(html.contains(r#"role="status""#), "{html}");
    }
}
