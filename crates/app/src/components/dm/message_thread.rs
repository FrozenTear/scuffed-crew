use dioxus::prelude::*;

use scuffed_api_client::{ApiClient, ClientError};

use super::reply_input::ReplyInput;
use super::types::{DmMessage, MarkReadBody, relative_time, truncate_pubkey};

const MESSAGE_THREAD_CSS: &str = r#"
.dm-thread {
    display: flex;
    flex-direction: column;
    height: 100%;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 10px;
    overflow: hidden;
}
.dm-thread-header {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 1rem;
    border-bottom: 1px solid var(--border);
}
.dm-thread-avatar {
    width: 36px;
    height: 36px;
    border-radius: 50%;
    background: var(--accent-soft);
    color: var(--accent-bright);
    display: flex;
    align-items: center;
    justify-content: center;
    font-weight: 700;
    font-size: 0.85rem;
    flex-shrink: 0;
}
.dm-thread-name {
    font-family: 'Rajdhani', sans-serif;
    font-weight: 700;
    font-size: 1.05rem;
    color: var(--text-bright);
}
.dm-thread-pubkey {
    font-family: monospace;
    font-size: 0.7rem;
    color: var(--text-muted);
}
.dm-thread-scroll {
    flex: 1;
    overflow-y: auto;
    padding: 1rem;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
}
.dm-thread-load-older {
    align-self: center;
    background: transparent;
    color: var(--text-secondary);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 0.35rem 0.75rem;
    font-size: 0.75rem;
    cursor: pointer;
    margin-bottom: 0.5rem;
}
.dm-thread-load-older:hover { color: var(--text-bright); border-color: var(--accent-soft); }
.dm-thread-load-older:disabled { opacity: 0.5; cursor: not-allowed; }
.dm-msg {
    display: flex;
    flex-direction: column;
    max-width: 75%;
    padding: 0.55rem 0.75rem;
    border-radius: 10px;
    background: var(--bg-surface, #1a1a2e);
    border: 1px solid var(--border);
    word-wrap: break-word;
    overflow-wrap: anywhere;
}
.dm-msg.outgoing {
    align-self: flex-end;
    background: rgba(124, 58, 237, 0.15);
    border-color: rgba(124, 58, 237, 0.4);
}
.dm-msg-meta {
    font-size: 0.65rem;
    color: var(--text-muted);
    margin-top: 0.25rem;
    display: flex;
    justify-content: flex-end;
    gap: 0.4rem;
}
.dm-msg-content { font-size: 0.9rem; color: var(--text-bright); white-space: pre-wrap; }
.dm-thread-empty {
    display: flex;
    flex: 1;
    align-items: center;
    justify-content: center;
    color: var(--text-muted);
    font-size: 0.9rem;
    padding: 2rem;
    text-align: center;
}
.dm-thread-loading {
    display: flex;
    flex: 1;
    align-items: center;
    justify-content: center;
    color: var(--text-muted);
    font-size: 0.9rem;
}
"#;

#[component]
pub fn MessageThread(
    peer_pubkey: String,
    peer_display_name: Option<String>,
    self_pubkey: Option<String>,
) -> Element {
    let peer = peer_pubkey.clone();
    let peer_for_resource = peer.clone();
    let self_pk = self_pubkey.clone();

    let mut messages = use_signal::<Vec<DmMessage>>(Vec::new);
    let mut loading_initial = use_signal(|| true);
    let mut loading_older = use_signal(|| false);
    let mut has_more = use_signal(|| true);
    let mut last_marked_until = use_signal(|| None::<String>);

    // Load initial page when peer changes.
    let _load_initial = use_resource(move || {
        let peer = peer_for_resource.clone();
        async move {
            loading_initial.set(true);
            messages.set(Vec::new());
            has_more.set(true);
            match fetch_thread(&peer, None, 50).await {
                Ok(page) => {
                    if page.len() < 50 {
                        has_more.set(false);
                    }
                    let mut sorted = page;
                    sorted.sort_by(|a, b| a.created_at.cmp(&b.created_at));
                    let until = sorted.last().map(|m| m.created_at.clone());
                    messages.set(sorted);
                    loading_initial.set(false);
                    if let Some(ts) = until
                        && last_marked_until() != Some(ts.clone())
                    {
                        mark_read(&peer, &ts).await;
                        last_marked_until.set(Some(ts));
                    }
                }
                Err(_) => {
                    loading_initial.set(false);
                }
            }
        }
    });

    let peer_for_older = peer.clone();
    let load_older = move |_| {
        if loading_older() || !has_more() {
            return;
        }
        let oldest = messages.read().first().map(|m| m.created_at.clone());
        let Some(before) = oldest else { return };
        let peer = peer_for_older.clone();
        loading_older.set(true);
        spawn(async move {
            match fetch_thread(&peer, Some(&before), 50).await {
                Ok(page) => {
                    if page.len() < 50 {
                        has_more.set(false);
                    }
                    let mut combined = page;
                    combined.sort_by(|a, b| a.created_at.cmp(&b.created_at));
                    let mut current = messages();
                    combined.append(&mut current);
                    messages.set(combined);
                }
                Err(_) => {
                    has_more.set(false);
                }
            }
            loading_older.set(false);
        });
    };

    let display_name = peer_display_name
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| truncate_pubkey(&peer));
    let initials = display_name
        .chars()
        .next()
        .unwrap_or('?')
        .to_uppercase()
        .to_string();

    let msgs = messages();
    let is_loading_initial = loading_initial();
    let reply_to = msgs.last().map(|m| m.gift_wrap_id.clone());

    let on_optimistic = move |msg: DmMessage| {
        // Append to the local thread cache so the UI updates without waiting
        // for the next /api/nostr/dm/sync. The unique gift_wrap_id index on
        // the server side dedupes when the next sync re-stores the message.
        let mut current = messages();
        current.push(msg);
        messages.set(current);
    };

    rsx! {
        style { {MESSAGE_THREAD_CSS} }
        div { class: "dm-thread",
            div { class: "dm-thread-header",
                div { class: "dm-thread-avatar", "{initials}" }
                div {
                    div { class: "dm-thread-name", "{display_name}" }
                    div { class: "dm-thread-pubkey", "{truncate_pubkey(&peer)}" }
                }
            }
            div { class: "dm-thread-scroll",
                if has_more() && !is_loading_initial && !msgs.is_empty() {
                    button {
                        class: "dm-thread-load-older",
                        disabled: loading_older(),
                        onclick: load_older,
                        if loading_older() { "Loading…" } else { "Load older messages" }
                    }
                }
                if is_loading_initial {
                    div { class: "dm-thread-loading", "Loading messages…" }
                } else if msgs.is_empty() {
                    div { class: "dm-thread-empty",
                        "No messages with this peer yet."
                    }
                } else {
                    for m in msgs.iter() {
                        {render_message(m, self_pk.as_deref())}
                    }
                }
            }
            ReplyInput {
                peer_pubkey: peer.clone(),
                self_pubkey: self_pk.clone(),
                reply_to_event_id: reply_to,
                on_optimistic: on_optimistic,
            }
        }
    }
}

fn render_message(m: &DmMessage, self_pubkey: Option<&str>) -> Element {
    let outgoing = self_pubkey
        .map(|pk| pk.eq_ignore_ascii_case(&m.sender_pubkey))
        .unwrap_or(false);
    let cls = if outgoing {
        "dm-msg outgoing"
    } else {
        "dm-msg"
    };
    let time = relative_time(&m.created_at);
    rsx! {
        div { key: "{m.id}", class: "{cls}",
            div { class: "dm-msg-content", "{m.content}" }
            div { class: "dm-msg-meta", "{time}" }
        }
    }
}

async fn fetch_thread(
    peer: &str,
    before_ts: Option<&str>,
    limit: usize,
) -> Result<Vec<DmMessage>, ClientError> {
    let mut url = format!(
        "/api/nostr/dm/thread?peer_pubkey={}&limit={}",
        urlencoding_encode(peer),
        limit
    );
    if let Some(ts) = before_ts {
        url.push_str(&format!("&before_ts={}", urlencoding_encode(ts)));
    }
    ApiClient::web().fetch::<Vec<DmMessage>>(&url).await
}

async fn mark_read(peer: &str, until_ts: &str) {
    let body = MarkReadBody {
        peer_pubkey: peer.to_string(),
        until_ts: until_ts.to_string(),
    };
    let _ = ApiClient::web()
        .post_json::<_, serde_json::Value>("/api/nostr/dm/mark-read", &body)
        .await;
}

fn urlencoding_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}
