//! Load the read directory and the write-side group map from SurrealDB.
//!
//! Both use the same rows: active members' Nostr pubkeys, active roster
//! edges, and `team_channel` rows. The read gate keeps deactivated channels
//! so officer history stays classified. The write map only includes active
//! channels, matching `POST /api/chat/send-public`.

use std::collections::{HashMap, HashSet};

use scuffed_db::{Database, GroupType};

use crate::read_policy::{GroupAcl, MemberAcl, ReadDirectory};

pub async fn load_read_directory(db: &Database) -> scuffed_db::DbResult<ReadDirectory> {
    let members = db.list_nostr_identities().await?;
    let pairs = db.list_active_roster_pairs().await?;
    let channels = db.list_all_team_channels().await?;

    let mut teams_by_member: HashMap<String, std::collections::HashSet<String>> = HashMap::new();
    for (member_id, team_id) in pairs {
        teams_by_member
            .entry(member_id)
            .or_default()
            .insert(team_id);
    }

    let acls = members.into_iter().filter_map(|member| {
        let pubkey = member.nostr_pubkey?;
        let teams = teams_by_member.remove(&member.id).unwrap_or_default();
        Some(MemberAcl {
            pubkey,
            member_id: member.id,
            officer: member.org_role.can_access_officer_channel(),
            teams,
        })
    });

    let groups = channels.into_iter().map(|channel| {
        (
            channel.group_id,
            GroupAcl {
                team_id: channel.team_id,
                officer_only: channel.group_type == GroupType::Officer,
            },
        )
    });

    Ok(ReadDirectory::from_parts(acls, groups))
}

/// `group_id` to the pubkeys that may write an `h` tag for that channel.
///
/// A pubkey is included when that member has an active roster edge on the
/// channel's team. Public and officer channel ids for the same team share
/// that roster. Kind 1059 gift wraps are not in this map: they have no `h` tag.
pub async fn load_group_writers(
    db: &Database,
) -> scuffed_db::DbResult<HashMap<String, HashSet<String>>> {
    let members = db.list_nostr_identities().await?;
    let pairs = db.list_active_roster_pairs().await?;
    let channels = db.list_all_team_channels().await?;

    let mut pubkey_by_member: HashMap<String, String> = HashMap::new();
    for member in members {
        let Some(pubkey) = member.nostr_pubkey else {
            continue;
        };
        let pubkey = pubkey.to_ascii_lowercase();
        if pubkey.len() == 64 && pubkey.chars().all(|c| c.is_ascii_hexdigit()) {
            pubkey_by_member.insert(member.id, pubkey);
        }
    }

    let mut pubkeys_by_team: HashMap<String, HashSet<String>> = HashMap::new();
    for (member_id, team_id) in pairs {
        let Some(pubkey) = pubkey_by_member.get(&member_id) else {
            continue;
        };
        pubkeys_by_team
            .entry(team_id)
            .or_default()
            .insert(pubkey.clone());
    }

    let mut groups = HashMap::new();
    for channel in channels {
        if !channel.is_active {
            continue;
        }
        let writers = pubkeys_by_team
            .get(&channel.team_id)
            .cloned()
            .unwrap_or_default();
        groups.insert(channel.group_id, writers);
    }
    Ok(groups)
}
