use chrono::Utc;
use serde::{Deserialize, Serialize};
use surrealdb_types::RecordId;
use surrealdb::types::Datetime as SurrealDatetime;
use surrealdb_types::SurrealValue;

use crate::types::Announcement;
use crate::{with_timeout, Database, DbResult};

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbAnnouncement {
    #[surreal(default)]
    #[allow(dead_code)]
    id: Option<RecordId>,
    title: String,
    content: String,
    author_id: String,
    pinned: bool,
    is_active: bool,
    created_at: SurrealDatetime,
    updated_at: SurrealDatetime,
}

fn db_to_announcement(db: DbAnnouncement) -> Announcement {
    let id = db
        .id
        .map(|r| crate::record_id_key_to_string(r.key))
        .unwrap_or_else(|| "unknown".to_string());
    Announcement {
        id,
        title: db.title,
        content: db.content,
        author_id: db.author_id,
        pinned: db.pinned,
        is_active: db.is_active,
        created_at: db.created_at.into(),
        updated_at: db.updated_at.into(),
    }
}

impl Database {
    /// Create a new announcement.
    pub async fn create_announcement(
        &self,
        title: &str,
        content: &str,
        author_id: &str,
        pinned: bool,
    ) -> DbResult<Announcement> {
        with_timeout(async {
            let now = SurrealDatetime::from(Utc::now());
            let db_ann = DbAnnouncement {
                id: None,
                title: title.to_string(),
                content: content.to_string(),
                author_id: author_id.to_string(),
                pinned,
                is_active: true,
                created_at: now.clone(),
                updated_at: now,
            };
            let created: Option<DbAnnouncement> =
                self.client.create("announcement").content(db_ann).await?;
            Ok(db_to_announcement(created.ok_or_else(|| {
                crate::DbError::NotFound("Failed to create announcement".into())
            })?))
        })
        .await
    }

    /// List active announcements (pinned first, then by date descending).
    pub async fn list_announcements(&self) -> DbResult<Vec<Announcement>> {
        with_timeout(async {
            let mut result = self
                .client
                .query(
                    "SELECT * FROM announcement WHERE is_active = true ORDER BY pinned DESC, created_at DESC",
                )
                .await?;
            let anns: Vec<DbAnnouncement> = result.take(0)?;
            Ok(anns.into_iter().map(db_to_announcement).collect())
        })
        .await
    }

    /// List active announcements with cursor-based pagination.
    pub async fn list_announcements_paginated(
        &self,
        limit: u32,
        offset: u32,
    ) -> DbResult<Vec<Announcement>> {
        with_timeout(async {
            let fetch = limit + 1;
            let mut result = self
                .client
                .query(
                    "SELECT * FROM announcement WHERE is_active = true ORDER BY pinned DESC, created_at DESC LIMIT $lim START $off",
                )
                .bind(("lim", fetch))
                .bind(("off", offset))
                .await?;
            let anns: Vec<DbAnnouncement> = result.take(0)?;
            Ok(anns.into_iter().map(db_to_announcement).collect())
        })
        .await
    }

    /// Update an announcement.
    pub async fn update_announcement(
        &self,
        id: &str,
        title: Option<&str>,
        content: Option<&str>,
        pinned: Option<bool>,
    ) -> DbResult<Announcement> {
        with_timeout(async {
            let existing: Option<DbAnnouncement> =
                self.client.select(("announcement", id)).await?;
            let mut db = existing.ok_or_else(|| {
                crate::DbError::NotFound(format!("Announcement {id} not found"))
            })?;

            if let Some(t) = title {
                db.title = t.to_string();
            }
            if let Some(c) = content {
                db.content = c.to_string();
            }
            if let Some(p) = pinned {
                db.pinned = p;
            }
            db.updated_at = SurrealDatetime::from(Utc::now());

            let updated: Option<DbAnnouncement> = self
                .client
                .update(("announcement", id))
                .content(db)
                .await?;
            Ok(db_to_announcement(updated.ok_or_else(|| {
                crate::DbError::NotFound(format!("Announcement {id} not found after update"))
            })?))
        })
        .await
    }

    /// Soft-delete an announcement.
    pub async fn deactivate_announcement(&self, id: &str) -> DbResult<()> {
        with_timeout(async {
            self.client
                .query("UPDATE $rid SET is_active = false")
                .bind(("rid", RecordId::new("announcement", id)))
                .await?;
            Ok(())
        })
        .await
    }
}
