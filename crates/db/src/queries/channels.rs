use chrono::Utc;
use serde::{Deserialize, Serialize};
use surrealdb::types::Datetime as SurrealDatetime;
use surrealdb_types::{RecordId, SurrealValue};

use crate::types::{GroupLastSeen, GroupType, TeamChannel};
use crate::{with_timeout, Database, DbResult};

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbTeamChannel {
    #[surreal(default)]
    #[allow(dead_code)]
    id: Option<RecordId>,
    team_id: String,
    group_id: String,
    group_type: String,
    relay_url: String,
    is_active: bool,
    created_at: SurrealDatetime,
    synced_at: Option<SurrealDatetime>,
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbGroupLastSeen {
    #[surreal(default)]
    #[allow(dead_code)]
    id: Option<RecordId>,
    member_id: String,
    group_id: String,
    last_seen_at: SurrealDatetime,
    updated_at: SurrealDatetime,
}

fn parse_group_type(s: &str) -> GroupType {
    match s {
        "officer" => GroupType::Officer,
        _ => GroupType::Public,
    }
}

fn db_to_team_channel(db: DbTeamChannel) -> TeamChannel {
    let id = db
        .id
        .map(|r| crate::record_id_key_to_string(r.key))
        .unwrap_or_else(|| "unknown".to_string());
    TeamChannel {
        id,
        team_id: db.team_id,
        group_id: db.group_id,
        group_type: parse_group_type(&db.group_type),
        relay_url: db.relay_url,
        is_active: db.is_active,
        created_at: db.created_at.into(),
        synced_at: db.synced_at.map(|d| d.into()),
    }
}

fn db_to_group_last_seen(db: DbGroupLastSeen) -> GroupLastSeen {
    let id = db
        .id
        .map(|r| crate::record_id_key_to_string(r.key))
        .unwrap_or_else(|| "unknown".to_string());
    GroupLastSeen {
        id,
        member_id: db.member_id,
        group_id: db.group_id,
        last_seen_at: db.last_seen_at.into(),
        updated_at: db.updated_at.into(),
    }
}

impl Database {
    /// Create a team channel record (maps a team to a NIP-29 group).
    pub async fn create_team_channel(
        &self,
        team_id: &str,
        group_id: &str,
        group_type: GroupType,
        relay_url: &str,
    ) -> DbResult<TeamChannel> {
        with_timeout(async {
            let db = DbTeamChannel {
                id: None,
                team_id: team_id.to_string(),
                group_id: group_id.to_string(),
                group_type: group_type.to_string(),
                relay_url: relay_url.to_string(),
                is_active: true,
                created_at: SurrealDatetime::from(Utc::now()),
                synced_at: None,
            };
            let created: Option<DbTeamChannel> =
                self.client.create("team_channel").content(db).await?;
            Ok(db_to_team_channel(created.ok_or_else(|| {
                crate::DbError::NotFound("Failed to create team channel".into())
            })?))
        })
        .await
    }

    /// Get all active channels for a team.
    pub async fn get_team_channels(&self, team_id: &str) -> DbResult<Vec<TeamChannel>> {
        let tid = team_id.to_string();
        with_timeout(async {
            let mut result = self
                .client
                .query("SELECT * FROM team_channel WHERE team_id = $tid AND is_active = true")
                .bind(("tid", tid))
                .await?;
            let channels: Vec<DbTeamChannel> = result.take(0)?;
            Ok(channels.into_iter().map(db_to_team_channel).collect())
        })
        .await
    }

    /// Get a channel by its NIP-29 group ID.
    pub async fn get_channel_by_group_id(&self, group_id: &str) -> DbResult<Option<TeamChannel>> {
        let gid = group_id.to_string();
        with_timeout(async {
            let mut result = self
                .client
                .query("SELECT * FROM team_channel WHERE group_id = $gid LIMIT 1")
                .bind(("gid", gid))
                .await?;
            let channels: Vec<DbTeamChannel> = result.take(0)?;
            Ok(channels.into_iter().next().map(db_to_team_channel))
        })
        .await
    }

    /// Mark a channel as synced with the relay.
    pub async fn update_channel_sync(&self, group_id: &str) -> DbResult<()> {
        let gid = group_id.to_string();
        with_timeout(async {
            self.client
                .query("UPDATE team_channel SET synced_at = time::now() WHERE group_id = $gid")
                .bind(("gid", gid))
                .await?;
            Ok(())
        })
        .await
    }

    /// List all group IDs that are officer-only (used for read ACL filtering).
    pub async fn list_officer_group_ids(&self) -> DbResult<Vec<String>> {
        with_timeout(async {
            let mut result = self
                .client
                .query(
                    "SELECT group_id FROM team_channel WHERE group_type = 'officer' AND is_active = true",
                )
                .await?;
            let rows: Vec<DbTeamChannel> = result.take(0)?;
            Ok(rows.into_iter().map(|r| r.group_id).collect())
        })
        .await
    }

    /// Soft-delete a team channel.
    pub async fn deactivate_team_channel(&self, group_id: &str) -> DbResult<()> {
        let gid = group_id.to_string();
        with_timeout(async {
            self.client
                .query("UPDATE team_channel SET is_active = false WHERE group_id = $gid")
                .bind(("gid", gid))
                .await?;
            Ok(())
        })
        .await
    }

    /// Upsert a member's last-seen timestamp for a group (for unread badges).
    pub async fn upsert_last_seen(&self, member_id: &str, group_id: &str) -> DbResult<()> {
        let mid = member_id.to_string();
        let gid = group_id.to_string();
        with_timeout(async {
            self.client
                .query(
                    r#"UPSERT group_last_seen
                       SET member_id = $mid, group_id = $gid,
                           last_seen_at = time::now(), updated_at = time::now()
                       WHERE member_id = $mid AND group_id = $gid"#,
                )
                .bind(("mid", mid))
                .bind(("gid", gid))
                .await?;
            Ok(())
        })
        .await
    }

    /// Get a member's last-seen timestamp for a specific group.
    pub async fn get_last_seen(
        &self,
        member_id: &str,
        group_id: &str,
    ) -> DbResult<Option<GroupLastSeen>> {
        let mid = member_id.to_string();
        let gid = group_id.to_string();
        with_timeout(async {
            let mut result = self
                .client
                .query(
                    "SELECT * FROM group_last_seen WHERE member_id = $mid AND group_id = $gid LIMIT 1",
                )
                .bind(("mid", mid))
                .bind(("gid", gid))
                .await?;
            let entries: Vec<DbGroupLastSeen> = result.take(0)?;
            Ok(entries.into_iter().next().map(db_to_group_last_seen))
        })
        .await
    }

    /// Get all last-seen entries for a member (for computing unread state across groups).
    pub async fn get_member_last_seen_all(&self, member_id: &str) -> DbResult<Vec<GroupLastSeen>> {
        let mid = member_id.to_string();
        with_timeout(async {
            let mut result = self
                .client
                .query("SELECT * FROM group_last_seen WHERE member_id = $mid")
                .bind(("mid", mid))
                .await?;
            let entries: Vec<DbGroupLastSeen> = result.take(0)?;
            Ok(entries.into_iter().map(db_to_group_last_seen).collect())
        })
        .await
    }
}
