use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{AuditAction, AuditTargetType, ModerationAction, ModerationActionType};

use crate::extractors::{AdminUser, OfficerUser};
use crate::routes::audit_log::audit;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct CreateModerationRequest {
    pub member_id: String,
    pub action_type: ModerationActionType,
    pub reason: String,
    pub expires_at: Option<DateTime<Utc>>,
}

/// POST /api/moderation — create moderation action (officer+)
pub async fn create_moderation_action(
    State(state): State<AppState>,
    officer: OfficerUser,
    Json(body): Json<CreateModerationRequest>,
) -> Result<(StatusCode, Json<ModerationAction>), (StatusCode, Json<ErrorResponse>)> {
    // Officers may only moderate strictly lower ranks; admins may moderate anyone
    // except they cannot ban themselves into a lockout via this path either.
    let target = state
        .db
        .get_member(&body.member_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Target member not found".into(),
                }),
            )
        })?;

    if target.id == officer.member.id {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Cannot moderate yourself".into(),
            }),
        ));
    }

    let actor_is_admin = matches!(officer.member.org_role, scuffed_db::OrgRole::Admin);
    if !actor_is_admin {
        // Officer: target must be strictly below Officer (member/recruit only)
        let target_ok = matches!(
            target.org_role,
            scuffed_db::OrgRole::Member | scuffed_db::OrgRole::Recruit
        );
        if !target_ok {
            return Err((
                StatusCode::FORBIDDEN,
                Json(ErrorResponse {
                    error: "Officers cannot moderate admins or other officers".into(),
                }),
            ));
        }
    }

    let action = state
        .db
        .create_moderation_action(
            &body.member_id,
            body.action_type,
            &body.reason,
            &officer.member.id,
            body.expires_at,
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
        AuditAction::CreatedModerationAction,
        AuditTargetType::Moderation,
        &action.id,
        Some(&format!(
            "{} on member {}: {}",
            body.action_type, body.member_id, body.reason
        )),
    )
    .await;

    Ok((StatusCode::CREATED, Json(action)))
}

#[derive(Deserialize)]
pub struct ListModerationQuery {
    #[serde(default = "default_limit")]
    pub limit: u32,
    #[serde(default)]
    pub offset: u32,
}

fn default_limit() -> u32 {
    50
}

/// GET /api/moderation — list all moderation actions (officer+)
pub async fn list_moderation(
    State(state): State<AppState>,
    _officer: OfficerUser,
    Query(q): Query<ListModerationQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    let entries = state
        .db
        .list_all_moderation(q.limit.min(100), q.offset)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
        })?;
    let total = state.db.count_moderation().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
    })?;

    Ok(Json(serde_json::json!({
        "entries": entries,
        "total": total,
    })))
}

/// GET /api/members/:id/moderation — member moderation history (officer+)
pub async fn member_moderation(
    State(state): State<AppState>,
    _officer: OfficerUser,
    Path(member_id): Path<String>,
) -> Result<Json<Vec<ModerationAction>>, (StatusCode, Json<ErrorResponse>)> {
    state
        .db
        .list_member_moderation(&member_id)
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

/// PATCH /api/moderation/:id/lift — lift a moderation action (admin only)
pub async fn lift_moderation_action(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<String>,
) -> Result<Json<ModerationAction>, (StatusCode, Json<ErrorResponse>)> {
    let action = state.db.lift_moderation_action(&id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
    })?;

    audit(
        &state.db,
        &admin.member.id,
        AuditAction::LiftedModerationAction,
        AuditTargetType::Moderation,
        &id,
        Some(&format!(
            "Lifted {} on member {}",
            action.action_type, action.member_id
        )),
    )
    .await;

    Ok(Json(action))
}
