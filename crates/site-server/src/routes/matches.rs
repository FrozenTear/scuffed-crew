use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{AuditAction, AuditTargetType, MatchResult, MatchType};
use scuffed_types::api::{CursorResponse, PaginationParams};

use crate::extractors::OfficerUser;
use crate::routes::audit_log::audit;
use crate::state::AppState;

/// GET /api/teams/:id/matches — team match history (cursor-paginated, public)
pub async fn list_team_matches(
    State(state): State<AppState>,
    Path(team_id): Path<String>,
    axum::extract::Query(pagination): axum::extract::Query<PaginationParams>,
) -> Result<Json<CursorResponse<MatchResult>>, (StatusCode, Json<ErrorResponse>)> {
    let (limit, offset) = pagination.resolve();
    let items = state
        .db
        .list_team_matches_paginated(&team_id, limit, offset)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
        })?;
    Ok(Json(CursorResponse::from_oversized(items, limit, offset)))
}

#[derive(Deserialize)]
pub struct RecordMatchRequest {
    pub team_id: String,
    pub opponent: String,
    pub score_us: u32,
    pub score_them: u32,
    pub map_name: Option<String>,
    pub game_mode: Option<String>,
    #[serde(default)]
    pub match_type: MatchType,
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
            body.match_type,
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
    audit(
        &state.db,
        &officer.member.id,
        AuditAction::RecordedMatch,
        AuditTargetType::Match,
        &result.id,
        Some(&format!(
            "{} vs {} ({}-{})",
            result.match_type, result.opponent, result.score_us, result.score_them
        )),
    )
    .await;

    Ok((StatusCode::CREATED, Json(result)))
}

#[derive(Deserialize)]
pub struct UpdateMatchRequest {
    pub opponent: Option<String>,
    pub score_us: Option<u32>,
    pub score_them: Option<u32>,
    pub map_name: Option<Option<String>>,
    pub game_mode: Option<Option<String>>,
    pub match_type: Option<MatchType>,
    pub notes: Option<Option<String>>,
}

/// PUT /api/matches/:id — update match (officer+)
pub async fn update_match(
    State(state): State<AppState>,
    officer: OfficerUser,
    Path(id): Path<String>,
    Json(body): Json<UpdateMatchRequest>,
) -> Result<Json<MatchResult>, (StatusCode, Json<ErrorResponse>)> {
    let result = state
        .db
        .update_match(
            &id,
            body.opponent.as_deref(),
            body.score_us,
            body.score_them,
            body.map_name.as_ref().map(|m| m.as_deref()),
            body.game_mode.as_ref().map(|g| g.as_deref()),
            body.match_type,
            body.notes.as_ref().map(|n| n.as_deref()),
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
        AuditAction::UpdatedMatch,
        AuditTargetType::Match,
        &id,
        None,
    )
    .await;

    Ok(Json(result))
}
