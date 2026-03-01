use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;

use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::Team;

use scuffed_db::{AuditAction, AuditTargetType};

use crate::extractors::{AdminUser, OfficerUser};
use crate::routes::audit_log::audit;
use crate::state::AppState;

/// GET /api/teams — list all teams (public)
pub async fn list_teams(
    State(state): State<AppState>,
) -> Result<Json<Vec<Team>>, (StatusCode, Json<ErrorResponse>)> {
    state.db.list_teams().await.map(Json).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
    })
}

/// GET /api/teams/:id — get team detail (public)
pub async fn get_team(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Team>, (StatusCode, Json<ErrorResponse>)> {
    state
        .db
        .get_team(&id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
        })?
        .map(Json)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Team not found".into(),
                }),
            )
        })
}

#[derive(Deserialize)]
pub struct CreateTeamRequest {
    pub name: String,
    pub game_id: String,
    pub color: Option<String>,
    pub division: Option<String>,
    pub lore_quote: Option<String>,
}

/// POST /api/teams — create team (admin only)
pub async fn create_team(
    State(state): State<AppState>,
    _admin: AdminUser,
    Json(body): Json<CreateTeamRequest>,
) -> Result<(StatusCode, Json<Team>), (StatusCode, Json<ErrorResponse>)> {
    let team = state
        .db
        .create_team(
            &body.name,
            &body.game_id,
            body.color.as_deref(),
            body.division.as_deref(),
            body.lore_quote.as_deref(),
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
        &_admin.member.id,
        AuditAction::CreatedTeam,
        AuditTargetType::Team,
        &team.id,
        Some(&format!("Created team: {}", team.name)),
    )
    .await;

    Ok((StatusCode::CREATED, Json(team)))
}

#[derive(Deserialize)]
pub struct UpdateTeamRequest {
    pub name: Option<String>,
    pub game_id: Option<String>,
    pub color: Option<Option<String>>,
    pub division: Option<Option<String>>,
    pub lore_quote: Option<Option<String>>,
}

/// PUT /api/teams/:id — update team (officer+)
pub async fn update_team(
    State(state): State<AppState>,
    _officer: OfficerUser,
    Path(id): Path<String>,
    Json(body): Json<UpdateTeamRequest>,
) -> Result<Json<Team>, (StatusCode, Json<ErrorResponse>)> {
    let team = state
        .db
        .update_team(
            &id,
            body.name.as_deref(),
            body.game_id.as_deref(),
            body.color.as_ref().map(|c| c.as_deref()),
            body.division.as_ref().map(|d| d.as_deref()),
            body.lore_quote.as_ref().map(|q| q.as_deref()),
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
        &_officer.member.id,
        AuditAction::UpdatedTeam,
        AuditTargetType::Team,
        &id,
        None,
    )
    .await;

    Ok(Json(team))
}
