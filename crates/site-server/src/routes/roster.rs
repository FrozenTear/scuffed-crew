use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};

use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{AuditAction, AuditTargetType, RosterEntry, TeamRole};

use crate::extractors::OfficerUser;
use crate::routes::audit_log::audit;
use crate::state::AppState;

/// Roster entry enriched with the member's display name for the admin UI.
#[derive(Serialize)]
pub struct RosterMemberResponse {
    pub member_id: String,
    pub member_name: String,
    pub team_role: String,
}

/// Enrich a raw roster entry with the member's display name.
async fn enrich(state: &AppState, entry: RosterEntry) -> RosterMemberResponse {
    let member_name = state
        .db
        .get_member_safe(&entry.member_id)
        .await
        .ok()
        .flatten()
        .map(|m| m.display_name)
        .unwrap_or_else(|| "Unknown".to_string());
    RosterMemberResponse {
        member_id: entry.member_id,
        member_name,
        team_role: entry.team_role.to_string(),
    }
}

/// GET /api/teams/:id/roster — get team roster (public)
pub async fn get_team_roster(
    State(state): State<AppState>,
    Path(team_id): Path<String>,
) -> Result<Json<Vec<RosterMemberResponse>>, (StatusCode, Json<ErrorResponse>)> {
    let entries = state.db.get_team_roster(&team_id).await.map_err(|_e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Internal error".into(),
            }),
        )
    })?;
    let mut roster = Vec::with_capacity(entries.len());
    for entry in entries {
        roster.push(enrich(&state, entry).await);
    }
    Ok(Json(roster))
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
) -> Result<(StatusCode, Json<RosterMemberResponse>), (StatusCode, Json<ErrorResponse>)> {
    let entry = state
        .db
        .add_to_roster(&body.member_id, &team_id, body.team_role)
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
        AuditAction::AddedToRoster,
        AuditTargetType::Roster,
        &team_id,
        Some(&format!(
            "Added member {} as {}",
            body.member_id, body.team_role
        )),
    )
    .await;

    Ok((StatusCode::CREATED, Json(enrich(&state, entry).await)))
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
        AuditAction::RemovedFromRoster,
        AuditTargetType::Roster,
        &team_id,
        Some(&format!("Removed member {member_id}")),
    )
    .await;

    Ok(StatusCode::OK)
}
