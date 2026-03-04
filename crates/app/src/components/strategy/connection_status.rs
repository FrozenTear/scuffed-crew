//! Connection status indicator component for the strategy editor.
//!
//! Displays the current WebSocket connection state as a compact pill/badge,
//! suitable for embedding in a toolbar or status bar.

use dioxus::prelude::*;

use crate::state::collab::ConnectionState;

// =============================================================================
// CSS
// =============================================================================

const CONNECTION_STATUS_CSS: &str = r#"
.connection-status {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 4px 10px;
    border-radius: 12px;
    font-size: 12px;
    font-weight: 500;
    line-height: 1;
    white-space: nowrap;
    user-select: none;
    transition: background-color 0.3s ease, color 0.3s ease;
}

.connection-status--connected {
    background-color: rgba(34, 197, 94, 0.15);
    color: #22c55e;
}

.connection-status--connecting {
    background-color: rgba(234, 179, 8, 0.15);
    color: #eab308;
}

.connection-status--reconnecting {
    background-color: rgba(249, 115, 22, 0.15);
    color: #f97316;
}

.connection-status--disconnected,
.connection-status--error {
    background-color: rgba(239, 68, 68, 0.15);
    color: #ef4444;
}

.connection-status__dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
    transition: background-color 0.3s ease;
}

.connection-status--connected .connection-status__dot {
    background-color: #22c55e;
}

.connection-status--connecting .connection-status__dot {
    background-color: #eab308;
    animation: connection-pulse 1.5s ease-in-out infinite;
}

.connection-status--reconnecting .connection-status__dot {
    background-color: #f97316;
    animation: connection-pulse 1s ease-in-out infinite;
}

.connection-status--disconnected .connection-status__dot,
.connection-status--error .connection-status__dot {
    background-color: #ef4444;
}

.connection-status__retry {
    margin-left: 4px;
    padding: 2px 8px;
    border: 1px solid currentColor;
    border-radius: 8px;
    background: transparent;
    color: inherit;
    font-size: 11px;
    font-weight: 600;
    cursor: pointer;
    transition: background-color 0.15s ease;
}

.connection-status__retry:hover {
    background-color: rgba(239, 68, 68, 0.2);
}

@keyframes connection-pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.4; }
}
"#;

// =============================================================================
// Component
// =============================================================================

/// Small pill/badge showing the WebSocket connection state.
///
/// Displays a colored dot and status text. When the connection is in an error
/// or disconnected state, a retry button is shown.
#[component]
pub fn ConnectionStatus(
    state: ConnectionState,
    on_retry: EventHandler<()>,
) -> Element {
    let modifier = match state {
        ConnectionState::Connected => "connected",
        ConnectionState::Connecting => "connecting",
        ConnectionState::Reconnecting(_) => "reconnecting",
        ConnectionState::Disconnected => "disconnected",
        ConnectionState::Error => "error",
    };

    let show_retry = matches!(state, ConnectionState::Error | ConnectionState::Disconnected);

    rsx! {
        style { {CONNECTION_STATUS_CSS} }
        div {
            class: "connection-status connection-status--{modifier}",
            span { class: "connection-status__dot" }
            span { "{state.display_text()}" }
            if show_retry {
                button {
                    class: "connection-status__retry",
                    onclick: move |_| on_retry.call(()),
                    "Retry"
                }
            }
        }
    }
}
