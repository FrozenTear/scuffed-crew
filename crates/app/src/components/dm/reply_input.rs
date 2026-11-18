use dioxus::prelude::*;
use scuffed_api_client::{ApiClient, ClientError};
use serde::{Deserialize, Serialize};

use super::types::DmMessage;
use crate::components::{Toast, use_toast};
use crate::routes::Route;

/// One composer per page, so fixed ids.
pub const REPLY_INPUT_ID: &str = "dm-reply-input";
pub const REPLY_STATUS_ID: &str = "dm-reply-status";
const REPLY_HINT_ID: &str = "dm-reply-hint";
pub(crate) const REPLY_INPUT_CSS_ID: &str = "scuffed-css-components-dm-reply_input-reply-input-css";

/// The composer of the `d1-dm` frame: a field and Send on one row, pinned
/// under the thread (on phone, just above the tab bar). Under it, an
/// always-rendered status line and, with a keyboard, the send shortcut.
pub(crate) const REPLY_INPUT_CSS: &str = r#"
.dm-reply {
    flex: 0 0 auto;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
    border-top: 1px solid var(--border);
    background: var(--surface);
}
.dm-reply-row {
    display: flex;
    gap: var(--space-2);
    align-items: flex-end;
}
.dm-reply textarea.field-input {
    flex: 1 1 auto;
    width: auto;
    min-width: 0;
    min-height: var(--hit-target-min);
    max-height: calc(var(--hit-target-min) * 4);
    padding: var(--space-2) var(--space-3);
    resize: vertical;
}
.dm-reply-row .btn { flex: 0 0 auto; }
.dm-reply-meta {
    display: flex;
    justify-content: space-between;
    gap: var(--space-3);
    margin: 0;
    color: var(--text-3);
    font-size: var(--text-xs);
}
.dm-reply-count { margin-inline-start: auto; }
.dm-reply-count.is-over { color: var(--loss); font-weight: var(--font-label-weight); }
.dm-reply-status {
    margin: 0;
    color: var(--text-3);
    font-size: var(--text-sm);
    line-height: var(--leading-body);
}
.dm-reply-status.is-error { color: var(--loss); }
.dm-reply-status a { color: var(--link); text-decoration: underline; }
/* No status: out of the layout, still in the tree so the next one is
   announced. */
.dm-reply-status:empty {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
    border: 0;
}
/* Phones have no Ctrl or Cmd key to speak of: Send is the way to send. */
@media (max-width: 820px) {
    .dm-reply-hint { display: none; }
    /* With the hint hidden and no count, the row is empty: drop it, so its
       gap does not pad the bottom of the composer. */
    .dm-reply-meta:not(:has(.dm-reply-count)) { display: none; }
}
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

/// What the composer's status line says.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SendState {
    Idle,
    Sending,
    /// The send was not tried: why (empty, too long).
    Invalid(&'static str),
    /// 412: the account has no server-managed key.
    NeedsIdentity,
    /// The request failed. Keeps the server's reason.
    Failed(String),
}

impl SendState {
    pub(crate) fn failed(reason: impl std::fmt::Display) -> Self {
        let reason = reason.to_string();
        let reason = reason.trim();
        if reason.is_empty() {
            SendState::Failed("Could not send the message.".to_string())
        } else {
            SendState::Failed(format!("Could not send the message. {reason}"))
        }
    }

    pub(crate) fn is_error(&self) -> bool {
        matches!(
            self,
            SendState::Invalid(_) | SendState::NeedsIdentity | SendState::Failed(_)
        )
    }
}

/// Why a body cannot be sent, or `None` when it can.
pub(crate) fn body_problem(body: &str) -> Option<&'static str> {
    if body.trim().is_empty() {
        Some("Write a message first.")
    } else if body.chars().count() > MAX_BODY_CHARS {
        Some("The message is over 2000 characters. Shorten it to send.")
    } else {
        None
    }
}

/// The status line of a send. Always rendered; only its content changes.
#[component]
pub(crate) fn SendStatus(id: String, state: SendState) -> Element {
    let class = if state.is_error() {
        "dm-reply-status is-error"
    } else {
        "dm-reply-status"
    };
    rsx! {
        p { id, class, role: "status",
            {match state {
                SendState::Idle => rsx! {},
                SendState::Sending => rsx! { "Sending..." },
                SendState::Invalid(reason) => rsx! { "{reason}" },
                SendState::Failed(reason) => rsx! { "{reason}" },
                SendState::NeedsIdentity => rsx! {
                    "Direct messages need a server-managed Nostr identity. "
                    Link { to: Route::IdentitySettings {}, "Open account keys" }
                    " to turn it on."
                },
            }}
        }
    }
}

pub(crate) fn is_send_shortcut(evt: &Event<KeyboardData>) -> bool {
    let modifiers = evt.modifiers();
    matches!(evt.key(), Key::Enter) && (modifiers.ctrl() || modifiers.meta())
}

#[component]
pub fn ReplyInput(
    peer_pubkey: String,
    /// The peer's name, for the field's label.
    peer_label: String,
    self_pubkey: Option<String>,
    /// The most recent gift_wrap_id in the thread, used as `reply_to_event_id`
    /// when the user sends. None for a brand-new conversation.
    reply_to_event_id: Option<String>,
    /// Fired when a send succeeds; the parent appends the message to its
    /// thread cache for optimistic UI. The DmMessage uses the returned
    /// gift_wrap_id as both `id` and `gift_wrap_id`; the next dm/sync will
    /// reconcile against the server's row.
    on_optimistic: EventHandler<DmMessage>,
) -> Element {
    let mut body = use_signal(String::new);
    let mut state = use_signal(|| SendState::Idle);
    let mut toast = use_toast();

    let do_send = use_callback({
        let peer_pubkey = peer_pubkey.clone();
        let self_pubkey = self_pubkey.clone();
        let reply_to_event_id = reply_to_event_id.clone();
        move |_: ()| {
            if state() == SendState::Sending {
                return;
            }
            let content = body().trim().to_string();
            if let Some(problem) = body_problem(&body()) {
                state.set(SendState::Invalid(problem));
                return;
            }
            let recipient = peer_pubkey.clone();
            let self_pk = self_pubkey.clone();
            let reply_to = reply_to_event_id.clone();

            state.set(SendState::Sending);
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

                match result {
                    Ok(resp) => {
                        state.set(SendState::Idle);
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
                        state.set(SendState::NeedsIdentity);
                    }
                    Err(e) => {
                        toast.show(Toast::error(format!("Failed to send: {e}")));
                        state.set(SendState::failed(&e));
                    }
                }
            });
        }
    });

    let body_text = body();
    let body_len = body_text.chars().count();
    let body_over = body_len > MAX_BODY_CHARS;
    let sending = state() == SendState::Sending;
    let can_submit = !sending && body_problem(&body_text).is_none();
    let counter_cls = if body_over {
        "dm-reply-count is-over"
    } else {
        "dm-reply-count"
    };

    rsx! {
        crate::styles::StyleOnce { id: REPLY_INPUT_CSS_ID, css: REPLY_INPUT_CSS }
        div { class: "dm-reply",
            label { class: "visually-hidden", r#for: REPLY_INPUT_ID, "Message to {peer_label}" }
            div { class: "dm-reply-row",
                textarea {
                    id: REPLY_INPUT_ID,
                    class: "field-input",
                    rows: "1",
                    placeholder: "Write a reply",
                    aria_describedby: "{REPLY_STATUS_ID} {REPLY_HINT_ID}",
                    aria_invalid: if body_over { "true" },
                    value: "{body_text}",
                    oninput: move |e| {
                        body.set(e.value());
                        if state.peek().is_error() {
                            state.set(SendState::Idle);
                        }
                    },
                    onkeydown: move |evt: Event<KeyboardData>| {
                        if is_send_shortcut(&evt) {
                            evt.prevent_default();
                            do_send.call(());
                        }
                    },
                }
                button {
                    class: "btn is-primary",
                    r#type: "button",
                    // Not `disabled`: focus stays on Send while it sends, and
                    // a click on an empty field says why nothing happened.
                    aria_disabled: if !can_submit { "true" },
                    aria_busy: if sending { "true" },
                    onclick: move |_| do_send.call(()),
                    if sending { "Sending..." } else { "Send" }
                }
            }
            SendStatus { id: REPLY_STATUS_ID.to_string(), state: state() }
            p { class: "dm-reply-meta",
                span { id: REPLY_HINT_ID, class: "dm-reply-hint", "Ctrl+Enter or Cmd+Enter sends." }
                if body_len > 0 {
                    span { class: "{counter_cls}", "{body_len} of {MAX_BODY_CHARS} characters" }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::ui::test_support::{assert_sizes_from_tokens, render_routed};

    #[test]
    fn body_problems_say_why_nothing_was_sent() {
        assert_eq!(body_problem("   "), Some("Write a message first."));
        assert!(body_problem(&"x".repeat(MAX_BODY_CHARS + 1)).is_some());
        assert_eq!(body_problem("Hey, want to duo later?"), None);
    }

    #[test]
    fn a_failed_send_keeps_the_server_reason() {
        let state = SendState::failed("HTTP 502: relay unreachable");
        assert_eq!(
            state,
            SendState::Failed("Could not send the message. HTTP 502: relay unreachable".into())
        );
        assert!(state.is_error());
        assert!(!SendState::Sending.is_error());
    }

    #[test]
    fn composer_has_a_label_a_status_line_and_no_phone_shortcut() {
        let html = render_routed(|| {
            rsx! {
                crate::components::ToastProvider {
                    ReplyInput {
                        peer_pubkey: "ab".repeat(32),
                        peer_label: "Player02",
                        self_pubkey: None,
                        reply_to_event_id: None,
                        on_optimistic: move |_| {},
                    }
                }
            }
        });
        assert!(
            html.contains(&format!("<label class=\"visually-hidden\" for=\"{REPLY_INPUT_ID}\">Message to Player02</label>")),
            "{html}"
        );
        assert!(html.contains(&format!("id=\"{REPLY_INPUT_ID}\"")), "{html}");
        // Empty but rendered, so the first error is announced.
        assert!(
            html.contains(&format!(
                "<p id=\"{REPLY_STATUS_ID}\" class=\"dm-reply-status\" role=\"status\">"
            )),
            "{html}"
        );
        assert!(!html.contains("Cmd/Ctrl"), "{html}");
        assert!(!html.contains(" disabled"), "{html}");
        let phone = REPLY_INPUT_CSS
            .split("@media (max-width: 820px)")
            .nth(1)
            .expect("phone block");
        assert!(phone.contains(".dm-reply-hint { display: none; }"));
    }

    #[test]
    fn composer_css_is_on_tokens() {
        assert_sizes_from_tokens(REPLY_INPUT_CSS);
    }
}
