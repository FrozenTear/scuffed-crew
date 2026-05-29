use chrono::Utc;
use serde::{Deserialize, Serialize};
use surrealdb::types::Datetime as SurrealDatetime;
use surrealdb_types::RecordId;
use surrealdb_types::SurrealValue;

use crate::types::Scrim;
use crate::{with_timeout, Database, DbResult};

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbScrim {
    #[surreal(default)]
    #[allow(dead_code)]
    id: Option<RecordId>,
    team_id: String,
    game_id: String,
    requested_by: String,
    opponent_name: Option<String>,
    scheduled_at: SurrealDatetime,
    duration_minutes: i64,
    status: String,
    notes: Option<String>,
    created_at: SurrealDatetime,
    updated_at: SurrealDatetime,
}

fn db_to_scrim(db: DbScrim) -> Scrim {
    let id = db
        .id
        .map(|r| crate::record_id_key_to_string(r.key))
        .unwrap_or_else(|| "unknown".to_string());
    Scrim {
        id,
        team_id: db.team_id,
        game_id: db.game_id,
        requested_by: db.requested_by,
        opponent_name: db.opponent_name,
        scheduled_at: db.scheduled_at.into(),
        duration_minutes: db.duration_minutes as u32,
        status: db.status,
        notes: db.notes,
        created_at: db.created_at.into(),
        updated_at: db.updated_at.into(),
    }
}

impl Database {
    pub async fn create_scrim(
        &self,
        team_id: &str,
        game_id: &str,
        requested_by: &str,
        scheduled_at: chrono::DateTime<Utc>,
        duration_minutes: u32,
        notes: Option<&str>,
    ) -> DbResult<Scrim> {
        with_timeout(async {
            let now = SurrealDatetime::from(Utc::now());
            let db_scrim = DbScrim {
                id: None,
                team_id: team_id.to_string(),
                game_id: game_id.to_string(),
                requested_by: requested_by.to_string(),
                opponent_name: None,
                scheduled_at: SurrealDatetime::from(scheduled_at),
                duration_minutes: duration_minutes as i64,
                status: "open".to_string(),
                notes: notes.map(|s| s.to_string()),
                created_at: now,
                updated_at: now,
            };
            let created: Option<DbScrim> = self.client.create("scrim").content(db_scrim).await?;
            Ok(db_to_scrim(created.ok_or_else(|| {
                crate::DbError::NotFound("Failed to create scrim".into())
            })?))
        })
        .await
    }

    pub async fn list_scrims(
        &self,
        team_id: Option<&str>,
        status: Option<&str>,
        limit: u32,
        offset: u32,
    ) -> DbResult<Vec<Scrim>> {
        with_timeout(async {
            let mut conditions = Vec::new();
            let mut query_str = String::from("SELECT * FROM scrim");

            if team_id.is_some() {
                conditions.push("team_id = $tid");
            }
            if status.is_some() {
                conditions.push("status = $st");
            }

            if !conditions.is_empty() {
                query_str.push_str(" WHERE ");
                query_str.push_str(&conditions.join(" AND "));
            }

            query_str.push_str(" ORDER BY scheduled_at DESC LIMIT $lim START $off");

            let mut q = self.client.query(&query_str);

            if let Some(tid) = team_id {
                q = q.bind(("tid", tid.to_string()));
            }
            if let Some(st) = status {
                q = q.bind(("st", st.to_string()));
            }

            q = q.bind(("lim", (limit + 1) as i64));
            q = q.bind(("off", offset as i64));

            let mut result = q.await?;
            let scrims: Vec<DbScrim> = result.take(0)?;
            Ok(scrims.into_iter().map(db_to_scrim).collect())
        })
        .await
    }

    pub async fn get_scrim(&self, id: &str) -> DbResult<Scrim> {
        with_timeout(async {
            let scrim: Option<DbScrim> = self.client.select(("scrim", id)).await?;
            scrim
                .map(db_to_scrim)
                .ok_or_else(|| crate::DbError::NotFound(format!("Scrim {id} not found")))
        })
        .await
    }

    pub async fn update_scrim_status(
        &self,
        id: &str,
        status: &str,
        opponent_name: Option<&str>,
    ) -> DbResult<Scrim> {
        with_timeout(async {
            let now = SurrealDatetime::from(Utc::now());
            let mut query_str = String::from("UPDATE $rid SET status = $st, updated_at = $now");

            if opponent_name.is_some() {
                query_str.push_str(", opponent_name = $opp");
            }

            query_str.push_str(" RETURN AFTER");

            let mut q = self
                .client
                .query(&query_str)
                .bind(("rid", RecordId::new("scrim", id)))
                .bind(("st", status.to_string()))
                .bind(("now", now));

            if let Some(opp) = opponent_name {
                q = q.bind(("opp", opp.to_string()));
            }

            let mut result = q.await?;
            let updated: Option<DbScrim> = result.take(0)?;
            updated
                .map(db_to_scrim)
                .ok_or_else(|| crate::DbError::NotFound(format!("Scrim {id} not found")))
        })
        .await
    }
}
