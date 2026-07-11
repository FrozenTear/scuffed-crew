use chrono::Utc;
use serde::{Deserialize, Serialize};
use surrealdb::types::Datetime as SurrealDatetime;
use surrealdb_types::{RecordId, SurrealValue};

use crate::types::Article;
use crate::{record_id_key_to_string, with_timeout, Database, DbResult};

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbArticle {
    #[surreal(default)]
    #[allow(dead_code)]
    id: Option<RecordId>,
    slug: String,
    title: String,
    content_markdown: String,
    summary: Option<String>,
    cover_image_url: Option<String>,
    author_member_id: String,
    published: bool,
    #[surreal(default)]
    #[serde(default)]
    nostr_event_id: Option<String>,
    published_at: Option<SurrealDatetime>,
    created_at: SurrealDatetime,
    updated_at: SurrealDatetime,
}

fn db_to_article(db: DbArticle) -> Article {
    let id = db
        .id
        .map(|r| record_id_key_to_string(r.key))
        .unwrap_or_else(|| "unknown".to_string());
    Article {
        id,
        slug: db.slug,
        title: db.title,
        content_markdown: db.content_markdown,
        summary: db.summary,
        cover_image_url: db.cover_image_url,
        author_member_id: db.author_member_id,
        published: db.published,
        nostr_event_id: db.nostr_event_id,
        published_at: db.published_at.map(|d| d.into()),
        created_at: db.created_at.into(),
        updated_at: db.updated_at.into(),
    }
}

impl Database {
    /// Create a new article (draft by default).
    pub async fn create_article(
        &self,
        slug: &str,
        title: &str,
        content_markdown: &str,
        summary: Option<&str>,
        cover_image_url: Option<&str>,
        author_member_id: &str,
    ) -> DbResult<Article> {
        with_timeout(async {
            let now = SurrealDatetime::from(Utc::now());
            let db_article = DbArticle {
                id: None,
                slug: slug.to_string(),
                title: title.to_string(),
                content_markdown: content_markdown.to_string(),
                summary: summary.map(|s| s.to_string()),
                cover_image_url: cover_image_url.map(|s| s.to_string()),
                author_member_id: author_member_id.to_string(),
                published: false,
                nostr_event_id: None,
                published_at: None,
                created_at: now,
                updated_at: now,
            };
            let created: Option<DbArticle> =
                self.client.create("article").content(db_article).await?;
            Ok(db_to_article(created.ok_or_else(|| {
                crate::DbError::NotFound("Failed to create article".into())
            })?))
        })
        .await
    }

    /// List published articles, ordered by published_at descending.
    pub async fn list_published_articles(&self, limit: u32, offset: u32) -> DbResult<Vec<Article>> {
        with_timeout(async {
            let mut result = self
                .client
                .query("SELECT * FROM article WHERE published = true ORDER BY published_at DESC LIMIT $lim START $off")
                .bind(("lim", limit))
                .bind(("off", offset))
                .await?;
            let articles: Vec<DbArticle> = result.take(0)?;
            Ok(articles.into_iter().map(db_to_article).collect())
        })
        .await
    }

    /// List all articles (admin/officer view), ordered by created_at descending.
    pub async fn list_all_articles(&self, limit: u32, offset: u32) -> DbResult<Vec<Article>> {
        with_timeout(async {
            let mut result = self
                .client
                .query("SELECT * FROM article ORDER BY created_at DESC LIMIT $lim START $off")
                .bind(("lim", limit))
                .bind(("off", offset))
                .await?;
            let articles: Vec<DbArticle> = result.take(0)?;
            Ok(articles.into_iter().map(db_to_article).collect())
        })
        .await
    }

    /// Get an article by its slug.
    pub async fn get_article_by_slug(&self, slug: &str) -> DbResult<Article> {
        with_timeout(async {
            let mut result = self
                .client
                .query("SELECT * FROM article WHERE slug = $slug LIMIT 1")
                .bind(("slug", slug.to_string()))
                .await?;
            let articles: Vec<DbArticle> = result.take(0)?;
            articles
                .into_iter()
                .next()
                .map(db_to_article)
                .ok_or_else(|| {
                    crate::DbError::NotFound(format!("Article with slug '{slug}' not found"))
                })
        })
        .await
    }

    /// Update an article's content fields.
    pub async fn update_article(
        &self,
        id: &str,
        title: Option<&str>,
        content_markdown: Option<&str>,
        summary: Option<Option<&str>>,
        cover_image_url: Option<Option<&str>>,
    ) -> DbResult<Article> {
        with_timeout(async {
            let existing: Option<DbArticle> = self.client.select(("article", id)).await?;
            let mut db = existing
                .ok_or_else(|| crate::DbError::NotFound(format!("Article {id} not found")))?;

            if let Some(t) = title {
                db.title = t.to_string();
            }
            if let Some(c) = content_markdown {
                db.content_markdown = c.to_string();
            }
            if let Some(s) = summary {
                db.summary = s.map(|v| v.to_string());
            }
            if let Some(c) = cover_image_url {
                db.cover_image_url = c.map(|v| v.to_string());
            }
            db.updated_at = SurrealDatetime::from(Utc::now());

            let updated: Option<DbArticle> =
                self.client.update(("article", id)).content(db).await?;
            Ok(db_to_article(updated.ok_or_else(|| {
                crate::DbError::NotFound(format!("Article {id} not found after update"))
            })?))
        })
        .await
    }

    /// Publish an article (sets published=true and published_at to now).
    pub async fn publish_article(&self, id: &str) -> DbResult<()> {
        with_timeout(async {
            self.client
                .query("UPDATE $rid SET published = true, published_at = time::now(), updated_at = time::now()")
                .bind(("rid", RecordId::new("article", id)))
                .await?;
            Ok(())
        })
        .await
    }

    /// Unpublish an article (sets published=false).
    pub async fn unpublish_article(&self, id: &str) -> DbResult<()> {
        with_timeout(async {
            self.client
                .query("UPDATE $rid SET published = false, updated_at = time::now()")
                .bind(("rid", RecordId::new("article", id)))
                .await?;
            Ok(())
        })
        .await
    }

    /// Delete an article permanently.
    pub async fn delete_article(&self, id: &str) -> DbResult<()> {
        with_timeout(async {
            let _: Option<DbArticle> = self.client.delete(("article", id)).await?;
            Ok(())
        })
        .await
    }
}
