use dioxus::prelude::*;
use scuffed_api_client::{ApiClient, ClientError};
use serde::{Deserialize, Serialize};

use super::conversation_list::NEW_MESSAGE_BUTTON_ID;
use super::reply_input::{
    REPLY_INPUT_CSS, REPLY_INPUT_CSS_ID, SendState, SendStatus, body_problem, is_send_shortcut,
};
use super::types::{DmMember, DmMessage, normalize_recipient_pubkey, truncate_pubkey};
use crate::components::admin_shared::trap_tab_in;
use crate::components::ui::inline_confirm::{
    focus_element_by_id, focus_element_by_id_soon, key_cancels,
};
use crate::components::{Toast, use_toast};

/// One New message dialog per page, so fixed ids.
const DIALOG_ID: &str = "dm-compose";
const TITLE_ID: &str = "dm-compose-title";
const TO_ID: &str = "dm-compose-to";
const TO_HINT_ID: &str = "dm-compose-to-hint";
const TO_LIST_ID: &str = "dm-compose-to-list";
const BODY_ID: &str = "dm-compose-body";
const COUNT_ID: &str = "dm-compose-count";
const STATUS_ID: &str = "dm-compose-status";

/// The New message dialog: a modal card on the overlay (a sheet from the
/// bottom on phone) with To and Message fields, a status line and the
/// actions. To is a combobox over the crew members who have a Nostr key.
const COMPOSE_CSS: &str = r#"
.dm-compose-overlay {
    position: fixed;
    inset: 0;
    z-index: 1000;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: var(--space-page);
    background: var(--overlay);
}
.dm-compose {
    box-sizing: border-box;
    width: 100%;
    max-width: var(--content-max-narrow);
    max-height: 100%;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    background: var(--surface);
    color: var(--text);
    border: 1px solid var(--card-border);
    border-radius: var(--card-radius);
    box-shadow: var(--shadow-overlay);
}
.dm-compose-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-card-pad);
    border-bottom: 1px solid var(--border);
}
.dm-compose-title {
    margin: 0;
    font-family: var(--font-head);
    font-weight: var(--font-head-weight);
    font-size: var(--text-xl);
    letter-spacing: var(--font-head-tracking);
    color: var(--text);
}
.dm-compose-body {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
    padding: var(--space-card-pad);
    overflow-y: auto;
}
.dm-compose-field {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
}
.dm-compose-options {
    margin: 0;
    padding: var(--space-1) 0;
    list-style: none;
    max-height: calc(var(--hit-target-min) * 5);
    overflow-y: auto;
    background: var(--surface);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-control);
}
.dm-compose-option {
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    justify-content: center;
    min-height: var(--hit-target-min);
    padding: var(--space-1) var(--space-3);
    cursor: pointer;
}
.dm-compose-option:hover,
.dm-compose-option[aria-selected="true"] { background: var(--accent-soft); }
.dm-compose-option[aria-selected="true"] {
    outline: var(--focus-ring-width) solid var(--focus-ring);
    outline-offset: calc(var(--focus-ring-width) * -1);
}
.dm-compose-option-name {
    color: var(--text);
    font-size: var(--text-md);
    font-weight: var(--font-label-weight);
}
.dm-compose-option-key {
    color: var(--text-3);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
}
.dm-compose-count {
    margin: 0;
    color: var(--text-3);
    font-size: var(--text-xs);
}
.dm-compose-count.is-over { color: var(--loss); font-weight: var(--font-label-weight); }
.dm-compose-foot {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2) var(--space-3);
    padding: var(--space-3) var(--space-card-pad);
    border-top: 1px solid var(--border);
}
.dm-compose-foot .dm-reply-status { flex: 1 1 100%; }
.dm-compose-hint {
    margin: 0 auto 0 0;
    color: var(--text-3);
    font-size: var(--text-xs);
}
.dm-compose-actions {
    display: flex;
    gap: var(--space-2);
    margin-inline-start: auto;
}
@media (max-width: 820px) {
    .dm-compose-hint { display: none; }
    .dm-compose-overlay {
        align-items: flex-end;
        padding: 0;
    }
    .dm-compose {
        max-width: none;
        border-radius: var(--card-radius) var(--card-radius) 0 0;
    }
}
"#;

const MAX_BODY_CHARS: usize = 2000;
/// Suggestions shown at once; typing narrows the rest.
const MAX_SUGGESTIONS: usize = 8;
const SELF_RECIPIENT: &str = "That is your own key. Pick another member.";

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
    /// Crew members, for the To suggestions and name lookup.
    #[props(default)]
    members: Vec<DmMember>,
    on_close: EventHandler<()>,
    /// Fired with `(recipient_pubkey_hex, freshly_sent_message)` after a
    /// successful POST. Parent decides whether to navigate, push to thread
    /// cache, etc.
    on_sent: EventHandler<(String, DmMessage)>,
) -> Element {
    // Hooks must run even while closed. The parent keeps this component
    // mounted and toggles `open`; returning before hooks changes the hook
    // count and panics.
    let mut recipient_input = use_signal(String::new);
    let mut body = use_signal(String::new);
    let mut state = use_signal(|| SendState::Idle);
    let mut suggestions_open = use_signal(|| false);
    let mut active = use_signal(|| None::<usize>);
    // The member picked from the list: (name shown in To, their key). Any
    // later edit of To clears it, so the field shows a name, never a key.
    let mut picked = use_signal(|| None::<(String, String)>);
    // Bumped on focus and input, so a close queued by an earlier blur does
    // not shut the list the visitor is typing into now.
    let mut blur_generation = use_signal(|| 0u64);
    let mut toast = use_toast();

    let members_for_send = others(&members, self_pubkey.as_deref());
    let do_send = use_callback({
        let self_pubkey = self_pubkey.clone();
        move |_: ()| {
            if state() == SendState::Sending {
                return;
            }
            let recipient_trimmed = recipient_input().trim().to_string();
            if recipient_trimmed.is_empty() {
                state.set(SendState::Invalid(
                    "Pick who the message is for: a member name, an npub or a hex key.",
                ));
                return;
            }
            if resolve_to(
                &recipient_trimmed,
                picked.peek().as_ref(),
                &members_for_send,
            )
            .is_some_and(|hex| is_self(&hex, self_pubkey.as_deref()))
            {
                state.set(SendState::Invalid(SELF_RECIPIENT));
                return;
            }
            let Some(recipient_hex) = resolve_to(
                &recipient_trimmed,
                picked.peek().as_ref(),
                &members_for_send,
            ) else {
                toast.show(Toast::error("Recipient not recognized"));
                state.set(SendState::Invalid(
                    "Recipient not recognized. Pick a member from the list, or paste an npub or a hex key.",
                ));
                return;
            };
            if let Some(problem) = body_problem(&body()) {
                state.set(SendState::Invalid(problem));
                return;
            }
            let content = body().trim().to_string();

            state.set(SendState::Sending);
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

                match result {
                    Ok(resp) => {
                        state.set(SendState::Idle);
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
                        picked.set(None);
                        on_sent.call((recipient_hex, optimistic));
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

    if !open {
        return rsx! {};
    }

    let sending = state() == SendState::Sending;
    let mut close = move |_: ()| {
        if *state.peek() == SendState::Sending {
            return;
        }
        state.set(SendState::Idle);
        on_close.call(());
        focus_element_by_id_soon(NEW_MESSAGE_BUTTON_ID.to_string());
    };

    let recipient_value = recipient_input();
    let recipient_trimmed = recipient_value.trim().to_string();
    let body_text = body();
    let body_len = body_text.chars().count();
    let body_over = body_len > MAX_BODY_CHARS;
    // Everyone but the viewer: nobody can pick themselves.
    let candidates = others(&members, self_pubkey.as_deref());
    let picked_now = picked();
    let resolved_any = resolve_to(&recipient_trimmed, picked_now.as_ref(), &candidates);
    let to_self = resolved_any
        .as_deref()
        .is_some_and(|hex| is_self(hex, self_pubkey.as_deref()));
    let resolved_recipient = resolved_any.filter(|_| !to_self);
    let can_submit = !sending && body_problem(&body_text).is_none() && resolved_recipient.is_some();
    let to_hint = if to_self {
        SELF_RECIPIENT.to_string()
    } else {
        recipient_hint(resolved_recipient.as_deref(), &members)
    };

    let (filtered, matching) = suggestions(&candidates, &recipient_trimmed);
    let more_hint = (matching > filtered.len()).then(|| {
        format!(
            "Showing {} of {matching} members. Type more of a name to find the rest.",
            filtered.len()
        )
    });
    let list_open = suggestions_open() && !filtered.is_empty();
    let active_index = active().filter(|i| *i < filtered.len());
    let active_id = active_index
        .filter(|_| list_open)
        .map(|i| option_id(&filtered[i].id));
    // Ids of the shown options, so the arrow keys can bring the highlighted
    // one into view (the list shows five rows of up to eight).
    let shown_option_ids: Vec<String> = filtered.iter().map(|m| option_id(&m.id)).collect();
    let picks: Vec<(String, String)> = filtered
        .iter()
        .map(|m| {
            (
                m.display_name.clone(),
                m.nostr_pubkey.clone().unwrap_or_default(),
            )
        })
        .collect();
    let count_cls = if body_over {
        "dm-compose-count is-over"
    } else {
        "dm-compose-count"
    };

    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-components-dm-compose-compose-css", css: COMPOSE_CSS }
        crate::styles::StyleOnce { id: REPLY_INPUT_CSS_ID, css: REPLY_INPUT_CSS }
        div {
            class: "dm-compose-overlay",
            onclick: move |_| close(()),
            onkeydown: move |evt| {
                if key_cancels(&evt.key()) {
                    evt.stop_propagation();
                    close(());
                } else if evt.key() == Key::Tab
                    && trap_tab_in(DIALOG_ID, evt.modifiers().contains(Modifiers::SHIFT))
                {
                    evt.prevent_default();
                }
            },
            div {
                id: DIALOG_ID,
                class: "dm-compose",
                role: "dialog",
                aria_modal: "true",
                aria_labelledby: TITLE_ID,
                aria_describedby: STATUS_ID,
                onclick: move |e| e.stop_propagation(),
                FocusOnMount { id: TO_ID }
                div { class: "dm-compose-head",
                    h2 { id: TITLE_ID, class: "dm-compose-title", "New message" }
                    button {
                        class: "btn is-ghost",
                        r#type: "button",
                        aria_label: "Close",
                        onclick: move |_| close(()),
                        "\u{00d7}"
                    }
                }
                div { class: "dm-compose-body",
                    div { class: "dm-compose-field",
                        label { class: "field-label", r#for: TO_ID, "To" }
                        input {
                            id: TO_ID,
                            class: "field-input",
                            r#type: "text",
                            role: "combobox",
                            aria_autocomplete: "list",
                            aria_expanded: if list_open { "true" } else { "false" },
                            aria_controls: TO_LIST_ID,
                            aria_activedescendant: active_id.clone(),
                            aria_describedby: TO_HINT_ID,
                            placeholder: "Member name, npub or hex key",
                            value: "{recipient_value}",
                            autocomplete: "off",
                            oninput: move |e| {
                                recipient_input.set(e.value());
                                picked.set(None);
                                *blur_generation.write() += 1;
                                suggestions_open.set(true);
                                active.set(None);
                                if state.peek().is_error() {
                                    state.set(SendState::Idle);
                                }
                            },
                            onkeydown: {
                                let picks = picks.clone();
                                let ids = shown_option_ids.clone();
                                move |evt: Event<KeyboardData>| {
                                    let len = picks.len();
                                    match evt.key() {
                                        Key::ArrowDown | Key::ArrowUp if len > 0 => {
                                            evt.prevent_default();
                                            suggestions_open.set(true);
                                            let next = step(active_index, len, evt.key() == Key::ArrowDown);
                                            active.set(Some(next));
                                            if let Some(id) = ids.get(next) {
                                                scroll_option_into_view_soon(id.clone());
                                            }
                                        }
                                        Key::Enter if list_open => {
                                            if let Some(i) = active_index {
                                                evt.prevent_default();
                                                recipient_input.set(picks[i].0.clone());
                                                picked.set(Some(picks[i].clone()));
                                                suggestions_open.set(false);
                                                active.set(None);
                                            }
                                        }
                                        Key::Escape if list_open => {
                                            // Close the list, not the dialog.
                                            evt.prevent_default();
                                            evt.stop_propagation();
                                            suggestions_open.set(false);
                                            active.set(None);
                                        }
                                        _ => {}
                                    }
                                }
                            },
                            onfocus: move |_| {
                                *blur_generation.write() += 1;
                                suggestions_open.set(true);
                            },
                            onblur: move |_| {
                                // Delay close so a click on a suggestion still
                                // registers before the list unmounts. A focus
                                // or keystroke since then cancels it.
                                let at_blur = *blur_generation.peek();
                                spawn(async move {
                                    #[cfg(feature = "web")]
                                    gloo_timers::future::TimeoutFuture::new(120).await;
                                    if *blur_generation.peek() == at_blur {
                                        suggestions_open.set(false);
                                    }
                                });
                            },
                        }
                        p { id: TO_HINT_ID, class: "field-hint", "{to_hint}" }
                        if list_open {
                            ul {
                                id: TO_LIST_ID,
                                class: "dm-compose-options",
                                role: "listbox",
                                aria_label: "Matching members",
                                for (i, m) in filtered.iter().enumerate() {
                                    Fragment { key: "{m.id}", {
                                        let pk = m.nostr_pubkey.clone().unwrap_or_default();
                                        let pk_short = truncate_pubkey(&pk);
                                        let name = m.display_name.clone();
                                        let selected = active_index == Some(i);
                                        rsx! {
                                            li {
                                                key: "{m.id}",
                                                id: option_id(&m.id),
                                                class: "dm-compose-option",
                                                role: "option",
                                                aria_selected: if selected { "true" } else { "false" },
                                                // mousedown beats blur so the pick actually fires
                                                onmousedown: move |e| {
                                                    e.prevent_default();
                                                    recipient_input.set(name.clone());
                                                    picked.set(Some((name.clone(), pk.clone())));
                                                    suggestions_open.set(false);
                                                    active.set(None);
                                                },
                                                span { class: "dm-compose-option-name", "{m.display_name}" }
                                                span { class: "dm-compose-option-key", "{pk_short}" }
                                            }
                                        }
                                    } }
                                }
                            }
                            if let Some(more) = more_hint.clone() {
                                p { class: "field-hint", "{more}" }
                            }
                        }
                    }
                    div { class: "dm-compose-field",
                        label { class: "field-label", r#for: BODY_ID, "Message" }
                        textarea {
                            id: BODY_ID,
                            class: "field-input",
                            placeholder: "Write a message",
                            aria_describedby: "{COUNT_ID} {STATUS_ID}",
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
                        p { id: COUNT_ID, class: "{count_cls}", "{body_len} of {MAX_BODY_CHARS} characters" }
                    }
                }
                div { class: "dm-compose-foot",
                    SendStatus { id: STATUS_ID.to_string(), state: state() }
                    p { class: "dm-compose-hint", "Ctrl+Enter or Cmd+Enter sends." }
                    div { class: "dm-compose-actions",
                        button {
                            class: "btn is-ghost",
                            r#type: "button",
                            aria_disabled: if sending { "true" },
                            onclick: move |_| close(()),
                            "Cancel"
                        }
                        button {
                            class: "btn is-primary",
                            r#type: "button",
                            // Not `disabled`: a click says what is missing.
                            aria_disabled: if !can_submit { "true" },
                            aria_busy: if sending { "true" },
                            onclick: move |_| do_send.call(()),
                            if sending { "Sending..." } else { "Send" }
                        }
                    }
                }
            }
        }
    }
}

/// Moves focus to `id` once, when the dialog opens.
#[component]
fn FocusOnMount(id: &'static str) -> Element {
    use_effect(move || focus_element_by_id(id));
    rsx! {}
}

/// Bring the highlighted option into view inside the capped list, after the
/// render that opens or moves it. `block: "nearest"` scrolls the list only as
/// far as needed and leaves the page where it is.
fn scroll_option_into_view_soon(id: String) {
    dioxus::core::spawn_forever(async move {
        #[cfg(feature = "web")]
        {
            gloo_timers::future::TimeoutFuture::new(0).await;
        }
        #[cfg(target_arch = "wasm32")]
        {
            use wasm_bindgen::JsCast;
            let Some(el) = web_sys::window()
                .and_then(|w| w.document())
                .and_then(|d| d.get_element_by_id(&id))
            else {
                return;
            };
            let options = js_sys::Object::new();
            let _ = js_sys::Reflect::set(&options, &"block".into(), &"nearest".into());
            let scroll = js_sys::Reflect::get(&el, &"scrollIntoView".into()).ok();
            if let Some(scroll) = scroll
                .as_ref()
                .and_then(|f| f.dyn_ref::<js_sys::Function>())
            {
                let _ = scroll.call1(&el, &options);
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = id;
        }
    });
}

/// An option's id, from the member's record id.
fn option_id(member_id: &str) -> String {
    let safe: String = member_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    format!("{TO_LIST_ID}-{safe}")
}

/// Next (or previous) option, wrapping at the ends.
fn step(current: Option<usize>, len: usize, down: bool) -> usize {
    match (current, down) {
        (None, true) => 0,
        (None, false) => len - 1,
        (Some(i), true) => (i + 1) % len,
        (Some(i), false) => (i + len - 1) % len,
    }
}

/// The To field's hint: who the message goes to once the input resolves.
fn recipient_hint(resolved: Option<&str>, members: &[DmMember]) -> String {
    match resolved {
        Some(hex) => {
            let name = members
                .iter()
                .find(|m| {
                    m.nostr_pubkey
                        .as_deref()
                        .is_some_and(|pk| pk.eq_ignore_ascii_case(hex))
                })
                .map(|m| m.display_name.clone())
                .unwrap_or_else(|| truncate_pubkey(hex));
            format!("Sending to {name}.")
        }
        None => {
            "Type a member name and pick from the list, or paste an npub or a hex key.".to_string()
        }
    }
}

/// Members with a key, minus the viewer.
fn others(members: &[DmMember], self_pubkey: Option<&str>) -> Vec<DmMember> {
    members
        .iter()
        .filter(|m| {
            m.nostr_pubkey
                .as_deref()
                .is_some_and(|p| !p.trim().is_empty() && !is_self(p, self_pubkey))
        })
        .cloned()
        .collect()
}

fn is_self(hex: &str, self_pubkey: Option<&str>) -> bool {
    self_pubkey
        .is_some_and(|me| !me.trim().is_empty() && me.trim().eq_ignore_ascii_case(hex.trim()))
}

/// The To suggestions in name order, at most [`MAX_SUGGESTIONS`], and how
/// many members match in all. Nothing while the input is a pasted key.
fn suggestions(candidates: &[DmMember], input: &str) -> (Vec<DmMember>, usize) {
    let query = input.trim().to_lowercase();
    if looks_like_pubkey(&query) {
        return (Vec::new(), 0);
    }
    let mut matching: Vec<DmMember> = candidates
        .iter()
        .filter(|m| query.is_empty() || m.display_name.to_lowercase().contains(&query))
        .cloned()
        .collect();
    matching.sort_by_key(|m| m.display_name.to_lowercase());
    let total = matching.len();
    matching.truncate(MAX_SUGGESTIONS);
    (matching, total)
}

/// The key To stands for: the picked member while To still shows their
/// name, else whatever the typed text resolves to.
fn resolve_to(
    input: &str,
    picked: Option<&(String, String)>,
    members: &[DmMember],
) -> Option<String> {
    match picked {
        Some((name, pk)) if name.trim() == input.trim() && !pk.trim().is_empty() => {
            Some(pk.trim().to_ascii_lowercase())
        }
        _ => resolve_recipient(input, members),
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
fn resolve_recipient(input: &str, members: &[DmMember]) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(hex) = normalize_recipient_pubkey(trimmed) {
        return Some(hex);
    }
    let members: Vec<&DmMember> = members
        .iter()
        .filter(|m| {
            m.nostr_pubkey
                .as_deref()
                .is_some_and(|p| !p.trim().is_empty())
        })
        .collect();
    let lowered = trimmed.to_lowercase();
    if let Some(m) = members
        .iter()
        .find(|m| m.display_name.to_lowercase() == lowered)
    {
        return m.nostr_pubkey.clone();
    }
    let matches: Vec<&&DmMember> = members
        .iter()
        .filter(|m| m.display_name.to_lowercase().contains(&lowered))
        .collect();
    if matches.len() == 1 {
        return matches[0].nostr_pubkey.clone();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::ui::test_support::{assert_sizes_from_tokens, render_routed};

    /// Source-order guard: the parent keeps this modal mounted and toggles
    /// `open`. Returning before hooks when closed panics on open/close.
    #[test]
    fn compose_modal_registers_hooks_before_closed_return() {
        let src = include_str!("compose.rs");
        let start = src
            .find("pub fn DmComposeModal")
            .expect("DmComposeModal component");
        let body = &src[start..];
        let first_return = body
            .find("return rsx!")
            .expect("expected a post-hooks return when closed");
        for hook in ["use_signal", "use_toast()", "use_callback"] {
            let pos = body
                .find(hook)
                .unwrap_or_else(|| panic!("{hook} should appear in DmComposeModal"));
            assert!(
                pos < first_return,
                "{hook} must be registered before the first `return rsx!` (was {pos} >= {first_return})"
            );
        }
    }

    fn member(name: &str, pk: Option<&str>) -> DmMember {
        DmMember {
            id: format!("member:{name}"),
            display_name: name.to_string(),
            nostr_pubkey: pk.map(str::to_string),
        }
    }

    fn hex_pk(byte: u8) -> String {
        format!("{byte:02x}").repeat(32)
    }

    #[test]
    fn resolve_recipient_accepts_hex_and_unique_name() {
        let p1_pk = hex_pk(0xab);
        let p2_pk = hex_pk(0xcd);
        let members = vec![
            member("Player01", Some(&p1_pk)),
            member("Gamer02", Some(&p2_pk)),
        ];
        assert_eq!(resolve_recipient(&p1_pk, &members), Some(p1_pk.clone()));
        assert_eq!(resolve_recipient("Player01", &members), Some(p1_pk.clone()));
        assert_eq!(resolve_recipient("play", &members), Some(p1_pk));
        assert_eq!(resolve_recipient("", &members), None);
        assert_eq!(resolve_recipient("unknown", &members), None);
    }

    #[test]
    fn resolve_recipient_ambiguous_name_stays_none() {
        let members = vec![
            member("Player", Some(&hex_pk(0xaa))),
            member("Player02", Some(&hex_pk(0xbb))),
        ];
        assert_eq!(resolve_recipient("pla", &members), None);
        assert_eq!(resolve_recipient("Player", &members), Some(hex_pk(0xaa)));
    }

    #[test]
    fn members_without_a_key_are_never_the_recipient() {
        let members = vec![
            member("Player03", None),
            member("Player04", Some(&hex_pk(1))),
        ];
        assert_eq!(resolve_recipient("Player03", &members), None);
        assert_eq!(resolve_recipient("player", &members), Some(hex_pk(1)));
    }

    #[test]
    fn the_viewer_is_never_suggested_or_resolved_by_name() {
        let me = hex_pk(0x01);
        let members = vec![
            member("Player01", Some(&me)),
            member("Player02", Some(&hex_pk(0x02))),
        ];
        let rest = others(&members, Some(&me.to_ascii_uppercase()));
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].display_name, "Player02");
        let (shown, total) = suggestions(&rest, "player");
        assert_eq!((shown.len(), total), (1, 1));
        assert_eq!(resolve_recipient("Player01", &rest), None);
        // A pasted own key still resolves, and the dialog refuses it.
        assert!(is_self(&resolve_recipient(&me, &rest).unwrap(), Some(&me)));
    }

    #[test]
    fn suggestions_are_sorted_and_capped_with_a_total() {
        let members: Vec<DmMember> = (0..12u8)
            .rev()
            .map(|i| member(&format!("Player{i:02}"), Some(&hex_pk(i + 1))))
            .collect();
        let (shown, total) = suggestions(&members, "");
        assert_eq!(total, 12);
        assert_eq!(shown.len(), MAX_SUGGESTIONS);
        assert_eq!(shown[0].display_name, "Player00");
        assert_eq!(suggestions(&members, &hex_pk(3)).1, 0);
    }

    #[test]
    fn a_pick_shows_the_name_and_sends_to_the_picked_key() {
        let members = vec![
            member("Player02", Some(&hex_pk(2))),
            member("Player02", Some(&hex_pk(3))),
        ];
        let pick = ("Player02".to_string(), hex_pk(3));
        // Two members share the name: the pick decides, the name alone does not.
        assert_eq!(
            resolve_to("Player02", Some(&pick), &members),
            Some(hex_pk(3))
        );
        // Editing To drops the pick.
        assert_eq!(resolve_to("Player0", Some(&pick), &members), None);
    }

    #[test]
    fn looks_like_pubkey_hex_and_npub() {
        assert!(looks_like_pubkey(&hex_pk(0xab)));
        assert!(looks_like_pubkey("npub1abc"));
        assert!(!looks_like_pubkey("Player01"));
    }

    #[test]
    fn arrow_keys_wrap_over_the_options() {
        assert_eq!(step(None, 3, true), 0);
        assert_eq!(step(None, 3, false), 2);
        assert_eq!(step(Some(2), 3, true), 0);
        assert_eq!(step(Some(0), 3, false), 2);
    }

    #[test]
    fn hint_names_the_resolved_recipient() {
        let members = vec![member("Player02", Some(&hex_pk(2)))];
        assert_eq!(
            recipient_hint(Some(&hex_pk(2)), &members),
            "Sending to Player02."
        );
        assert!(recipient_hint(None, &members).contains("npub"));
    }

    #[test]
    fn option_ids_come_from_the_record_id() {
        assert_eq!(option_id("member:abc"), "dm-compose-to-list-member-abc");
    }

    #[test]
    fn arrow_keys_bring_the_highlighted_option_into_view() {
        // Vision blocking 11 on PR 706: the list shows five rows of up to
        // eight, and the highlight could move out of sight.
        let src = include_str!("compose.rs");
        let prod = src.split("mod tests").next().expect("production half");
        let keys = &prod[prod
            .find("Key::ArrowDown | Key::ArrowUp if len > 0")
            .expect("arrow keys")..];
        let keys = &keys[..keys.find("Key::Enter").expect("enter")];
        assert!(keys.contains("active.set(Some(next));"), "{keys}");
        assert!(
            keys.contains("scroll_option_into_view_soon(id.clone());"),
            "{keys}"
        );
        assert!(prod.contains("let shown_option_ids: Vec<String> = filtered.iter().map(|m| option_id(&m.id)).collect();"));
        assert!(prod.contains("&\"block\".into(), &\"nearest\".into()"));
        // The cap is still five rows: scrolling is what keeps it usable.
        let list = &COMPOSE_CSS[COMPOSE_CSS.find(".dm-compose-options {").unwrap()..];
        assert!(list.contains("max-height: calc(var(--hit-target-min) * 5);"));
    }

    #[test]
    fn open_dialog_is_named_modal_and_labels_its_fields() {
        let html = render_routed(|| {
            rsx! {
                crate::components::ToastProvider {
                    DmComposeModal {
                        open: true,
                        self_pubkey: None,
                        members: vec![member("Player02", Some(&hex_pk(2)))],
                        on_close: move |_| {},
                        on_sent: move |_| {},
                    }
                }
            }
        });
        assert!(html.contains("role=\"dialog\""), "{html}");
        assert!(html.contains("aria-modal=\"true\""), "{html}");
        assert!(
            html.contains(&format!("aria-labelledby=\"{TITLE_ID}\"")),
            "{html}"
        );
        assert!(
            html.contains(&format!(
                "<h2 id=\"{TITLE_ID}\" class=\"dm-compose-title\">New message</h2>"
            )),
            "{html}"
        );
        for id in [TO_ID, BODY_ID] {
            assert!(html.contains(&format!("for=\"{id}\"")), "{id}: {html}");
            assert!(html.contains(&format!("id=\"{id}\"")), "{id}: {html}");
        }
        assert!(html.contains("aria-label=\"Close\""), "{html}");
        assert!(
            html.contains(&format!(
                "<p id=\"{STATUS_ID}\" class=\"dm-reply-status\" role=\"status\">"
            )),
            "{html}"
        );
        assert!(!html.contains("Cmd/Ctrl"), "{html}");
        assert!(!html.contains(" disabled"), "{html}");
    }

    #[test]
    fn closed_dialog_renders_nothing() {
        let html = render_routed(|| {
            rsx! {
                crate::components::ToastProvider {
                    DmComposeModal {
                        open: false,
                        self_pubkey: None,
                        on_close: move |_| {},
                        on_sent: move |_| {},
                    }
                }
            }
        });
        assert!(!html.contains("role=\"dialog\""), "{html}");
    }

    #[test]
    fn compose_css_is_on_tokens() {
        assert_sizes_from_tokens(COMPOSE_CSS);
    }
}
