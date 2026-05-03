use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};

use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{Article, AuditAction, AuditTargetType};

use crate::extractors::{AdminUser, OfficerUser};
use crate::routes::audit_log::audit;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct ArticleListQuery {
    #[serde(default = "default_limit")]
    pub limit: u32,
    #[serde(default)]
    pub offset: u32,
}

fn default_limit() -> u32 {
    20
}

#[derive(Serialize)]
pub struct ArticleListResponse {
    pub articles: Vec<Article>,
    pub total: u64,
}

/// GET /api/articles — list published articles (public, paginated)
pub async fn list_articles(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<ArticleListQuery>,
) -> Result<Json<ArticleListResponse>, (StatusCode, Json<ErrorResponse>)> {
    let articles = state
        .db
        .list_published_articles(query.limit, query.offset)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
        })?;

    let total = state.db.count_published_articles().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
    })?;

    Ok(Json(ArticleListResponse { articles, total }))
}

/// GET /api/articles/:slug — get article by slug (public)
pub async fn get_article(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<Article>, (StatusCode, Json<ErrorResponse>)> {
    let article = state.db.get_article_by_slug(&slug).await.map_err(|e| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
    })?;

    if !article.published {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Article not found".into(),
            }),
        ));
    }

    Ok(Json(article))
}

/// GET /api/articles/admin/all — list all articles including drafts (officer+)
pub async fn list_all_articles(
    State(state): State<AppState>,
    _officer: OfficerUser,
) -> Result<Json<Vec<Article>>, (StatusCode, Json<ErrorResponse>)> {
    state.db.list_all_articles().await.map(Json).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
    })
}

#[derive(Deserialize)]
pub struct CreateArticleRequest {
    pub title: String,
    pub slug: String,
    pub content_markdown: String,
    pub summary: Option<String>,
    pub cover_image_url: Option<String>,
}

/// POST /api/articles — create draft article (officer+)
pub async fn create_article(
    State(state): State<AppState>,
    officer: OfficerUser,
    Json(body): Json<CreateArticleRequest>,
) -> Result<(StatusCode, Json<Article>), (StatusCode, Json<ErrorResponse>)> {
    let article = state
        .db
        .create_article(
            &body.slug,
            &body.title,
            &body.content_markdown,
            body.summary.as_deref(),
            body.cover_image_url.as_deref(),
            &officer.member.id,
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
        })?;

    audit(
        &state.db,
        &officer.member.id,
        AuditAction::CreatedArticle,
        AuditTargetType::Article,
        &article.id,
        Some(&format!("Created: {}", article.title)),
    )
    .await;

    Ok((StatusCode::CREATED, Json(article)))
}

#[derive(Deserialize)]
pub struct UpdateArticleRequest {
    pub title: Option<String>,
    pub slug: Option<String>,
    pub content_markdown: Option<String>,
    pub summary: Option<Option<String>>,
    pub cover_image_url: Option<Option<String>>,
}

/// PUT /api/articles/:slug — update article (officer+)
pub async fn update_article(
    State(state): State<AppState>,
    officer: OfficerUser,
    Path(slug): Path<String>,
    Json(body): Json<UpdateArticleRequest>,
) -> Result<Json<Article>, (StatusCode, Json<ErrorResponse>)> {
    let article = state
        .db
        .update_article(
            &slug,
            body.title.as_deref(),
            body.slug.as_deref(),
            body.content_markdown.as_deref(),
            body.summary.as_ref().map(|s| s.as_deref()),
            body.cover_image_url.as_ref().map(|c| c.as_deref()),
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
        })?;

    audit(
        &state.db,
        &officer.member.id,
        AuditAction::UpdatedArticle,
        AuditTargetType::Article,
        &article.id,
        None,
    )
    .await;

    Ok(Json(article))
}

/// POST /api/articles/:slug/publish — publish article (officer+)
pub async fn publish_article(
    State(state): State<AppState>,
    officer: OfficerUser,
    Path(slug): Path<String>,
) -> Result<Json<Article>, (StatusCode, Json<ErrorResponse>)> {
    let article = state.db.publish_article(&slug).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
    })?;

    audit(
        &state.db,
        &officer.member.id,
        AuditAction::PublishedArticle,
        AuditTargetType::Article,
        &article.id,
        Some(&format!("Published: {}", article.title)),
    )
    .await;

    Ok(Json(article))
}

/// POST /api/articles/:slug/unpublish — unpublish article (officer+)
pub async fn unpublish_article(
    State(state): State<AppState>,
    officer: OfficerUser,
    Path(slug): Path<String>,
) -> Result<Json<Article>, (StatusCode, Json<ErrorResponse>)> {
    let article = state.db.unpublish_article(&slug).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
    })?;

    audit(
        &state.db,
        &officer.member.id,
        AuditAction::UnpublishedArticle,
        AuditTargetType::Article,
        &article.id,
        None,
    )
    .await;

    Ok(Json(article))
}

/// DELETE /api/articles/:slug — delete article (admin only)
pub async fn delete_article(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(slug): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let article = state.db.get_article_by_slug(&slug).await.map_err(|e| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
    })?;

    state.db.delete_article(&slug).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
    })?;

    audit(
        &state.db,
        &admin.member.id,
        AuditAction::DeletedArticle,
        AuditTargetType::Article,
        &article.id,
        Some(&format!("Deleted: {}", article.title)),
    )
    .await;

    Ok(StatusCode::OK)
}
