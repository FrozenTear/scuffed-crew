use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;

use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{Announcement, AuditAction, AuditTargetType};
use scuffed_types::api::{CursorResponse, PaginationParams};

use crate::extractors::OfficerUser;
use crate::routes::audit_log::audit;
use crate::state::AppState;

/// GET /api/announcements — list active announcements (cursor-paginated, public)
pub async fn list_announcements(
    State(state): State<AppState>,
    axum::extract::Query(pagination): axum::extract::Query<PaginationParams>,
) -> Result<Json<CursorResponse<Announcement>>, (StatusCode, Json<ErrorResponse>)> {
    let (limit, offset) = pagination.resolve();
    let items = state
        .db
        .list_announcements_paginated(limit, offset)
        .await
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })?;
    Ok(Json(CursorResponse::from_oversized(items, limit, offset)))
}

#[derive(Deserialize)]
pub struct CreateAnnouncementRequest {
    pub title: String,
    pub content: String,
    #[serde(default)]
    pub pinned: bool,
}

/// POST /api/announcements — create announcement (officer+)
pub async fn create_announcement(
    State(state): State<AppState>,
    officer: OfficerUser,
    Json(body): Json<CreateAnnouncementRequest>,
) -> Result<(StatusCode, Json<Announcement>), (StatusCode, Json<ErrorResponse>)> {
    let ann = state
        .db
        .create_announcement(&body.title, &body.content, &officer.member.id, body.pinned)
        .await
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })?;

    // Notify general room about new announcement
    if let Some(ref notifier) = state.notifier {
        notifier.notify_general(format!("New announcement: {}", ann.title));
    }

    audit(
        &state.db,
        &officer.member.id,
        AuditAction::CreatedAnnouncement,
        AuditTargetType::Announcement,
        &ann.id,
        Some(&format!("Created: {}", ann.title)),
    )
    .await;

    Ok((StatusCode::CREATED, Json(ann)))
}

#[derive(Deserialize)]
pub struct UpdateAnnouncementRequest {
    pub title: Option<String>,
    pub content: Option<String>,
    pub pinned: Option<bool>,
}

/// PUT /api/announcements/:id — update announcement (officer+)
pub async fn update_announcement(
    State(state): State<AppState>,
    officer: OfficerUser,
    Path(id): Path<String>,
    Json(body): Json<UpdateAnnouncementRequest>,
) -> Result<Json<Announcement>, (StatusCode, Json<ErrorResponse>)> {
    let ann = state
        .db
        .update_announcement(
            &id,
            body.title.as_deref(),
            body.content.as_deref(),
            body.pinned,
        )
        .await
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })?;

    audit(
        &state.db,
        &officer.member.id,
        AuditAction::UpdatedAnnouncement,
        AuditTargetType::Announcement,
        &id,
        None,
    )
    .await;

    Ok(Json(ann))
}

/// DELETE /api/announcements/:id — soft delete (officer+)
pub async fn delete_announcement(
    State(state): State<AppState>,
    officer: OfficerUser,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    state.db.deactivate_announcement(&id).await.map_err(|_e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Internal error".into(),
            }),
        )
    })?;

    audit(
        &state.db,
        &officer.member.id,
        AuditAction::DeletedAnnouncement,
        AuditTargetType::Announcement,
        &id,
        None,
    )
    .await;

    Ok(StatusCode::OK)
}
