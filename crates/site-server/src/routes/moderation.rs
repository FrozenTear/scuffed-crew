use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{AuditAction, AuditTargetType, ModerationAction, ModerationActionType, OrgRole};

use crate::extractors::{AdminUser, OfficerUser};
use crate::membership_policy::{
    can_moderate, can_suspend_or_ban_admin, moderation_revokes_sessions,
};
use crate::routes::audit_log::audit;
use crate::state::AppState;

fn internal_err(e: impl std::fmt::Display, ctx: &str) -> (StatusCode, Json<ErrorResponse>) {
    tracing::error!(error = %e, "{ctx}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse {
            error: "Internal server error".into(),
        }),
    )
}

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
    if body.reason.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Reason is required".into(),
            }),
        ));
    }

    let target = state
        .db
        .get_member(&body.member_id)
        .await
        .map_err(|e| internal_err(e, "get_member for moderation"))?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Target member not found".into(),
                }),
            )
        })?;

    if let Err(denial) = can_moderate(
        officer.member.org_role,
        target.org_role,
        &officer.member.id,
        &target.id,
    ) {
        return Err((
            denial.status(),
            Json(ErrorResponse {
                error: denial.message().into(),
            }),
        ));
    }

    let admin_count = state
        .db
        .count_actionable_admins()
        .await
        .map_err(|e| internal_err(e, "count_actionable_admins for moderation"))?;

    let target_suspended = state
        .db
        .is_member_suspended_or_banned(&target.id)
        .await
        .map_err(|e| internal_err(e, "suspension check for moderation"))?;
    let target_is_actionable_admin =
        target.is_active && target.org_role == OrgRole::Admin && !target_suspended;

    if let Err(denial) = can_suspend_or_ban_admin(
        target.org_role,
        target.is_active,
        body.action_type,
        admin_count,
        target_is_actionable_admin,
    ) {
        return Err((
            denial.status(),
            Json(ErrorResponse {
                error: denial.message().into(),
            }),
        ));
    }

    // Permanent ban deactivates membership before creating the action so we never
    // return success with an active banned member. Fail hard if deactivate fails.
    // Suspension keeps is_active so lift restores access without a second toggle.
    if body.action_type == ModerationActionType::Ban && target.is_active {
        state
            .db
            .update_member(
                &target.id,
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
            .map_err(|e| internal_err(e, "deactivate member on ban"))?;

        // Concurrent race: another admin may have been removed in parallel.
        if target_is_actionable_admin && let Err(e) = state.db.assert_has_actionable_admin().await {
            if let Err(re) = state
                .db
                .update_member(
                    &target.id,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some(true),
                    None,
                    None,
                    None,
                )
                .await
            {
                tracing::error!(error = %re, "failed to compensate ban deactivation");
            }
            return Err(match e {
                scuffed_db::DbError::Conflict(msg) => {
                    (StatusCode::CONFLICT, Json(ErrorResponse { error: msg }))
                }
                other => internal_err(other, "assert_has_actionable_admin after ban"),
            });
        }

        audit(
            &state.db,
            &officer.member.id,
            AuditAction::DeactivatedMember,
            AuditTargetType::Member,
            &target.id,
            Some("Deactivated by ban"),
        )
        .await;
    }

    let action = state
        .db
        .create_moderation_action(
            &body.member_id,
            body.action_type,
            body.reason.trim(),
            &officer.member.id,
            body.expires_at,
        )
        .await
        .map_err(|e| internal_err(e, "create_moderation_action"))?;

    // Suspension of actionable admin: re-check after the moderation row exists.
    if body.action_type == ModerationActionType::Suspension
        && target_is_actionable_admin
        && let Err(e) = state.db.assert_has_actionable_admin().await
    {
        if let Err(re) = state.db.lift_moderation_action(&action.id).await {
            tracing::error!(error = %re, "failed to compensate suspension of last admin");
        }
        return Err(match e {
            scuffed_db::DbError::Conflict(msg) => {
                (StatusCode::CONFLICT, Json(ErrorResponse { error: msg }))
            }
            other => internal_err(other, "assert_has_actionable_admin after suspend"),
        });
    }

    // Ban / suspension: kill sessions immediately so extractor blocks next request
    if moderation_revokes_sessions(body.action_type)
        && let Err(e) = state.db.delete_sessions_for_user(&target.user_id).await
    {
        tracing::error!(
            error = %e,
            user_id = %target.user_id,
            "failed to revoke sessions after moderation"
        );
    }

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
        .map_err(|e| internal_err(e, "list_all_moderation"))?;
    let total = state
        .db
        .count_moderation()
        .await
        .map_err(|e| internal_err(e, "count_moderation"))?;

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
        .map_err(|e| internal_err(e, "list_member_moderation"))
}

/// PATCH /api/moderation/:id/lift — lift a moderation action (admin only)
pub async fn lift_moderation_action(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<String>,
) -> Result<Json<ModerationAction>, (StatusCode, Json<ErrorResponse>)> {
    let action = state
        .db
        .lift_moderation_action(&id)
        .await
        .map_err(|e| internal_err(e, "lift_moderation_action"))?;

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
