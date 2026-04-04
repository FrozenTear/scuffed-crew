//! Nostr chat backend crate.
//!
//! Provides server-side relay client, event construction, NIP-42 authentication,
//! and NIP-29 group provisioning for the Scuffed Crew chat system.

pub mod nostr;

pub use nostr::auth::{AuthError, AuthTokenRequest, AuthTokenResponse, KeyMode, NostrAuthService};
pub use nostr::encryption::{EncryptionError, EncryptionService, GiftWrappedEvent, UnwrappedMessage};
pub use nostr::events::{EventBuilder, EventError};
pub use nostr::groups::{GroupError, GroupManager};
pub use nostr::relay::{RelayClient, RelayError};
