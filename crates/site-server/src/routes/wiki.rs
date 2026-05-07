use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};

use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{AuditAction, AuditTargetType, WikiPage, WikiRevision};

use crate::extractors::{OfficerUser, OrgMember};
use crate::routes::audit_log::audit;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct WikiListQuery {
    pub q: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: u32,
    #[serde(default)]
    pub offset: u32,
}

fn default_limit() -> u32 {
    50
}

#[derive(Serialize)]
pub struct WikiListResponse {
    pub data: Vec<WikiPage>,
}

/// GET /api/wiki — list wiki pages (public)
pub async fn list_wiki_pages(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<WikiListQuery>,
) -> Result<Json<WikiListResponse>, (StatusCode, Json<ErrorResponse>)> {
    let pages = state
        .db
        .list_wiki_pages(query.q.as_deref(), query.limit, query.offset)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
        })?;

    Ok(Json(WikiListResponse { data: pages }))
}

/// GET /api/wiki/:topic — get page by topic slug (public)
pub async fn get_wiki_page(
    State(state): State<AppState>,
    Path(topic): Path<String>,
) -> Result<Json<WikiPage>, (StatusCode, Json<ErrorResponse>)> {
    state
        .db
        .get_wiki_page_by_topic(&topic)
        .await
        .map(Json)
        .map_err(|e| {
            let status = match &e {
                scuffed_db::DbError::NotFound(_) => StatusCode::NOT_FOUND,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            (
                status,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
        })
}

#[derive(Deserialize)]
pub struct CreateWikiPageRequest {
    pub topic: String,
    pub title: String,
    pub content_markdown: String,
}

/// POST /api/wiki — create wiki page (member auth)
pub async fn create_wiki_page(
    State(state): State<AppState>,
    member: OrgMember,
    Json(body): Json<CreateWikiPageRequest>,
) -> Result<(StatusCode, Json<WikiPage>), (StatusCode, Json<ErrorResponse>)> {
    let page = state
        .db
        .create_wiki_page(
            &body.topic,
            &body.title,
            &body.content_markdown,
            &member.member.id,
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
        &member.member.id,
        AuditAction::CreatedWikiPage,
        AuditTargetType::WikiPage,
        &page.id,
        Some(&format!("Created wiki page: {}", page.title)),
    )
    .await;

    Ok((StatusCode::CREATED, Json(page)))
}

#[derive(Deserialize)]
pub struct UpdateWikiPageRequest {
    pub content_markdown: String,
    pub revision_note: Option<String>,
}

/// PUT /api/wiki/:topic — update wiki page, creates revision (member auth)
pub async fn update_wiki_page(
    State(state): State<AppState>,
    member: OrgMember,
    Path(topic): Path<String>,
    Json(body): Json<UpdateWikiPageRequest>,
) -> Result<Json<WikiPage>, (StatusCode, Json<ErrorResponse>)> {
    let page = state
        .db
        .update_wiki_page(
            &topic,
            &body.content_markdown,
            &member.member.id,
            body.revision_note.as_deref(),
        )
        .await
        .map_err(|e| {
            let status = match &e {
                scuffed_db::DbError::NotFound(_) => StatusCode::NOT_FOUND,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            (
                status,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
        })?;

    audit(
        &state.db,
        &member.member.id,
        AuditAction::UpdatedWikiPage,
        AuditTargetType::WikiPage,
        &page.id,
        body.revision_note.as_deref(),
    )
    .await;

    Ok(Json(page))
}

#[derive(Deserialize)]
pub struct WikiRevisionsQuery {
    #[serde(default = "default_limit")]
    pub limit: u32,
    #[serde(default)]
    pub offset: u32,
}

#[derive(Serialize)]
pub struct WikiRevisionsResponse {
    pub data: Vec<WikiRevision>,
}

/// GET /api/wiki/:topic/revisions — revision history (public)
pub async fn list_wiki_revisions(
    State(state): State<AppState>,
    Path(topic): Path<String>,
) -> Result<Json<WikiRevisionsResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Look up page to get the ID
    let page = state
        .db
        .get_wiki_page_by_topic(&topic)
        .await
        .map_err(|e| {
            let status = match &e {
                scuffed_db::DbError::NotFound(_) => StatusCode::NOT_FOUND,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            (
                status,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
        })?;

    let revisions = state
        .db
        .list_wiki_revisions(&page.id, 50, 0)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
        })?;

    Ok(Json(WikiRevisionsResponse { data: revisions }))
}

/// DELETE /api/wiki/:topic — deactivate wiki page (officer+)
pub async fn delete_wiki_page(
    State(state): State<AppState>,
    officer: OfficerUser,
    Path(topic): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    // Get page ID for audit log before deactivating
    let page = state
        .db
        .get_wiki_page_by_topic(&topic)
        .await
        .map_err(|e| {
            let status = match &e {
                scuffed_db::DbError::NotFound(_) => StatusCode::NOT_FOUND,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            (
                status,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
        })?;

    state
        .db
        .deactivate_wiki_page(&topic)
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
        AuditAction::DeletedWikiPage,
        AuditTargetType::WikiPage,
        &page.id,
        Some(&format!("Deleted wiki page: {}", topic)),
    )
    .await;

    Ok(StatusCode::OK)
}
