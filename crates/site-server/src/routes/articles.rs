use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;

use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{Article, AuditAction, AuditTargetType};

use crate::extractors::{AdminUser, OfficerUser};
use crate::routes::audit_log::audit;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct ListArticlesQuery {
    #[serde(default = "default_limit")]
    pub limit: u32,
    #[serde(default)]
    pub offset: u32,
}

fn default_limit() -> u32 {
    20
}

/// GET /api/articles — list published articles (public)
pub async fn list_articles(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<ListArticlesQuery>,
) -> Result<Json<Vec<Article>>, (StatusCode, Json<ErrorResponse>)> {
    state
        .db
        .list_published_articles(query.limit, query.offset)
        .await
        .map(Json)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
        })
}

/// GET /api/articles/admin/all — list all articles, including drafts (officer+)
pub async fn list_all_articles(
    State(state): State<AppState>,
    _officer: OfficerUser,
    axum::extract::Query(query): axum::extract::Query<ListArticlesQuery>,
) -> Result<Json<Vec<Article>>, (StatusCode, Json<ErrorResponse>)> {
    state
        .db
        .list_all_articles(query.limit, query.offset)
        .await
        .map(Json)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
        })
}

/// GET /api/articles/:slug — get article by slug (public)
pub async fn get_article(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<Article>, (StatusCode, Json<ErrorResponse>)> {
    state
        .db
        .get_article_by_slug(&slug)
        .await
        .map(Json)
        .map_err(|e| match &e {
            scuffed_db::DbError::NotFound(_) => (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            ),
            _ => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            ),
        })
}

#[derive(Deserialize)]
pub struct CreateArticleRequest {
    pub slug: String,
    pub title: String,
    pub content_markdown: String,
    pub summary: Option<String>,
    pub cover_image_url: Option<String>,
}

/// POST /api/articles — create a draft article (officer+)
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
    // Look up the article by slug to get its ID
    let existing = state
        .db
        .get_article_by_slug(&slug)
        .await
        .map_err(|e| match &e {
            scuffed_db::DbError::NotFound(_) => (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            ),
            _ => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            ),
        })?;

    let article = state
        .db
        .update_article(
            &existing.id,
            body.title.as_deref(),
            body.content_markdown.as_deref(),
            body.summary.as_ref().map(|s| s.as_deref()),
            body.cover_image_url.as_ref().map(|s| s.as_deref()),
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
        &existing.id,
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
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let existing = state
        .db
        .get_article_by_slug(&slug)
        .await
        .map_err(|e| match &e {
            scuffed_db::DbError::NotFound(_) => (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            ),
            _ => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            ),
        })?;

    state.db.publish_article(&existing.id).await.map_err(|e| {
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
        &existing.id,
        Some(&format!("Published: {}", existing.title)),
    )
    .await;

    Ok(StatusCode::OK)
}

/// POST /api/articles/:slug/unpublish — unpublish article (officer+)
pub async fn unpublish_article(
    State(state): State<AppState>,
    officer: OfficerUser,
    Path(slug): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let existing = state
        .db
        .get_article_by_slug(&slug)
        .await
        .map_err(|e| match &e {
            scuffed_db::DbError::NotFound(_) => (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            ),
            _ => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            ),
        })?;

    state
        .db
        .unpublish_article(&existing.id)
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
        AuditAction::UnpublishedArticle,
        AuditTargetType::Article,
        &existing.id,
        Some(&format!("Unpublished: {}", existing.title)),
    )
    .await;

    Ok(StatusCode::OK)
}

/// DELETE /api/articles/:slug — delete article (admin only)
pub async fn delete_article(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(slug): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let existing = state
        .db
        .get_article_by_slug(&slug)
        .await
        .map_err(|e| match &e {
            scuffed_db::DbError::NotFound(_) => (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            ),
            _ => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            ),
        })?;

    state.db.delete_article(&existing.id).await.map_err(|e| {
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
        &existing.id,
        Some(&format!("Deleted: {}", existing.title)),
    )
    .await;

    Ok(StatusCode::OK)
}
