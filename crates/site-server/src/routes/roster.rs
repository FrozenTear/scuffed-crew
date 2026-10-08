use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};

use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{AuditAction, AuditTargetType, NamedRosterEntry, RosterEntry, TeamRole};

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

/// Build the admin response from a joined roster entry, warning on a dangling edge.
fn to_response(entry: NamedRosterEntry) -> RosterMemberResponse {
    let NamedRosterEntry {
        member_id,
        member_name,
        team_role,
        ..
    } = entry;
    let member_name = member_name.unwrap_or_else(|| {
        tracing::warn!(
            member_id = %member_id,
            "roster references a member with no row (dangling plays_on edge)"
        );
        "Unknown".to_string()
    });
    RosterMemberResponse {
        member_id,
        member_name,
        team_role: team_role.to_string(),
    }
}

/// Enrich the single entry returned by an add (POST) via one member lookup.
/// The bulk GET path uses the db-level join instead of this per-entry lookup.
async fn enrich(state: &AppState, entry: RosterEntry) -> RosterMemberResponse {
    let member_name = match state.db.get_member_safe(&entry.member_id).await {
        Ok(Some(m)) => m.display_name,
        Ok(None) => {
            tracing::warn!(
                member_id = %entry.member_id,
                "roster member row not found after add"
            );
            "Unknown".to_string()
        }
        Err(e) => {
            tracing::warn!(
                member_id = %entry.member_id,
                error = %e,
                "failed to load member for roster add"
            );
            "Unknown".to_string()
        }
    };
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
    if let Some(team) = state.db.get_team(&team_id).await.map_err(|_e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Internal error".into(),
            }),
        )
    })? && !team.is_active
    {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Team not found".into(),
            }),
        ));
    }
    let entries = state
        .db
        .get_team_roster_named(&team_id)
        .await
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })?;
    let roster: Vec<RosterMemberResponse> = entries.into_iter().map(to_response).collect();
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
///
/// Returns the updated entry as JSON (same shape as POST/GET) — the admin UI
/// deserializes success bodies, so a bare `200 OK` reads as a false failure
/// (F-AUI-001). 404 when the member has no active roster edge on the team.
pub async fn update_roster_role(
    State(state): State<AppState>,
    _officer: OfficerUser,
    Path((team_id, member_id)): Path<(String, String)>,
    Json(body): Json<UpdateRosterRoleRequest>,
) -> Result<Json<RosterMemberResponse>, (StatusCode, Json<ErrorResponse>)> {
    let entry = state
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
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Member is not on this team's roster".into(),
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

    Ok(Json(enrich(&state, entry).await))
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

#[cfg(test)]
mod tests {
    use axum::Json;
    use axum::extract::{Path, State};
    use axum::http::StatusCode;
    use scuffed_db::{OrgRole, TeamRole};

    use super::get_team_roster;
    use crate::test_support::{must_err, must_ok, seed_user, test_state};

    #[tokio::test]
    async fn public_roster_hides_inactive_members_and_inactive_teams() {
        let state = test_state().await;
        let team = state
            .db
            .create_team("Alpha", "ow2", None, None, None)
            .await
            .expect("team");
        seed_user(&state, "activeuser", "Active").await;
        seed_user(&state, "goneuser", "Gone").await;
        let active = state
            .db
            .create_member("activeuser", "Active", OrgRole::Member)
            .await
            .expect("active");
        let gone = state
            .db
            .create_member("goneuser", "Gone", OrgRole::Member)
            .await
            .expect("gone");
        state
            .db
            .update_member(
                &gone.id,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(false),
                None,
                None,
                None,
            )
            .await
            .expect("deactivate member");
        state
            .db
            .add_to_roster(&active.id, &team.id, TeamRole::Captain)
            .await
            .expect("add active");
        state
            .db
            .add_to_roster(&gone.id, &team.id, TeamRole::Player)
            .await
            .expect("add gone");

        let Json(roster) = must_ok(
            get_team_roster(State(state.clone()), Path(team.id.clone())).await,
            "roster",
        );
        assert_eq!(roster.len(), 1);
        assert_eq!(roster[0].member_id, active.id);

        state
            .db
            .client
            .query("UPDATE team SET is_active = false WHERE meta::id(id) = $id")
            .bind(("id", team.id.clone()))
            .await
            .expect("deactivate team");
        let (status, body) = must_err(
            get_team_roster(State(state), Path(team.id)).await,
            "inactive team",
        );
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body.error, "Team not found");
    }
}
