use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use surrealdb::types::Datetime as SurrealDatetime;
use surrealdb_types::{RecordId, SurrealValue};

use crate::types::Season;
use crate::{with_timeout, Database, DbResult};

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbSeason {
    #[surreal(default)]
    #[allow(dead_code)]
    id: Option<RecordId>,
    name: String,
    starts_at: SurrealDatetime,
    ends_at: SurrealDatetime,
    #[surreal(default)]
    is_current: bool,
}

fn db_to_season(db: DbSeason) -> Season {
    let id = db
        .id
        .map(|r| crate::record_id_key_to_string(r.key))
        .unwrap_or_else(|| "unknown".to_string());
    Season {
        id,
        name: db.name,
        starts_at: db.starts_at.into(),
        ends_at: db.ends_at.into(),
        is_current: db.is_current,
    }
}

impl Database {
    pub async fn list_seasons(&self) -> DbResult<Vec<Season>> {
        with_timeout(async {
            let mut result = self
                .client
                .query("SELECT * FROM season ORDER BY starts_at DESC")
                .await?;
            let rows: Vec<DbSeason> = result.take(0)?;
            Ok(rows.into_iter().map(db_to_season).collect())
        })
        .await
    }

    pub async fn create_season(
        &self,
        name: &str,
        starts_at: DateTime<Utc>,
        ends_at: DateTime<Utc>,
        is_current: bool,
    ) -> DbResult<Season> {
        with_timeout(async {
            if is_current {
                // Clear other current flags (best-effort).
                let _ = self
                    .client
                    .query("UPDATE season SET is_current = false WHERE is_current = true")
                    .await?;
            }
            let row = DbSeason {
                id: None,
                name: name.to_string(),
                starts_at: SurrealDatetime::from(starts_at),
                ends_at: SurrealDatetime::from(ends_at),
                is_current,
            };
            let created: Option<DbSeason> = self.client.create("season").content(row).await?;
            Ok(db_to_season(created.ok_or_else(|| {
                crate::DbError::NotFound("Failed to create season".into())
            })?))
        })
        .await
    }

    pub async fn get_season(&self, id: &str) -> DbResult<Option<Season>> {
        with_timeout(async {
            let row: Option<DbSeason> = self.client.select(("season", id)).await?;
            Ok(row.map(db_to_season))
        })
        .await
    }
    /// Replace every field of a season. `is_current = true` clears the flag on
    /// all other seasons first (same best-effort rule as create).
    pub async fn update_season(
        &self,
        id: &str,
        name: &str,
        starts_at: DateTime<Utc>,
        ends_at: DateTime<Utc>,
        is_current: bool,
    ) -> DbResult<Season> {
        with_timeout(async {
            let existing: Option<DbSeason> = self.client.select(("season", id)).await?;
            let mut db = existing
                .ok_or_else(|| crate::DbError::NotFound(format!("Season {id} not found")))?;
            if is_current && !db.is_current {
                let _ = self
                    .client
                    .query("UPDATE season SET is_current = false WHERE is_current = true")
                    .await?;
            }
            db.name = name.to_string();
            db.starts_at = SurrealDatetime::from(starts_at);
            db.ends_at = SurrealDatetime::from(ends_at);
            db.is_current = is_current;
            let updated: Option<DbSeason> = self.client.update(("season", id)).content(db).await?;
            Ok(db_to_season(updated.ok_or_else(|| {
                crate::DbError::NotFound(format!("Season {id} not found after update"))
            })?))
        })
        .await
    }

    /// Delete a season. Returns `false` when no such season existed. Matches
    /// are never touched — a season is only a window over `played_at`.
    pub async fn delete_season(&self, id: &str) -> DbResult<bool> {
        with_timeout(async {
            let deleted: Option<DbSeason> = self.client.delete(("season", id)).await?;
            Ok(deleted.is_some())
        })
        .await
    }
}
