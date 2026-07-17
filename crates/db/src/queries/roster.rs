use serde::{Deserialize, Serialize};
use surrealdb::types::Datetime as SurrealDatetime;
use surrealdb_types::RecordId;
use surrealdb_types::SurrealValue;

use crate::types::{RosterEntry, TeamRole};
use crate::{with_timeout, Database, DbResult};

/// Raw DB result from a RELATE / graph query on plays_on.
#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbRosterEntry {
    id: Option<String>,
    #[serde(rename = "in")]
    #[surreal(rename = "in")]
    in_id: Option<String>,
    out: Option<String>,
    team_role: String,
    joined_at: SurrealDatetime,
    is_active: bool,
}

fn parse_team_role(s: &str) -> TeamRole {
    match s {
        "captain" => TeamRole::Captain,
        "player" => TeamRole::Player,
        "sub" => TeamRole::Sub,
        "coach" => TeamRole::Coach,
        _ => TeamRole::Player,
    }
}

fn extract_record_id(thing_str: &str) -> String {
    // SurrealDB returns IDs like "member:abc" or "team:xyz" — extract after colon
    thing_str
        .split_once(':')
        .map(|(_, id)| id.to_string())
        .unwrap_or_else(|| thing_str.to_string())
}

fn db_to_roster_entry(db: DbRosterEntry) -> RosterEntry {
    let id = db
        .id
        .as_deref()
        .map(extract_record_id)
        .unwrap_or_else(|| "unknown".to_string());
    let member_id = db
        .in_id
        .as_deref()
        .map(extract_record_id)
        .unwrap_or_default();
    let team_id = db.out.as_deref().map(extract_record_id).unwrap_or_default();

    RosterEntry {
        id,
        member_id,
        team_id,
        team_role: parse_team_role(&db.team_role),
        joined_at: db.joined_at.into(),
        is_active: db.is_active,
    }
}

fn member_rid(id: &str) -> RecordId {
    RecordId::new("member", id)
}

fn team_rid(id: &str) -> RecordId {
    RecordId::new("team", id)
}

impl Database {
    /// Add a member to a team's roster.
    pub async fn add_to_roster(
        &self,
        member_id: &str,
        team_id: &str,
        role: TeamRole,
    ) -> DbResult<RosterEntry> {
        with_timeout(async {
            // RELATE then SELECT back with string casts for proper deserialization
            let mut result = self
                .client
                .query(
                    r#"LET $entry = (RELATE $member_rid -> plays_on -> $team_rid
                       SET team_role = $role, joined_at = time::now(), is_active = true);
                       SELECT *, meta::id(id) as id, <string>in as in, <string>out as out
                       FROM $entry"#,
                )
                .bind(("member_rid", member_rid(member_id)))
                .bind(("team_rid", team_rid(team_id)))
                .bind(("role", role.to_string()))
                .await?;
            let entries: Vec<DbRosterEntry> = result.take(1)?;
            entries
                .into_iter()
                .next()
                .map(db_to_roster_entry)
                .ok_or_else(|| crate::DbError::NotFound("Failed to create roster entry".into()))
        })
        .await
    }

    /// Get a team's roster (all active members on the team).
    pub async fn get_team_roster(&self, team_id: &str) -> DbResult<Vec<RosterEntry>> {
        with_timeout(async {
            let mut result = self
                .client
                .query(
                    r#"SELECT *, meta::id(id) as id, <string>in as in, <string>out as out
                       FROM plays_on
                       WHERE out = $team_rid AND is_active = true"#,
                )
                .bind(("team_rid", team_rid(team_id)))
                .await?;
            let entries: Vec<DbRosterEntry> = result.take(0)?;
            Ok(entries.into_iter().map(db_to_roster_entry).collect())
        })
        .await
    }

    /// Get all teams a member is on.
    pub async fn get_member_teams(&self, member_id: &str) -> DbResult<Vec<RosterEntry>> {
        with_timeout(async {
            let mut result = self
                .client
                .query(
                    r#"SELECT *, meta::id(id) as id, <string>in as in, <string>out as out
                       FROM plays_on
                       WHERE in = $member_rid AND is_active = true"#,
                )
                .bind(("member_rid", member_rid(member_id)))
                .await?;
            let entries: Vec<DbRosterEntry> = result.take(0)?;
            Ok(entries.into_iter().map(db_to_roster_entry).collect())
        })
        .await
    }

    /// True if `member_id` is an active roster entry on `team_id`.
    pub async fn is_on_team_roster(&self, member_id: &str, team_id: &str) -> DbResult<bool> {
        with_timeout(async {
            #[derive(Debug, Deserialize, SurrealValue)]
            struct CountResult {
                count: i64,
            }
            let mut result = self
                .client
                .query(
                    r#"SELECT count() FROM plays_on
                       WHERE in = $member_rid AND out = $team_rid AND is_active = true
                       GROUP ALL"#,
                )
                .bind(("member_rid", member_rid(member_id)))
                .bind(("team_rid", team_rid(team_id)))
                .await?;
            let rows: Vec<CountResult> = result.take(0)?;
            Ok(rows.first().map(|r| r.count > 0).unwrap_or(false))
        })
        .await
    }

    /// Update a member's role on a team.
    pub async fn update_roster_role(
        &self,
        member_id: &str,
        team_id: &str,
        new_role: TeamRole,
    ) -> DbResult<()> {
        with_timeout(async {
            self.client
                .query(
                    r#"UPDATE plays_on SET team_role = $role
                       WHERE in = $member_rid
                       AND out = $team_rid
                       AND is_active = true"#,
                )
                .bind(("role", new_role.to_string()))
                .bind(("member_rid", member_rid(member_id)))
                .bind(("team_rid", team_rid(team_id)))
                .await?;
            Ok(())
        })
        .await
    }

    /// Remove a member from a team (soft delete).
    pub async fn remove_from_roster(&self, member_id: &str, team_id: &str) -> DbResult<()> {
        with_timeout(async {
            self.client
                .query(
                    r#"UPDATE plays_on SET is_active = false
                       WHERE in = $member_rid
                       AND out = $team_rid"#,
                )
                .bind(("member_rid", member_rid(member_id)))
                .bind(("team_rid", team_rid(team_id)))
                .await?;
            Ok(())
        })
        .await
    }
}
