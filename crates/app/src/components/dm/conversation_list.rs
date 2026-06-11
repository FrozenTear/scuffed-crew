use dioxus::prelude::*;

use super::types::{ConversationSummary, relative_time, truncate_pubkey};
use crate::routes::Route;

const CONVERSATION_LIST_CSS: &str = r#"
.dm-conv-list {
    display: flex;
    flex-direction: column;
    height: 100%;
    overflow-y: auto;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 10px;
}
.dm-conv-list-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 1rem 1rem 0.75rem;
    border-bottom: 1px solid var(--border);
    position: sticky;
    top: 0;
    background: var(--surface);
    z-index: 1;
}
.dm-conv-list-title {
    font-family: var(--font-head);
    font-weight: 700;
    font-size: 1rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text);
    margin: 0;
}
.dm-conv-list-refresh {
    background: transparent;
    color: var(--text-2);
    border: 1px solid var(--border);
    border-radius: 6px;
    font-size: 0.75rem;
    padding: 0.3rem 0.6rem;
    cursor: pointer;
}
.dm-conv-list-refresh:hover { color: var(--text); border-color: var(--accent-soft); }
.dm-conv-list-refresh:disabled { opacity: 0.5; cursor: not-allowed; }
.dm-conv-row {
    display: flex;
    gap: 0.75rem;
    align-items: center;
    padding: 0.75rem 1rem;
    border-bottom: 1px solid var(--border);
    text-decoration: none;
    color: inherit;
    transition: background-color 0.15s;
}
.dm-conv-row:hover { background: color-mix(in srgb, var(--accent) 4%, transparent); }
.dm-conv-row.active { background: color-mix(in srgb, var(--accent) 10%, transparent); }
.dm-conv-avatar {
    width: 36px;
    height: 36px;
    border-radius: 50%;
    background: var(--accent-soft);
    color: var(--accent);
    display: flex;
    align-items: center;
    justify-content: center;
    font-weight: 700;
    font-size: 0.85rem;
    flex-shrink: 0;
}
.dm-conv-body {
    flex: 1;
    min-width: 0;
}
.dm-conv-row-top {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 0.5rem;
}
.dm-conv-name {
    font-family: var(--font-head);
    font-weight: 700;
    font-size: 0.95rem;
    color: var(--text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.dm-conv-time {
    font-size: 0.7rem;
    color: var(--text-3);
    white-space: nowrap;
}
.dm-conv-row-bottom {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 0.5rem;
    margin-top: 0.15rem;
}
.dm-conv-preview {
    font-size: 0.8rem;
    color: var(--text-2);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    flex: 1;
}
.dm-conv-unread {
    background: var(--accent);
    color: var(--accent-fg);
    font-size: 0.7rem;
    font-weight: 700;
    border-radius: var(--radius-pill);
    padding: 0.1rem 0.5rem;
    flex-shrink: 0;
}
.dm-conv-empty {
    padding: 2.5rem 1rem;
    text-align: center;
    color: var(--text-3);
    font-size: 0.85rem;
}
.dm-conv-empty-cta {
    margin-top: 0.75rem;
    background: var(--accent);
    color: var(--accent-fg);
    border: none;
    border-radius: 6px;
    padding: 0.5rem 1rem;
    font-size: 0.85rem;
    font-weight: 600;
    cursor: pointer;
}
.dm-conv-empty-cta:hover { filter: brightness(1.15); }
"#;

#[component]
pub fn ConversationList(
    conversations: Vec<ConversationSummary>,
    selected_peer: Option<String>,
    refreshing: bool,
    on_refresh: EventHandler<()>,
    on_compose: EventHandler<()>,
) -> Element {
    rsx! {
        style { {CONVERSATION_LIST_CSS} }
        div { class: "dm-conv-list",
            div { class: "dm-conv-list-header",
                h2 { class: "dm-conv-list-title", "Messages" }
                button {
                    class: "dm-conv-list-refresh",
                    disabled: refreshing,
                    onclick: move |_| on_refresh.call(()),
                    if refreshing { "Syncing…" } else { "Refresh" }
                }
            }
            if conversations.is_empty() {
                div { class: "dm-conv-empty",
                    "No conversations yet."
                    div {
                        button {
                            class: "dm-conv-empty-cta",
                            onclick: move |_| on_compose.call(()),
                            "Start a conversation"
                        }
                    }
                }
            } else {
                for c in conversations.iter() {
                    {render_row(c, selected_peer.as_deref())}
                }
            }
        }
    }
}

fn render_row(c: &ConversationSummary, selected: Option<&str>) -> Element {
    let display_name = c
        .peer_display_name
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| truncate_pubkey(&c.peer_pubkey));
    let initials = display_name
        .chars()
        .next()
        .unwrap_or('?')
        .to_uppercase()
        .to_string();
    let time = relative_time(&c.last_message_at);
    let is_active = selected == Some(c.peer_pubkey.as_str());
    let cls = if is_active {
        "dm-conv-row active"
    } else {
        "dm-conv-row"
    };
    let unread = c.unread_count;

    rsx! {
        Link {
            to: Route::DmThread { peer_pubkey: c.peer_pubkey.clone() },
            class: "{cls}",
            div { class: "dm-conv-avatar", "{initials}" }
            div { class: "dm-conv-body",
                div { class: "dm-conv-row-top",
                    span { class: "dm-conv-name", "{display_name}" }
                    span { class: "dm-conv-time", "{time}" }
                }
                div { class: "dm-conv-row-bottom",
                    span { class: "dm-conv-preview", "{c.last_message_preview}" }
                    if unread > 0 {
                        span { class: "dm-conv-unread", "{unread}" }
                    }
                }
            }
        }
    }
}
