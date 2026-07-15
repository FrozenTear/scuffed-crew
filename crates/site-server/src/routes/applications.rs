use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;

use scuffed_auth::server::AuthUser;
use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{Application, ApplicationStatus, AuditAction, AuditTargetType, Member, OrgRole};

use crate::extractors::OfficerUser;
use crate::membership_policy::{
    applicant_may_self_withdraw, application_blocks_resubmit,
    application_status_deactivates_member, application_status_ensures_member,
    is_valid_application_transition, role_on_application_accept,
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

fn bad_request(msg: &str) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse {
            error: msg.into(),
        }),
    )
}

fn conflict(msg: &str) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::CONFLICT,
        Json(ErrorResponse {
            error: msg.into(),
        }),
    )
}

#[derive(Deserialize)]
pub struct SubmitApplicationRequest {
    pub preferred_games: Vec<String>,
    pub preferred_roles: Vec<String>,
    pub message: Option<String>,
}

/// POST /api/applications — submit application (any logged-in user)
pub async fn submit_application(
    State(state): State<AppState>,
    user: AuthUser<AppState>,
    Json(body): Json<SubmitApplicationRequest>,
) -> Result<(StatusCode, Json<Application>), (StatusCode, Json<ErrorResponse>)> {
    // Already an active org member → no application needed
    if let Some(m) = state
        .db
        .get_member_by_user(&user.id)
        .await
        .map_err(|e| internal_err(e, "get_member_by_user on submit"))?
        && m.is_active
    {
        return Err(conflict("Already an active org member"));
    }

    // Open pipeline (pending/trial) blocks a second application
    if let Some(existing) = state
        .db
        .get_application_by_user(&user.id)
        .await
        .map_err(|e| internal_err(e, "get_application_by_user on submit"))?
        && application_blocks_resubmit(existing.status)
    {
        return Err(conflict(&format!(
            "You already have an open application ({})",
            existing.status
        )));
    }

    let app = state
        .db
        .submit_application(
            &user.id,
            body.preferred_games,
            body.preferred_roles,
            body.message.as_deref(),
        )
        .await
        .map_err(|e| internal_err(e, "submit_application"))?;

    // Concurrent double-submit: two POSTs can both pass the pre-check. Keep one open app.
    let open = state
        .db
        .count_open_applications(&user.id)
        .await
        .map_err(|e| internal_err(e, "count_open_applications"))?;
    if open > 1 {
        if let Err(e) = state.db.delete_application(&app.id).await {
            tracing::error!(error = %e, app_id = %app.id, "failed to roll back duplicate application");
        }
        return Err(conflict("You already have an open application"));
    }

    if let Some(ref notifier) = state.notifier {
        notifier.notify_officers(format!(
            "New application received from user {}",
            user.username
        ));
    }

    Ok((StatusCode::CREATED, Json(app)))
}

/// GET /api/applications — list applications (officer+), cursor-paginated.
pub async fn list_applications(
    State(state): State<AppState>,
    _officer: OfficerUser,
    axum::extract::Query(pagination): axum::extract::Query<scuffed_types::api::PaginationParams>,
) -> Result<Json<scuffed_types::api::CursorResponse<Application>>, (StatusCode, Json<ErrorResponse>)>
{
    let (limit, offset) = pagination.resolve();
    let items = state
        .db
        .list_applications_paginated(limit, offset)
        .await
        .map_err(|e| internal_err(e, "list_applications"))?;
    Ok(Json(scuffed_types::api::CursorResponse::from_oversized(
        items, limit, offset,
    )))
}

/// GET /api/applications/mine — own application status (any logged-in)
pub async fn my_application(
    State(state): State<AppState>,
    user: AuthUser<AppState>,
) -> Result<Json<Option<Application>>, (StatusCode, Json<ErrorResponse>)> {
    state
        .db
        .get_application_by_user(&user.id)
        .await
        .map(Json)
        .map_err(|e| internal_err(e, "my_application"))
}

/// POST /api/applications/mine/withdraw — applicant self-withdraw (pending/trial only)
pub async fn withdraw_my_application(
    State(state): State<AppState>,
    user: AuthUser<AppState>,
) -> Result<Json<Application>, (StatusCode, Json<ErrorResponse>)> {
    let existing = state
        .db
        .get_application_by_user(&user.id)
        .await
        .map_err(|e| internal_err(e, "get_application_by_user withdraw"))?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "No application found".into(),
                }),
            )
        })?;

    if !applicant_may_self_withdraw(existing.status) {
        return Err(bad_request(&format!(
            "Cannot withdraw application in status {}",
            existing.status
        )));
    }

    // Audit actor: member id when provisioned (trial), else user id.
    let actor_id = state
        .db
        .get_member_by_user(&user.id)
        .await
        .map_err(|e| internal_err(e, "get_member_by_user withdraw actor"))?
        .map(|m| m.id)
        .unwrap_or_else(|| user.id.clone());

    let app = apply_application_transition(
        &state,
        &existing,
        ApplicationStatus::Withdrawn,
        &actor_id,
        Some("Self-withdrawn by applicant"),
    )
    .await?;

    Ok(Json(app))
}

#[derive(Deserialize)]
pub struct UpdateApplicationRequest {
    pub status: ApplicationStatus,
    pub review_notes: Option<String>,
}

/// PATCH /api/applications/:id — update status (officer+)
pub async fn update_application(
    State(state): State<AppState>,
    officer: OfficerUser,
    Path(id): Path<String>,
    Json(body): Json<UpdateApplicationRequest>,
) -> Result<Json<Application>, (StatusCode, Json<ErrorResponse>)> {
    let existing = state
        .db
        .get_application(&id)
        .await
        .map_err(|e| internal_err(e, "get_application"))?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Application not found".into(),
                }),
            )
        })?;

    if !is_valid_application_transition(existing.status, body.status) {
        return Err(bad_request(&format!(
            "Invalid transition: {} → {}",
            existing.status, body.status
        )));
    }

    let app = apply_application_transition(
        &state,
        &existing,
        body.status,
        &officer.member.id,
        body.review_notes.as_deref(),
    )
    .await?;

    Ok(Json(app))
}

/// Shared transition path: side effects before CAS status write, then audit.
async fn apply_application_transition(
    state: &AppState,
    existing: &Application,
    to: ApplicationStatus,
    actor_id: &str,
    review_notes: Option<&str>,
) -> Result<Application, (StatusCode, Json<ErrorResponse>)> {
    // Side effects run *before* the CAS status write when they must not leave a
    // terminal application without matching membership state:
    // - trial/accepted: provision first so we never accept without a member
    // - reject/withdraw: deactivate recruit first so reject never leaves an
    //   active trial recruit if deactivate would fail after status write
    if application_status_ensures_member(to) {
        ensure_member_for_application(
            state,
            &existing.user_id,
            existing.status,
            to,
            &existing.id,
        )
        .await?;
    } else if application_status_deactivates_member(to)
        && let Some(member) = state
            .db
            .get_member_by_user(&existing.user_id)
            .await
            .map_err(|e| internal_err(e, "get_member_by_user on reject"))?
        // Only auto-deactivate recruits (trial pipeline); leave higher roles alone
        && member.is_active
        && member.org_role == OrgRole::Recruit
    {
        state
            .db
            .update_member(
                &member.id,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(false),
            )
            .await
            .map_err(|e| internal_err(e, "deactivate recruit on reject"))?;
        if let Err(e) = state.db.delete_sessions_for_user(&existing.user_id).await {
            tracing::error!(
                error = %e,
                user_id = %existing.user_id,
                "failed to revoke sessions after application reject/withdraw"
            );
        }
        audit(
            &state.db,
            actor_id,
            AuditAction::DeactivatedMember,
            AuditTargetType::Member,
            &member.id,
            Some("Deactivated after application rejected/withdrawn"),
        )
        .await;
    }

    let app = state
        .db
        .update_application_status(
            &existing.id,
            existing.status,
            to,
            actor_id,
            review_notes,
        )
        .await
        .map_err(|e| match e {
            scuffed_db::DbError::Conflict(msg) => conflict(&msg),
            other => internal_err(other, "update_application_status"),
        })?;

    let action = match to {
        ApplicationStatus::Accepted => AuditAction::AcceptedApplication,
        ApplicationStatus::Rejected => AuditAction::RejectedApplication,
        ApplicationStatus::Trial => AuditAction::StartedTrialApplication,
        ApplicationStatus::Withdrawn => AuditAction::WithdrawnApplication,
        ApplicationStatus::Pending => AuditAction::AcceptedApplication, // unreachable via transitions
    };

    audit(
        &state.db,
        actor_id,
        action,
        AuditTargetType::Application,
        &existing.id,
        Some(&format!("{} → {}", existing.status, to)),
    )
    .await;

    Ok(app)
}

/// Create or update member when application moves to trial/accepted.
async fn ensure_member_for_application(
    state: &AppState,
    user_id: &str,
    from: ApplicationStatus,
    to: ApplicationStatus,
    application_id: &str,
) -> Result<Member, (StatusCode, Json<ErrorResponse>)> {
    let desired_role = if to == ApplicationStatus::Accepted {
        role_on_application_accept(from)
    } else {
        OrgRole::Recruit
    };

    let existing_member = state
        .db
        .get_member_by_user(user_id)
        .await
        .map_err(|e| internal_err(e, "get_member_by_user ensure"))?;

    if let Some(member) = existing_member {
        // Reactivate if needed
        let mut current = member;
        if !current.is_active {
            current = state
                .db
                .update_member(
                    &current.id,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some(true),
                )
                .await
                .map_err(|e| internal_err(e, "reactivate member"))?;
        }
        // Promote role when accepting after trial (or if still recruit on direct accept path)
        if to == ApplicationStatus::Accepted && current.org_role != desired_role {
            // Don't demote officers/admins via application accept
            if current.org_role.is_at_least(OrgRole::Officer) {
                return Ok(current);
            }
            current = state
                .db
                .change_member_role(&current.id, desired_role)
                .await
                .map_err(|e| internal_err(e, "promote on accept"))?;
        }
        return Ok(current);
    }

    let user = state
        .db
        .get_user(user_id)
        .await
        .map_err(|e| internal_err(e, "get_user for new member"))?
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Application user no longer exists".into(),
                }),
            )
        })?;
    let display_name = user.username.clone();

    let member = state
        .db
        .create_member(user_id, &display_name, desired_role)
        .await
        .map_err(|e| {
            // Fail the request — don't leave accepted apps without membership
            internal_err(e, "create_member for application")
        })?;

    tracing::info!(
        member_id = %member.id,
        application_id,
        role = %desired_role,
        "Provisioned member from application"
    );

    if to == ApplicationStatus::Accepted
        && let Some(ref notifier) = state.notifier
    {
        let org = state
            .db
            .get_settings()
            .await
            .map(|s| s.org_name)
            .unwrap_or_else(|_| "the clan".into());
        notifier.notify_general(format!("Welcome {display_name} to {org}!"));
    }

    Ok(member)
}

#[derive(Deserialize)]
pub struct ExpiringTrialsQuery {
    #[serde(default = "default_days")]
    pub days: i64,
}

fn default_days() -> i64 {
    7
}

/// GET /api/applications/expiring — list applications with trials expiring soon (officer+)
pub async fn expiring_trials(
    State(state): State<AppState>,
    _officer: OfficerUser,
    Query(query): Query<ExpiringTrialsQuery>,
) -> Result<Json<Vec<Application>>, (StatusCode, Json<ErrorResponse>)> {
    state
        .db
        .list_expiring_trials(query.days)
        .await
        .map(Json)
        .map_err(|e| internal_err(e, "list_expiring_trials"))
}
