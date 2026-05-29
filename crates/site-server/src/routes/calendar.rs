use axum::{
    Json,
    extract::{Path, State},
    http::{StatusCode, header},
    response::IntoResponse,
};

use scuffed_auth::server::session::ErrorResponse;

use crate::calendar::generate_ical;
use crate::state::AppState;

/// GET /api/calendar/all.ics — ICS feed with all active events (public)
pub async fn all_events_ics(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, (StatusCode, Json<ErrorResponse>)> {
    let events = state.db.list_events().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
    })?;

    let host = state
        .oauth_config
        .redirect_base_url
        .replace("https://", "")
        .replace("http://", "");

    let ical = generate_ical(&events, &host, "The Scuffed Crew - All Events");

    Ok((
        [
            (header::CONTENT_TYPE, "text/calendar; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        ical,
    ))
}

/// GET /api/calendar/team/:id.ics — ICS feed for a specific team's events (public)
pub async fn team_events_ics(
    State(state): State<AppState>,
    Path(team_id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, Json<ErrorResponse>)> {
    let all_events = state.db.list_events().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
    })?;

    let team_events: Vec<_> = all_events
        .into_iter()
        .filter(|e| e.team_id.as_deref() == Some(&team_id))
        .collect();

    let host = state
        .oauth_config
        .redirect_base_url
        .replace("https://", "")
        .replace("http://", "");

    let ical = generate_ical(
        &team_events,
        &host,
        &format!("The Scuffed Crew - Team {}", team_id),
    );

    Ok((
        [
            (header::CONTENT_TYPE, "text/calendar; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        ical,
    ))
}
