use axum::{extract::State, http::StatusCode, Json};
use serde::Serialize;

use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{Event, Team, TeamRecord};

use crate::state::AppState;

#[derive(Serialize)]
pub struct TeamOverview {
    #[serde(flatten)]
    pub team: Team,
    pub roster_count: usize,
    pub record: TeamRecord,
}

#[derive(Serialize)]
pub struct PublicOverview {
    pub teams: Vec<TeamOverview>,
    pub events: Vec<Event>,
}

/// GET /api/public/overview — aggregated public data for the site
pub async fn overview(
    State(state): State<AppState>,
) -> Result<Json<PublicOverview>, (StatusCode, Json<ErrorResponse>)> {
    let teams = state.db.list_teams().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
    })?;

    let mut team_overviews = Vec::with_capacity(teams.len());
    for team in teams {
        let roster = state.db.get_team_roster(&team.id).await.map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
        })?;
        let record = state.db.get_team_record(&team.id).await.map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
        })?;
        team_overviews.push(TeamOverview {
            roster_count: roster.len(),
            team,
            record,
        });
    }

    let events = state.db.list_events().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
    })?;

    Ok(Json(PublicOverview {
        teams: team_overviews,
        events,
    }))
}
