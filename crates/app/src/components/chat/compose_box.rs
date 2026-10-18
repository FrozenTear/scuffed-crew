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

/// Why a compose attempt finished. Only [`ComposeOutcome::Success`] clears the box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComposeOutcome {
    Success,
    Error,
    RateLimited,
}

/// Typed text after the server answers. Errors leave the draft unchanged.
pub fn draft_after_outcome(typed: &str, outcome: ComposeOutcome) -> String {
    match outcome {
        ComposeOutcome::Success => String::new(),
        ComposeOutcome::Error | ComposeOutcome::RateLimited => typed.to_string(),
    }
}

/// Trimmed payload when a send may start.
///
/// `pending` blocks a second send. The caller's typed text is not modified.
pub fn begin_send(typed: &str, pending: bool, blocked: bool) -> Option<String> {
    if pending || blocked {
        return None;
    }
    let payload = typed.trim().to_string();
    if !chat_compose_can_send(&payload) {
        return None;
    }
    Some(payload)
}

/// In-flight compose draft. Tests and the widget share this settle rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComposerState {
    pub text: String,
    pub pending: bool,
}

impl ComposerState {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            pending: false,
        }
    }

    pub fn begin_send(&mut self, blocked: bool) -> Option<String> {
        let payload = begin_send(&self.text, self.pending, blocked)?;
        self.pending = true;
        Some(payload)
    }

    pub fn settle(&mut self, outcome: ComposeOutcome) {
        self.text = draft_after_outcome(&self.text, outcome);
        self.pending = false;
    }
}

/// A send the parent must finish. The composer clears only on [`ComposeAttempt::succeed`].
#[derive(Clone)]
pub struct ComposeAttempt {
    pub text: String,
    finish: EventHandler<ComposeOutcome>,
}

impl ComposeAttempt {
    pub fn succeed(&self) {
        self.finish.call(ComposeOutcome::Success);
    }

    pub fn fail(&self) {
        self.finish.call(ComposeOutcome::Error);
    }

    pub fn rate_limited(&self) {
        self.finish.call(ComposeOutcome::RateLimited);
    }
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
    on_send: EventHandler<ComposeAttempt>,
    #[props(default = false)] disabled: bool,
    #[props(default = "Type a message...".to_string())] placeholder: String,
    #[props(default = false)] encrypted: bool,
    /// Blocks the send button without locking the input (rate-limit countdown).
    #[props(default = false)]
    send_locked: bool,
) -> Element {
    let mut draft = use_signal(String::new);
    let mut pending = use_signal(|| false);
    let finish = use_callback(move |outcome: ComposeOutcome| {
        let current = draft();
        draft.set(draft_after_outcome(&current, outcome));
        pending.set(false);
    });
    let draft_text = draft();
    let trimmed_len = trimmed_chat_len(&draft_text);
    let over_limit = trimmed_len > MAX_CHAT_MESSAGE_CHARS;
    let in_flight = pending();
    let can_send = begin_send(&draft_text, in_flight, disabled || send_locked).is_some();
    let counter = chat_counter_label(&draft_text);

    let mut submit = move || {
        let typed = draft();
        let Some(text) = begin_send(&typed, pending(), disabled || send_locked) else {
            return;
        };
        pending.set(true);
        on_send.call(ComposeAttempt { text, finish });
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
                disabled: disabled || in_flight,
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

    #[test]
    fn success_clears_the_typed_text() {
        let mut state = ComposerState::new("  hello team  ");
        let payload = state.begin_send(false).expect("send starts");
        assert_eq!(payload, "hello team");
        assert_eq!(state.text, "  hello team  ");
        assert!(state.pending);
        assert!(
            state.begin_send(false).is_none(),
            "a second send waits until the first settles"
        );
        state.settle(ComposeOutcome::Success);
        assert_eq!(state.text, "");
        assert!(!state.pending);
    }

    #[test]
    fn error_keeps_the_typed_text() {
        let typed = "  not sent yet  ";
        let mut state = ComposerState::new(typed);
        assert_eq!(state.begin_send(false).as_deref(), Some("not sent yet"));
        state.settle(ComposeOutcome::Error);
        assert_eq!(state.text, typed);
        assert!(!state.pending);
        assert_eq!(state.begin_send(false).as_deref(), Some("not sent yet"));
    }

    #[test]
    fn rate_limit_keeps_the_typed_text() {
        let typed = "  slow down  ";
        let mut state = ComposerState::new(typed);
        assert_eq!(state.begin_send(false).as_deref(), Some("slow down"));
        state.settle(ComposeOutcome::RateLimited);
        assert_eq!(state.text, typed);
        assert!(!state.pending);
        assert_eq!(
            draft_after_outcome(typed, ComposeOutcome::RateLimited),
            typed
        );
        assert_eq!(draft_after_outcome(typed, ComposeOutcome::Success), "");
    }

    #[test]
    fn composer_clears_only_through_the_outcome_helper() {
        let src = include_str!("compose_box.rs");
        let prod = src.split("mod tests").next().expect("production source");
        assert!(prod.contains("draft_after_outcome"));
        assert!(prod.contains("begin_send"));
        assert!(
            !prod.contains("draft.set(String::new())"),
            "the box must not wipe the draft before the parent settles"
        );
    }
}
