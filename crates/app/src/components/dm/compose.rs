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
    let mut toast = use_toast();

    let members_for_send = members.clone();
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
            let Some(recipient_hex) = resolve_recipient(&recipient_trimmed, &members_for_send)
            else {
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
    let resolved_recipient = resolve_recipient(&recipient_trimmed, &members);
    let can_submit = !sending && body_problem(&body_text).is_none() && resolved_recipient.is_some();
    let to_hint = recipient_hint(resolved_recipient.as_deref(), &members);

    let query = recipient_trimmed.to_lowercase();
    let filtered: Vec<DmMember> = if looks_like_pubkey(&query) {
        Vec::new()
    } else {
        members
            .iter()
            .filter(|m| {
                m.nostr_pubkey
                    .as_deref()
                    .is_some_and(|p| !p.trim().is_empty())
            })
            .filter(|m| query.is_empty() || m.display_name.to_lowercase().contains(&query))
            .take(8)
            .cloned()
            .collect()
    };
    let list_open = suggestions_open() && !filtered.is_empty();
    let active_index = active().filter(|i| *i < filtered.len());
    let active_id = active_index
        .filter(|_| list_open)
        .map(|i| option_id(&filtered[i].id));
    let picks: Vec<String> = filtered
        .iter()
        .map(|m| m.nostr_pubkey.clone().unwrap_or_default())
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
                                suggestions_open.set(true);
                                active.set(None);
                                if state.peek().is_error() {
                                    state.set(SendState::Idle);
                                }
                            },
                            onkeydown: {
                                let picks = picks.clone();
                                move |evt: Event<KeyboardData>| {
                                    let len = picks.len();
                                    match evt.key() {
                                        Key::ArrowDown if len > 0 => {
                                            evt.prevent_default();
                                            suggestions_open.set(true);
                                            active.set(Some(step(active_index, len, true)));
                                        }
                                        Key::ArrowUp if len > 0 => {
                                            evt.prevent_default();
                                            suggestions_open.set(true);
                                            active.set(Some(step(active_index, len, false)));
                                        }
                                        Key::Enter if list_open => {
                                            if let Some(i) = active_index {
                                                evt.prevent_default();
                                                recipient_input.set(picks[i].clone());
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
                            onfocus: move |_| suggestions_open.set(true),
                            onblur: move |_| {
                                // Delay close so a click on a suggestion still
                                // registers before the list unmounts.
                                spawn(async move {
                                    #[cfg(feature = "web")]
                                    gloo_timers::future::TimeoutFuture::new(120).await;
                                    suggestions_open.set(false);
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
                                    {
                                        let pk = m.nostr_pubkey.clone().unwrap_or_default();
                                        let pk_short = truncate_pubkey(&pk);
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
                                                    recipient_input.set(pk.clone());
                                                    suggestions_open.set(false);
                                                    active.set(None);
                                                },
                                                span { class: "dm-compose-option-name", "{m.display_name}" }
                                                span { class: "dm-compose-option-key", "{pk_short}" }
                                            }
                                        }
                                    }
                                }
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
