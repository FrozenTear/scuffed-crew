use chrono::Utc;
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
    pub async fn update_wiki_page(
        &self,
        topic: &str,
        content_markdown: &str,
        edited_by: &str,
        revision_note: Option<&str>,
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

            // Create a revision of the old content
            let now = SurrealDatetime::from(Utc::now());
            let revision = DbWikiRevision {
                id: None,
                page_id: page_id.clone(),
                content_markdown: page.content_markdown.clone(),
                edited_by: edited_by.to_string(),
                edited_at: now,
                revision_note: revision_note.map(|s| s.to_string()),
            };
            let _: Option<DbWikiRevision> = self
                .client
                .create("wiki_revision")
                .content(revision)
                .await?;

            // Update the page
            let rid = RecordId::new("wiki_page", &*page_id);
            let mut result = self
                .client
                .query("UPDATE $rid SET content_markdown = $content, updated_at = time::now() RETURN AFTER")
                .bind(("rid", rid))
                .bind(("content", content_markdown.to_string()))
                .await?;
            let updated: Vec<DbWikiPage> = result.take(0)?;
            let updated_page = updated.into_iter().next().ok_or_else(|| {
                crate::DbError::NotFound(format!("Wiki page '{topic}' not found after update"))
            })?;
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
