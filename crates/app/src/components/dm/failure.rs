use dioxus::prelude::*;

use super::types::DmLoadFailure;
use crate::routes::Route;

pub(crate) const FAILURE_CSS: &str = r#"
.dm-failure {
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-3);
    margin: var(--space-4);
    padding: var(--space-card-pad);
    background: var(--surface);
    border: 1px solid var(--loss);
    border-radius: var(--radius-card);
    color: var(--text);
}
.dm-failure-text {
    margin: 0;
    font-size: var(--text-md);
    line-height: var(--leading-body);
}
.dm-failure-title {
    display: block;
    color: var(--loss);
    font-family: var(--font-head);
    font-weight: var(--font-label-weight);
}
.dm-failure a {
    color: var(--link);
    font-weight: var(--font-label-weight);
    text-decoration: underline;
}
"#;

/// Whether a Retry can help. Membership and the identity setting are fixed
/// somewhere else, so they get no button.
pub(crate) fn failure_can_retry(failure: &DmLoadFailure) -> bool {
    matches!(
        failure,
        DmLoadFailure::RelayConfig | DmLoadFailure::Other(_)
    )
}

/// A failed DM load: a plain title, the reason (the server's own words for
/// anything that is not one of the known statuses), and Retry when a retry
/// can help. `role="alert"`, because it replaces the list or the thread.
#[component]
pub fn DmFailureNotice(
    failure: DmLoadFailure,
    /// "Could not load your conversations".
    #[props(default = "Could not load messages".to_string())]
    title: String,
    #[props(default)] on_retry: Option<EventHandler<()>>,
) -> Element {
    let message = failure.message();
    let identity = matches!(failure, DmLoadFailure::IdentitySettings);
    let retry = on_retry.filter(|_| failure_can_retry(&failure));
    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-components-dm-failure-failure-css", css: FAILURE_CSS }
        div { class: "dm-failure", role: "alert",
            p { class: "dm-failure-text",
                span { class: "dm-failure-title", "{title}." }
                " {message}"
                if identity {
                    " "
                    Link { to: Route::IdentitySettings {}, "Open account keys" }
                    " to turn it on."
                }
            }
            if let Some(retry) = retry {
                button {
                    r#type: "button",
                    class: "btn is-ghost",
                    onclick: move |_| retry.call(()),
                    "Retry"
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::ui::test_support::assert_sizes_from_tokens;

    #[test]
    fn retry_only_where_a_retry_can_help() {
        assert!(failure_can_retry(&DmLoadFailure::RelayConfig));
        assert!(failure_can_retry(&DmLoadFailure::Other("HTTP 502".into())));
        assert!(!failure_can_retry(&DmLoadFailure::Membership));
        assert!(!failure_can_retry(&DmLoadFailure::IdentitySettings));
    }

    #[test]
    fn failure_css_is_on_tokens() {
        assert_sizes_from_tokens(FAILURE_CSS);
    }
}
