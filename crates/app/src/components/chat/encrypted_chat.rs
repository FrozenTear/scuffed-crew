//! NIP-44 encrypted chat support for officer/private channels.
//!
//! For server-managed keys, encryption happens server-side: the frontend sends
//! plaintext to Axum, which encrypts + signs + publishes. Decryption is also
//! server-side: the frontend fetches decrypted messages via API.
//!
//! For external key users (NIP-07), encryption/decryption can happen client-side
//! via `window.nostr.nip44.encrypt()`/`decrypt()`.

use dioxus::prelude::*;

use scuffed_types::nostr::ChatMessage;

use crate::components::ui::EmptyCard;

use super::compose_box::{ComposeAttempt, ComposeBox};
use super::message_list::MessageList;

const ENCRYPTED_CHAT_CSS: &str = r#"
.encrypted-chat {
  flex: 1 1 auto;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
}
.encrypted-chat__badge {
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  gap: var(--space-2);
  margin: 0;
  padding: var(--space-2) var(--space-card-pad);
  border-bottom: 1px solid var(--border);
  background: var(--surface);
  color: var(--text-3);
  font-size: var(--text-xs);
  line-height: var(--leading-ui);
}
.encrypted-chat__no-key {
  box-sizing: border-box;
  flex: 1 1 auto;
  min-height: 0;
  overflow-y: auto;
  padding: var(--space-section) var(--space-page);
  background: var(--bg);
}
"#;

/// Key mode for the current user.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KeyMode {
    /// Server manages the keypair: encryption/decryption happens server-side.
    ServerManaged,
    /// User has their own Nostr key (NIP-07 extension or imported).
    External,
    /// No key available: cannot participate in encrypted channels.
    None,
}

/// Encrypted chat wrapper that handles NIP-44 channel display.
///
/// For server-managed keys, messages arrive pre-decrypted from the API.
/// For external keys, client-side NIP-44 decryption would happen here.
#[component]
pub fn EncryptedChat(
    messages: Vec<ChatMessage>,
    key_mode: KeyMode,
    on_send: EventHandler<ComposeAttempt>,
    #[props(default = false)] loading: bool,
    /// The viewer's own key, passed on to the message list.
    #[props(default)]
    own_pubkey: Option<String>,
    /// The viewer's display name, passed on to the message list.
    #[props(default)]
    own_name: Option<String>,
) -> Element {
    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-components-chat-encrypted_chat-encrypted-chat-css", css: ENCRYPTED_CHAT_CSS }
        div { class: "encrypted-chat",
            p { class: "encrypted-chat__badge",
                span { aria_hidden: "true", "\u{1F512}" }
                "Encrypted"
            }
            match key_mode {
                KeyMode::None => rsx! {
                    div { class: "encrypted-chat__no-key",
                        EmptyCard {
                            title: "Encrypted channel",
                            message: "You need a Nostr keypair to view encrypted messages. \
                                Set up a server-managed key in your profile, or connect \
                                a NIP-07 browser extension.",
                        }
                    }
                },
                _ => rsx! {
                    MessageList {
                        messages: messages,
                        loading: loading,
                        own_pubkey: own_pubkey,
                        own_name: own_name,
                    }
                    ComposeBox {
                        on_send: move |attempt: ComposeAttempt| on_send.call(attempt),
                        disabled: false,
                        encrypted: true,
                    }
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::ui::test_support::{assert_sizes_from_tokens, css_rule, has_decl};

    #[component]
    fn Probe(key_mode: KeyMode) -> Element {
        rsx! {
            EncryptedChat {
                messages: Vec::new(),
                key_mode,
                on_send: |_| {},
            }
        }
    }

    fn render(key_mode: KeyMode) -> String {
        let mut dom = VirtualDom::new_with_props(Probe, ProbeProps { key_mode });
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    #[test]
    fn a_member_with_no_key_gets_the_empty_card_and_no_composer() {
        let html = render(KeyMode::None);
        assert!(html.contains("<h2>Encrypted channel</h2>"), "{html}");
        assert!(html.contains("class=\"empty-card\""), "{html}");
        assert!(!html.contains("<h1"), "{html}");
        assert!(!html.contains("<main"), "{html}");
        assert!(!html.contains("<input"), "{html}");
        assert!(!html.contains("role=\"log\""), "{html}");
    }

    #[test]
    fn a_member_with_a_key_gets_the_list_and_the_composer() {
        for mode in [KeyMode::ServerManaged, KeyMode::External] {
            let html = render(mode);
            assert_eq!(html.matches("role=\"log\"").count(), 1, "{html}");
            assert!(html.contains("aria-label=\"Write a message\""), "{html}");
            assert!(
                html.contains("Messages in this channel are end-to-end encrypted"),
                "{html}"
            );
            assert!(
                html.contains("<span aria-hidden=\"true\">\u{1F512}</span>Encrypted"),
                "the lock is decoration, the word says it: {html}"
            );
            assert!(!html.contains("<h1"), "{html}");
        }
    }

    #[test]
    fn encrypted_css_is_on_tokens() {
        assert_sizes_from_tokens(ENCRYPTED_CHAT_CSS);
        let wrap = css_rule(ENCRYPTED_CHAT_CSS, ".encrypted-chat {");
        for decl in ["flex: 1 1 auto", "min-height: 0", "flex-direction: column"] {
            assert!(has_decl(&wrap, decl), "missing {decl}: {wrap}");
        }
    }
}
