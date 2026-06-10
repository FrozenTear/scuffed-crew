use dioxus::prelude::*;
use scuffed_api_client::{ApiClient, ClientError};
use serde::{Deserialize, Serialize};

use super::types::DmMessage;
use crate::components::{Toast, use_toast};
use crate::routes::Route;

const REPLY_INPUT_CSS: &str = r#"
.dm-reply {
    border-top: 1px solid var(--border);
    padding: 0.65rem 0.75rem 0.75rem;
    display: flex;
    flex-direction: column;
    gap: 0.45rem;
    background: var(--surface);
}
.dm-reply-banner {
    background: color-mix(in srgb, var(--danger) 8%, transparent);
    border: 1px solid color-mix(in srgb, var(--danger) 40%, transparent);
    border-radius: 8px;
    padding: 0.5rem 0.75rem;
    color: var(--danger);
    font-size: 0.8rem;
}
.dm-reply-banner a { color: var(--danger); font-weight: 600; text-decoration: underline; }
.dm-reply-row {
    display: flex;
    gap: 0.5rem;
    align-items: stretch;
}
.dm-reply-textarea {
    flex: 1;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 0.55rem 0.7rem;
    color: var(--text);
    font-size: 0.9rem;
    font-family: var(--font-body);
    line-height: 1.5;
    min-height: 44px;
    max-height: 160px;
    resize: vertical;
}
.dm-reply-textarea:focus { outline: none; border-color: var(--accent); }
.dm-reply-textarea::placeholder { color: var(--text-3); }
.dm-reply-send {
    background: var(--accent);
    color: var(--accent-fg);
    border: none;
    border-radius: 8px;
    padding: 0 1.1rem;
    font-weight: 600;
    font-size: 0.85rem;
    cursor: pointer;
    align-self: flex-end;
    height: 44px;
}
.dm-reply-send:hover:not(:disabled) { filter: brightness(1.15); }
.dm-reply-send:disabled { opacity: 0.5; cursor: not-allowed; }
.dm-reply-meta {
    display: flex;
    justify-content: space-between;
    font-size: 0.7rem;
    color: var(--text-3);
}
.dm-reply-counter.over { color: var(--danger); }
"#;

const MAX_BODY_CHARS: usize = 2000;

#[derive(Debug, Serialize)]
struct DmSendBody {
    recipient_pubkey: String,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    reply_to_event_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DmSendResponse {
    gift_wrap_id: String,
}

#[component]
pub fn ReplyInput(
    peer_pubkey: String,
    self_pubkey: Option<String>,
    /// The most recent gift_wrap_id in the thread, used as `reply_to_event_id`
    /// when the user sends. None for a brand-new conversation.
    reply_to_event_id: Option<String>,
    /// Fired when a send succeeds — the parent appends the message to its
    /// thread cache for optimistic UI. The DmMessage uses the returned
    /// gift_wrap_id as both `id` and `gift_wrap_id`; the next dm/sync will
    /// reconcile against the server's row.
    on_optimistic: EventHandler<DmMessage>,
) -> Element {
    let mut body = use_signal(String::new);
    let mut submitting = use_signal(|| false);
    let mut needs_server_managed = use_signal(|| false);
    let mut toast = use_toast();

    let do_send = use_callback({
        let peer_pubkey = peer_pubkey.clone();
        let self_pubkey = self_pubkey.clone();
        let reply_to_event_id = reply_to_event_id.clone();
        move |_: ()| {
            if submitting() {
                return;
            }
            let content = body().trim().to_string();
            if content.is_empty() || content.chars().count() > MAX_BODY_CHARS {
                return;
            }
            let recipient = peer_pubkey.clone();
            let self_pk = self_pubkey.clone();
            let reply_to = reply_to_event_id.clone();

            submitting.set(true);
            needs_server_managed.set(false);
            spawn(async move {
                let result = ApiClient::web()
                    .post_json::<_, DmSendResponse>(
                        "/api/nostr/dm/send",
                        &DmSendBody {
                            recipient_pubkey: recipient.clone(),
                            content: content.clone(),
                            reply_to_event_id: reply_to.clone(),
                        },
                    )
                    .await;

                submitting.set(false);
                match result {
                    Ok(resp) => {
                        let gift_wrap_id = resp.gift_wrap_id;
                        let now = chrono::Utc::now().to_rfc3339();
                        let optimistic = DmMessage {
                            id: gift_wrap_id.clone(),
                            gift_wrap_id,
                            sender_pubkey: self_pk.unwrap_or_default(),
                            recipient_pubkey: recipient,
                            content,
                            created_at: now,
                            reply_to_event_id: reply_to,
                        };
                        body.set(String::new());
                        on_optimistic.call(optimistic);
                    }
                    Err(ClientError::Http { status: 412, .. }) => {
                        needs_server_managed.set(true);
                    }
                    Err(e) => {
                        toast.show(Toast::error(format!("Failed to send: {e}")));
                    }
                }
            });
        }
    });

    let body_text = body();
    let body_len = body_text.chars().count();
    let body_over = body_len > MAX_BODY_CHARS;
    let can_submit = !submitting() && !body_over && !body_text.trim().is_empty();
    let counter_cls = if body_over {
        "dm-reply-counter over"
    } else {
        "dm-reply-counter"
    };

    rsx! {
        style { {REPLY_INPUT_CSS} }
        div { class: "dm-reply",
            if needs_server_managed() {
                div { class: "dm-reply-banner",
                    "Direct messages require a server-managed Nostr identity. "
                    Link { to: Route::IdentitySettings {}, "Visit identity settings" }
                    " to enable it."
                }
            }
            div { class: "dm-reply-row",
                textarea {
                    class: "dm-reply-textarea",
                    placeholder: "Write a reply… (Cmd/Ctrl+Enter to send)",
                    value: "{body_text}",
                    oninput: move |e| body.set(e.value()),
                    onkeydown: move |evt: Event<KeyboardData>| {
                        let modifiers = evt.modifiers();
                        if matches!(evt.key(), Key::Enter) && (modifiers.ctrl() || modifiers.meta()) {
                            evt.prevent_default();
                            do_send.call(());
                        }
                    },
                }
                button {
                    class: "dm-reply-send",
                    disabled: !can_submit,
                    onclick: move |_| do_send.call(()),
                    if submitting() { "Sending…" } else { "Send" }
                }
            }
            div { class: "dm-reply-meta",
                span { "Cmd/Ctrl+Enter to send" }
                span { class: "{counter_cls}", "{body_len}/{MAX_BODY_CHARS}" }
            }
        }
    }
}
