use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;

use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{AuditAction, AuditTargetType, Event};
use scuffed_types::api::{CursorResponse, PaginationParams};

use crate::extractors::{OfficerUser, OptionalOrgMember};
use crate::routes::audit_log::audit;
use crate::state::AppState;

/// GET /api/events — list active events (cursor-paginated).
/// Anonymous: `is_public` only. Org members: all active events.
pub async fn list_events(
    State(state): State<AppState>,
    OptionalOrgMember(member): OptionalOrgMember,
    axum::extract::Query(pagination): axum::extract::Query<PaginationParams>,
) -> Result<Json<CursorResponse<Event>>, (StatusCode, Json<ErrorResponse>)> {
    let (limit, offset) = pagination.resolve();
    let items = state
        .db
        .list_events_paginated(limit, offset)
        .await
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })?;
    let items = if member.is_some() {
        items
    } else {
        items.into_iter().filter(|e| e.is_public).collect()
    };
    Ok(Json(CursorResponse::from_oversized(items, limit, offset)))
}

#[derive(Deserialize)]
pub struct CreateEventRequest {
    pub title: String,
    pub day_of_week: u8,
    pub time: String,
    #[serde(default = "default_timezone")]
    pub timezone: String,
    #[serde(default = "default_duration")]
    pub duration_minutes: u32,
    #[serde(default = "default_true")]
    pub is_recurring: bool,
    pub team_id: Option<String>,
    /// Default false — practice slots stay private until published.
    #[serde(default)]
    pub is_public: bool,
}

fn default_timezone() -> String {
    "Europe/Berlin".to_string()
}
fn default_duration() -> u32 {
    120
}
fn default_true() -> bool {
    true
}

/// POST /api/events — create event (officer+)
pub async fn create_event(
    State(state): State<AppState>,
    officer: OfficerUser,
    Json(body): Json<CreateEventRequest>,
) -> Result<(StatusCode, Json<Event>), (StatusCode, Json<ErrorResponse>)> {
    let event = state
        .db
        .create_event(
            &body.title,
            body.day_of_week,
            &body.time,
            &body.timezone,
            body.duration_minutes,
            body.is_recurring,
            body.team_id.as_deref(),
            &officer.member.id,
            body.is_public,
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
        AuditAction::CreatedEvent,
        AuditTargetType::Event,
        &event.id,
        Some(&format!("Created event: {}", event.title)),
    )
    .await;

    Ok((StatusCode::CREATED, Json(event)))
}

#[derive(Deserialize)]
pub struct UpdateEventRequest {
    pub title: Option<String>,
    pub day_of_week: Option<u8>,
    pub time: Option<String>,
    pub timezone: Option<String>,
    pub duration_minutes: Option<u32>,
    pub is_recurring: Option<bool>,
    pub team_id: Option<Option<String>>,
    pub is_public: Option<bool>,
}

/// PUT /api/events/:id — update event (officer+)
pub async fn update_event(
    State(state): State<AppState>,
    officer: OfficerUser,
    Path(id): Path<String>,
    Json(body): Json<UpdateEventRequest>,
) -> Result<Json<Event>, (StatusCode, Json<ErrorResponse>)> {
    let event = state
        .db
        .update_event(
            &id,
            body.title.as_deref(),
            body.day_of_week,
            body.time.as_deref(),
            body.timezone.as_deref(),
            body.duration_minutes,
            body.is_recurring,
            body.team_id.as_ref().map(|t| t.as_deref()),
            body.is_public,
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
        AuditAction::UpdatedEvent,
        AuditTargetType::Event,
        &id,
        None,
    )
    .await;

    Ok(Json(event))
}

/// DELETE /api/events/:id — deactivate event (officer+)
pub async fn delete_event(
    State(state): State<AppState>,
    officer: OfficerUser,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    state.db.deactivate_event(&id).await.map_err(|_e| {
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
        AuditAction::DeletedEvent,
        AuditTargetType::Event,
        &id,
        None,
    )
    .await;

    Ok(StatusCode::OK)
}
