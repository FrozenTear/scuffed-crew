use chrono::Utc;
use serde::{Deserialize, Serialize};
use surrealdb::types::Datetime as SurrealDatetime;
use surrealdb_types::RecordId;
use surrealdb_types::SurrealValue;

use crate::types::Team;
use crate::{with_timeout, Database, DbResult};

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbTeam {
    #[surreal(default)]
    #[allow(dead_code)]
    id: Option<RecordId>,
    name: String,
    game_id: String,
    color: Option<String>,
    division: Option<String>,
    lore_quote: Option<String>,
    logo_url: Option<String>,
    is_active: bool,
    created_at: SurrealDatetime,
}

fn db_to_team(db: DbTeam) -> Team {
    let id = db
        .id
        .map(|r| crate::record_id_key_to_string(r.key))
        .unwrap_or_else(|| "unknown".to_string());
    Team {
        id,
        name: db.name,
        game_id: db.game_id,
        color: db.color,
        division: db.division,
        lore_quote: db.lore_quote,
        logo_url: db.logo_url,
        is_active: db.is_active,
        created_at: db.created_at.into(),
    }
}

impl Database {
    /// Create a new team.
    pub async fn create_team(
        &self,
        name: &str,
        game_id: &str,
        color: Option<&str>,
        division: Option<&str>,
        lore_quote: Option<&str>,
    ) -> DbResult<Team> {
        with_timeout(async {
            let db_team = DbTeam {
                id: None,
                name: name.to_string(),
                game_id: game_id.to_string(),
                color: color.map(|s| s.to_string()),
                division: division.map(|s| s.to_string()),
                lore_quote: lore_quote.map(|s| s.to_string()),
                logo_url: None,
                is_active: true,
                created_at: SurrealDatetime::from(Utc::now()),
            };
            let created: Option<DbTeam> = self.client.create("team").content(db_team).await?;
            Ok(db_to_team(created.ok_or_else(|| {
                crate::DbError::NotFound("Failed to create team".into())
            })?))
        })
        .await
    }

    /// List all active teams.
    pub async fn list_teams(&self) -> DbResult<Vec<Team>> {
        with_timeout(async {
            let mut result = self
                .client
                .query("SELECT * FROM team WHERE is_active = true ORDER BY name ASC")
                .await?;
            let teams: Vec<DbTeam> = result.take(0)?;
            Ok(teams.into_iter().map(db_to_team).collect())
        })
        .await
    }

    /// List active teams with cursor-based pagination.
    pub async fn list_teams_paginated(&self, limit: u32, offset: u32) -> DbResult<Vec<Team>> {
        with_timeout(async {
            let fetch = limit + 1;
            let mut result = self
                .client
                .query("SELECT * FROM team WHERE is_active = true ORDER BY name ASC LIMIT $lim START $off")
                .bind(("lim", fetch))
                .bind(("off", offset))
                .await?;
            let teams: Vec<DbTeam> = result.take(0)?;
            Ok(teams.into_iter().map(db_to_team).collect())
        })
        .await
    }

    /// Get a team by ID.
    pub async fn get_team(&self, id: &str) -> DbResult<Option<Team>> {
        with_timeout(async {
            let db_team: Option<DbTeam> = self.client.select(("team", id)).await?;
            Ok(db_team.map(db_to_team))
        })
        .await
    }

    /// Update a team's fields.
    pub async fn update_team(
        &self,
        id: &str,
        name: Option<&str>,
        game_id: Option<&str>,
        color: Option<Option<&str>>,
        division: Option<Option<&str>>,
        lore_quote: Option<Option<&str>>,
    ) -> DbResult<Team> {
        with_timeout(async {
            let existing: Option<DbTeam> = self.client.select(("team", id)).await?;
            let mut db =
                existing.ok_or_else(|| crate::DbError::NotFound(format!("Team {id} not found")))?;

            if let Some(n) = name {
                db.name = n.to_string();
            }
            if let Some(g) = game_id {
                db.game_id = g.to_string();
            }
            if let Some(c) = color {
                db.color = c.map(|s| s.to_string());
            }
            if let Some(d) = division {
                db.division = d.map(|s| s.to_string());
            }
            if let Some(q) = lore_quote {
                db.lore_quote = q.map(|s| s.to_string());
            }

            let updated: Option<DbTeam> = self.client.update(("team", id)).content(db).await?;
            Ok(db_to_team(updated.ok_or_else(|| {
                crate::DbError::NotFound(format!("Team {id} not found after update"))
            })?))
        })
        .await
    }
}
