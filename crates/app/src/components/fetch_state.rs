//! Error UI for public loaders. Loading and empty stay on the page; this is only the failure path.

use dioxus::prelude::*;

/// Failed fetch with a retry that bumps the resource's refresh counter.
pub fn fetch_error(message: &str, mut refresh: Signal<u32>) -> Element {
    let message = message.to_string();
    rsx! {
        div { class: "fetch-error-wrap", role: "alert",
            p { class: "fetch-error", "{message}" }
            button {
                r#type: "button",
                class: "fetch-error__retry",
                onclick: move |_| refresh += 1,
                "Retry"
            }
        }
    }
}
