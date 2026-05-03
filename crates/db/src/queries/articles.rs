use chrono::Utc;
use serde::{Deserialize, Serialize};
use surrealdb::types::Datetime as SurrealDatetime;
use surrealdb_types::{RecordId, SurrealValue};

use crate::types::Article;
use crate::{Database, DbResult, with_timeout};

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
    nostr_event_id: Option<String>,
    created_at: SurrealDatetime,
    updated_at: SurrealDatetime,
    published_at: Option<SurrealDatetime>,
}

fn db_to_article(db: DbArticle) -> Article {
    let id = db
        .id
        .map(|r| crate::record_id_key_to_string(r.key))
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
        created_at: db.created_at.into(),
        updated_at: db.updated_at.into(),
        published_at: db.published_at.map(|d| d.into()),
    }
}

impl Database {
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
                created_at: now.clone(),
                updated_at: now,
                published_at: None,
            };
            let created: Option<DbArticle> =
                self.client.create("article").content(db_article).await?;
            Ok(db_to_article(created.ok_or_else(|| {
                crate::DbError::NotFound("Failed to create article".into())
            })?))
        })
        .await
    }

    pub async fn list_published_articles(&self, limit: u32, offset: u32) -> DbResult<Vec<Article>> {
        with_timeout(async {
            let mut result = self
                .client
                .query("SELECT * FROM article WHERE published = true ORDER BY published_at DESC LIMIT $limit START $offset")
                .bind(("limit", limit))
                .bind(("offset", offset))
                .await?;
            let articles: Vec<DbArticle> = result.take(0)?;
            Ok(articles.into_iter().map(db_to_article).collect())
        })
        .await
    }

    pub async fn count_published_articles(&self) -> DbResult<u64> {
        with_timeout(async {
            let mut result = self
                .client
                .query("SELECT count() FROM article WHERE published = true GROUP ALL")
                .await?;
            let count: Option<u64> = result.take("count")?;
            Ok(count.unwrap_or(0))
        })
        .await
    }

    pub async fn list_all_articles(&self) -> DbResult<Vec<Article>> {
        with_timeout(async {
            let mut result = self
                .client
                .query("SELECT * FROM article ORDER BY created_at DESC")
                .await?;
            let articles: Vec<DbArticle> = result.take(0)?;
            Ok(articles.into_iter().map(db_to_article).collect())
        })
        .await
    }

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

    pub async fn update_article(
        &self,
        slug: &str,
        title: Option<&str>,
        new_slug: Option<&str>,
        content_markdown: Option<&str>,
        summary: Option<Option<&str>>,
        cover_image_url: Option<Option<&str>>,
    ) -> DbResult<Article> {
        with_timeout(async {
            let existing = self.get_article_by_slug(slug).await?;

            let mut db: DbArticle = self
                .client
                .select(("article", existing.id.as_str()))
                .await?
                .ok_or_else(|| crate::DbError::NotFound(format!("Article {slug} not found")))?;

            if let Some(t) = title {
                db.title = t.to_string();
            }
            if let Some(s) = new_slug {
                db.slug = s.to_string();
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

            let updated: Option<DbArticle> = self
                .client
                .update(("article", existing.id.as_str()))
                .content(db)
                .await?;
            Ok(db_to_article(updated.ok_or_else(|| {
                crate::DbError::NotFound(format!("Article {slug} not found after update"))
            })?))
        })
        .await
    }

    pub async fn publish_article(&self, slug: &str) -> DbResult<Article> {
        with_timeout(async {
            let existing = self.get_article_by_slug(slug).await?;
            let now = SurrealDatetime::from(Utc::now());
            self.client
                .query("UPDATE $rid SET published = true, published_at = $now, updated_at = $now")
                .bind(("rid", RecordId::new("article", existing.id.as_str())))
                .bind(("now", now))
                .await?;
            self.get_article_by_slug(slug).await
        })
        .await
    }

    pub async fn unpublish_article(&self, slug: &str) -> DbResult<Article> {
        with_timeout(async {
            let existing = self.get_article_by_slug(slug).await?;
            let now = SurrealDatetime::from(Utc::now());
            self.client
                .query("UPDATE $rid SET published = false, published_at = NONE, updated_at = $now")
                .bind(("rid", RecordId::new("article", existing.id.as_str())))
                .bind(("now", now))
                .await?;
            self.get_article_by_slug(slug).await
        })
        .await
    }

    pub async fn delete_article(&self, slug: &str) -> DbResult<()> {
        with_timeout(async {
            let existing = self.get_article_by_slug(slug).await?;
            let _: Option<DbArticle> = self
                .client
                .delete(("article", existing.id.as_str()))
                .await?;
            Ok(())
        })
        .await
    }
}
