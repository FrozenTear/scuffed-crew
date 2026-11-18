//! Reply input + send button for the chat widget.

use dioxus::prelude::*;
use scuffed_types::MAX_CHAT_MESSAGE_CHARS;

use crate::components::ui::btn_class;

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

/// Typed text after the server answers.
///
/// Success clears the box only when `current` is still the exact input captured
/// at send time (`sent_exact`). Text the user added or edited while the request
/// was in flight stays. Errors always keep `current`.
pub fn draft_after_outcome(current: &str, sent_exact: &str, outcome: ComposeOutcome) -> String {
    match outcome {
        ComposeOutcome::Success if current == sent_exact => String::new(),
        _ => current.to_string(),
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
    /// Input value at the moment Send was pressed.
    sent_exact: String,
}

impl ComposerState {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            pending: false,
            sent_exact: String::new(),
        }
    }

    pub fn begin_send(&mut self, blocked: bool) -> Option<String> {
        let payload = begin_send(&self.text, self.pending, blocked)?;
        self.sent_exact = self.text.clone();
        self.pending = true;
        Some(payload)
    }

    pub fn settle(&mut self, outcome: ComposeOutcome) {
        self.text = draft_after_outcome(&self.text, &self.sent_exact, outcome);
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

/// The composer of the `d1-chat` frame: the shared 44px field and the primary
/// button on one row, pinned under the message list. The wrapper carries the
/// top border, so the encrypted hint and the counter sit inside the same bar.
const COMPOSE_BOX_CSS: &str = r#"
.chat-composer {
  box-sizing: border-box;
  flex: 0 0 auto;
  min-width: 0;
  border-top: 1px solid var(--border);
  background: var(--surface);
  color: var(--text);
  font-family: var(--font-body);
}
.chat-compose {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-3) var(--space-card-pad);
}
.chat-compose .field-input {
  flex: 1 1 auto;
  width: auto;
  min-width: 0;
}
.chat-compose .btn {
  flex: 0 0 auto;
}
/* Nothing to send yet. The button keeps focus and ignores the click. */
.chat-compose .btn[aria-disabled="true"] {
  cursor: not-allowed;
}
.chat-compose__hint {
  margin: 0;
  padding: var(--space-2) var(--space-card-pad) 0;
  color: var(--text-3);
  font-size: var(--text-xs);
  line-height: var(--leading-ui);
}
.chat-compose__counter {
  margin: 0;
  padding: 0 var(--space-card-pad) var(--space-2);
  color: var(--text-3);
  font-size: var(--text-xs);
  line-height: var(--leading-ui);
  text-align: right;
}
.chat-compose__counter:empty {
  padding: 0;
}
.chat-compose__counter--over {
  color: var(--loss);
}
"#;

/// Accessible name and placeholder of the message field.
pub const COMPOSE_PLACEHOLDER: &str = "Write a message";

/// Near-limit counter. The live region stays mounted so the first count is announced.
fn compose_counter(label: Option<String>, over_limit: bool) -> Element {
    let class = if over_limit {
        "chat-compose__counter chat-compose__counter--over"
    } else {
        "chat-compose__counter"
    };
    rsx! {
        p {
            class: "{class}",
            role: "status",
            if let Some(label) = label {
                "{label}"
            }
        }
    }
}

/// Chat message compose box with text input and send button.
#[component]
pub fn ComposeBox(
    on_send: EventHandler<ComposeAttempt>,
    #[props(default = false)] disabled: bool,
    #[props(default = COMPOSE_PLACEHOLDER.to_string())] placeholder: String,
    #[props(default = false)] encrypted: bool,
    /// Blocks the send button without locking the input (rate-limit countdown).
    #[props(default = false)]
    send_locked: bool,
    /// Starting text. Empty for a fresh box.
    #[props(default)]
    initial_draft: String,
) -> Element {
    let mut draft = use_signal(|| initial_draft);
    let mut pending = use_signal(|| false);
    let mut sent_exact = use_signal(String::new);
    let finish = use_callback(move |outcome: ComposeOutcome| {
        let current = draft();
        draft.set(draft_after_outcome(&current, &sent_exact(), outcome));
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
        sent_exact.set(typed);
        pending.set(true);
        on_send.call(ComposeAttempt { text, finish });
    };

    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-components-chat-compose_box-compose-box-css", css: COMPOSE_BOX_CSS }
        div { class: "chat-composer",
            if encrypted {
                p { class: "chat-compose__hint",
                    "Messages in this channel are end-to-end encrypted"
                }
            }
            div { class: "chat-compose",
                input {
                    class: "field-input",
                    r#type: "text",
                    aria_label: "{COMPOSE_PLACEHOLDER}",
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
                // Never natively disabled: a disabled button drops focus the
                // moment a send clears the draft. `submit` ignores the click.
                button {
                    r#type: "button",
                    class: btn_class(true),
                    aria_disabled: if !can_send { "true" },
                    onclick: move |_| submit(),
                    "Send"
                }
            }
            {compose_counter(counter, over_limit)}
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
            draft_after_outcome(typed, typed, ComposeOutcome::RateLimited),
            typed
        );
        assert_eq!(
            draft_after_outcome(typed, typed, ComposeOutcome::Success),
            ""
        );
    }

    #[test]
    fn success_keeps_text_typed_while_the_send_is_in_flight() {
        let mut state = ComposerState::new("hello");
        assert_eq!(state.begin_send(false).as_deref(), Some("hello"));
        assert!(state.begin_send(false).is_none());
        state.text.push_str(" there");
        state.settle(ComposeOutcome::Success);
        assert_eq!(state.text, "hello there");
        assert!(!state.pending);
        assert_eq!(
            draft_after_outcome("hello there", "hello", ComposeOutcome::Success),
            "hello there"
        );
        assert_eq!(
            draft_after_outcome("hello", "hello", ComposeOutcome::Success),
            ""
        );
    }

    #[component]
    fn CounterProbe(initial_draft: String) -> Element {
        rsx! {
            ComposeBox {
                on_send: |_| {},
                initial_draft,
            }
        }
    }

    fn render_box(initial_draft: String) -> String {
        let mut dom = VirtualDom::new_with_props(CounterProbe, CounterProbeProps { initial_draft });
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    #[test]
    fn message_input_is_named_and_the_counter_stays_mounted() {
        let empty = render_box(String::new());
        assert!(
            empty.contains("aria-label=\"Write a message\""),
            "the message field needs an accessible name: {empty}"
        );
        assert!(empty.contains("placeholder=\"Write a message\""), "{empty}");
        assert_eq!(
            empty.matches("role=\"status\"").count(),
            1,
            "the live region is mounted before the count appears: {empty}"
        );
        assert!(
            empty.contains("<p class=\"chat-compose__counter\" role=\"status\"></p>"),
            "{empty}"
        );
        assert!(
            !empty.contains("characters left"),
            "an empty draft does not announce a count: {empty}"
        );

        let near = "a".repeat(MAX_CHAT_MESSAGE_CHARS - 1);
        let html = render_box(near);
        let live_at = html.find("role=\"status\"").expect("live region");
        let label_at = html.find("1 character left").expect("near-limit count");
        assert!(live_at < label_at, "{html}");
        assert!(
            !html[live_at..label_at].contains("</p>"),
            "the count is inside the live region: {html}"
        );
    }

    #[test]
    fn composer_is_the_shared_field_and_primary_button() {
        let empty = render_box(String::new());
        assert!(!empty.contains("<h1"), "{empty}");
        assert!(!empty.contains("<main"), "{empty}");
        assert!(empty.contains("<input class=\"field-input\""), "{empty}");
        assert!(
            empty.contains(
                "<button type=\"button\" class=\"btn is-primary\" aria-disabled=\"true\">Send</button>"
            ),
            "an empty draft marks Send unavailable without disabling it: {empty}"
        );
        assert!(
            !empty.contains(" disabled"),
            "nothing is natively disabled: {empty}"
        );

        let ready = render_box("hello team".to_string());
        assert!(
            ready.contains("<button type=\"button\" class=\"btn is-primary\">Send</button>"),
            "{ready}"
        );

        let over = render_box("a".repeat(MAX_CHAT_MESSAGE_CHARS + 1));
        assert!(over.contains("aria-disabled=\"true\""), "{over}");
        assert!(
            over.contains("chat-compose__counter chat-compose__counter--over"),
            "{over}"
        );
        assert!(over.contains("1 character over the limit"), "{over}");
    }

    #[test]
    fn composer_css_is_on_tokens_and_does_not_shrink() {
        use crate::components::ui::test_support::{assert_sizes_from_tokens, css_rule, has_decl};

        assert_sizes_from_tokens(COMPOSE_BOX_CSS);
        let bar = css_rule(COMPOSE_BOX_CSS, ".chat-composer {");
        for decl in [
            "flex: 0 0 auto",
            "border-top: 1px solid var(--border)",
            "background: var(--surface)",
        ] {
            assert!(has_decl(&bar, decl), "missing {decl}: {bar}");
        }
        let row = css_rule(COMPOSE_BOX_CSS, ".chat-compose {");
        assert!(
            has_decl(&row, "padding: var(--space-3) var(--space-card-pad)"),
            "{row}"
        );
        assert!(has_decl(&row, "gap: var(--space-2)"), "{row}");
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
        assert!(
            !prod.contains("disabled || in_flight"),
            "the input stays editable while a send is in flight"
        );
    }
}
