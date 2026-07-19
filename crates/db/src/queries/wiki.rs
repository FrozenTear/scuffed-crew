use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use surrealdb::types::Datetime as SurrealDatetime;
use surrealdb_types::{RecordId, SurrealValue};

use crate::types::{WikiPage, WikiRevision};
use crate::{with_timeout, Database, DbResult};

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbWikiPage {
    #[surreal(default)]
    #[allow(dead_code)]
    id: Option<RecordId>,
    topic: String,
    title: String,
    content_markdown: String,
    author_member_id: String,
    created_at: SurrealDatetime,
    updated_at: SurrealDatetime,
    is_active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbWikiRevision {
    #[surreal(default)]
    #[allow(dead_code)]
    id: Option<RecordId>,
    page_id: String,
    content_markdown: String,
    edited_by: String,
    edited_at: SurrealDatetime,
    revision_note: Option<String>,
}

fn db_to_wiki_page(db: DbWikiPage) -> WikiPage {
    let id = db
        .id
        .map(|r| crate::record_id_key_to_string(r.key))
        .unwrap_or_else(|| "unknown".to_string());
    WikiPage {
        id,
        topic: db.topic,
        title: db.title,
        content_markdown: db.content_markdown,
        author_member_id: db.author_member_id,
        created_at: db.created_at.into(),
        updated_at: db.updated_at.into(),
        is_active: db.is_active,
    }
}

fn db_to_wiki_revision(db: DbWikiRevision) -> WikiRevision {
    let id = db
        .id
        .map(|r| crate::record_id_key_to_string(r.key))
        .unwrap_or_else(|| "unknown".to_string());
    WikiRevision {
        id,
        page_id: db.page_id,
        content_markdown: db.content_markdown,
        edited_by: db.edited_by,
        edited_at: db.edited_at.into(),
        revision_note: db.revision_note,
    }
}

impl Database {
    /// Create a new wiki page.
    pub async fn create_wiki_page(
        &self,
        topic: &str,
        title: &str,
        content_markdown: &str,
        author_member_id: &str,
    ) -> DbResult<WikiPage> {
        with_timeout(async {
            let now = SurrealDatetime::from(Utc::now());
            let db_page = DbWikiPage {
                id: None,
                topic: topic.to_string(),
                title: title.to_string(),
                content_markdown: content_markdown.to_string(),
                author_member_id: author_member_id.to_string(),
                created_at: now,
                updated_at: now,
                is_active: true,
            };
            let created: Option<DbWikiPage> =
                self.client.create("wiki_page").content(db_page).await?;
            Ok(db_to_wiki_page(created.ok_or_else(|| {
                crate::DbError::NotFound("Failed to create wiki page".into())
            })?))
        })
        .await
    }

    /// List active wiki pages, optionally filtered by search query.
    pub async fn list_wiki_pages(
        &self,
        search: Option<&str>,
        limit: u32,
        offset: u32,
    ) -> DbResult<Vec<WikiPage>> {
        with_timeout(async {
            let pages: Vec<DbWikiPage> = if let Some(q) = search {
                // CONTAINS is substring match — do not wrap with SQL LIKE wildcards
                let mut result = self
                    .client
                    .query(
                        "SELECT * FROM wiki_page WHERE is_active = true AND (string::lowercase(title) CONTAINS string::lowercase($q) OR string::lowercase(topic) CONTAINS string::lowercase($q)) ORDER BY updated_at DESC LIMIT $lim START $off",
                    )
                    .bind(("q", q.to_string()))
                    .bind(("lim", limit))
                    .bind(("off", offset))
                    .await?;
                result.take(0)?
            } else {
                let mut result = self
                    .client
                    .query(
                        "SELECT * FROM wiki_page WHERE is_active = true ORDER BY updated_at DESC LIMIT $lim START $off",
                    )
                    .bind(("lim", limit))
                    .bind(("off", offset))
                    .await?;
                result.take(0)?
            };
            Ok(pages.into_iter().map(db_to_wiki_page).collect())
        })
        .await
    }

    /// Get a wiki page by its topic slug.
    pub async fn get_wiki_page_by_topic(&self, topic: &str) -> DbResult<WikiPage> {
        with_timeout(async {
            let mut result = self
                .client
                .query("SELECT * FROM wiki_page WHERE topic = $topic AND is_active = true LIMIT 1")
                .bind(("topic", topic.to_string()))
                .await?;
            let pages: Vec<DbWikiPage> = result.take(0)?;
            let page = pages.into_iter().next().ok_or_else(|| {
                crate::DbError::NotFound(format!("Wiki page '{topic}' not found"))
            })?;
            Ok(db_to_wiki_page(page))
        })
        .await
    }

    /// Update a wiki page's content and create a revision record.
    ///
    /// **Optimistic concurrency (DR1-DB-008):** when `prev_updated_at` is `Some`,
    /// the update is a compare-and-swap gated on `WHERE updated_at = $prev`. If the
    /// page tip has moved since the caller last read it (a concurrent editor saved
    /// first), zero rows match and this returns [`crate::DbError::Conflict`] (→ 409
    /// at the route) instead of silently clobbering the other write. When `None`,
    /// the update is unconditional last-write-wins (backwards compatible for
    /// callers that don't yet pass a base version).
    ///
    /// The revision-history record of the *old* content is written only after the
    /// page write commits, so a lost CAS leaves no orphan revision.
    pub async fn update_wiki_page(
        &self,
        topic: &str,
        content_markdown: &str,
        edited_by: &str,
        revision_note: Option<&str>,
        prev_updated_at: Option<DateTime<Utc>>,
    ) -> DbResult<WikiPage> {
        with_timeout(async {
            // Fetch existing page
            let mut result = self
                .client
                .query("SELECT * FROM wiki_page WHERE topic = $topic AND is_active = true LIMIT 1")
                .bind(("topic", topic.to_string()))
                .await?;
            let pages: Vec<DbWikiPage> = result.take(0)?;
            let page = pages.into_iter().next().ok_or_else(|| {
                crate::DbError::NotFound(format!("Wiki page '{topic}' not found"))
            })?;

            let page_id = page
                .id
                .as_ref()
                .map(|r| crate::record_id_key_to_string(r.key.clone()))
                .unwrap_or_else(|| "unknown".to_string());

            // Update the page. With a base version, gate on it (CAS) so a stale
            // write loses cleanly rather than overwriting a newer tip.
            let rid = RecordId::new("wiki_page", &*page_id);
            let updated_page = if let Some(prev) = prev_updated_at {
                let mut result = self
                    .client
                    .query(
                        "UPDATE $rid SET content_markdown = $content, updated_at = time::now() \
                         WHERE updated_at = $prev RETURN AFTER",
                    )
                    .bind(("rid", rid))
                    .bind(("content", content_markdown.to_string()))
                    .bind(("prev", SurrealDatetime::from(prev)))
                    .await?;
                let updated: Vec<DbWikiPage> = result.take(0)?;
                updated.into_iter().next().ok_or_else(|| {
                    crate::DbError::Conflict(format!(
                        "Wiki page '{topic}' was modified by another editor"
                    ))
                })?
            } else {
                let mut result = self
                    .client
                    .query(
                        "UPDATE $rid SET content_markdown = $content, updated_at = time::now() \
                         RETURN AFTER",
                    )
                    .bind(("rid", rid))
                    .bind(("content", content_markdown.to_string()))
                    .await?;
                let updated: Vec<DbWikiPage> = result.take(0)?;
                updated.into_iter().next().ok_or_else(|| {
                    crate::DbError::NotFound(format!("Wiki page '{topic}' not found after update"))
                })?
            };

            // Record a revision of the OLD content now that the tip write committed.
            let revision = DbWikiRevision {
                id: None,
                page_id: page_id.clone(),
                content_markdown: page.content_markdown.clone(),
                edited_by: edited_by.to_string(),
                edited_at: SurrealDatetime::from(Utc::now()),
                revision_note: revision_note.map(|s| s.to_string()),
            };
            let _: Option<DbWikiRevision> = self
                .client
                .create("wiki_revision")
                .content(revision)
                .await?;

            Ok(db_to_wiki_page(updated_page))
        })
        .await
    }

    /// List revisions for a wiki page by page ID.
    pub async fn list_wiki_revisions(
        &self,
        page_id: &str,
        limit: u32,
        offset: u32,
    ) -> DbResult<Vec<WikiRevision>> {
        with_timeout(async {
            let mut result = self
                .client
                .query(
                    "SELECT * FROM wiki_revision WHERE page_id = $pid ORDER BY edited_at DESC LIMIT $lim START $off",
                )
                .bind(("pid", page_id.to_string()))
                .bind(("lim", limit))
                .bind(("off", offset))
                .await?;
            let revisions: Vec<DbWikiRevision> = result.take(0)?;
            Ok(revisions.into_iter().map(db_to_wiki_revision).collect())
        })
        .await
    }

    /// Soft-delete a wiki page.
    pub async fn deactivate_wiki_page(&self, topic: &str) -> DbResult<()> {
        with_timeout(async {
            // Look up the page by topic first
            let mut result = self
                .client
                .query("SELECT * FROM wiki_page WHERE topic = $topic LIMIT 1")
                .bind(("topic", topic.to_string()))
                .await?;
            let pages: Vec<DbWikiPage> = result.take(0)?;
            let page = pages.into_iter().next().ok_or_else(|| {
                crate::DbError::NotFound(format!("Wiki page '{topic}' not found"))
            })?;

            let page_id = page
                .id
                .as_ref()
                .map(|r| crate::record_id_key_to_string(r.key.clone()))
                .unwrap_or_else(|| "unknown".to_string());

            let rid = RecordId::new("wiki_page", &*page_id);
            self.client
                .query("UPDATE $rid SET is_active = false")
                .bind(("rid", rid))
                .await?;
            Ok(())
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use crate::migrations::run_migrations;
    use crate::{Database, DbError};

    async fn test_db() -> Database {
        let db = Database::connect_memory().await.unwrap();
        run_migrations(&db.client).await.unwrap();
        db
    }

    /// DB-008: a stale base version loses the CAS (409) instead of clobbering the
    /// newer tip; the winning write survives.
    #[tokio::test]
    async fn cas_rejects_stale_update() {
        let db = test_db().await;
        let page = db
            .create_wiki_page("t", "Title", "v1", "author")
            .await
            .unwrap();
        let base = page.updated_at;

        // Editor A commits first against the correct base version.
        let after_a = db
            .update_wiki_page("t", "v2", "a", None, Some(base))
            .await
            .unwrap();
        assert_eq!(after_a.content_markdown, "v2");

        // Editor B still holds the ORIGINAL base → stale → Conflict.
        let err = db
            .update_wiki_page("t", "v3", "b", None, Some(base))
            .await
            .unwrap_err();
        assert!(
            matches!(err, DbError::Conflict(_)),
            "stale CAS must conflict, got {err:?}"
        );

        // The lost write left the tip untouched.
        let cur = db.get_wiki_page_by_topic("t").await.unwrap();
        assert_eq!(cur.content_markdown, "v2");
    }

    /// DB-008: a fresh base version succeeds and still records the old content as a
    /// revision.
    #[tokio::test]
    async fn cas_fresh_base_succeeds_and_records_revision() {
        let db = test_db().await;
        let page = db
            .create_wiki_page("t", "Title", "v1", "author")
            .await
            .unwrap();

        let updated = db
            .update_wiki_page("t", "v2", "editor", Some("note"), Some(page.updated_at))
            .await
            .unwrap();
        assert_eq!(updated.content_markdown, "v2");

        let revs = db.list_wiki_revisions(&page.id, 10, 0).await.unwrap();
        assert_eq!(revs.len(), 1, "old content must be preserved as a revision");
        assert_eq!(revs[0].content_markdown, "v1");
    }

    /// No base version → unconditional last-write-wins (backwards compatible).
    #[tokio::test]
    async fn no_base_is_last_write_wins() {
        let db = test_db().await;
        db.create_wiki_page("t", "Title", "v1", "author")
            .await
            .unwrap();
        let updated = db
            .update_wiki_page("t", "v2", "editor", None, None)
            .await
            .unwrap();
        assert_eq!(updated.content_markdown, "v2");
    }
}
