//! Nostr chat backend crate.
//!
//! Provides server-side relay client, event construction, NIP-42 authentication,
//! NIP-29 group provisioning, team channel auto-provisioning, and community
//! event builders for the Scuffed Crew chat system.

pub mod community;
pub mod nostr;
pub mod provisioning;

pub use community::{
    build_event_announcement, build_lfg_event, build_match_result_event, kinds as community_kinds,
    EventAnnouncement, LfgRequest, MatchResult,
};
pub use nostr::auth::{AuthError, AuthTokenRequest, AuthTokenResponse, KeyMode, NostrAuthService};
pub use nostr::encryption::{
    EncryptionError, EncryptionService, GiftWrappedEvent, UnwrappedMessage,
};
pub use nostr::events::{EventBuilder, EventError};
pub use nostr::groups::{GroupError, GroupManager};
pub use nostr::relay::{publish_event_oneshot, RelayClient, RelayError};
pub use provisioning::{
    ensure_team_channel_rows, officer_group_id, provision_all_team_channels,
    provision_team_channels, public_group_id, sync_team_roster, ProvisionedChannels,
    ProvisioningError,
};
