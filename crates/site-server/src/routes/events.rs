use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;

use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{AuditAction, AuditTargetType, Event};

use crate::extractors::OfficerUser;
use crate::routes::audit_log::audit;
use crate::state::AppState;

/// GET /api/events — list all active events (public)
pub async fn list_events(
    State(state): State<AppState>,
) -> Result<Json<Vec<Event>>, (StatusCode, Json<ErrorResponse>)> {
    state.db.list_events().await.map(Json).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
    })
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
    state.db.deactivate_event(&id).await.map_err(|e| {
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
        AuditAction::DeletedEvent,
        AuditTargetType::Event,
        &id,
        None,
    )
    .await;

    Ok(StatusCode::OK)
}
