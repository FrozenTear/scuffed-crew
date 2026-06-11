//! NIP-44 encrypted chat support for officer/private channels.
//!
//! For server-managed keys, encryption happens server-side — the frontend sends
//! plaintext to Axum, which encrypts + signs + publishes. Decryption is also
//! server-side: the frontend fetches decrypted messages via API.
//!
//! For external key users (NIP-07), encryption/decryption can happen client-side
//! via `window.nostr.nip44.encrypt()`/`decrypt()`.

use dioxus::prelude::*;

use scuffed_types::nostr::ChatMessage;

use super::compose_box::ComposeBox;
use super::message_list::MessageList;

const ENCRYPTED_CHAT_CSS: &str = r#"
.encrypted-chat {
    display: flex;
    flex-direction: column;
    height: 100%;
}

.encrypted-chat__badge {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 12px;
    background: var(--accent-soft);
    border-bottom: 1px solid var(--border);
    font-size: 11px;
    color: var(--accent);
}

.encrypted-chat__badge-icon {
    font-size: 12px;
}

.encrypted-chat__no-key {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    padding: 2rem;
    text-align: center;
    color: var(--text-2);
    font-size: 13px;
    flex: 1;
}

.encrypted-chat__no-key-title {
    font-weight: 600;
    color: var(--text);
    font-size: 14px;
}
"#;

/// Key mode for the current user.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KeyMode {
    /// Server manages the keypair — encryption/decryption happens server-side.
    ServerManaged,
    /// User has their own Nostr key (NIP-07 extension or imported).
    External,
    /// No key available — cannot participate in encrypted channels.
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
    on_send: EventHandler<String>,
    #[props(default = false)] loading: bool,
) -> Element {
    rsx! {
        style { {ENCRYPTED_CHAT_CSS} }
        div { class: "encrypted-chat",
            div { class: "encrypted-chat__badge",
                span { class: "encrypted-chat__badge-icon", "\u{1F512}" }
                "End-to-end encrypted channel"
            }
            match key_mode {
                KeyMode::None => rsx! {
                    div { class: "encrypted-chat__no-key",
                        div { class: "encrypted-chat__no-key-title",
                            "Encrypted Channel"
                        }
                        p {
                            "You need a Nostr keypair to view encrypted messages. "
                            "Set up a server-managed key in your profile, or connect "
                            "a NIP-07 browser extension."
                        }
                    }
                },
                _ => rsx! {
                    MessageList {
                        messages: messages,
                        loading: loading,
                    }
                    ComposeBox {
                        on_send: move |text: String| on_send.call(text),
                        disabled: false,
                        encrypted: true,
                    }
                },
            }
        }
    }
}
