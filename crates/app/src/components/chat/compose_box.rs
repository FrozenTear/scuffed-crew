//! Reply input + send button for the chat widget.

use dioxus::prelude::*;
use scuffed_types::MAX_CHAT_MESSAGE_CHARS;

/// Show the remaining count once this many characters or fewer are left after trim.
const CHAT_COUNTER_WINDOW: usize = 200;

/// Unicode scalar length after `trim`.
///
/// Matches server `trimmed_chat_content`: `str::trim`, then `chars().count()`.
/// The cap is [`MAX_CHAT_MESSAGE_CHARS`] (2000).
fn trimmed_chat_len(raw: &str) -> usize {
    raw.trim().chars().count()
}

/// True when trimmed text is non-empty and within the server cap.
fn chat_compose_can_send(raw: &str) -> bool {
    let len = trimmed_chat_len(raw);
    len > 0 && len <= MAX_CHAT_MESSAGE_CHARS
}

/// Remaining-character label. Hidden until the draft is near the cap.
fn chat_counter_label(raw: &str) -> Option<String> {
    let len = trimmed_chat_len(raw);
    let over = len > MAX_CHAT_MESSAGE_CHARS;
    let near = len + CHAT_COUNTER_WINDOW >= MAX_CHAT_MESSAGE_CHARS;
    if !over && !near {
        return None;
    }
    if over {
        let extra = len - MAX_CHAT_MESSAGE_CHARS;
        return Some(if extra == 1 {
            "1 character over the limit".to_string()
        } else {
            format!("{extra} characters over the limit")
        });
    }
    let left = MAX_CHAT_MESSAGE_CHARS - len;
    Some(if left == 1 {
        "1 character left".to_string()
    } else {
        format!("{left} characters left")
    })
}

const COMPOSE_BOX_CSS: &str = r#"
.chat-compose {
    display: flex;
    gap: 8px;
    padding: 8px 12px;
    border-top: 1px solid var(--border);
    background: var(--surface);
    align-items: flex-end;
}

.chat-compose__input {
    flex: 1;
    min-height: 36px;
    max-height: 100px;
    padding: 8px 12px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--bg);
    color: var(--text);
    font-family: var(--font-body);
    font-size: 13px;
    line-height: 1.4;
    resize: none;
    outline: none;
    transition: border-color 0.15s ease;
}

.chat-compose__input::placeholder {
    color: var(--text-3);
}

.chat-compose__input:focus {
    border-color: var(--accent);
}

.chat-compose__input:disabled {
    opacity: 0.5;
    cursor: not-allowed;
}

.chat-compose__send {
    padding: 8px 14px;
    border: none;
    border-radius: 8px;
    background: var(--accent);
    color: var(--accent-fg);
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    transition: opacity 0.15s ease, transform 0.1s ease;
    white-space: nowrap;
}

.chat-compose__send:hover:not(:disabled) {
    opacity: 0.9;
}

.chat-compose__send:active:not(:disabled) {
    transform: scale(0.96);
}

.chat-compose__send:disabled {
    opacity: 0.4;
    cursor: not-allowed;
}

.chat-compose__hint {
    font-size: 10px;
    color: var(--text-3);
    padding: 2px 12px 4px;
    background: var(--surface);
}

.chat-compose__counter {
    font-size: 11px;
    color: var(--text-3);
    text-align: right;
    padding: 0 12px 6px;
    background: var(--surface);
}

.chat-compose__counter--over {
    color: var(--danger);
}
"#;

/// Chat message compose box with text input and send button.
#[component]
pub fn ComposeBox(
    on_send: EventHandler<String>,
    #[props(default = false)] disabled: bool,
    #[props(default = "Type a message...".to_string())] placeholder: String,
    #[props(default = false)] encrypted: bool,
    /// Blocks the send button without locking the input (rate-limit countdown).
    #[props(default = false)]
    send_locked: bool,
) -> Element {
    let mut draft = use_signal(String::new);
    let draft_text = draft();
    let trimmed_len = trimmed_chat_len(&draft_text);
    let over_limit = trimmed_len > MAX_CHAT_MESSAGE_CHARS;
    let can_send = !disabled && !send_locked && chat_compose_can_send(&draft_text);
    let counter = chat_counter_label(&draft_text);

    let mut submit = move || {
        let text = draft.read().trim().to_string();
        if chat_compose_can_send(&text) && !disabled && !send_locked {
            on_send.call(text);
            draft.set(String::new());
        }
    };

    rsx! {
        style { {COMPOSE_BOX_CSS} }
        if encrypted {
            div { class: "chat-compose__hint",
                "Messages in this channel are end-to-end encrypted"
            }
        }
        div { class: "chat-compose",
            input {
                class: "chat-compose__input",
                r#type: "text",
                placeholder: "{placeholder}",
                disabled: disabled,
                value: "{draft}",
                oninput: move |evt| draft.set(evt.value()),
                onkeydown: move |evt| {
                    if evt.key() == Key::Enter {
                        submit();
                    }
                },
            }
            button {
                class: "chat-compose__send",
                disabled: !can_send,
                onclick: move |_| submit(),
                "Send"
            }
        }
        if let Some(label) = counter {
            div {
                class: if over_limit {
                    "chat-compose__counter chat-compose__counter--over"
                } else {
                    "chat-compose__counter"
                },
                "{label}"
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_then_count_matches_server_scalars() {
        assert_eq!(trimmed_chat_len("  hello  "), 5);
        assert_eq!(trimmed_chat_len("\n\thi\t\n"), 2);
        assert_eq!(trimmed_chat_len("\u{00A0}hi\u{00A0}"), 2);
        assert_eq!(trimmed_chat_len("   "), 0);
        assert_eq!(trimmed_chat_len("é"), 1);
        assert_eq!(trimmed_chat_len("👍"), 1);
        // ZWJ family is several scalars, not one grapheme and not bytes.
        assert_eq!(trimmed_chat_len("👨‍👩‍👧"), 5);
        assert_eq!(trimmed_chat_len(&format!("  {}  ", "a".repeat(3))), 3);
    }

    #[test]
    fn send_blocks_empty_and_over_cap() {
        assert!(!chat_compose_can_send(""));
        assert!(!chat_compose_can_send("   \n"));
        assert!(chat_compose_can_send(" hi "));
        assert!(chat_compose_can_send(&"a".repeat(MAX_CHAT_MESSAGE_CHARS)));
        assert!(!chat_compose_can_send(
            &"a".repeat(MAX_CHAT_MESSAGE_CHARS + 1)
        ));
        let edge = format!("{}é", "a".repeat(MAX_CHAT_MESSAGE_CHARS - 1));
        assert_eq!(trimmed_chat_len(&edge), MAX_CHAT_MESSAGE_CHARS);
        assert!(chat_compose_can_send(&edge));
    }

    #[test]
    fn counter_shows_remaining_only_near_the_limit() {
        assert_eq!(chat_counter_label("hello"), None);
        let under = "a".repeat(MAX_CHAT_MESSAGE_CHARS - CHAT_COUNTER_WINDOW - 1);
        assert_eq!(chat_counter_label(&under), None);
        let at_window = "a".repeat(MAX_CHAT_MESSAGE_CHARS - CHAT_COUNTER_WINDOW);
        assert_eq!(
            chat_counter_label(&at_window).as_deref(),
            Some("200 characters left")
        );
        assert_eq!(
            chat_counter_label(&"a".repeat(MAX_CHAT_MESSAGE_CHARS - 1)).as_deref(),
            Some("1 character left")
        );
        assert_eq!(
            chat_counter_label(&"a".repeat(MAX_CHAT_MESSAGE_CHARS)).as_deref(),
            Some("0 characters left")
        );
        assert_eq!(
            chat_counter_label(&"a".repeat(MAX_CHAT_MESSAGE_CHARS + 1)).as_deref(),
            Some("1 character over the limit")
        );
        assert_eq!(
            chat_counter_label(&format!("  {}  ", "a".repeat(MAX_CHAT_MESSAGE_CHARS + 2)))
                .as_deref(),
            Some("2 characters over the limit")
        );
    }
}
