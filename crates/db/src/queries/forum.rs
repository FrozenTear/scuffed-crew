use chrono::Utc;
use serde::{Deserialize, Serialize};
use surrealdb::types::Datetime as SurrealDatetime;
use surrealdb_types::RecordId;
use surrealdb_types::SurrealValue;

use crate::types::{ForumReply, ForumThread};
use crate::{with_timeout, Database, DbResult};

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbForumThread {
    #[surreal(default)]
    #[allow(dead_code)]
    id: Option<RecordId>,
    title: String,
    category: String,
    author_member_id: String,
    content: String,
    pinned: bool,
    locked: bool,
    nostr_event_id: Option<String>,
    created_at: SurrealDatetime,
    updated_at: SurrealDatetime,
    is_active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbForumReply {
    #[surreal(default)]
    #[allow(dead_code)]
    id: Option<RecordId>,
    thread_id: String,
    author_member_id: String,
    content: String,
    created_at: SurrealDatetime,
    is_active: bool,
}

fn db_to_thread(db: DbForumThread) -> ForumThread {
    let id = db
        .id
        .map(|r| crate::record_id_key_to_string(r.key))
        .unwrap_or_else(|| "unknown".to_string());
    ForumThread {
        id,
        title: db.title,
        category: db.category,
        author_member_id: db.author_member_id,
        content: db.content,
        pinned: db.pinned,
        locked: db.locked,
        nostr_event_id: db.nostr_event_id,
        created_at: db.created_at.into(),
        updated_at: db.updated_at.into(),
        is_active: db.is_active,
    }
}

fn db_to_reply(db: DbForumReply) -> ForumReply {
    let id = db
        .id
        .map(|r| crate::record_id_key_to_string(r.key))
        .unwrap_or_else(|| "unknown".to_string());
    ForumReply {
        id,
        thread_id: db.thread_id,
        author_member_id: db.author_member_id,
        content: db.content,
        created_at: db.created_at.into(),
        is_active: db.is_active,
    }
}

impl Database {
    /// Create a new forum thread.
    pub async fn create_forum_thread(
        &self,
        title: &str,
        category: &str,
        author_member_id: &str,
        content: &str,
    ) -> DbResult<ForumThread> {
        with_timeout(async {
            let now = SurrealDatetime::from(Utc::now());
            let db_thread = DbForumThread {
                id: None,
                title: title.to_string(),
                category: category.to_string(),
                author_member_id: author_member_id.to_string(),
                content: content.to_string(),
                pinned: false,
                locked: false,
                nostr_event_id: None,
                created_at: now,
                updated_at: now,
                is_active: true,
            };
            let created: Option<DbForumThread> = self
                .client
                .create("forum_thread")
                .content(db_thread)
                .await?;
            Ok(db_to_thread(created.ok_or_else(|| {
                crate::DbError::NotFound("Failed to create forum thread".into())
            })?))
        })
        .await
    }

    /// List active forum threads, optionally filtered by category.
    /// Pinned threads come first, then ordered by created_at descending.
    pub async fn list_forum_threads(
        &self,
        category: Option<&str>,
        limit: u32,
        offset: u32,
    ) -> DbResult<Vec<ForumThread>> {
        with_timeout(async {
            let mut result = if let Some(cat) = category {
                self.client
                    .query(
                        "SELECT * FROM forum_thread WHERE is_active = true AND category = $cat \
                         ORDER BY pinned DESC, created_at DESC LIMIT $lim START $off",
                    )
                    .bind(("cat", cat.to_string()))
                    .bind(("lim", limit))
                    .bind(("off", offset))
                    .await?
            } else {
                self.client
                    .query(
                        "SELECT * FROM forum_thread WHERE is_active = true \
                         ORDER BY pinned DESC, created_at DESC LIMIT $lim START $off",
                    )
                    .bind(("lim", limit))
                    .bind(("off", offset))
                    .await?
            };
            let threads: Vec<DbForumThread> = result.take(0)?;
            Ok(threads.into_iter().map(db_to_thread).collect())
        })
        .await
    }

    /// Get a single forum thread by ID.
    pub async fn get_forum_thread(&self, id: &str) -> DbResult<ForumThread> {
        with_timeout(async {
            let db: Option<DbForumThread> = self.client.select(("forum_thread", id)).await?;
            Ok(db_to_thread(db.ok_or_else(|| {
                crate::DbError::NotFound(format!("Forum thread {id} not found"))
            })?))
        })
        .await
    }

    /// Create a reply to a forum thread.
    pub async fn create_forum_reply(
        &self,
        thread_id: &str,
        author_member_id: &str,
        content: &str,
    ) -> DbResult<ForumReply> {
        with_timeout(async {
            let now = SurrealDatetime::from(Utc::now());
            let db_reply = DbForumReply {
                id: None,
                thread_id: thread_id.to_string(),
                author_member_id: author_member_id.to_string(),
                content: content.to_string(),
                created_at: now,
                is_active: true,
            };
            let created: Option<DbForumReply> =
                self.client.create("forum_reply").content(db_reply).await?;
            Ok(db_to_reply(created.ok_or_else(|| {
                crate::DbError::NotFound("Failed to create forum reply".into())
            })?))
        })
        .await
    }

    /// List active replies for a thread, ordered by created_at ascending.
    pub async fn list_forum_replies(
        &self,
        thread_id: &str,
        limit: u32,
        offset: u32,
    ) -> DbResult<Vec<ForumReply>> {
        with_timeout(async {
            let mut result = self
                .client
                .query(
                    "SELECT * FROM forum_reply WHERE is_active = true AND thread_id = $tid \
                     ORDER BY created_at ASC LIMIT $lim START $off",
                )
                .bind(("tid", thread_id.to_string()))
                .bind(("lim", limit))
                .bind(("off", offset))
                .await?;
            let replies: Vec<DbForumReply> = result.take(0)?;
            Ok(replies.into_iter().map(db_to_reply).collect())
        })
        .await
    }

    /// Pin or unpin a forum thread.
    pub async fn pin_forum_thread(&self, id: &str, pinned: bool) -> DbResult<()> {
        with_timeout(async {
            self.client
                .query("UPDATE $rid SET pinned = $pinned, updated_at = time::now()")
                .bind(("rid", RecordId::new("forum_thread", id)))
                .bind(("pinned", pinned))
                .await?;
            Ok(())
        })
        .await
    }

    /// Lock or unlock a forum thread.
    pub async fn lock_forum_thread(&self, id: &str, locked: bool) -> DbResult<()> {
        with_timeout(async {
            self.client
                .query("UPDATE $rid SET locked = $locked, updated_at = time::now()")
                .bind(("rid", RecordId::new("forum_thread", id)))
                .bind(("locked", locked))
                .await?;
            Ok(())
        })
        .await
    }

    /// Store the Nostr event ID for a forum thread after relay publish.
    pub async fn update_thread_nostr_event_id(
        &self,
        id: &str,
        nostr_event_id: &str,
    ) -> DbResult<()> {
        with_timeout(async {
            self.client
                .query("UPDATE $rid SET nostr_event_id = $eid, updated_at = time::now()")
                .bind(("rid", RecordId::new("forum_thread", id)))
                .bind(("eid", nostr_event_id.to_string()))
                .await?;
            Ok(())
        })
        .await
    }

    /// Soft-delete a forum thread.
    pub async fn deactivate_forum_thread(&self, id: &str) -> DbResult<()> {
        with_timeout(async {
            self.client
                .query("UPDATE $rid SET is_active = false, updated_at = time::now()")
                .bind(("rid", RecordId::new("forum_thread", id)))
                .await?;
            Ok(())
        })
        .await
    }

    /// Soft-delete a forum reply.
    pub async fn deactivate_forum_reply(&self, id: &str) -> DbResult<()> {
        with_timeout(async {
            self.client
                .query("UPDATE $rid SET is_active = false")
                .bind(("rid", RecordId::new("forum_reply", id)))
                .await?;
            Ok(())
        })
        .await
    }

    /// Count active replies for a thread.
    pub async fn count_forum_replies(&self, thread_id: &str) -> DbResult<u64> {
        with_timeout(async {
            let mut result = self
                .client
                .query(
                    "SELECT count() as total FROM forum_reply \
                     WHERE is_active = true AND thread_id = $tid GROUP ALL",
                )
                .bind(("tid", thread_id.to_string()))
                .await?;
            let row: Option<CountRow> = result.take(0)?;
            Ok(row.map(|r| r.total).unwrap_or(0))
        })
        .await
    }
}

#[derive(Debug, Deserialize, SurrealValue)]
struct CountRow {
    total: u64,
}
