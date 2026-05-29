//! Scrollable message feed for the chat widget.

use dioxus::prelude::*;

use scuffed_types::nostr::ChatMessage;

const MESSAGE_LIST_CSS: &str = r#"
.chat-messages {
    flex: 1;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 8px 12px;
    min-height: 0;
}

.chat-messages--empty {
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--text-muted);
    font-size: 13px;
    font-style: italic;
    padding: 2rem;
}

.chat-msg {
    display: flex;
    gap: 8px;
    padding: 6px 8px;
    border-radius: 6px;
    transition: background-color 0.1s ease;
}

.chat-msg:hover {
    background: var(--bg-card-alt);
}

.chat-msg__avatar {
    width: 28px;
    height: 28px;
    border-radius: 50%;
    background: var(--accent-soft);
    color: var(--accent-bright);
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 12px;
    font-weight: 600;
    flex-shrink: 0;
    margin-top: 2px;
}

.chat-msg__avatar img {
    width: 100%;
    height: 100%;
    border-radius: 50%;
    object-fit: cover;
}

.chat-msg__body {
    flex: 1;
    min-width: 0;
}

.chat-msg__header {
    display: flex;
    align-items: baseline;
    gap: 6px;
    margin-bottom: 1px;
}

.chat-msg__author {
    font-size: 13px;
    font-weight: 600;
    color: var(--accent-bright);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 140px;
}

.chat-msg__time {
    font-size: 10px;
    color: var(--text-muted);
    white-space: nowrap;
}

.chat-msg__content {
    font-size: 13px;
    color: var(--text-primary);
    line-height: 1.4;
    word-wrap: break-word;
    overflow-wrap: break-word;
}

.chat-msg__encrypted {
    font-size: 12px;
    color: var(--text-muted);
    font-style: italic;
    display: flex;
    align-items: center;
    gap: 4px;
}

.chat-msg__encrypted::before {
    content: '\01F512';
    font-style: normal;
}

.chat-msg--loading {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 12px;
    color: var(--text-muted);
    font-size: 12px;
}

.chat-msg--loading .chat-msg__spinner {
    width: 14px;
    height: 14px;
    border: 2px solid var(--border);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: chat-spin 0.6s linear infinite;
    margin-right: 8px;
}

@keyframes chat-spin {
    to { transform: rotate(360deg); }
}
"#;

/// Scrollable message feed component.
#[component]
pub fn MessageList(messages: Vec<ChatMessage>, #[props(default = false)] loading: bool) -> Element {
    if messages.is_empty() && !loading {
        return rsx! {
            style { {MESSAGE_LIST_CSS} }
            div { class: "chat-messages chat-messages--empty",
                "No messages yet — say something!"
            }
        };
    }

    rsx! {
        style { {MESSAGE_LIST_CSS} }
        div { class: "chat-messages",
            if loading {
                div { class: "chat-msg--loading",
                    div { class: "chat-msg__spinner" }
                    "Loading messages..."
                }
            }
            for msg in messages.iter() {
                MessageBubble {
                    key: "{msg.id}",
                    message: msg.clone(),
                }
            }
        }
    }
}

/// A single message bubble.
#[component]
fn MessageBubble(message: ChatMessage) -> Element {
    let display_name = message
        .display_name
        .as_deref()
        .unwrap_or_else(|| &message.pubkey[..8]);

    let initials = display_name
        .chars()
        .next()
        .unwrap_or('?')
        .to_uppercase()
        .to_string();

    let time_str = format_relative_time(message.created_at);

    rsx! {
        div { class: "chat-msg",
            div { class: "chat-msg__avatar",
                if let Some(avatar) = &message.avatar_url {
                    img { src: "{avatar}", alt: "{display_name}" }
                } else {
                    "{initials}"
                }
            }
            div { class: "chat-msg__body",
                div { class: "chat-msg__header",
                    span { class: "chat-msg__author", "{display_name}" }
                    span { class: "chat-msg__time", "{time_str}" }
                }
                if message.encrypted {
                    div { class: "chat-msg__encrypted",
                        "Encrypted message"
                    }
                } else {
                    div { class: "chat-msg__content",
                        "{message.content}"
                    }
                }
            }
        }
    }
}

/// Format a unix timestamp as a relative time string.
fn format_relative_time(timestamp: u64) -> String {
    let now = js_sys::Date::now() as u64 / 1000;
    let diff = now.saturating_sub(timestamp);

    if diff < 60 {
        "now".to_string()
    } else if diff < 3600 {
        format!("{}m ago", diff / 60)
    } else if diff < 86400 {
        format!("{}h ago", diff / 3600)
    } else {
        format!("{}d ago", diff / 86400)
    }
}
