//! Relay connection status indicator for the chat widget.

use dioxus::prelude::*;

use crate::state::nostr::RelayConnectionState;

const RELAY_STATUS_CSS: &str = r#"
.relay-status {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 4px 10px;
    border-radius: 12px;
    font-size: 11px;
    font-weight: 500;
    line-height: 1;
    white-space: nowrap;
    user-select: none;
    transition: background-color 0.3s ease, color 0.3s ease;
}

.relay-status--ready {
    background-color: color-mix(in srgb, var(--ok) 15%, transparent);
    color: var(--ok);
}

.relay-status--connected,
.relay-status--authenticating {
    background-color: color-mix(in srgb, var(--warn) 15%, transparent);
    color: var(--warn);
}

.relay-status--connecting {
    background-color: color-mix(in srgb, var(--warn) 15%, transparent);
    color: var(--warn);
}

.relay-status--reconnecting {
    background-color: color-mix(in srgb, var(--danger) 15%, transparent);
    color: var(--danger);
}

.relay-status--disconnected,
.relay-status--error {
    background-color: color-mix(in srgb, var(--danger) 15%, transparent);
    color: var(--danger);
}

.relay-status__dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    flex-shrink: 0;
    transition: background-color 0.3s ease;
}

.relay-status--ready .relay-status__dot {
    background-color: var(--ok);
}

.relay-status--connected .relay-status__dot,
.relay-status--authenticating .relay-status__dot,
.relay-status--connecting .relay-status__dot {
    background-color: var(--warn);
    animation: relay-pulse 1.5s ease-in-out infinite;
}

.relay-status--reconnecting .relay-status__dot {
    background-color: var(--danger);
    animation: relay-pulse 1s ease-in-out infinite;
}

.relay-status--disconnected .relay-status__dot,
.relay-status--error .relay-status__dot {
    background-color: var(--danger);
}

.relay-status__retry {
    margin-left: 2px;
    padding: 1px 6px;
    border: 1px solid currentColor;
    border-radius: 6px;
    background: transparent;
    color: inherit;
    font-size: 10px;
    font-weight: 600;
    cursor: pointer;
    transition: background-color 0.15s ease;
}

.relay-status__retry:hover {
    background-color: color-mix(in srgb, var(--danger) 20%, transparent);
}

@keyframes relay-pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.4; }
}
"#;

/// Compact pill showing relay connection state.
#[component]
pub fn RelayStatus(state: RelayConnectionState, on_retry: EventHandler<()>) -> Element {
    let modifier = match state {
        RelayConnectionState::Ready => "ready",
        RelayConnectionState::Connected => "connected",
        RelayConnectionState::Authenticating => "authenticating",
        RelayConnectionState::Connecting => "connecting",
        RelayConnectionState::Reconnecting(_) => "reconnecting",
        RelayConnectionState::Disconnected => "disconnected",
        RelayConnectionState::Error => "error",
    };

    let show_retry = matches!(
        state,
        RelayConnectionState::Error | RelayConnectionState::Disconnected
    );

    rsx! {
        style { {RELAY_STATUS_CSS} }
        div {
            class: "relay-status relay-status--{modifier}",
            span { class: "relay-status__dot" }
            span { "{state.display_text()}" }
            if show_retry {
                button {
                    class: "relay-status__retry",
                    onclick: move |_| on_retry.call(()),
                    "Retry"
                }
            }
        }
    }
}
