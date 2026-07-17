use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;

use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{AuditAction, AuditTargetType, Member, OrgRole, Scrim};
use scuffed_types::api::{CursorResponse, PaginationParams};

use crate::extractors::OrgMember;
use crate::routes::audit_log::audit;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct ScrimListParams {
    #[serde(flatten)]
    pub pagination: PaginationParams,
    pub team_id: Option<String>,
    pub status: Option<String>,
}

/// GET /api/scrims — list scrims (cursor-paginated, member auth)
pub async fn list_scrims(
    State(state): State<AppState>,
    _member: OrgMember,
    axum::extract::Query(params): axum::extract::Query<ScrimListParams>,
) -> Result<Json<CursorResponse<Scrim>>, (StatusCode, Json<ErrorResponse>)> {
    let (limit, offset) = params.pagination.resolve();
    let items = state
        .db
        .list_scrims(
            params.team_id.as_deref(),
            params.status.as_deref(),
            limit,
            offset,
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
    Ok(Json(CursorResponse::from_oversized(items, limit, offset)))
}

#[derive(Deserialize)]
pub struct CreateScrimRequest {
    pub team_id: String,
    pub game_id: String,
    pub scheduled_at: chrono::DateTime<chrono::Utc>,
    #[serde(default = "default_duration")]
    pub duration_minutes: u32,
    pub notes: Option<String>,
}

fn default_duration() -> u32 {
    90
}

/// Require caller on team roster **or** Officer+ (authz for scrim mutations).
async fn authorize_scrim_team(
    state: &AppState,
    member: &Member,
    team_id: &str,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if member.org_role.is_at_least(OrgRole::Officer) {
        return Ok(());
    }

    let on_roster = state
        .db
        .is_on_team_roster(&member.id, team_id)
        .await
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })?;

    if on_roster {
        return Ok(());
    }

    Err((
        StatusCode::FORBIDDEN,
        Json(ErrorResponse {
            error: "Must be on the team roster or an officer".into(),
        }),
    ))
}

/// POST /api/scrims — create scrim request (roster member or officer+)
pub async fn create_scrim(
    State(state): State<AppState>,
    member: OrgMember,
    Json(body): Json<CreateScrimRequest>,
) -> Result<(StatusCode, Json<Scrim>), (StatusCode, Json<ErrorResponse>)> {
    authorize_scrim_team(&state, &member.member, &body.team_id).await?;

    let scrim = state
        .db
        .create_scrim(
            &body.team_id,
            &body.game_id,
            &member.member.id,
            body.scheduled_at,
            body.duration_minutes,
            body.notes.as_deref(),
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
        &member.member.id,
        AuditAction::CreatedScrim,
        AuditTargetType::Scrim,
        &scrim.id,
        Some(&format!("Created scrim for team {}", body.team_id)),
    )
    .await;

    Ok((StatusCode::CREATED, Json(scrim)))
}

#[derive(Deserialize)]
pub struct UpdateScrimStatusRequest {
    pub status: String,
    pub opponent_name: Option<String>,
}

/// PATCH /api/scrims/:id — update scrim status (confirm, cancel, complete)
/// (roster member of the scrim's team or officer+)
pub async fn update_scrim_status(
    State(state): State<AppState>,
    member: OrgMember,
    Path(id): Path<String>,
    Json(body): Json<UpdateScrimStatusRequest>,
) -> Result<Json<Scrim>, (StatusCode, Json<ErrorResponse>)> {
    let valid = ["open", "confirmed", "cancelled", "completed"];
    if !valid.contains(&body.status.as_str()) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!(
                    "Invalid status '{}'. Must be one of: {}",
                    body.status,
                    valid.join(", ")
                ),
            }),
        ));
    }

    let existing = state.db.get_scrim(&id).await.map_err(|e| {
        let not_found = matches!(e, scuffed_db::DbError::NotFound(_));
        if not_found {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Scrim not found".into(),
                }),
            )
        } else {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        }
    })?;

    authorize_scrim_team(&state, &member.member, &existing.team_id).await?;

    let scrim = state
        .db
        .update_scrim_status(&id, &body.status, body.opponent_name.as_deref())
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
        &member.member.id,
        AuditAction::UpdatedScrimStatus,
        AuditTargetType::Scrim,
        &id,
        Some(&format!("Status changed to {}", body.status)),
    )
    .await;

    Ok(Json(scrim))
}
