//! Load the read directory from SurrealDB.

use std::collections::HashMap;

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
