//! Encrypted DM (NIP-44) UI components.
//!
//! Frontend-only types live in `types`; once the backend types crate exposes
//! the same shapes, swap to `scuffed_types::nostr::dm::*`.

mod compose;
mod conversation_list;
mod message_thread;
mod reply_input;
pub mod types;

pub use compose::DmComposeModal;
pub use conversation_list::ConversationList;
pub use message_thread::MessageThread;
pub use types::*;
