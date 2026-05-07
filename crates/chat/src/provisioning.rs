//! Team channel auto-provisioning service.
//!
//! When a team is created or updated, this service:
//! 1. Creates a public NIP-29 group for the team (general chat)
//! 2. Creates a private NIP-29 group for officers (encrypted via NIP-44)
//! 3. Syncs team roster → NIP-29 group membership with role mapping
//!
//! Group creation is server-driven only — members cannot create arbitrary groups.

use scuffed_db::{Database, GroupType, Nip29GroupRole};

use crate::nostr::groups::{GroupError, GroupManager};

/// Errors from provisioning operations.
#[derive(Debug, thiserror::Error)]
pub enum ProvisioningError {
    #[error("database error: {0}")]
    Db(#[from] scuffed_db::DbError),
    #[error("group management error: {0}")]
    Group(#[from] GroupError),
}

/// Result of provisioning channels for a team.
#[derive(Debug)]
pub struct ProvisionedChannels {
    /// The public team channel group ID.
    pub public_group_id: String,
    /// The officer channel group ID (if created).
    pub officer_group_id: Option<String>,
    /// Number of members synced.
    pub members_synced: usize,
}

/// Provision NIP-29 groups for a team and sync membership.
///
/// Creates two channels per team:
/// - `{team_slug}` — public group for all team members
/// - `{team_slug}-officers` — private group for officers/admins (NIP-44 encrypted)
///
/// Idempotent: if channels already exist in the DB, skips creation.
pub async fn provision_team_channels(
    db: &Database,
    group_manager: &GroupManager,
    team_id: &str,
    team_name: &str,
    team_slug: &str,
    relay_url: &str,
) -> Result<ProvisionedChannels, ProvisioningError> {
    let existing = db.get_team_channels(team_id).await?;

    // Create public channel if not exists
    let public_group_id = format!("{team_slug}");
    if !existing.iter().any(|c| c.group_type == GroupType::Public) {
        group_manager
            .create_group(
                &public_group_id,
                &format!("{team_name} — General"),
                Some(&format!("Public channel for {team_name}")),
                true,  // public
                false, // not open (server-managed membership)
            )
            .await?;

        db.create_team_channel(team_id, &public_group_id, GroupType::Public, relay_url)
            .await?;

        tracing::info!(team_id, group_id = %public_group_id, "Provisioned public team channel");
    }

    // Create officer channel if not exists
    let officer_group_id = format!("{team_slug}-officers");
    let has_officer = existing
        .iter()
        .any(|c| c.group_type == GroupType::Officer);

    if !has_officer {
        group_manager
            .create_group(
                &officer_group_id,
                &format!("{team_name} — Officers"),
                Some("Officer-only channel. Messages are NIP-44 encrypted."),
                false, // private
                false, // not open
            )
            .await?;

        db.create_team_channel(team_id, &officer_group_id, GroupType::Officer, relay_url)
            .await?;

        tracing::info!(team_id, group_id = %officer_group_id, "Provisioned officer channel");
    }

    Ok(ProvisionedChannels {
        public_group_id,
        officer_group_id: Some(officer_group_id),
        members_synced: 0, // sync_team_roster fills this in
    })
}

/// Sync a team's roster to NIP-29 group membership.
///
/// Role mapping (from plan):
/// - Admin → group admin (all channels)
/// - Officer → group admin (all channels)
/// - Member → group member (public channels only)
/// - Recruit → group member (public channels only)
///
/// Officers and admins get access to both public and officer channels.
/// Members and recruits only get access to public channels.
pub async fn sync_team_roster(
    db: &Database,
    group_manager: &GroupManager,
    team_id: &str,
) -> Result<usize, ProvisioningError> {
    let roster = db.get_team_roster(team_id).await?;
    let channels = db.get_team_channels(team_id).await?;

    let mut synced = 0;

    for entry in &roster {
        // Look up the member to get their pubkey
        let member = match db.get_member(&entry.member_id).await? {
            Some(m) => m,
            None => continue,
        };

        let pubkey = match &member.nostr_pubkey {
            Some(pk) => pk.clone(),
            None => continue, // No Nostr key — skip
        };

        let nip29_role = member.org_role.to_nip29_role();

        for channel in &channels {
            // Officers/admins get all channels; members/recruits only public
            let has_access = match channel.group_type {
                GroupType::Public => true,
                GroupType::Officer => matches!(
                    nip29_role,
                    Nip29GroupRole::GroupAdmin
                ),
            };

            if has_access {
                // Add member to group (NIP-29 add-user is idempotent on the relay)
                if let Err(e) = group_manager.add_member(&channel.group_id, &pubkey).await {
                    tracing::warn!(
                        member_id = %entry.member_id,
                        group_id = %channel.group_id,
                        "Failed to sync member to group: {e}"
                    );
                } else {
                    synced += 1;
                }
            }
        }
    }

    // Mark channels as synced
    for channel in &channels {
        let _ = db.update_channel_sync(&channel.group_id).await;
    }

    tracing::info!(team_id, synced, "Roster synced to NIP-29 groups");
    Ok(synced)
}
