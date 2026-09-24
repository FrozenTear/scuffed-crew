use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;

use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{EventRsvp, Member, RsvpStatus, RsvpSummary};

use crate::extractors::{OptionalOrgMember, OrgMember};
use crate::state::AppState;

fn not_found() -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse {
            error: "Event not found".into(),
        }),
    )
}

fn internal_err() -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse {
            error: "Internal error".into(),
        }),
    )
}

/// Anonymous callers may read RSVPs only for a public, active event.
/// Org members may read any active event (including private). Inactive and
/// unknown events are 404 so a hidden id is indistinguishable from a missing one.
async fn ensure_rsvp_visible(
    state: &AppState,
    event_id: &str,
    member: Option<&Member>,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let event = state
        .db
        .get_event(event_id)
        .await
        .map_err(|_| internal_err())?
        .ok_or_else(not_found)?;
    if !event.is_active {
        return Err(not_found());
    }
    if event.is_public || member.is_some() {
        return Ok(());
    }
    Err(not_found())
}

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
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })?;

    Ok(Json(rsvp))
}

/// GET /api/events/:id/rsvps — RSVP list.
/// Anonymous: public + active only. Otherwise an org member. Hidden → 404.
pub async fn get_event_rsvps(
    State(state): State<AppState>,
    OptionalOrgMember(member): OptionalOrgMember,
    Path(event_id): Path<String>,
) -> Result<Json<Vec<EventRsvp>>, (StatusCode, Json<ErrorResponse>)> {
    ensure_rsvp_visible(&state, &event_id, member.as_ref()).await?;
    state
        .db
        .get_event_rsvps(&event_id)
        .await
        .map(Json)
        .map_err(|_| internal_err())
}

/// GET /api/events/:id/rsvp-summary — RSVP counts.
/// Same visibility as [`get_event_rsvps`].
pub async fn get_rsvp_summary(
    State(state): State<AppState>,
    OptionalOrgMember(member): OptionalOrgMember,
    Path(event_id): Path<String>,
) -> Result<Json<RsvpSummary>, (StatusCode, Json<ErrorResponse>)> {
    ensure_rsvp_visible(&state, &event_id, member.as_ref()).await?;
    state
        .db
        .get_rsvp_summary(&event_id)
        .await
        .map(Json)
        .map_err(|_| internal_err())
}
