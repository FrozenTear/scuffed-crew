//! Main expandable chat widget — sidebar/panel layout.
//!
//! This is the top-level chat component that integrates the relay manager,
//! message list, compose box, relay status, and channel switcher.

use dioxus::prelude::*;

use scuffed_types::nostr::{ChatMessage, NostrGroup};

use crate::state::nostr::{NostrRelayEvent, NostrRelayManager, RelayConnectionState};

use super::compose_box::ComposeBox;
use super::message_list::MessageList;
use super::relay_status::RelayStatus;

const CHAT_WIDGET_CSS: &str = r#"
.chat-widget {
    position: fixed;
    bottom: 0;
    right: 0;
    width: 360px;
    max-height: 520px;
    display: flex;
    flex-direction: column;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-bottom: none;
    border-radius: 12px 12px 0 0;
    box-shadow: 0 -4px 24px rgba(0, 0, 0, 0.3);
    z-index: 1000;
    transition: transform 0.25s ease, opacity 0.2s ease;
    overflow: hidden;
}

.chat-widget--collapsed {
    max-height: 44px;
    cursor: pointer;
}

.chat-widget--collapsed .chat-widget__body {
    display: none;
}

.chat-widget__header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 14px;
    background: var(--bg-elevated);
    border-bottom: 1px solid var(--border);
    cursor: pointer;
    user-select: none;
    min-height: 44px;
}

.chat-widget__header-left {
    display: flex;
    align-items: center;
    gap: 8px;
}

.chat-widget__title {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-bright);
}

.chat-widget__header-right {
    display: flex;
    align-items: center;
    gap: 6px;
}

.chat-widget__toggle {
    width: 24px;
    height: 24px;
    display: flex;
    align-items: center;
    justify-content: center;
    border: none;
    background: transparent;
    color: var(--text-secondary);
    cursor: pointer;
    border-radius: 4px;
    font-size: 16px;
    transition: color 0.15s ease, background-color 0.15s ease;
}

.chat-widget__toggle:hover {
    color: var(--text-bright);
    background: var(--bg-card-alt);
}

.chat-widget__body {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
    height: 420px;
}

.chat-widget__channel-bar {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 6px 12px;
    border-bottom: 1px solid var(--border);
    overflow-x: auto;
    scrollbar-width: none;
}

.chat-widget__channel-bar::-webkit-scrollbar {
    display: none;
}

.chat-widget__channel {
    padding: 4px 10px;
    border: 1px solid var(--border);
    border-radius: 12px;
    background: transparent;
    color: var(--text-secondary);
    font-size: 11px;
    font-weight: 500;
    cursor: pointer;
    white-space: nowrap;
    transition: all 0.15s ease;
}

.chat-widget__channel:hover {
    border-color: var(--accent);
    color: var(--text-primary);
}

.chat-widget__channel--active {
    background: var(--accent-soft);
    border-color: var(--accent);
    color: var(--accent-bright);
}

.chat-widget__presence {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 12px;
    border-bottom: 1px solid var(--border);
    font-size: 11px;
    color: var(--text-muted);
}

.chat-widget__presence-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: #22c55e;
}

.chat-widget__voice-btn {
    margin-left: auto;
    padding: 2px 8px;
    border: 1px solid var(--accent);
    border-radius: 8px;
    background: transparent;
    color: var(--accent-bright);
    font-size: 10px;
    font-weight: 600;
    cursor: pointer;
    transition: background-color 0.15s ease;
}

.chat-widget__voice-btn:hover {
    background: var(--accent-soft);
}

@media (max-width: 480px) {
    .chat-widget {
        width: 100%;
        border-radius: 0;
        border: none;
        border-top: 1px solid var(--border);
    }
}
"#;

/// Top-level expandable chat widget.
///
/// Manages its own relay connection lifecycle via `NostrRelayManager`.
#[component]
pub fn ChatWidget(
    /// Relay WebSocket URL path (e.g., "/relay").
    #[props(default = "/relay".to_string())]
    relay_path: String,
    /// Available NIP-29 groups/channels.
    #[props(default = vec![])]
    groups: Vec<NostrGroup>,
    /// Number of online members to display.
    #[props(default = 0)]
    online_count: u32,
) -> Element {
    let mut expanded = use_signal(|| true);
    let mut connection_state = use_signal(|| RelayConnectionState::Disconnected);
    let mut messages = use_signal(Vec::<ChatMessage>::new);
    let mut active_group = use_signal(|| groups.first().map(|g| g.id.clone()));
    let mut current_sub_id = use_signal(|| Option::<String>::None);
    let mut loading = use_signal(|| false);

    // Initialize relay manager with FnMut callbacks that update signals
    let relay = use_hook(|| {
        NostrRelayManager::new(
            move |state| {
                connection_state.set(state);
            },
            move |event| match event {
                NostrRelayEvent::ChatMessage(msg) => {
                    messages.write().push(msg);
                }
                NostrRelayEvent::Eose { .. } => {
                    loading.set(false);
                }
                NostrRelayEvent::AuthSuccess => {
                    connection_state.set(RelayConnectionState::Ready);
                }
                NostrRelayEvent::Notice(msg) => {
                    tracing::info!("Relay notice: {}", msg);
                }
                _ => {}
            },
        )
    });

    // Configure NIP-42 auth endpoint for challenge-response authentication
    relay.set_auth_endpoint("/api/chat/auth-token");

    // Disconnect WebSocket when component unmounts
    let relay_cleanup = relay.clone();
    use_drop(move || {
        relay_cleanup.disconnect();
    });

    // Connect on mount
    let relay_connect = relay.clone();
    let relay_path_owned = relay_path.clone();
    use_effect(move || {
        relay_connect.connect_same_origin(&relay_path_owned);
        if let Some(group_id) = active_group.peek().as_ref() {
            loading.set(true);
            let sub_id = relay_connect.subscribe_group(group_id, Some(50));
            current_sub_id.set(Some(sub_id));
        }
    });

    let conn = *connection_state.read();
    let is_loading = *loading.read();
    let is_expanded = *expanded.read();

    let relay_retry = relay.clone();

    rsx! {
        style { {CHAT_WIDGET_CSS} }
        div {
            class: if is_expanded { "chat-widget" } else { "chat-widget chat-widget--collapsed" },

            // Header bar
            div {
                class: "chat-widget__header",
                onclick: move |_| expanded.toggle(),

                div { class: "chat-widget__header-left",
                    span { class: "chat-widget__title", "Chat" }
                    RelayStatus {
                        state: conn,
                        on_retry: move |_| relay_retry.retry(),
                    }
                }
                div { class: "chat-widget__header-right",
                    button {
                        class: "chat-widget__toggle",
                        onclick: move |evt| {
                            evt.stop_propagation();
                            expanded.toggle();
                        },
                        if is_expanded { "\u{25BC}" } else { "\u{25B2}" }
                    }
                }
            }

            // Body (hidden when collapsed via CSS)
            div { class: "chat-widget__body",

                // Channel switcher
                if !groups.is_empty() {
                    div { class: "chat-widget__channel-bar",
                        for group in groups.iter() {
                            {
                                let is_active = active_group.read().as_deref() == Some(group.id.as_str());
                                let group_id = group.id.clone();
                                let group_name = group.name.clone();
                                let relay_ch = relay.clone();
                                rsx! {
                                    button {
                                        class: if is_active { "chat-widget__channel chat-widget__channel--active" } else { "chat-widget__channel" },
                                        onclick: move |_| {
                                            let gid = group_id.clone();
                                            active_group.set(Some(gid.clone()));
                                            messages.write().clear();
                                            loading.set(true);
                                            // Unsubscribe old group before subscribing new
                                            if let Some(old_sub) = current_sub_id.peek().as_ref() {
                                                relay_ch.unsubscribe(old_sub);
                                            }
                                            let sub_id = relay_ch.subscribe_group(&gid, Some(50));
                                            current_sub_id.set(Some(sub_id));
                                        },
                                        "{group_name}"
                                    }
                                }
                            }
                        }
                    }
                }

                // Presence bar
                if online_count > 0 {
                    div { class: "chat-widget__presence",
                        span { class: "chat-widget__presence-dot" }
                        "{online_count} online"
                        button {
                            class: "chat-widget__voice-btn",
                            "Join Voice"
                        }
                    }
                }

                // Message list + compose
                MessageList {
                    messages: messages.read().clone(),
                    loading: is_loading,
                }
                ComposeBox {
                    on_send: move |text: String| {
                        // Full flow: POST to Axum for server-side signing, or NIP-07 client-side.
                        // For now, log the intent — message publishing requires THE-218.
                        tracing::info!("Send message: {}", text);
                    },
                    disabled: !conn.is_ready(),
                    placeholder: if conn.is_ready() {
                        "Type a message...".to_string()
                    } else {
                        "Connecting to relay...".to_string()
                    },
                }
            }
        }
    }
}
