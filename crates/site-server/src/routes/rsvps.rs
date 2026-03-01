use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;

use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{EventRsvp, RsvpStatus, RsvpSummary};

use crate::extractors::OrgMember;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct RsvpRequest {
    pub status: RsvpStatus,
}

/// POST /api/events/:id/rsvp — RSVP to an event (org member)
pub async fn rsvp_event(
    State(state): State<AppState>,
    member: OrgMember,
    Path(event_id): Path<String>,
    Json(body): Json<RsvpRequest>,
) -> Result<Json<EventRsvp>, (StatusCode, Json<ErrorResponse>)> {
    let rsvp = state
        .db
        .upsert_rsvp(&event_id, &member.member.id, body.status)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
        })?;

    Ok(Json(rsvp))
}

/// GET /api/events/:id/rsvps — get RSVPs for an event (public)
pub async fn get_event_rsvps(
    State(state): State<AppState>,
    Path(event_id): Path<String>,
) -> Result<Json<Vec<EventRsvp>>, (StatusCode, Json<ErrorResponse>)> {
    state
        .db
        .get_event_rsvps(&event_id)
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

/// GET /api/events/:id/rsvp-summary — get RSVP counts for an event (public)
pub async fn get_rsvp_summary(
    State(state): State<AppState>,
    Path(event_id): Path<String>,
) -> Result<Json<RsvpSummary>, (StatusCode, Json<ErrorResponse>)> {
    state
        .db
        .get_rsvp_summary(&event_id)
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
