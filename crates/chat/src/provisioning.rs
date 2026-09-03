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

/// Stable NIP-29 group id for a team's public channel.
///
/// Uses the team record id (unique, rename-stable). Site discovers ids via
/// `GET /api/teams/:id/channels` rather than guessing slugs.
pub fn public_group_id(team_id: &str) -> String {
    team_id.to_string()
}

/// Stable NIP-29 group id for a team's officer (gift-wrap) channel.
pub fn officer_group_id(team_id: &str) -> String {
    format!("{team_id}-officers")
}

/// Write `team_channel` rows for a team if they are missing (F-API-003).
///
/// This is the path `send_encrypted` actually needs: a `GroupType::Officer`
/// row so `get_channel_by_group_id` does not 404. Relay NIP-29 group
/// creation is optional — there is no configured relay-admin key today, so
/// team create/update must not depend on a live `GroupManager`.
///
/// Idempotent: existing public/officer rows are left untouched.
pub async fn ensure_team_channel_rows(
    db: &Database,
    team_id: &str,
    relay_url: &str,
) -> Result<ProvisionedChannels, ProvisioningError> {
    let existing = db.get_team_channels(team_id).await?;
    let public_group_id = public_group_id(team_id);
    let officer_group_id = officer_group_id(team_id);

    if !existing.iter().any(|c| c.group_type == GroupType::Public) {
        db.create_team_channel(team_id, &public_group_id, GroupType::Public, relay_url)
            .await?;
        tracing::info!(team_id, group_id = %public_group_id, "Provisioned public team channel");
    }

    if !existing.iter().any(|c| c.group_type == GroupType::Officer) {
        db.create_team_channel(team_id, &officer_group_id, GroupType::Officer, relay_url)
            .await?;
        tracing::info!(team_id, group_id = %officer_group_id, "Provisioned officer channel");
    }

    Ok(ProvisionedChannels {
        public_group_id,
        officer_group_id: Some(officer_group_id),
        members_synced: 0,
    })
}

/// Ensure every active team has public + officer `team_channel` rows.
///
/// Safe backfill for teams created before F-API-003. Continues past per-team
/// errors so one bad row cannot block the rest.
pub async fn provision_all_team_channels(
    db: &Database,
    relay_url: &str,
) -> Result<usize, ProvisioningError> {
    let teams = db.list_teams().await?;
    let mut ok = 0usize;
    for team in teams {
        match ensure_team_channel_rows(db, &team.id, relay_url).await {
            Ok(_) => ok += 1,
            Err(e) => {
                tracing::error!(team_id = %team.id, error = %e, "Team channel backfill failed");
            }
        }
    }
    Ok(ok)
}

/// Provision NIP-29 groups for a team and sync membership.
///
/// Writes DB rows first (so encrypt can find a channel), then best-effort
/// publishes group metadata if a `GroupManager` is provided. Relay failures
/// do not roll back the DB rows.
///
/// Group ids:
/// - `{team_id}` — public group for all team members
/// - `{team_id}-officers` — private group for officers/admins (NIP-44)
///
/// `team_slug` is accepted for compatibility but ignored; ids are team-id
/// based so a rename cannot collide or orphan `send_encrypted` lookups.
///
/// Idempotent: if channels already exist in the DB, skips creation.
pub async fn provision_team_channels(
    db: &Database,
    group_manager: Option<&GroupManager>,
    team_id: &str,
    team_name: &str,
    _team_slug: &str,
    relay_url: &str,
) -> Result<ProvisionedChannels, ProvisioningError> {
    let provisioned = ensure_team_channel_rows(db, team_id, relay_url).await?;

    if let Some(group_manager) = group_manager {
        if let Err(e) = group_manager
            .create_group(
                &provisioned.public_group_id,
                &format!("{team_name} — General"),
                Some(&format!("Public channel for {team_name}")),
                true,  // public
                false, // not open (server-managed membership)
            )
            .await
        {
            tracing::warn!(
                team_id,
                group_id = %provisioned.public_group_id,
                error = %e,
                "NIP-29 public group publish failed; DB channel row kept"
            );
        }

        if let Some(ref officer_group_id) = provisioned.officer_group_id {
            if let Err(e) = group_manager
                .create_group(
                    officer_group_id,
                    &format!("{team_name} — Officers"),
                    Some("Officer-only channel. Messages are NIP-44 encrypted."),
                    false, // private
                    false, // not open
                )
                .await
            {
                tracing::warn!(
                    team_id,
                    group_id = %officer_group_id,
                    error = %e,
                    "NIP-29 officer group publish failed; DB channel row kept"
                );
            }
        }
    }

    Ok(provisioned)
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
                GroupType::Officer => matches!(nip29_role, Nip29GroupRole::GroupAdmin),
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

#[cfg(test)]
mod tests {
    use super::*;
    use scuffed_db::migrations::run_migrations;
    use scuffed_db::Database;

    #[test]
    fn group_ids_are_team_id_stable() {
        assert_eq!(public_group_id("teamalpha"), "teamalpha");
        assert_eq!(officer_group_id("teamalpha"), "teamalpha-officers");
    }

    #[tokio::test]
    async fn ensure_rows_creates_officer_channel_send_encrypted_can_find() {
        let db = Database::connect_memory().await.expect("mem db");
        run_migrations(&db.client).await.expect("migrations");
        let team = db
            .create_team("Alpha Squad", "ow2", None, None, None)
            .await
            .expect("create team");

        let first = ensure_team_channel_rows(&db, &team.id, "ws://relay.test")
            .await
            .expect("provision");
        let second = ensure_team_channel_rows(&db, &team.id, "ws://other")
            .await
            .expect("idempotent");
        assert_eq!(first.public_group_id, second.public_group_id);
        assert_eq!(first.officer_group_id, second.officer_group_id);

        let channels = db.get_team_channels(&team.id).await.expect("list");
        assert_eq!(channels.len(), 2);
        assert!(channels.iter().any(|c| c.group_type == GroupType::Public));
        assert!(channels.iter().any(|c| c.group_type == GroupType::Officer));

        let officer_id = first.officer_group_id.expect("officer id");
        let found = db
            .get_channel_by_group_id(&officer_id)
            .await
            .expect("lookup")
            .expect("send_encrypted must find officer channel");
        assert_eq!(found.group_type, GroupType::Officer);
        assert_eq!(found.team_id, team.id);
        assert_eq!(found.relay_url, "ws://relay.test");
    }

    #[tokio::test]
    async fn backfill_provisions_existing_teams() {
        let db = Database::connect_memory().await.expect("mem db");
        run_migrations(&db.client).await.expect("migrations");
        let a = db.create_team("A", "ow2", None, None, None).await.unwrap();
        let b = db.create_team("B", "ow2", None, None, None).await.unwrap();

        let n = provision_all_team_channels(&db, "")
            .await
            .expect("backfill");
        assert_eq!(n, 2);
        assert_eq!(db.get_team_channels(&a.id).await.unwrap().len(), 2);
        assert_eq!(db.get_team_channels(&b.id).await.unwrap().len(), 2);
        assert!(db
            .get_channel_by_group_id(&officer_group_id(&a.id))
            .await
            .unwrap()
            .is_some());
    }
}
