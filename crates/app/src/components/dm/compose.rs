use dioxus::prelude::*;
use scuffed_api_client::{ApiClient, ClientError};
use serde::{Deserialize, Serialize};

use super::types::{DmMessage, normalize_recipient_pubkey, truncate_pubkey};
use crate::components::{Toast, use_toast};
use crate::routes::Route;

const COMPOSE_CSS: &str = r#"
.dm-compose-overlay {
    position: fixed;
    inset: 0;
    background: var(--overlay);
    z-index: 1000;
    display: flex;
    align-items: center;
    justify-content: center;
}
.dm-compose-modal {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 12px;
    width: min(560px, 92vw);
    max-height: 85vh;
    display: flex;
    flex-direction: column;
    overflow: hidden;
}
.dm-compose-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 1rem 1.25rem;
    border-bottom: 1px solid var(--border);
}
.dm-compose-title {
    font-family: var(--font-head);
    font-weight: 700;
    font-size: 1.1rem;
    color: var(--text);
    margin: 0;
    letter-spacing: 0.02em;
}
.dm-compose-close {
    background: transparent;
    border: none;
    color: var(--text-3);
    font-size: 1.25rem;
    cursor: pointer;
    padding: 0.25rem 0.5rem;
    border-radius: 4px;
}
.dm-compose-close:hover { color: var(--text); background: var(--surface-2); }
.dm-compose-body { padding: 1rem 1.25rem; display: flex; flex-direction: column; gap: 0.85rem; overflow-y: auto; }
.dm-compose-banner {
    background: color-mix(in srgb, var(--danger) 8%, transparent);
    border: 1px solid color-mix(in srgb, var(--danger) 40%, transparent);
    border-radius: 8px;
    padding: 0.75rem 1rem;
    color: var(--danger);
    font-size: 0.85rem;
}
.dm-compose-banner a { color: var(--danger); font-weight: 600; text-decoration: underline; }
.dm-compose-field { display: flex; flex-direction: column; gap: 0.35rem; position: relative; }
.dm-compose-label {
    font-family: var(--font-head);
    font-weight: 600;
    font-size: 0.8rem;
    color: var(--text-2);
    text-transform: uppercase;
    letter-spacing: 0.05em;
}
.dm-compose-input,
.dm-compose-textarea {
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 0.6rem 0.75rem;
    color: var(--text);
    font-size: 0.9rem;
    font-family: var(--font-body);
}
.dm-compose-textarea { min-height: 110px; resize: vertical; line-height: 1.5; }
.dm-compose-input:focus,
.dm-compose-textarea:focus { outline: none; border-color: var(--accent); }
.dm-compose-input::placeholder,
.dm-compose-textarea::placeholder { color: var(--text-3); }
.dm-compose-suggestions {
    position: absolute;
    top: 100%;
    left: 0;
    right: 0;
    margin-top: 0.25rem;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 8px;
    max-height: 220px;
    overflow-y: auto;
    z-index: 10;
    box-shadow: 0 8px 24px var(--overlay);
}
.dm-compose-suggestion {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
    padding: 0.55rem 0.75rem;
    cursor: pointer;
    border-bottom: 1px solid var(--border);
}
.dm-compose-suggestion:last-child { border-bottom: none; }
.dm-compose-suggestion:hover { background: color-mix(in srgb, var(--accent) 12%, transparent); }
.dm-compose-suggestion-name { font-weight: 600; font-size: 0.9rem; color: var(--text); }
.dm-compose-suggestion-pk { font-family: var(--font-mono); font-size: 0.7rem; color: var(--text-3); }
.dm-compose-counter {
    align-self: flex-end;
    font-size: 0.7rem;
    color: var(--text-3);
}
.dm-compose-counter.over { color: var(--danger); }
.dm-compose-footer {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 0.85rem 1.25rem;
    border-top: 1px solid var(--border);
    gap: 0.75rem;
}
.dm-compose-hint { font-size: 0.7rem; color: var(--text-3); }
.dm-compose-actions { display: flex; gap: 0.5rem; }
.dm-compose-cancel,
.dm-compose-submit {
    border: none;
    border-radius: 6px;
    padding: 0.5rem 1.1rem;
    font-size: 0.85rem;
    font-weight: 600;
    cursor: pointer;
}
.dm-compose-cancel { background: transparent; color: var(--text-2); border: 1px solid var(--border); }
.dm-compose-cancel:hover { color: var(--text); border-color: var(--accent-soft); }
.dm-compose-submit { background: var(--accent); color: var(--accent-fg); }
.dm-compose-submit:hover:not(:disabled) { filter: brightness(1.15); }
.dm-compose-submit:disabled { opacity: 0.5; cursor: not-allowed; }
"#;

const MAX_BODY_CHARS: usize = 2000;

#[derive(Debug, Clone, Deserialize)]
struct MemberLite {
    #[allow(dead_code)]
    id: String,
    display_name: String,
    #[serde(default)]
    nostr_pubkey: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct MemberListResponse {
    data: Vec<MemberLite>,
}

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
pub fn DmComposeModal(
    open: bool,
    self_pubkey: Option<String>,
    on_close: EventHandler<()>,
    /// Fired with `(recipient_pubkey_hex, freshly_sent_message)` after a
    /// successful POST. Parent decides whether to navigate, push to thread
    /// cache, etc.
    on_sent: EventHandler<(String, DmMessage)>,
) -> Element {
    if !open {
        return rsx! {};
    }

    let mut recipient_input = use_signal(String::new);
    let mut body = use_signal(String::new);
    let mut submitting = use_signal(|| false);
    let mut needs_server_managed = use_signal(|| false);
    let mut suggestions_open = use_signal(|| false);
    let mut toast = use_toast();

    let members = use_resource(move || async move {
        match ApiClient::web()
            .fetch::<MemberListResponse>("/api/members?limit=200")
            .await
        {
            Ok(resp) => resp
                .data
                .into_iter()
                .filter(|m| {
                    m.nostr_pubkey
                        .as_deref()
                        .is_some_and(|p| !p.trim().is_empty())
                })
                .collect::<Vec<_>>(),
            Err(_) => Vec::new(),
        }
    });

    let do_send = use_callback({
        let self_pubkey = self_pubkey.clone();
        move |_: ()| {
            if submitting() {
                return;
            }
            let recipient_trimmed = recipient_input().trim().to_string();
            let content = body().trim().to_string();
            if content.is_empty() || content.chars().count() > MAX_BODY_CHARS {
                return;
            }
            let members_snapshot: Vec<MemberLite> = members
                .read()
                .as_deref()
                .map(|v| v.to_vec())
                .unwrap_or_default();
            let Some(recipient_hex) = resolve_recipient(&recipient_trimmed, &members_snapshot)
            else {
                toast.show(Toast::error("Recipient not recognized"));
                return;
            };

            submitting.set(true);
            needs_server_managed.set(false);
            let self_pk = self_pubkey.clone();
            spawn(async move {
                let result = ApiClient::web()
                    .post_json::<_, DmSendResponse>(
                        "/api/nostr/dm/send",
                        &DmSendBody {
                            recipient_pubkey: recipient_hex.clone(),
                            content: content.clone(),
                            reply_to_event_id: None,
                        },
                    )
                    .await;

                submitting.set(false);
                match result {
                    Ok(resp) => {
                        let now = chrono::Utc::now().to_rfc3339();
                        let gift_wrap_id = resp.gift_wrap_id;
                        let optimistic = DmMessage {
                            id: gift_wrap_id.clone(),
                            gift_wrap_id,
                            sender_pubkey: self_pk.unwrap_or_default(),
                            recipient_pubkey: recipient_hex.clone(),
                            content,
                            created_at: now,
                            reply_to_event_id: None,
                        };
                        body.set(String::new());
                        recipient_input.set(String::new());
                        on_sent.call((recipient_hex, optimistic));
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

    let recipient_value = recipient_input();
    let recipient_trimmed = recipient_value.trim().to_string();
    let body_text = body();
    let body_len = body_text.chars().count();
    let body_over = body_len > MAX_BODY_CHARS;
    let members_snapshot: Vec<MemberLite> = members
        .read()
        .as_deref()
        .map(|v| v.to_vec())
        .unwrap_or_default();
    let resolved_recipient = resolve_recipient(&recipient_trimmed, &members_snapshot);
    let can_submit =
        !submitting() && !body_over && !body_text.trim().is_empty() && resolved_recipient.is_some();

    let query = recipient_trimmed.to_lowercase();
    let filtered: Vec<MemberLite> = if looks_like_pubkey(&query) {
        Vec::new()
    } else {
        members_snapshot
            .iter()
            .filter(|m| query.is_empty() || m.display_name.to_lowercase().contains(&query))
            .take(8)
            .cloned()
            .collect()
    };

    rsx! {
        style { {COMPOSE_CSS} }
        div {
            class: "dm-compose-overlay",
            onclick: move |_| {
                if !submitting() {
                    on_close.call(());
                }
            },
            div {
                class: "dm-compose-modal",
                onclick: move |e| e.stop_propagation(),
                div { class: "dm-compose-header",
                    h2 { class: "dm-compose-title", "New message" }
                    button {
                        class: "dm-compose-close",
                        disabled: submitting(),
                        onclick: move |_| on_close.call(()),
                        "\u{00d7}"
                    }
                }
                div { class: "dm-compose-body",
                    if needs_server_managed() {
                        div { class: "dm-compose-banner",
                            "Direct messages require a server-managed Nostr identity. "
                            Link { to: Route::IdentitySettings {}, "Visit identity settings" }
                            " to enable it."
                        }
                    }
                    div { class: "dm-compose-field",
                        label { class: "dm-compose-label", "To" }
                        input {
                            class: "dm-compose-input",
                            r#type: "text",
                            placeholder: "npub1…, hex pubkey, or member name",
                            value: "{recipient_value}",
                            autocomplete: "off",
                            oninput: move |e| {
                                recipient_input.set(e.value());
                                suggestions_open.set(true);
                            },
                            onfocus: move |_| suggestions_open.set(true),
                            onblur: move |_| {
                                // Delay close so a click on a suggestion still
                                // registers before the dropdown unmounts.
                                spawn(async move {
                                    gloo_timers::future::TimeoutFuture::new(120).await;
                                    suggestions_open.set(false);
                                });
                            },
                        }
                        if suggestions_open() && !filtered.is_empty() {
                            div { class: "dm-compose-suggestions",
                                for m in filtered.iter() {
                                    {
                                        let pk = m.nostr_pubkey.clone().unwrap_or_default();
                                        let pk_short = truncate_pubkey(&pk);
                                        rsx! {
                                            div {
                                                key: "{m.id}",
                                                class: "dm-compose-suggestion",
                                                // mousedown beats blur so the picker actually fires
                                                onmousedown: move |e| {
                                                    e.prevent_default();
                                                    recipient_input.set(pk.clone());
                                                    suggestions_open.set(false);
                                                },
                                                span { class: "dm-compose-suggestion-name", "{m.display_name}" }
                                                span { class: "dm-compose-suggestion-pk", "{pk_short}" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    div { class: "dm-compose-field",
                        label { class: "dm-compose-label", "Message" }
                        textarea {
                            class: "dm-compose-textarea",
                            placeholder: "Write a message…",
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
                        {
                            let cls = if body_over { "dm-compose-counter over" } else { "dm-compose-counter" };
                            rsx! { span { class: "{cls}", "{body_len}/{MAX_BODY_CHARS}" } }
                        }
                    }
                }
                div { class: "dm-compose-footer",
                    span { class: "dm-compose-hint", "Cmd/Ctrl+Enter to send" }
                    div { class: "dm-compose-actions",
                        button {
                            class: "dm-compose-cancel",
                            disabled: submitting(),
                            onclick: move |_| on_close.call(()),
                            "Cancel"
                        }
                        button {
                            class: "dm-compose-submit",
                            disabled: !can_submit,
                            onclick: move |_| do_send.call(()),
                            if submitting() { "Sending…" } else { "Send" }
                        }
                    }
                }
            }
        }
    }
}

fn looks_like_pubkey(s: &str) -> bool {
    let s = s.trim();
    s.starts_with("npub1") || (s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit()))
}

/// Resolve free-form recipient input to a 64-char hex pubkey.
///
/// Accepts: `npub1…`, raw hex, or an exact-or-fuzzy match against a member's
/// display name (when the member has a `nostr_pubkey`). Returns `None` while
/// the input is still ambiguous so the submit button stays disabled.
fn resolve_recipient(input: &str, members: &[MemberLite]) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(hex) = normalize_recipient_pubkey(trimmed) {
        return Some(hex);
    }
    let lowered = trimmed.to_lowercase();
    if let Some(m) = members
        .iter()
        .find(|m| m.display_name.to_lowercase() == lowered)
    {
        return m.nostr_pubkey.clone();
    }
    let matches: Vec<&MemberLite> = members
        .iter()
        .filter(|m| m.display_name.to_lowercase().contains(&lowered))
        .collect();
    if matches.len() == 1 {
        return matches[0].nostr_pubkey.clone();
    }
    None
}
