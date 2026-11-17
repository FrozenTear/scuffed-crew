//! Scrollable message feed for the chat thread.
//!
//! Follows `.msg-list` and `.msg` of the `d1-chat` frame: each message is a
//! small card with the sender and the time above the text. The viewer's own
//! messages sit at the end of the row on the soft accent fill, under the
//! viewer's own name as the frame draws it ("You" when it is not known).

use dioxus::prelude::*;

use crate::util::truncate_chars;
use scuffed_types::nostr::ChatMessage;

/// The scroller is a plain top-down column around one inner column, the same
/// scroll behaviour as before the redesign: oldest first, no scroll pinning.
/// A short room sits at the bottom edge (as in the frame) through the inner
/// column's auto top margin, which, unlike `justify-content: flex-end`, never
/// cuts off the top of a long room. The DOM order stays oldest to newest.
const MESSAGE_LIST_CSS: &str = r#"
.chat-messages {
  box-sizing: border-box;
  flex: 1 1 auto;
  min-width: 0;
  min-height: 0;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  background: var(--bg);
  color: var(--text);
  font-family: var(--font-body);
}
/* A scroller clips an outer ring, so the ring is drawn inside. */
.chat-messages:focus-visible {
  outline-offset: calc(-1 * var(--focus-ring-width));
}
.chat-messages__inner {
  box-sizing: border-box;
  flex: 0 0 auto;
  margin-top: auto;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
  padding: var(--space-card-pad);
}
.chat-messages__note {
  align-self: center;
  margin: 0;
  padding: var(--space-section) 0;
  color: var(--text-3);
  font-size: var(--text-sm);
  line-height: var(--leading-body);
  text-align: center;
}
.chat-msg {
  box-sizing: border-box;
  align-self: flex-start;
  width: fit-content;
  min-width: 0;
  max-width: min(60ch, 100%);
  padding: var(--space-3);
  background: var(--surface);
  color: var(--text);
  border: 1px solid var(--border);
  border-radius: var(--radius-card);
}
.chat-msg--me {
  align-self: flex-end;
  background: var(--accent-soft);
  border-color: var(--accent-soft);
}
.chat-msg__meta {
  display: flex;
  align-items: baseline;
  gap: var(--space-2);
  min-width: 0;
  margin: 0 0 var(--space-1);
  color: var(--text-3);
  font-size: var(--text-sm);
  line-height: var(--leading-ui);
}
.chat-msg__author {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.chat-msg__time {
  flex: 0 0 auto;
  white-space: nowrap;
}
.chat-msg__content,
.chat-msg__encrypted {
  margin: 0;
  color: var(--text);
  font-size: var(--text-md);
  line-height: var(--leading-body);
  overflow-wrap: anywhere;
}
/* The lock repeats the channel badge; the empty alt keeps it out of the
   accessible text where the alt syntax is supported. */
.chat-msg__encrypted::before {
  content: '\01F512';
  content: '\01F512' / '';
  margin-inline-end: var(--space-1);
}
.chat-msg__encrypted.is-sealed {
  color: var(--text-3);
}
"#;

const LOADING_NOTE: &str = "Loading messages";
const EMPTY_NOTE: &str = "No messages yet. Say something!";

/// Scrollable message feed. The `role="log"` region is rendered in every
/// state (loading, empty, filled); only its contents change, so new messages
/// are announced. It takes keyboard focus so it can be scrolled with the keys.
#[component]
pub fn MessageList(
    messages: Vec<ChatMessage>,
    #[props(default = false)] loading: bool,
    /// The viewer's own key. Their messages sit at the end.
    #[props(default)]
    own_pubkey: Option<String>,
    /// The viewer's display name, shown on their own messages.
    #[props(default)]
    own_name: Option<String>,
) -> Element {
    let now = now_secs();
    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-components-chat-message_list-message-list-css", css: MESSAGE_LIST_CSS }
        div {
            class: "chat-messages",
            role: "log",
            aria_label: "Messages",
            tabindex: "0",
            div { class: "chat-messages__inner",
                if loading {
                    p { class: "chat-messages__note", "{LOADING_NOTE}" }
                } else if messages.is_empty() {
                    p { class: "chat-messages__note", "{EMPTY_NOTE}" }
                }
                for msg in messages.iter() {
                    MessageBubble {
                        key: "{msg.id}",
                        message: msg.clone(),
                        mine: is_own(own_pubkey.as_deref(), &msg.pubkey),
                        own_name: own_name.clone(),
                        now,
                    }
                }
            }
        }
    }
}

/// True when `pubkey` is the viewer's key. An empty key never matches.
fn is_own(own_pubkey: Option<&str>, pubkey: &str) -> bool {
    !pubkey.is_empty() && own_pubkey == Some(pubkey)
}

/// Name above a message. Pubkeys are usually 64 hex chars, but a short or
/// empty sender id must not panic on a fixed byte slice.
fn sender_name(message: &ChatMessage, mine: bool, own_name: Option<&str>) -> String {
    if mine {
        let own = own_name.map(str::trim).filter(|name| !name.is_empty());
        return own.unwrap_or("You").to_string();
    }
    match message.display_name.as_deref() {
        Some(name) => name.to_string(),
        None => truncate_chars(&message.pubkey, 8).to_string(),
    }
}

/// A single message card.
#[component]
fn MessageBubble(message: ChatMessage, mine: bool, own_name: Option<String>, now: u64) -> Element {
    let name = sender_name(&message, mine, own_name.as_deref());
    let time_str = format_relative_time(now, message.created_at);
    let stamp = iso_timestamp(message.created_at);
    let sealed = message.encrypted && message.content.is_empty();

    rsx! {
        div { class: if mine { "chat-msg chat-msg--me" } else { "chat-msg" },
            div { class: "chat-msg__meta",
                span { class: "chat-msg__author", "{name}" }
                time { class: "chat-msg__time", datetime: stamp, "{time_str}" }
            }
            if sealed {
                p { class: "chat-msg__encrypted is-sealed", "Encrypted message" }
            } else if message.encrypted {
                p { class: "chat-msg__encrypted", "{message.content}" }
            } else {
                p { class: "chat-msg__content", "{message.content}" }
            }
        }
    }
}

/// Seconds since the Unix epoch. The browser clock in the app, the system
/// clock in native tests.
fn now_secs() -> u64 {
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Date::now() as u64 / 1000
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        u64::try_from(chrono::Utc::now().timestamp()).unwrap_or(0)
    }
}

/// Machine-readable time for the `datetime` attribute.
fn iso_timestamp(timestamp: u64) -> Option<String> {
    let secs = i64::try_from(timestamp).ok()?;
    chrono::DateTime::from_timestamp(secs, 0).map(|at| at.to_rfc3339())
}

/// Format a unix timestamp as a relative time string.
fn format_relative_time(now: u64, timestamp: u64) -> String {
    let diff = now.saturating_sub(timestamp);

    if diff < 60 {
        "now".to_string()
    } else if diff < 3600 {
        format!("{}m ago", diff / 60)
    } else if diff < 86400 {
        format!("{}h ago", diff / 3600)
    } else {
        format!("{}d ago", diff / 86400)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::ui::test_support::{
        assert_ink_clears_aa, assert_sizes_from_tokens, css_rule, has_decl,
    };

    const ME: &str = "aa11bb22cc33dd44";
    const OTHER: &str = "ff00ee11dd22cc33";

    fn message(id: &str, pubkey: &str, name: Option<&str>, content: &str) -> ChatMessage {
        ChatMessage {
            id: id.to_string(),
            pubkey: pubkey.to_string(),
            display_name: name.map(str::to_string),
            avatar_url: None,
            content: content.to_string(),
            created_at: now_secs().saturating_sub(300),
            group_id: "g".to_string(),
            encrypted: false,
            reply_to: None,
        }
    }

    #[derive(Props, Clone, PartialEq)]
    struct ProbeProps {
        messages: Vec<ChatMessage>,
        loading: bool,
        own_name: Option<String>,
    }

    #[allow(non_snake_case)]
    fn Probe(props: ProbeProps) -> Element {
        rsx! {
            MessageList {
                messages: props.messages,
                loading: props.loading,
                own_pubkey: Some(ME.to_string()),
                own_name: props.own_name,
            }
        }
    }

    fn render(messages: Vec<ChatMessage>, loading: bool) -> String {
        render_as(messages, loading, Some("Player01"))
    }

    fn render_as(messages: Vec<ChatMessage>, loading: bool, own_name: Option<&str>) -> String {
        let props = ProbeProps {
            messages,
            loading,
            own_name: own_name.map(str::to_string),
        };
        let mut dom = VirtualDom::new_with_props(Probe, props);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    const LOG_OPEN: &str =
        "<div class=\"chat-messages\" role=\"log\" aria-label=\"Messages\" tabindex=\"0\">";

    #[test]
    fn the_log_region_is_rendered_in_every_state() {
        let empty = render(Vec::new(), false);
        let loading = render(Vec::new(), true);
        let filled = render(
            vec![message("1", OTHER, Some("Player03"), "Scrim at 19:00?")],
            false,
        );
        for html in [&empty, &loading, &filled] {
            assert_eq!(html.matches(LOG_OPEN).count(), 1, "{html}");
            assert!(!html.contains("<h1"), "{html}");
            assert!(!html.contains("<main"), "{html}");
        }
        assert!(empty.contains(EMPTY_NOTE), "{empty}");
        assert!(!empty.contains(LOADING_NOTE), "{empty}");
        assert!(loading.contains(LOADING_NOTE), "{loading}");
        assert!(!loading.contains(EMPTY_NOTE), "{loading}");
        assert!(!filled.contains(EMPTY_NOTE), "{filled}");
        assert!(!filled.contains(LOADING_NOTE), "{filled}");
    }

    #[test]
    fn a_message_is_a_card_with_sender_time_and_text() {
        let html = render(
            vec![message("1", OTHER, Some("Player03"), "Scrim at 19:00?")],
            false,
        );
        assert!(html.contains("<div class=\"chat-msg\">"), "{html}");
        assert!(
            html.contains("<span class=\"chat-msg__author\">Player03</span>"),
            "{html}"
        );
        assert!(
            html.contains("<time class=\"chat-msg__time\" datetime=\""),
            "{html}"
        );
        assert!(html.contains(">5m ago</time>"), "{html}");
        assert!(
            html.contains("<p class=\"chat-msg__content\">Scrim at 19:00?</p>"),
            "{html}"
        );
        assert!(!html.contains("chat-msg--me"), "{html}");
        assert!(!html.contains("<img"), "{html}");
    }

    #[test]
    fn own_messages_carry_the_viewer_name_and_sit_at_the_end() {
        let html = render(
            vec![
                message("1", OTHER, None, "First"),
                message("2", ME, None, "Works for me."),
            ],
            false,
        );
        assert!(
            html.contains("<div class=\"chat-msg chat-msg--me\">"),
            "{html}"
        );
        assert!(
            html.contains("<span class=\"chat-msg__author\">Player01</span>"),
            "the frame names the viewer on their own messages: {html}"
        );
        for unknown in [None, Some("  ")] {
            let anon = render_as(
                vec![message("2", ME, None, "Works for me.")],
                false,
                unknown,
            );
            assert!(
                anon.contains("<span class=\"chat-msg__author\">You</span>"),
                "{anon}"
            );
        }
        assert!(
            html.contains("<span class=\"chat-msg__author\">ff00ee11</span>"),
            "a sender with no name falls back to the key prefix: {html}"
        );
        assert!(
            html.find("First").unwrap() < html.find("Works for me.").unwrap(),
            "oldest first in the DOM: {html}"
        );
        assert!(!is_own(Some(""), ""));
        assert!(!is_own(None, OTHER));
        assert!(is_own(Some(ME), ME));
    }

    #[test]
    fn encrypted_rows_keep_their_two_forms() {
        let mut sealed = message("1", OTHER, None, "");
        sealed.encrypted = true;
        let mut opened = message("2", OTHER, None, "Officer note");
        opened.encrypted = true;
        let html = render(vec![sealed, opened], false);
        assert!(
            html.contains("<p class=\"chat-msg__encrypted is-sealed\">Encrypted message</p>"),
            "{html}"
        );
        assert!(
            html.contains("<p class=\"chat-msg__encrypted\">Officer note</p>"),
            "{html}"
        );
    }

    #[test]
    fn relative_time_steps() {
        assert_eq!(format_relative_time(1_000, 1_000), "now");
        assert_eq!(format_relative_time(1_000, 2_000), "now");
        assert_eq!(format_relative_time(1_059, 1_000), "now");
        assert_eq!(format_relative_time(1_060, 1_000), "1m ago");
        assert_eq!(format_relative_time(4_600, 1_000), "1h ago");
        assert_eq!(format_relative_time(200_000, 1_000), "2d ago");
        assert_eq!(
            iso_timestamp(0).as_deref(),
            Some("1970-01-01T00:00:00+00:00")
        );
        assert_eq!(iso_timestamp(u64::MAX), None);
    }

    #[test]
    fn list_css_is_on_tokens_and_scrolls_inside() {
        assert_sizes_from_tokens(MESSAGE_LIST_CSS);
        let list = css_rule(MESSAGE_LIST_CSS, ".chat-messages {");
        for decl in [
            "flex: 1 1 auto",
            "min-height: 0",
            "overflow-y: auto",
            "flex-direction: column",
            "background: var(--bg)",
        ] {
            assert!(has_decl(&list, decl), "missing {decl}: {list}");
        }
        let msg = css_rule(MESSAGE_LIST_CSS, ".chat-msg {");
        for decl in [
            "align-self: flex-start",
            "padding: var(--space-3)",
            "background: var(--surface)",
            "border: 1px solid var(--border)",
            "border-radius: var(--radius-card)",
        ] {
            assert!(has_decl(&msg, decl), "missing {decl}: {msg}");
        }
        let mine = css_rule(MESSAGE_LIST_CSS, ".chat-msg--me {");
        assert!(has_decl(&mine, "align-self: flex-end"), "{mine}");
        assert!(has_decl(&mine, "background: var(--accent-soft)"), "{mine}");
        assert!(!MESSAGE_LIST_CSS.contains("animation"));
        let lock = css_rule(MESSAGE_LIST_CSS, ".chat-msg__encrypted::before {");
        assert!(
            has_decl(&lock, "content: '\\01F512' / ''"),
            "the lock glyph has an empty alt: {lock}"
        );
    }

    #[test]
    fn message_ink_clears_contrast_on_both_fills() {
        for fill in ["--surface", "--accent-soft"] {
            assert_ink_clears_aa("--text", fill, 4.5);
            assert_ink_clears_aa("--text-3", fill, 4.5);
        }
        assert_ink_clears_aa("--text-3", "--bg", 4.5);
    }
}
