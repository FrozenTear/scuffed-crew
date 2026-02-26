use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::MatchResult;

use crate::extractors::OfficerUser;
use crate::state::AppState;

/// GET /api/teams/:id/matches — team match history (public)
pub async fn list_team_matches(
    State(state): State<AppState>,
    Path(team_id): Path<String>,
) -> Result<Json<Vec<MatchResult>>, (StatusCode, Json<ErrorResponse>)> {
    state
        .db
        .list_team_matches(&team_id)
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

#[derive(Deserialize)]
pub struct RecordMatchRequest {
    pub team_id: String,
    pub opponent: String,
    pub score_us: u32,
    pub score_them: u32,
    pub map_name: Option<String>,
    pub game_mode: Option<String>,
    pub played_at: DateTime<Utc>,
    pub notes: Option<String>,
}

/// POST /api/matches — record match result (officer+)
pub async fn record_match(
    State(state): State<AppState>,
    officer: OfficerUser,
    Json(body): Json<RecordMatchRequest>,
) -> Result<(StatusCode, Json<MatchResult>), (StatusCode, Json<ErrorResponse>)> {
    let result = state
        .db
        .record_match(
            &body.team_id,
            &body.opponent,
            body.score_us,
            body.score_them,
            body.map_name.as_deref(),
            body.game_mode.as_deref(),
            body.played_at,
            &officer.member.id,
            body.notes.as_deref(),
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
    Ok((StatusCode::CREATED, Json(result)))
}

#[derive(Deserialize)]
pub struct UpdateMatchRequest {
    pub opponent: Option<String>,
    pub score_us: Option<u32>,
    pub score_them: Option<u32>,
    pub map_name: Option<Option<String>>,
    pub game_mode: Option<Option<String>>,
    pub notes: Option<Option<String>>,
}

/// PUT /api/matches/:id — update match (officer+)
pub async fn update_match(
    State(state): State<AppState>,
    _officer: OfficerUser,
    Path(id): Path<String>,
    Json(body): Json<UpdateMatchRequest>,
) -> Result<Json<MatchResult>, (StatusCode, Json<ErrorResponse>)> {
    state
        .db
        .update_match(
            &id,
            body.opponent.as_deref(),
            body.score_us,
            body.score_them,
            body.map_name.as_ref().map(|m| m.as_deref()),
            body.game_mode.as_ref().map(|g| g.as_deref()),
            body.notes.as_ref().map(|n| n.as_deref()),
        )
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
