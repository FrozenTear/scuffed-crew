use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;

use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{AuditAction, AuditTargetType, RosterEntry, TeamRole};

use crate::extractors::OfficerUser;
use crate::routes::audit_log::audit;
use crate::state::AppState;

/// GET /api/teams/:id/roster — get team roster (public)
pub async fn get_team_roster(
    State(state): State<AppState>,
    Path(team_id): Path<String>,
) -> Result<Json<Vec<RosterEntry>>, (StatusCode, Json<ErrorResponse>)> {
    state
        .db
        .get_team_roster(&team_id)
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
pub struct AddToRosterRequest {
    pub member_id: String,
    pub team_role: TeamRole,
}

/// POST /api/teams/:id/roster — add member to roster (officer+)
pub async fn add_to_roster(
    State(state): State<AppState>,
    _officer: OfficerUser,
    Path(team_id): Path<String>,
    Json(body): Json<AddToRosterRequest>,
) -> Result<(StatusCode, Json<RosterEntry>), (StatusCode, Json<ErrorResponse>)> {
    let entry = state
        .db
        .add_to_roster(&body.member_id, &team_id, body.team_role)
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
        AuditAction::AddedToRoster,
        AuditTargetType::Roster,
        &team_id,
        Some(&format!(
            "Added member {} as {}",
            body.member_id, body.team_role
        )),
    )
    .await;

    Ok((StatusCode::CREATED, Json(entry)))
}

#[derive(Deserialize)]
pub struct UpdateRosterRoleRequest {
    pub team_role: TeamRole,
}

/// PUT /api/teams/:id/roster/:member_id — update team role (officer+)
pub async fn update_roster_role(
    State(state): State<AppState>,
    _officer: OfficerUser,
    Path((team_id, member_id)): Path<(String, String)>,
    Json(body): Json<UpdateRosterRoleRequest>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    state
        .db
        .update_roster_role(&member_id, &team_id, body.team_role)
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
        AuditAction::UpdatedRosterRole,
        AuditTargetType::Roster,
        &team_id,
        Some(&format!("Changed {} role to {}", member_id, body.team_role)),
    )
    .await;

    Ok(StatusCode::OK)
}

/// DELETE /api/teams/:id/roster/:member_id — remove from roster (officer+)
pub async fn remove_from_roster(
    State(state): State<AppState>,
    _officer: OfficerUser,
    Path((team_id, member_id)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    state
        .db
        .remove_from_roster(&member_id, &team_id)
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
        AuditAction::RemovedFromRoster,
        AuditTargetType::Roster,
        &team_id,
        Some(&format!("Removed member {member_id}")),
    )
    .await;

    Ok(StatusCode::OK)
}
