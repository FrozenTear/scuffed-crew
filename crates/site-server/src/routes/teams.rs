use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};

use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{AuditAction, AuditTargetType, GroupType, Team, TeamChannel};

use scuffed_types::api::{CursorResponse, PaginationParams};

use crate::extractors::{AdminUser, OfficerUser, OrgMember};
use crate::routes::audit_log::audit;
use crate::state::AppState;
use crate::team_channels;

/// GET /api/teams — list all teams (cursor-paginated, public)
pub async fn list_teams(
    State(state): State<AppState>,
    axum::extract::Query(pagination): axum::extract::Query<PaginationParams>,
) -> Result<Json<CursorResponse<Team>>, (StatusCode, Json<ErrorResponse>)> {
    let (limit, offset) = pagination.resolve();
    let items = state
        .db
        .list_teams_paginated(limit, offset)
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

/// GET /api/teams/:id — get team detail (public)
pub async fn get_team(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Team>, (StatusCode, Json<ErrorResponse>)> {
    state
        .db
        .get_team(&id)
        .await
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
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
        &_admin.member.id,
        AuditAction::CreatedTeam,
        AuditTargetType::Team,
        &team.id,
        Some(&format!("Created team: {}", team.name)),
    )
    .await;

    // F-API-003: write team_channel rows so send_encrypted can find a channel.
    // Best-effort — team create already committed.
    team_channels::provision_for_team(&state, &team.id).await;

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
        &_officer.member.id,
        AuditAction::UpdatedTeam,
        AuditTargetType::Team,
        &id,
        None,
    )
    .await;

    // Idempotent: also backfills teams created before F-API-003.
    team_channels::provision_for_team(&state, &team.id).await;

    Ok(Json(team))
}

/// GET /api/teams/:id/channels — list provisioned chat channels for a team.
///
/// Site chat mount: use `group_id` of the `officer` row as `POST
/// /api/chat/send-encrypted` `group_id`. Members see public channels only;
/// officer+ also see the officer channel.
pub async fn list_team_channels(
    State(state): State<AppState>,
    caller: OrgMember,
    Path(id): Path<String>,
) -> Result<Json<Vec<TeamChannel>>, (StatusCode, Json<ErrorResponse>)> {
    let team = state
        .db
        .get_team(&id)
        .await
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Team not found".into(),
                }),
            )
        })?;

    let mut channels = state.db.get_team_channels(&team.id).await.map_err(|_e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Internal error".into(),
            }),
        )
    })?;

    if !caller.member.org_role.can_access_officer_channel() {
        channels.retain(|c| c.group_type != GroupType::Officer);
    }

    Ok(Json(channels))
}

#[derive(Serialize)]
pub struct ProvisionChannelsResponse {
    pub teams_provisioned: usize,
}

/// POST /api/admin/teams/provision-channels — backfill channels for all teams.
pub async fn provision_all_channels(
    State(state): State<AppState>,
    admin: AdminUser,
) -> Result<Json<ProvisionChannelsResponse>, (StatusCode, Json<ErrorResponse>)> {
    let n = team_channels::backfill_all_teams(&state.db, state.relay_url.as_deref()).await;
    audit(
        &state.db,
        &admin.member.id,
        AuditAction::UpdatedTeam,
        AuditTargetType::Team,
        "all",
        Some(&format!("Backfilled chat channels for {n} team(s)")),
    )
    .await;
    Ok(Json(ProvisionChannelsResponse {
        teams_provisioned: n,
    }))
}
