use serde::{Deserialize, Serialize};
use surrealdb_types::RecordId;
use surrealdb_types::SurrealValue;

use crate::types::Event;
use crate::{with_timeout, Database, DbResult};

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbEvent {
    #[surreal(default)]
    #[allow(dead_code)]
    id: Option<RecordId>,
    title: String,
    day_of_week: u8,
    time: String,
    timezone: String,
    duration_minutes: u32,
    is_recurring: bool,
    team_id: Option<String>,
    created_by: String,
    is_active: bool,
}

fn db_to_event(db: DbEvent) -> Event {
    let id = db
        .id
        .map(|r| crate::record_id_key_to_string(r.key))
        .unwrap_or_else(|| "unknown".to_string());
    Event {
        id,
        title: db.title,
        day_of_week: db.day_of_week,
        time: db.time,
        timezone: db.timezone,
        duration_minutes: db.duration_minutes,
        is_recurring: db.is_recurring,
        team_id: db.team_id,
        created_by: db.created_by,
        is_active: db.is_active,
    }
}

impl Database {
    pub async fn create_event(
        &self,
        title: &str,
        day_of_week: u8,
        time: &str,
        timezone: &str,
        duration_minutes: u32,
        is_recurring: bool,
        team_id: Option<&str>,
        created_by: &str,
    ) -> DbResult<Event> {
        with_timeout(async {
            let db_event = DbEvent {
                id: None,
                title: title.to_string(),
                day_of_week,
                time: time.to_string(),
                timezone: timezone.to_string(),
                duration_minutes,
                is_recurring,
                team_id: team_id.map(|s| s.to_string()),
                created_by: created_by.to_string(),
                is_active: true,
            };
            let created: Option<DbEvent> = self.client.create("event").content(db_event).await?;
            Ok(db_to_event(created.ok_or_else(|| {
                crate::DbError::NotFound("Failed to create event".into())
            })?))
        })
        .await
    }

    pub async fn list_events(&self) -> DbResult<Vec<Event>> {
        with_timeout(async {
            let mut result = self
                .client
                .query(
                    "SELECT * FROM event WHERE is_active = true ORDER BY day_of_week ASC, time ASC",
                )
                .await?;
            let events: Vec<DbEvent> = result.take(0)?;
            Ok(events.into_iter().map(db_to_event).collect())
        })
        .await
    }

    /// List active events with cursor-based pagination.
    pub async fn list_events_paginated(&self, limit: u32, offset: u32) -> DbResult<Vec<Event>> {
        with_timeout(async {
            let fetch = limit + 1;
            let mut result = self
                .client
                .query("SELECT * FROM event WHERE is_active = true ORDER BY day_of_week ASC, time ASC LIMIT $lim START $off")
                .bind(("lim", fetch))
                .bind(("off", offset))
                .await?;
            let events: Vec<DbEvent> = result.take(0)?;
            Ok(events.into_iter().map(db_to_event).collect())
        })
        .await
    }

    pub async fn update_event(
        &self,
        id: &str,
        title: Option<&str>,
        day_of_week: Option<u8>,
        time: Option<&str>,
        timezone: Option<&str>,
        duration_minutes: Option<u32>,
        is_recurring: Option<bool>,
        team_id: Option<Option<&str>>,
    ) -> DbResult<Event> {
        with_timeout(async {
            let existing: Option<DbEvent> = self.client.select(("event", id)).await?;
            let mut db = existing
                .ok_or_else(|| crate::DbError::NotFound(format!("Event {id} not found")))?;

            if let Some(t) = title {
                db.title = t.to_string();
            }
            if let Some(d) = day_of_week {
                db.day_of_week = d;
            }
            if let Some(t) = time {
                db.time = t.to_string();
            }
            if let Some(tz) = timezone {
                db.timezone = tz.to_string();
            }
            if let Some(dur) = duration_minutes {
                db.duration_minutes = dur;
            }
            if let Some(r) = is_recurring {
                db.is_recurring = r;
            }
            if let Some(tid) = team_id {
                db.team_id = tid.map(|s| s.to_string());
            }

            let updated: Option<DbEvent> = self.client.update(("event", id)).content(db).await?;
            Ok(db_to_event(updated.ok_or_else(|| {
                crate::DbError::NotFound(format!("Event {id} not found after update"))
            })?))
        })
        .await
    }

    pub async fn deactivate_event(&self, id: &str) -> DbResult<()> {
        with_timeout(async {
            self.client
                .query("UPDATE $rid SET is_active = false")
                .bind(("rid", RecordId::new("event", id)))
                .await?;
            Ok(())
        })
        .await
    }
}
