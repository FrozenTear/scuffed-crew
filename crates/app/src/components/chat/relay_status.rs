//! Relay connection status line for the chat thread head.
//!
//! The `d1-chat` frame draws the status as plain text with a small dot. The
//! word carries the state; the dot only repeats it, so nothing rests on
//! colour. "Connected" is never drawn in a warning colour.

use dioxus::prelude::*;

use crate::components::ui::btn_class;
use crate::state::nostr::RelayConnectionState;

const RELAY_STATUS_CSS: &str = r#"
.relay-status {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2);
  min-width: 0;
}
.relay-status__text {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  margin: 0;
  color: var(--text);
  font-family: var(--font-body);
  font-size: var(--text-sm);
  line-height: var(--leading-ui);
}
.relay-status__dot {
  box-sizing: border-box;
  flex: 0 0 auto;
  width: var(--space-2);
  height: var(--space-2);
  border-radius: 50%;
  background: var(--text-3);
}
.relay-status--ok .relay-status__dot { background: var(--win); }
/* Waiting states draw a ring, a failure a square: a second cue next to the word. */
.relay-status--busy .relay-status__dot {
  background: transparent;
  border: 1px solid var(--text-3);
}
.relay-status--down .relay-status__dot {
  border-radius: 0;
  background: var(--loss);
}
"#;

/// What the member reads. `Connected` and `Ready` both carry messages, so
/// both say "Connected".
pub fn status_label(state: RelayConnectionState) -> &'static str {
    match state {
        RelayConnectionState::Ready | RelayConnectionState::Connected => "Connected",
        RelayConnectionState::Authenticating => "Signing in",
        RelayConnectionState::Connecting => "Connecting",
        RelayConnectionState::Reconnecting(_) => "Reconnecting",
        RelayConnectionState::Disconnected => "Disconnected",
        RelayConnectionState::Error => "Connection error",
    }
}

fn status_modifier(state: RelayConnectionState) -> &'static str {
    match state {
        RelayConnectionState::Ready | RelayConnectionState::Connected => "ok",
        RelayConnectionState::Authenticating
        | RelayConnectionState::Connecting
        | RelayConnectionState::Reconnecting(_) => "busy",
        RelayConnectionState::Disconnected | RelayConnectionState::Error => "down",
    }
}

fn offers_retry(state: RelayConnectionState) -> bool {
    matches!(
        state,
        RelayConnectionState::Error | RelayConnectionState::Disconnected
    )
}

/// Status line: a dot, the state as a word, and Retry when the relay is down.
/// The `role="status"` line is rendered in every state; only its text changes.
#[component]
pub fn RelayStatus(state: RelayConnectionState, on_retry: EventHandler<()>) -> Element {
    let modifier = status_modifier(state);
    let label = status_label(state);

    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-components-chat-relay_status-relay-status-css", css: RELAY_STATUS_CSS }
        div { class: "relay-status relay-status--{modifier}",
            p { class: "relay-status__text", role: "status",
                span { class: "relay-status__dot", aria_hidden: "true" }
                "{label}"
            }
            if offers_retry(state) {
                button {
                    r#type: "button",
                    class: btn_class(false),
                    aria_label: "Retry the chat connection",
                    // The head bar toggles the floating widget on click.
                    onclick: move |evt| {
                        evt.stop_propagation();
                        on_retry.call(());
                    },
                    "Retry"
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::ui::test_support::{assert_sizes_from_tokens, css_rule, has_decl};

    #[component]
    fn Probe(state: RelayConnectionState) -> Element {
        rsx! { RelayStatus { state, on_retry: |_| {} } }
    }

    fn render(state: RelayConnectionState) -> String {
        let mut dom = VirtualDom::new_with_props(Probe, ProbeProps { state });
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    const ALL: [RelayConnectionState; 7] = [
        RelayConnectionState::Ready,
        RelayConnectionState::Connected,
        RelayConnectionState::Authenticating,
        RelayConnectionState::Connecting,
        RelayConnectionState::Reconnecting(2),
        RelayConnectionState::Disconnected,
        RelayConnectionState::Error,
    ];

    #[test]
    fn the_status_line_is_always_rendered_with_a_word() {
        for state in ALL {
            let html = render(state);
            assert_eq!(html.matches("role=\"status\"").count(), 1, "{html}");
            assert!(html.contains(status_label(state)), "{html}");
            assert!(
                html.contains("<span class=\"relay-status__dot\" aria-hidden=\"true\"></span>"),
                "the dot is decoration: {html}"
            );
        }
    }

    #[test]
    fn connected_reads_connected_and_is_not_a_warning() {
        for state in [RelayConnectionState::Ready, RelayConnectionState::Connected] {
            let html = render(state);
            assert!(html.contains("relay-status--ok"), "{html}");
            assert!(html.contains("Connected"), "{html}");
            assert!(!html.contains("<button"), "{html}");
        }
        assert!(!RELAY_STATUS_CSS.contains("--warn"));
        let text = css_rule(RELAY_STATUS_CSS, ".relay-status__text {");
        assert!(has_decl(&text, "color: var(--text)"), "{text}");
    }

    #[test]
    fn a_dropped_relay_says_so_and_offers_a_named_retry() {
        for state in [
            RelayConnectionState::Disconnected,
            RelayConnectionState::Error,
        ] {
            let html = render(state);
            assert!(html.contains("relay-status--down"), "{html}");
            assert!(
                html.contains("aria-label=\"Retry the chat connection\""),
                "{html}"
            );
            assert!(html.contains("class=\"btn is-ghost\""), "{html}");
            assert!(html.contains(">Retry</button>"), "{html}");
            let status = html.find("role=\"status\"").expect("status");
            let close = html[status..].find("</p>").expect("status end") + status;
            let button = html.find("<button").expect("retry");
            assert!(close < button, "Retry sits outside the live region: {html}");
        }
        assert_eq!(
            status_label(RelayConnectionState::Disconnected),
            "Disconnected"
        );
    }

    #[test]
    fn waiting_states_have_no_retry() {
        for state in [
            RelayConnectionState::Authenticating,
            RelayConnectionState::Connecting,
            RelayConnectionState::Reconnecting(1),
        ] {
            let html = render(state);
            assert!(html.contains("relay-status--busy"), "{html}");
            assert!(!html.contains("<button"), "{html}");
        }
    }

    #[test]
    fn status_css_is_on_tokens() {
        assert_sizes_from_tokens(RELAY_STATUS_CSS);
        assert!(!RELAY_STATUS_CSS.contains("color-mix"));
        assert!(!RELAY_STATUS_CSS.contains("animation"));
    }
}
