//! NIP-29 group provisioning: create, update membership, delete groups.
//!
//! Groups are server-driven only — members cannot create arbitrary groups.
//! The GroupManager uses the relay admin key to manage group lifecycle.

use nostr::key::Keys;

use super::events::{EventBuilder, EventError};
use super::relay::{RelayClient, RelayError};
use scuffed_types::nostr::NostrEvent;

/// Errors from group management operations.
#[derive(Debug, thiserror::Error)]
pub enum GroupError {
    #[error("relay error: {0}")]
    Relay(#[from] RelayError),
    #[error("event construction error: {0}")]
    Event(#[from] EventError),
}

/// Manages NIP-29 group lifecycle on a relay.
///
/// Uses the relay admin keypair for all group management operations.
/// The admin key is typically the relay's own key or a designated
/// org admin key authorized in strfry's write policy.
pub struct GroupManager {
    relay: RelayClient,
    admin_keys: Keys,
}

impl GroupManager {
    /// Create a new group manager.
    ///
    /// - `relay`: connected relay client
    /// - `admin_keys`: keypair authorized for group admin operations
    pub fn new(relay: RelayClient, admin_keys: Keys) -> Self {
        Self { relay, admin_keys }
    }

    /// Create a new NIP-29 group on the relay.
    ///
    /// Publishes a group metadata event (kind 39000) with the group's
    /// name, description, and access settings.
    pub async fn create_group(
        &self,
        group_id: &str,
        name: &str,
        about: Option<&str>,
        is_public: bool,
        is_open: bool,
    ) -> Result<NostrEvent, GroupError> {
        let event = EventBuilder::build_group_metadata(
            &self.admin_keys,
            group_id,
            name,
            about,
            is_public,
            is_open,
        )?;

        let relay_event = EventBuilder::to_relay_event(&event);
        self.relay.publish_event(relay_event.clone()).await?;

        tracing::info!(group_id, name, "Created NIP-29 group");
        Ok(relay_event)
    }

    /// Add a member to a NIP-29 group (kind 9000).
    pub async fn add_member(
        &self,
        group_id: &str,
        member_pubkey: &str,
    ) -> Result<NostrEvent, GroupError> {
        let event = EventBuilder::build_add_user(&self.admin_keys, group_id, member_pubkey)?;
        let relay_event = EventBuilder::to_relay_event(&event);
        self.relay.publish_event(relay_event.clone()).await?;

        tracing::info!(group_id, member_pubkey, "Added member to NIP-29 group");
        Ok(relay_event)
    }

    /// Remove a member from a NIP-29 group (kind 9001).
    pub async fn remove_member(
        &self,
        group_id: &str,
        member_pubkey: &str,
    ) -> Result<NostrEvent, GroupError> {
        let event = EventBuilder::build_remove_user(&self.admin_keys, group_id, member_pubkey)?;
        let relay_event = EventBuilder::to_relay_event(&event);
        self.relay.publish_event(relay_event.clone()).await?;

        tracing::info!(group_id, member_pubkey, "Removed member from NIP-29 group");
        Ok(relay_event)
    }

    /// Update group metadata (name, about, access settings).
    pub async fn update_group(
        &self,
        group_id: &str,
        name: &str,
        about: Option<&str>,
        is_public: bool,
        is_open: bool,
    ) -> Result<NostrEvent, GroupError> {
        // Re-publishing metadata event replaces the previous one (same "d" tag)
        self.create_group(group_id, name, about, is_public, is_open)
            .await
    }

    /// Delete events from a group (NIP-09, kind 5).
    ///
    /// The relay may or may not honor deletion depending on policy.
    pub async fn delete_events(
        &self,
        event_ids: &[&str],
        reason: Option<&str>,
    ) -> Result<NostrEvent, GroupError> {
        let event = EventBuilder::build_delete_event(&self.admin_keys, event_ids, reason)?;
        let relay_event = EventBuilder::to_relay_event(&event);
        self.relay.publish_event(relay_event.clone()).await?;

        tracing::info!(?event_ids, "Deleted events from relay");
        Ok(relay_event)
    }

    /// Add multiple members to a group in a batch.
    pub async fn add_members(
        &self,
        group_id: &str,
        member_pubkeys: &[&str],
    ) -> Result<Vec<NostrEvent>, GroupError> {
        let mut results = Vec::with_capacity(member_pubkeys.len());
        for pubkey in member_pubkeys {
            results.push(self.add_member(group_id, pubkey).await?);
        }
        Ok(results)
    }

    /// Remove multiple members from a group in a batch.
    pub async fn remove_members(
        &self,
        group_id: &str,
        member_pubkeys: &[&str],
    ) -> Result<Vec<NostrEvent>, GroupError> {
        let mut results = Vec::with_capacity(member_pubkeys.len());
        for pubkey in member_pubkeys {
            results.push(self.remove_member(group_id, pubkey).await?);
        }
        Ok(results)
    }

    /// Get the admin public key hex string.
    pub fn admin_pubkey(&self) -> String {
        self.admin_keys.public_key().to_hex()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_manager_admin_pubkey() {
        let keys = Keys::generate();
        let expected = keys.public_key().to_hex();
        let client = RelayClient::new("ws://localhost:7777");
        let manager = GroupManager::new(client, keys);
        assert_eq!(manager.admin_pubkey(), expected);
    }
}
