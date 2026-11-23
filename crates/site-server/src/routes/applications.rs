use std::collections::HashMap;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};

use scuffed_auth::server::AuthUser;
use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{Application, ApplicationStatus, AuditAction, AuditTargetType, Member, OrgRole};
use scuffed_types::{
    ApplicationPreferenceError, MAX_APPLICATION_MESSAGE_CHARS,
    MAX_APPLICATION_PREFERENCE_ITEM_CHARS, MAX_APPLICATION_PREFERENCE_ITEMS,
    MAX_WITHDRAWN_APPLICATIONS, MIN_APPLICATION_MESSAGE_CHARS, WITHDRAWN_APPLICATION_WINDOW_DAYS,
    application_message_long_enough, application_message_within_max, check_application_preferences,
};

use crate::extractors::OfficerUser;
use crate::membership_policy::{
    applicant_may_self_withdraw, application_blocks_resubmit, application_resubmit_wait_minutes,
    application_status_deactivates_member, application_status_ensures_member,
    can_reactivate_via_application, is_valid_application_transition, role_on_application_accept,
    withdrawn_application_cap_reached, withdrawn_application_cap_wait,
    withdrawn_application_window_start,
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
        Json(ErrorResponse { error: msg.into() }),
    )
}

fn conflict(msg: &str) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::CONFLICT,
        Json(ErrorResponse { error: msg.into() }),
    )
}

fn too_many_requests(msg: &str) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::TOO_MANY_REQUESTS,
        Json(ErrorResponse { error: msg.into() }),
    )
}

/// The rolling withdrawn cap: `Some(reason)` for the 429 when the user has
/// withdrawn [`MAX_WITHDRAWN_APPLICATIONS`] applications in the last
/// [`WITHDRAWN_APPLICATION_WINDOW_DAYS`] days, `None` when a slot is free.
///
/// `again` is what the user was doing ("apply" or "withdraw"). The reason
/// says when the next slot opens: when the oldest of the newest
/// [`MAX_WITHDRAWN_APPLICATIONS`] in-window rows leaves the window. Rows
/// withdrawn longer ago do not count, and no row is rewritten or deleted.
async fn withdrawn_cap_refusal(
    state: &AppState,
    user_id: &str,
    again: &str,
) -> Result<Option<String>, scuffed_db::DbError> {
    let now = chrono::Utc::now();
    let recent = state
        .db
        .withdrawn_application_times_since(
            user_id,
            withdrawn_application_window_start(now),
            MAX_WITHDRAWN_APPLICATIONS,
        )
        .await?;
    if !withdrawn_application_cap_reached(recent.len() as u64) {
        return Ok(None);
    }
    // Newest first, so the last one is the next to leave the window.
    let wait = recent
        .last()
        .map(|slot| withdrawn_application_cap_wait(*slot, now));
    let limit = format!(
        "You have withdrawn {MAX_WITHDRAWN_APPLICATIONS} applications in the last \
         {WITHDRAWN_APPLICATION_WINDOW_DAYS} days"
    );
    Ok(Some(match wait {
        Some(wait) => format!("{limit}. You can {again} again in {wait}"),
        None => limit,
    }))
}

/// `Ok(true)` is open. `Ok(false)` is 403. A settings read error fails closed
/// with 500 so a new application is not stored.
fn application_recruitment_gate(
    recruitment_open: Result<bool, ()>,
) -> Result<(), (StatusCode, &'static str)> {
    match recruitment_open {
        Ok(true) => Ok(()),
        Ok(false) => Err((StatusCode::FORBIDDEN, "recruitment is currently closed")),
        Err(()) => Err((StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")),
    }
}

/// A missing message counts as empty. The rule (trim, then count chars, not
/// bytes) is shared with the apply page: see
/// [`scuffed_types::application_message_long_enough`].
fn application_message_is_long_enough(message: Option<&str>) -> bool {
    application_message_long_enough(message.unwrap_or(""))
}

/// The text is stored as sent, not trimmed, so the shared caps (which trim
/// before they count) would let whitespace padding through. This bounds what
/// is stored: the untrimmed text may not be longer than the cap either.
fn stored_chars_within(text: &str, max: usize) -> bool {
    text.chars().count() <= max
}

/// Upper bounds of a submit body: message length, then both preference
/// lists (count, then entry length). The 400 text names the field. Runs
/// after the minimum check and before any DB work.
fn check_application_caps(body: &SubmitApplicationRequest) -> Result<(), String> {
    let message = body.message.as_deref().unwrap_or("");
    if !application_message_within_max(message)
        || !stored_chars_within(message, MAX_APPLICATION_MESSAGE_CHARS)
    {
        return Err(format!(
            "Message must be at most {MAX_APPLICATION_MESSAGE_CHARS} characters"
        ));
    }
    for (plural, singular, items) in [
        ("games", "game", &body.preferred_games),
        ("roles", "role", &body.preferred_roles),
    ] {
        let check = check_application_preferences(items);
        if check == Err(ApplicationPreferenceError::TooMany) {
            return Err(format!(
                "Too many preferred {plural} (max {MAX_APPLICATION_PREFERENCE_ITEMS})"
            ));
        }
        if check.is_err()
            || items
                .iter()
                .any(|item| !stored_chars_within(item, MAX_APPLICATION_PREFERENCE_ITEM_CHARS))
        {
            return Err(format!(
                "Preferred {singular} is too long (max {MAX_APPLICATION_PREFERENCE_ITEM_CHARS} characters)"
            ));
        }
    }
    Ok(())
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
    // Input check first: no DB read or write for a body that cannot be stored.
    // `preferred_roles` may be empty.
    if !application_message_is_long_enough(body.message.as_deref()) {
        return Err(bad_request(&format!(
            "Message must be at least {MIN_APPLICATION_MESSAGE_CHARS} characters"
        )));
    }
    if let Err(msg) = check_application_caps(&body) {
        return Err(bad_request(&msg));
    }

    let recruitment = match state.db.get_settings().await {
        Ok(settings) => Ok(settings.recruitment_open),
        Err(e) => {
            tracing::error!(error = %e, "get_settings on submit");
            Err(())
        }
    };
    if let Err((status, msg)) = application_recruitment_gate(recruitment) {
        return Err((status, Json(ErrorResponse { error: msg.into() })));
    }

    // Banned accounts cannot re-enter through recruitment. 403 here so the
    // application is never stored. Accept/trial also refuses (409) below, so an
    // application that already exists cannot set `is_active` and undo the ban.
    // A deactivated member who is not banned can still apply.
    if let Some(m) = state
        .db
        .get_member_by_user(&user.id)
        .await
        .map_err(|e| internal_err(e, "get_member_by_user on submit"))?
    {
        if state
            .db
            .is_member_banned(&m.id)
            .await
            .map_err(|e| internal_err(e, "ban check on submit"))?
        {
            return Err((
                StatusCode::FORBIDDEN,
                Json(ErrorResponse {
                    error: "Banned members cannot apply".into(),
                }),
            ));
        }
        if m.is_active {
            return Err(conflict("Already an active org member"));
        }
    }

    // Open pipeline (pending/trial) blocks a second application
    let latest = state
        .db
        .get_application_by_user(&user.id)
        .await
        .map_err(|e| internal_err(e, "get_application_by_user on submit"))?;
    if let Some(ref existing) = latest
        && application_blocks_resubmit(existing.status)
    {
        return Err(conflict(&format!(
            "You already have an open application ({})",
            existing.status
        )));
    }

    // Per-user rolling withdrawn cap: apply and withdraw cannot be repeated
    // without end. Only rows withdrawn inside the window count, and rows
    // already over the cap are left as they are.
    if let Some(reason) = withdrawn_cap_refusal(&state, &user.id, "apply")
        .await
        .map_err(|e| internal_err(e, "withdrawn cap on submit"))?
    {
        return Err(too_many_requests(&reason));
    }

    // Per-user cooldown: the latest application is closed (withdrawn,
    // rejected or accepted), so wait from its last status write.
    if let Some(ref existing) = latest
        && let Some(minutes) =
            application_resubmit_wait_minutes(existing.updated_at, chrono::Utc::now())
    {
        let unit = if minutes == 1 { "minute" } else { "minutes" };
        return Err(too_many_requests(&format!(
            "You can apply again in {minutes} {unit}"
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

/// Application row enriched for admin display: raw ids resolved to names on read.
#[derive(Serialize)]
pub struct ApplicationListEntry {
    #[serde(flatten)]
    pub application: Application,
    /// Member display name when provisioned, else account username, else the raw user id.
    pub applicant_name: String,
    /// Game names in the same order as `preferred_games`; unknown ids pass through raw.
    pub preferred_game_names: Vec<String>,
}

/// GET /api/applications — list applications (officer+), cursor-paginated.
pub async fn list_applications(
    State(state): State<AppState>,
    _officer: OfficerUser,
    axum::extract::Query(pagination): axum::extract::Query<scuffed_types::api::PaginationParams>,
) -> Result<
    Json<scuffed_types::api::CursorResponse<ApplicationListEntry>>,
    (StatusCode, Json<ErrorResponse>),
> {
    let (limit, offset) = pagination.resolve();
    let items = state
        .db
        .list_applications_paginated(limit, offset)
        .await
        .map_err(|e| internal_err(e, "list_applications"))?;

    let members = state
        .db
        .list_members()
        .await
        .map_err(|e| internal_err(e, "list_members for application names"))?;
    let member_names: HashMap<&str, &str> = members
        .iter()
        .map(|m| (m.user_id.as_str(), m.display_name.as_str()))
        .collect();
    let games = state
        .db
        .list_games()
        .await
        .map_err(|e| internal_err(e, "list_games for application names"))?;
    let game_names: HashMap<&str, &str> = games
        .iter()
        .map(|g| (g.id.as_str(), g.name.as_str()))
        .collect();

    let mut entries = Vec::with_capacity(items.len());
    for app in items {
        let applicant_name = match member_names.get(app.user_id.as_str()) {
            Some(name) => (*name).to_string(),
            // Applicants without a member row (pending, never provisioned): use the
            // account username. A vanished user degrades to the raw id.
            None => state
                .db
                .get_user(&app.user_id)
                .await
                .map_err(|e| internal_err(e, "get_user for application name"))?
                .map(|u| u.username)
                .unwrap_or_else(|| app.user_id.clone()),
        };
        let preferred_game_names = app
            .preferred_games
            .iter()
            .map(|id| {
                game_names
                    .get(id.as_str())
                    .map(|n| (*n).to_string())
                    .unwrap_or_else(|| id.clone())
            })
            .collect();
        entries.push(ApplicationListEntry {
            application: app,
            applicant_name,
            preferred_game_names,
        });
    }

    Ok(Json(scuffed_types::api::CursorResponse::from_oversized(
        entries, limit, offset,
    )))
}

/// Applicant-facing application. Officer review fields stay on the admin list.
#[derive(Serialize)]
pub struct ApplicantApplication {
    pub id: String,
    pub user_id: String,
    pub status: ApplicationStatus,
    pub preferred_games: Vec<String>,
    pub preferred_roles: Vec<String>,
    pub message: Option<String>,
    pub trial_started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub trial_ends_at: Option<chrono::DateTime<chrono::Utc>>,
    pub mentor_id: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<Application> for ApplicantApplication {
    fn from(app: Application) -> Self {
        Self {
            id: app.id,
            user_id: app.user_id,
            status: app.status,
            preferred_games: app.preferred_games,
            preferred_roles: app.preferred_roles,
            message: app.message,
            trial_started_at: app.trial_started_at,
            trial_ends_at: app.trial_ends_at,
            mentor_id: app.mentor_id,
            created_at: app.created_at,
            updated_at: app.updated_at,
        }
    }
}

/// GET /api/applications/mine — own application status (any logged-in).
/// Omits `review_notes` and `reviewed_by`.
pub async fn my_application(
    State(state): State<AppState>,
    user: AuthUser<AppState>,
) -> Result<Json<Option<ApplicantApplication>>, (StatusCode, Json<ErrorResponse>)> {
    let app = state
        .db
        .get_application_by_user(&user.id)
        .await
        .map_err(|e| internal_err(e, "my_application"))?;
    Ok(Json(app.map(ApplicantApplication::from)))
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

    // A withdraw that would put the user over the rolling withdrawn cap is
    // refused, so a row over the cap is never added inside the window. The
    // open row is left as it is. An officer can still close it.
    if let Some(reason) = withdrawn_cap_refusal(&state, &user.id, "withdraw")
        .await
        .map_err(|e| internal_err(e, "withdrawn cap on withdraw"))?
    {
        return Err(too_many_requests(&reason));
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
        // Self-withdraw is not a review and never provisions a member.
        None,
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
        Some(officer.member.org_role),
        body.review_notes.as_deref(),
    )
    .await?;

    Ok(Json(app))
}

/// Shared transition path: the atomic CAS status write gates the side effects,
/// then audit.
///
/// `reviewer_role` is the reviewing officer's or admin's role, `None` for an
/// applicant self-withdraw. It decides who may reactivate an inactive member
/// on trial/accept.
async fn apply_application_transition(
    state: &AppState,
    existing: &Application,
    to: ApplicationStatus,
    actor_id: &str,
    reviewer_role: Option<OrgRole>,
    review_notes: Option<&str>,
) -> Result<Application, (StatusCode, Json<ErrorResponse>)> {
    // Provision membership *before* the CAS for trial/accepted so we never move an
    // application into a member-bearing state without a matching member row, and so a
    // provisioning failure aborts before the status changes. This branch is idempotent
    // under a lost CAS race — the member is left active, which is correct for whichever
    // accept won.
    if application_status_ensures_member(to) {
        ensure_member_for_application(
            state,
            &existing.user_id,
            existing.status,
            to,
            &existing.id,
            reviewer_role,
        )
        .await?;
    }

    // The CAS is the atomic gate: run it FIRST for reject/withdraw so a lost race
    // returns Conflict (409) having touched NO membership state. Deactivating + revoking
    // sessions only *after* the CAS commits means an Accept racing a self-Withdraw can
    // never leave an accepted member deactivated with sessions revoked (DR1-ACCT-001
    // lockout): on Conflict we short-circuit here before any side effect runs.
    let app = state
        .db
        .update_application_status(&existing.id, existing.status, to, actor_id, review_notes)
        .await
        .map_err(|e| match e {
            scuffed_db::DbError::Conflict(msg) => conflict(&msg),
            other => internal_err(other, "update_application_status"),
        })?;

    // Deactivate the failed trial recruit only after the reject/withdraw CAS committed.
    if application_status_deactivates_member(to)
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
                None,
                None,
                None,
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
    reviewer_role: Option<OrgRole>,
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
        // Do not clear a ban. Submit already 403s; this covers an application
        // that was stored before the ban (or written past the submit gate).
        if state
            .db
            .is_member_banned(&member.id)
            .await
            .map_err(|e| internal_err(e, "ban check on application transition"))?
        {
            return Err(conflict("An active ban blocks this application"));
        }
        // Reactivate if needed
        let mut current = member;
        if !current.is_active {
            // Reactivation keeps an officer or admin role. Trial writes no
            // role, so a member or recruit keeps theirs; accept is what applies
            // the application role to a member or recruit below. Officers
            // cannot set `is_active` on officers or admins anywhere else
            // (`can_set_is_active`), so only an admin may bring one back here.
            // 403 before any write: the member stays inactive and the
            // application keeps its status.
            if let Err(denial) = can_reactivate_via_application(reviewer_role, current.org_role) {
                return Err((
                    denial.status(),
                    Json(ErrorResponse {
                        error: denial.message().into(),
                    }),
                ));
            }
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
                    None,
                    None,
                    None,
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
            // Fail the request. Do not leave a trial/accepted app without a member.
            // A taken or invalid username-as-display-name is the same JSON the
            // profile editor returns, so the client can show "That name is taken".
            if let Some(response) = crate::routes::members::display_name_write_error(&e) {
                return response;
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use scuffed_db::OrgRole;

    #[test]
    fn application_gate_refuses_closed_and_unreadable_settings() {
        assert!(application_recruitment_gate(Ok(true)).is_ok());
        let closed = application_recruitment_gate(Ok(false)).unwrap_err();
        assert_eq!(closed.0, StatusCode::FORBIDDEN);
        assert_eq!(closed.1, "recruitment is currently closed");
        let unread = application_recruitment_gate(Err(())).unwrap_err();
        assert_eq!(unread.0, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(unread.1, "Internal server error");
    }

    use crate::extractors::OfficerUser;
    use crate::state::AppState;
    use crate::test_support::{must_err, must_ok, seed_user, test_state};

    async fn session_count(state: &AppState, user_id: &str) -> usize {
        let mut r = state
            .db
            .client
            .query("SELECT VALUE user_id FROM session WHERE user_id = $uid")
            .bind(("uid", user_id.to_string()))
            .await
            .expect("count sessions");
        let rows: Vec<String> = r.take(0).expect("take sessions");
        rows.len()
    }

    /// List enrichment: applicant_name prefers the member display name, falls back to
    /// the account username for never-provisioned applicants; game ids resolve to
    /// names with unknown ids passing through raw.
    #[tokio::test]
    async fn list_applications_enriches_applicant_and_game_names() {
        let state = test_state().await;

        let game = state
            .db
            .create_game("Overwatch", Some("OW"))
            .await
            .expect("create game");

        // Applicant WITHOUT a member row → username.
        seed_user(&state, "applicant1", "AppliedAlice").await;
        state
            .db
            .submit_application(
                "applicant1",
                vec![game.id.clone(), "ghostgame".into()],
                vec![],
                Some("hi"),
            )
            .await
            .expect("submit app1");

        // Applicant WITH a member row → member display name wins over username.
        seed_user(&state, "applicant2", "bob_login").await;
        state
            .db
            .create_member("applicant2", "BobDisplay", OrgRole::Recruit)
            .await
            .expect("provision bob");
        state
            .db
            .submit_application("applicant2", vec![], vec![], None)
            .await
            .expect("submit app2");

        seed_user(&state, "officeruser", "Off").await;
        let officer_member = state
            .db
            .create_member("officeruser", "Off", OrgRole::Officer)
            .await
            .expect("officer member");
        let officer_user = state
            .db
            .get_user("officeruser")
            .await
            .expect("get officer user")
            .expect("officer user exists");

        let Json(resp) = list_applications(
            State(state.clone()),
            OfficerUser {
                user: officer_user,
                member: officer_member,
            },
            axum::extract::Query(scuffed_types::api::PaginationParams {
                cursor: None,
                limit: 25,
            }),
        )
        .await
        .unwrap_or_else(|(status, _)| panic!("list applications failed: {status}"));

        let alice = resp
            .data
            .iter()
            .find(|e| e.application.user_id == "applicant1")
            .expect("alice row");
        assert_eq!(alice.applicant_name, "AppliedAlice");
        assert_eq!(
            alice.preferred_game_names,
            vec!["Overwatch".to_string(), "ghostgame".to_string()]
        );

        let bob = resp
            .data
            .iter()
            .find(|e| e.application.user_id == "applicant2")
            .expect("bob row");
        assert_eq!(bob.applicant_name, "BobDisplay");
    }

    /// DR1-ACCT-001 regression: an Accept racing a self-Withdraw must not leave the
    /// just-accepted member deactivated with sessions revoked.
    ///
    /// We reproduce the exact interleave deterministically: the self-withdraw handler
    /// captured a *stale* `Pending` snapshot of the application, but by the time its
    /// transition runs the concurrent direct-accept has already provisioned the member
    /// (active recruit) AND won the CAS (row is now `Accepted`). The withdraw transition
    /// must Conflict on the CAS and touch NO membership state.
    ///
    /// Under the old side-effect-before-CAS ordering this test fails: the deactivate +
    /// session-revoke ran before the (then-conflicting) CAS, locking the member out.
    #[tokio::test]
    async fn cas_conflict_in_deactivate_branch_does_not_lock_out_member() {
        let state = test_state().await;
        let user_id = "raceuser";

        // User row (needed for member/application foreign keys).
        state
            .db
            .client
            .query(
                r#"CREATE user:raceuser SET
                    provider = 'discord',
                    username = 'Racer',
                    avatar_url = NONE,
                    provider_id = 'raceuser-pid',
                    provider_id_hash = 'raceuser-pidh',
                    provider_id_encrypted = NONE,
                    created_at = time::now()"#,
            )
            .await
            .expect("seed user");

        // The member the concurrent direct-accept just provisioned: active recruit.
        let member = state
            .db
            .create_member(user_id, "Racer", OrgRole::Recruit)
            .await
            .expect("provision recruit");
        assert!(member.is_active);

        // A live session for that member.
        state
            .db
            .client
            .query(
                r#"CREATE session:sess_raceuser SET
                    user_id = 'raceuser',
                    token = 'live-token-hash',
                    expires_at = time::now() + 365d,
                    created_at = time::now()"#,
            )
            .await
            .expect("seed session");
        assert_eq!(session_count(&state, user_id).await, 1);

        // The applicant's own (now stale) view of the application: still Pending.
        let stale = state
            .db
            .submit_application(user_id, vec![], vec![], None)
            .await
            .expect("submit application");
        assert_eq!(stale.status, ApplicationStatus::Pending);

        // The concurrent direct-accept WON the CAS: advance the row Pending -> Accepted.
        state
            .db
            .update_application_status(
                &stale.id,
                ApplicationStatus::Pending,
                ApplicationStatus::Accepted,
                "officer",
                None,
            )
            .await
            .expect("concurrent accept wins CAS");

        // Self-withdraw fires the transition with the stale Pending snapshot.
        let result = apply_application_transition(
            &state,
            &stale,
            ApplicationStatus::Withdrawn,
            member.id.as_str(),
            None,
            Some("Self-withdrawn by applicant"),
        )
        .await;

        // CAS-first: expected Pending vs actual Accepted -> Conflict (409).
        let (status, _) = result.expect_err("stale withdraw must conflict");
        assert_eq!(status, StatusCode::CONFLICT);

        // No collateral lockout: the accepted member stays active with its session intact.
        let after = state
            .db
            .get_member_by_user(user_id)
            .await
            .expect("reload member")
            .expect("member still exists");
        assert!(
            after.is_active,
            "CAS conflict must not deactivate the accepted member"
        );
        assert_eq!(
            session_count(&state, user_id).await,
            1,
            "CAS conflict must not revoke the member's sessions"
        );
        // The application row is unchanged by the losing transition: still Accepted.
        let app_now = state
            .db
            .get_application_by_user(user_id)
            .await
            .expect("reload application")
            .expect("application exists");
        assert_eq!(app_now.status, ApplicationStatus::Accepted);
    }

    /// A normal reject of an active trial recruit still deactivates it and revokes
    /// sessions — the side effect must still run once the CAS succeeds.
    #[tokio::test]
    async fn normal_reject_still_deactivates_recruit_after_cas() {
        let state = test_state().await;
        let user_id = "rejuser";

        state
            .db
            .client
            .query(
                r#"CREATE user:rejuser SET
                    provider = 'discord',
                    username = 'Rej',
                    avatar_url = NONE,
                    provider_id = 'rejuser-pid',
                    provider_id_hash = 'rejuser-pidh',
                    provider_id_encrypted = NONE,
                    created_at = time::now()"#,
            )
            .await
            .expect("seed user");

        let member = state
            .db
            .create_member(user_id, "Rej", OrgRole::Recruit)
            .await
            .expect("provision recruit");
        assert!(member.is_active);

        state
            .db
            .client
            .query(
                r#"CREATE session:sess_rejuser SET
                    user_id = 'rejuser',
                    token = 'rej-token-hash',
                    expires_at = time::now() + 365d,
                    created_at = time::now()"#,
            )
            .await
            .expect("seed session");

        // Application sits in Trial; fresh snapshot matches the DB row (no race).
        let submitted = state
            .db
            .submit_application(user_id, vec![], vec![], None)
            .await
            .expect("submit application");
        let trial = state
            .db
            .update_application_status(
                &submitted.id,
                ApplicationStatus::Pending,
                ApplicationStatus::Trial,
                "officer",
                None,
            )
            .await
            .expect("move to trial");

        let result = apply_application_transition(
            &state,
            &trial,
            ApplicationStatus::Rejected,
            "officer",
            Some(OrgRole::Officer),
            Some("not a fit"),
        )
        .await;
        assert!(result.is_ok(), "normal reject should succeed");

        let after = state
            .db
            .get_member_by_user(user_id)
            .await
            .expect("reload member")
            .expect("member exists");
        assert!(
            !after.is_active,
            "reject should deactivate the trial recruit"
        );
        assert_eq!(
            session_count(&state, user_id).await,
            0,
            "reject should revoke the recruit's sessions"
        );
    }

    const VALID_MESSAGE: &str = "I would like to join the crew";

    fn auth_user(user: scuffed_auth::User) -> AuthUser<AppState> {
        AuthUser::from_user(user)
    }

    async fn officer(state: &AppState) -> OfficerUser {
        seed_user(state, "officeruser", "Off").await;
        let member = state
            .db
            .create_member("officeruser", "Off", OrgRole::Officer)
            .await
            .expect("officer");
        let user = state
            .db
            .get_user("officeruser")
            .await
            .expect("get user")
            .expect("user");
        OfficerUser { user, member }
    }

    /// Submit is 403 for a banned member, so accept never gets a new application.
    /// A deactivated member who is not banned can still apply and be reactivated.
    #[tokio::test]
    async fn banned_member_cannot_submit_application() {
        let state = test_state().await;
        seed_user(&state, "banneduser", "Banned").await;
        let member = state
            .db
            .create_member("banneduser", "Banned", OrgRole::Member)
            .await
            .expect("member");
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
                None,
                None,
                None,
            )
            .await
            .expect("deactivate");
        state
            .db
            .create_moderation_action(
                &member.id,
                scuffed_db::ModerationActionType::Ban,
                "no return",
                "officer",
                None,
            )
            .await
            .expect("ban");
        let user = state
            .db
            .get_user("banneduser")
            .await
            .expect("get user")
            .expect("user");

        let (status, body) = must_err(
            submit_application(
                State(state.clone()),
                auth_user(user),
                Json(SubmitApplicationRequest {
                    preferred_games: vec!["Overwatch".into()],
                    preferred_roles: vec!["flex".into()],
                    message: Some(VALID_MESSAGE.into()),
                }),
            )
            .await,
            "banned submit must fail",
        );
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body.error, "Banned members cannot apply");
        assert!(
            state
                .db
                .get_application_by_user("banneduser")
                .await
                .expect("lookup")
                .is_none(),
            "a banned submit must not store an application"
        );
    }

    /// An application that already exists must not clear the ban on accept.
    #[tokio::test]
    async fn accepting_application_does_not_clear_a_ban() {
        let state = test_state().await;
        seed_user(&state, "banneduser", "Banned").await;
        let member = state
            .db
            .create_member("banneduser", "Banned", OrgRole::Member)
            .await
            .expect("member");
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
                None,
                None,
                None,
            )
            .await
            .expect("deactivate");
        let app = state
            .db
            .submit_application("banneduser", vec![], vec![], None)
            .await
            .expect("stored before the ban");
        state
            .db
            .create_moderation_action(
                &member.id,
                scuffed_db::ModerationActionType::Ban,
                "no return",
                "officer",
                None,
            )
            .await
            .expect("ban");

        let (status, body) = must_err(
            update_application(
                State(state.clone()),
                officer(&state).await,
                Path(app.id.clone()),
                Json(UpdateApplicationRequest {
                    status: ApplicationStatus::Accepted,
                    review_notes: None,
                }),
            )
            .await,
            "accept must refuse",
        );
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body.error, "An active ban blocks this application");

        let after = state
            .db
            .get_member_by_user("banneduser")
            .await
            .expect("reload")
            .expect("member");
        assert!(
            !after.is_active,
            "accept must not reactivate a banned member"
        );
        let stored = state
            .db
            .get_application(&app.id)
            .await
            .expect("reload app")
            .expect("app");
        assert_eq!(stored.status, ApplicationStatus::Pending);

        let (status, body) = must_err(
            crate::routes::public::public_member_profile(State(state), Path(member.id.clone()))
                .await,
            "public profile",
        );
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body.error, "Member not found");
    }

    #[tokio::test]
    async fn deactivated_unbanned_member_can_reapply_and_be_accepted() {
        let state = test_state().await;
        seed_user(&state, "returnuser", "Return").await;
        let member = state
            .db
            .create_member("returnuser", "Return", OrgRole::Member)
            .await
            .expect("member");
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
                None,
                None,
                None,
            )
            .await
            .expect("deactivate");
        let user = state
            .db
            .get_user("returnuser")
            .await
            .expect("get user")
            .expect("user");

        let (status, Json(app)) = must_ok(
            submit_application(
                State(state.clone()),
                auth_user(user),
                Json(SubmitApplicationRequest {
                    preferred_games: vec![],
                    preferred_roles: vec![],
                    message: Some(VALID_MESSAGE.into()),
                }),
            )
            .await,
            "unbanned reapply",
        );
        assert_eq!(status, StatusCode::CREATED);

        let Json(updated) = must_ok(
            update_application(
                State(state.clone()),
                officer(&state).await,
                Path(app.id),
                Json(UpdateApplicationRequest {
                    status: ApplicationStatus::Accepted,
                    review_notes: None,
                }),
            )
            .await,
            "accept",
        );
        assert_eq!(updated.status, ApplicationStatus::Accepted);
        let after = state
            .db
            .get_member_by_user("returnuser")
            .await
            .expect("reload")
            .expect("member");
        assert!(
            after.is_active,
            "an unbanned applicant is reactivated on accept"
        );
    }

    #[test]
    fn message_length_counts_trimmed_unicode_chars() {
        assert!(!application_message_is_long_enough(None));
        assert!(!application_message_is_long_enough(Some("")));
        assert!(!application_message_is_long_enough(Some("   123456789   ")));
        assert!(application_message_is_long_enough(Some("  1234567890  ")));
        // 10 chars, 20 bytes: passes on chars.
        assert!(application_message_is_long_enough(Some("æøåæøåæøåæ")));
        // 9 chars, 36 bytes: bytes are not the measure.
        assert!(!application_message_is_long_enough(Some(
            "🎮🎮🎮🎮🎮🎮🎮🎮🎮"
        )));
    }

    #[test]
    fn message_length_trims_unicode_whitespace_padding() {
        // U+3000 and U+00A0 are Unicode White_Space, so `trim` removes them.
        // Padding to 10+ chars with them does not count.
        let ideographic = "\u{3000}123456789\u{3000}";
        assert!(ideographic.chars().count() >= 10);
        assert!(!application_message_is_long_enough(Some(ideographic)));
        let no_break = "\u{00A0}123456789\u{00A0}";
        assert!(no_break.chars().count() >= 10);
        assert!(!application_message_is_long_enough(Some(no_break)));
        // Exactly 10 chars, one of them the padding.
        assert!(!application_message_is_long_enough(Some(
            "\u{3000}123456789"
        )));
        assert!(!application_message_is_long_enough(Some(
            "123456789\u{00A0}"
        )));
    }

    #[test]
    fn message_length_counts_combining_marks_and_skin_tone_modifiers_as_chars() {
        // 5 graphemes, 10 chars: base letter + U+0301 combining acute accent.
        let combining = "e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}";
        assert_eq!(combining.chars().count(), 10);
        assert!(application_message_is_long_enough(Some(combining)));
        // 5 graphemes, 10 chars: thumbs up + U+1F3FD skin-tone modifier.
        let skin_tone = "\u{1F44D}\u{1F3FD}".repeat(5);
        assert_eq!(skin_tone.chars().count(), 10);
        assert!(application_message_is_long_enough(Some(&skin_tone)));
    }

    /// Submit as a fresh non-member account with the given roles and message.
    async fn submit_as(
        state: &AppState,
        username: &str,
        preferred_roles: Vec<String>,
        message: Option<&str>,
    ) -> Result<(StatusCode, Json<Application>), (StatusCode, Json<ErrorResponse>)> {
        seed_user(state, username, username).await;
        let user = state
            .db
            .get_user(username)
            .await
            .expect("get user")
            .expect("user");
        submit_application(
            State(state.clone()),
            auth_user(user),
            Json(SubmitApplicationRequest {
                preferred_games: vec![],
                preferred_roles,
                message: message.map(str::to_string),
            }),
        )
        .await
    }

    async fn assert_short_message_refused(state: &AppState, username: &str, message: Option<&str>) {
        let (status, body) = must_err(
            submit_as(state, username, vec!["tank".into()], message).await,
            "short message must fail",
        );
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            body.error,
            format!("Message must be at least {MIN_APPLICATION_MESSAGE_CHARS} characters")
        );
        assert!(
            state
                .db
                .get_application_by_user(username)
                .await
                .expect("lookup")
                .is_none(),
            "a refused submit must not store an application"
        );
    }

    #[tokio::test]
    async fn submit_refuses_message_under_ten_chars_after_trim() {
        let state = test_state().await;
        assert_short_message_refused(&state, "nine", Some("  123456789\n\t ")).await;
    }

    #[tokio::test]
    async fn submit_refuses_missing_message() {
        let state = test_state().await;
        assert_short_message_refused(&state, "nomsg", None).await;
        assert_short_message_refused(&state, "blankmsg", Some("          ")).await;
    }

    #[tokio::test]
    async fn submit_refuses_nine_multibyte_chars() {
        let state = test_state().await;
        assert_short_message_refused(&state, "ninemb", Some("æøåæøåæøå")).await;
        assert_short_message_refused(&state, "nineemoji", Some("🎮🎮🎮🎮🎮🎮🎮🎮🎮")).await;
    }

    #[tokio::test]
    async fn submit_refuses_unicode_space_padded_message() {
        let state = test_state().await;
        assert_short_message_refused(&state, "ideopad", Some("\u{3000}123456789\u{3000}")).await;
        assert_short_message_refused(&state, "nbsppad", Some("\u{00A0}123456789\u{00A0}")).await;
    }

    #[tokio::test]
    async fn submit_accepts_combining_marks_and_skin_tone_modifiers() {
        let state = test_state().await;
        let (status, _) = must_ok(
            submit_as(
                &state,
                "combining",
                vec![],
                Some("e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}"),
            )
            .await,
            "10 chars with combining marks",
        );
        assert_eq!(status, StatusCode::CREATED);
        let skin_tone = "\u{1F44D}\u{1F3FD}".repeat(5);
        let (status, _) = must_ok(
            submit_as(&state, "skintone", vec![], Some(&skin_tone)).await,
            "10 chars with skin-tone modifiers",
        );
        assert_eq!(status, StatusCode::CREATED);
    }

    #[tokio::test]
    async fn submit_accepts_padded_ten_char_message_and_stores_it_as_sent() {
        let state = test_state().await;
        let (status, Json(app)) = must_ok(
            submit_as(
                &state,
                "padded",
                vec!["tank".into()],
                Some("   1234567890   "),
            )
            .await,
            "padded 10 chars",
        );
        assert_eq!(status, StatusCode::CREATED);
        // Storage is unchanged: the message is kept as sent, not trimmed.
        assert_eq!(app.message.as_deref(), Some("   1234567890   "));
    }

    #[tokio::test]
    async fn submit_accepts_ten_multibyte_chars() {
        let state = test_state().await;
        let msg = "æøåæøåæøåæ";
        assert!(msg.len() > 10);
        let (status, _) = must_ok(
            submit_as(&state, "tenmb", vec![], Some(msg)).await,
            "10 multibyte chars",
        );
        assert_eq!(status, StatusCode::CREATED);
    }

    #[tokio::test]
    async fn submit_accepts_empty_preferred_roles_with_valid_message() {
        let state = test_state().await;
        let (status, Json(app)) = must_ok(
            submit_as(&state, "noroles", vec![], Some(VALID_MESSAGE)).await,
            "empty roles",
        );
        assert_eq!(status, StatusCode::CREATED);
        assert!(app.preferred_roles.is_empty());
    }

    /// Submit as an account that already exists.
    async fn submit_existing(
        state: &AppState,
        user_id: &str,
        preferred_games: Vec<String>,
        preferred_roles: Vec<String>,
        message: &str,
    ) -> Result<(StatusCode, Json<Application>), (StatusCode, Json<ErrorResponse>)> {
        let user = state
            .db
            .get_user(user_id)
            .await
            .expect("get user")
            .expect("user");
        submit_application(
            State(state.clone()),
            auth_user(user),
            Json(SubmitApplicationRequest {
                preferred_games,
                preferred_roles,
                message: Some(message.to_string()),
            }),
        )
        .await
    }

    /// Seed a fresh account, then submit with the given lists and message.
    async fn submit_new(
        state: &AppState,
        user_id: &str,
        preferred_games: Vec<String>,
        preferred_roles: Vec<String>,
        message: &str,
    ) -> Result<(StatusCode, Json<Application>), (StatusCode, Json<ErrorResponse>)> {
        seed_user(state, user_id, user_id).await;
        submit_existing(state, user_id, preferred_games, preferred_roles, message).await
    }

    async fn withdraw_existing(
        state: &AppState,
        user_id: &str,
    ) -> Result<Json<Application>, (StatusCode, Json<ErrorResponse>)> {
        let user = state
            .db
            .get_user(user_id)
            .await
            .expect("get user")
            .expect("user");
        withdraw_my_application(State(state.clone()), auth_user(user)).await
    }

    async fn assert_refused_400(
        state: &AppState,
        user_id: &str,
        result: Result<(StatusCode, Json<Application>), (StatusCode, Json<ErrorResponse>)>,
        expected: &str,
    ) {
        let (status, body) = must_err(result, "over the cap must fail");
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body.error, expected);
        assert!(
            state
                .db
                .get_application_by_user(user_id)
                .await
                .expect("lookup")
                .is_none(),
            "a refused submit must not store an application"
        );
    }

    #[tokio::test]
    async fn submit_accepts_a_four_thousand_char_message() {
        let state = test_state().await;
        let exact = "a".repeat(4000);
        let (status, Json(app)) = must_ok(
            submit_new(&state, "maxmsg", vec![], vec![], &exact).await,
            "4000 chars",
        );
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(app.message.as_deref(), Some(exact.as_str()));
        // 4000 chars, 16000 bytes: chars are the measure.
        let (status, _) = must_ok(
            submit_new(&state, "maxemoji", vec![], vec![], &"🎮".repeat(4000)).await,
            "4000 multibyte chars",
        );
        assert_eq!(status, StatusCode::CREATED);
    }

    #[tokio::test]
    async fn submit_refuses_a_message_over_four_thousand_chars() {
        let state = test_state().await;
        let expected = "Message must be at most 4000 characters";
        let over = "a".repeat(4001);
        let result = submit_new(&state, "overmsg", vec![], vec![], &over).await;
        assert_refused_400(&state, "overmsg", result, expected).await;
        let result = submit_new(&state, "overmb", vec![], vec![], &"æ".repeat(4001)).await;
        assert_refused_400(&state, "overmb", result, expected).await;
        // Trimmed before the count, like the minimum: still over.
        let padded = format!("  {over}  ");
        let result = submit_new(&state, "overpad", vec![], vec![], &padded).await;
        assert_refused_400(&state, "overpad", result, expected).await;
    }

    /// The message is stored as sent, so whitespace padding may not carry a
    /// short message past the cap.
    #[tokio::test]
    async fn submit_refuses_whitespace_padding_past_the_message_cap() {
        let state = test_state().await;
        let padded = format!("{}{}", VALID_MESSAGE, " ".repeat(4000));
        assert!(application_message_within_max(&padded));
        let result = submit_new(&state, "padmsg", vec![], vec![], &padded).await;
        assert_refused_400(
            &state,
            "padmsg",
            result,
            "Message must be at most 4000 characters",
        )
        .await;
        // Padding that still fits is stored as sent.
        let fits = format!("{}{}", "a".repeat(3990), " ".repeat(10));
        let (status, Json(app)) = must_ok(
            submit_new(&state, "padfits", vec![], vec![], &fits).await,
            "padding inside the cap",
        );
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(app.message.as_deref(), Some(fits.as_str()));
    }

    #[tokio::test]
    async fn submit_accepts_ten_games_and_ten_roles() {
        let state = test_state().await;
        let ten: Vec<String> = (0..10).map(|i| format!("item{i}")).collect();
        let (status, Json(app)) = must_ok(
            submit_new(&state, "tenitems", ten.clone(), ten.clone(), VALID_MESSAGE).await,
            "10 games and 10 roles",
        );
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(app.preferred_games, ten);
        assert_eq!(app.preferred_roles, ten);
    }

    #[tokio::test]
    async fn submit_refuses_eleven_games_or_roles() {
        let state = test_state().await;
        let eleven: Vec<String> = (0..11).map(|i| format!("item{i}")).collect();
        let result = submit_new(&state, "manygames", eleven.clone(), vec![], VALID_MESSAGE).await;
        assert_refused_400(
            &state,
            "manygames",
            result,
            "Too many preferred games (max 10)",
        )
        .await;
        let result = submit_new(&state, "manyroles", vec![], eleven, VALID_MESSAGE).await;
        assert_refused_400(
            &state,
            "manyroles",
            result,
            "Too many preferred roles (max 10)",
        )
        .await;
    }

    #[tokio::test]
    async fn submit_accepts_sixty_four_char_items() {
        let state = test_state().await;
        let exact = "a".repeat(64);
        // 64 chars, 128 bytes.
        let multibyte = "æ".repeat(64);
        let (status, _) = must_ok(
            submit_new(
                &state,
                "longitems",
                vec![exact.clone(), multibyte.clone()],
                vec![exact, multibyte],
                VALID_MESSAGE,
            )
            .await,
            "64 char items",
        );
        assert_eq!(status, StatusCode::CREATED);
    }

    #[tokio::test]
    async fn submit_refuses_a_sixty_five_char_item() {
        let state = test_state().await;
        let over = "a".repeat(65);
        let game_text = "Preferred game is too long (max 64 characters)";
        let role_text = "Preferred role is too long (max 64 characters)";
        let result = submit_new(
            &state,
            "longgame",
            vec!["ow2".into(), over.clone()],
            vec![],
            VALID_MESSAGE,
        )
        .await;
        assert_refused_400(&state, "longgame", result, game_text).await;
        let result = submit_new(&state, "longrole", vec![], vec![over], VALID_MESSAGE).await;
        assert_refused_400(&state, "longrole", result, role_text).await;
        // Stored as sent, so padding counts toward what is stored.
        let padded = format!("ow2{}", " ".repeat(64));
        let result = submit_new(&state, "padgame", vec![padded], vec![], VALID_MESSAGE).await;
        assert_refused_400(&state, "padgame", result, game_text).await;
    }

    /// The minimum is still checked first, with its own text.
    #[tokio::test]
    async fn short_message_is_refused_before_the_list_caps() {
        let state = test_state().await;
        let eleven: Vec<String> = (0..11).map(|i| format!("item{i}")).collect();
        let result = submit_new(&state, "shortfirst", eleven, vec![], "short").await;
        assert_refused_400(
            &state,
            "shortfirst",
            result,
            "Message must be at least 10 characters",
        )
        .await;
    }

    /// Move every `updated_at` of the user's applications into the past.
    /// `created_at` is left alone: it orders "latest application".
    async fn age_applications(state: &AppState, user_id: &str, minutes: u32) {
        // `minutes` is a test literal, so it is safe in the query text
        // (site-server has no direct surrealdb dep for a datetime bind).
        state
            .db
            .client
            .query(format!(
                "UPDATE application SET updated_at = time::now() - {minutes}m WHERE user_id = $uid"
            ))
            .bind(("uid", user_id.to_string()))
            .await
            .expect("age applications");
    }

    /// Store `count` withdrawn applications for the user, straight in the DB.
    async fn seed_withdrawn(state: &AppState, user_id: &str, count: usize) {
        for _ in 0..count {
            let app = state
                .db
                .submit_application(user_id, vec![], vec![], Some(VALID_MESSAGE))
                .await
                .expect("seed application");
            state
                .db
                .update_application_status(
                    &app.id,
                    ApplicationStatus::Pending,
                    ApplicationStatus::Withdrawn,
                    user_id,
                    None,
                )
                .await
                .expect("seed withdraw");
        }
    }

    async fn application_count(state: &AppState, user_id: &str) -> usize {
        let mut r = state
            .db
            .client
            .query("SELECT VALUE status FROM application WHERE user_id = $uid")
            .bind(("uid", user_id.to_string()))
            .await
            .expect("count applications");
        let rows: Vec<String> = r.take(0).expect("take applications");
        rows.len()
    }

    #[tokio::test]
    async fn resubmit_right_after_withdraw_is_429_until_the_cooldown_is_over() {
        let state = test_state().await;
        let (status, _) = must_ok(
            submit_new(&state, "cooldown", vec![], vec![], VALID_MESSAGE).await,
            "first application",
        );
        assert_eq!(status, StatusCode::CREATED);
        let Json(withdrawn) = must_ok(withdraw_existing(&state, "cooldown").await, "withdraw");
        assert_eq!(withdrawn.status, ApplicationStatus::Withdrawn);

        let (status, body) = must_err(
            submit_existing(&state, "cooldown", vec![], vec![], VALID_MESSAGE).await,
            "resubmit inside the cooldown",
        );
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(body.error, "You can apply again in 60 minutes");
        assert_eq!(application_count(&state, "cooldown").await, 1);

        // 59 minutes on: still waiting, and the text counts down.
        age_applications(&state, "cooldown", 59).await;
        let (status, body) = must_err(
            submit_existing(&state, "cooldown", vec![], vec![], VALID_MESSAGE).await,
            "resubmit at 59 minutes",
        );
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(body.error, "You can apply again in 1 minute");
        assert_eq!(application_count(&state, "cooldown").await, 1);

        // Past the hour: accepted.
        age_applications(&state, "cooldown", 61).await;
        let (status, Json(app)) = must_ok(
            submit_existing(&state, "cooldown", vec![], vec![], VALID_MESSAGE).await,
            "resubmit after the cooldown",
        );
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(app.status, ApplicationStatus::Pending);
        assert_eq!(application_count(&state, "cooldown").await, 2);
    }

    /// A rejected application does not block a resubmit either, so the same
    /// cooldown runs from the reject.
    #[tokio::test]
    async fn resubmit_right_after_reject_waits_for_the_cooldown() {
        let state = test_state().await;
        let (_, Json(app)) = must_ok(
            submit_new(&state, "rejected", vec![], vec![], VALID_MESSAGE).await,
            "first application",
        );
        let Json(rejected) = must_ok(
            update_application(
                State(state.clone()),
                officer(&state).await,
                Path(app.id),
                Json(UpdateApplicationRequest {
                    status: ApplicationStatus::Rejected,
                    review_notes: None,
                }),
            )
            .await,
            "reject",
        );
        assert_eq!(rejected.status, ApplicationStatus::Rejected);
        let (status, body) = must_err(
            submit_existing(&state, "rejected", vec![], vec![], VALID_MESSAGE).await,
            "resubmit inside the cooldown",
        );
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(body.error, "You can apply again in 60 minutes");

        age_applications(&state, "rejected", 61).await;
        let (status, _) = must_ok(
            submit_existing(&state, "rejected", vec![], vec![], VALID_MESSAGE).await,
            "resubmit after the cooldown",
        );
        assert_eq!(status, StatusCode::CREATED);
    }

    /// An open application is still the 409 it was, not the cooldown 429.
    #[tokio::test]
    async fn open_application_is_still_409_inside_the_cooldown() {
        let state = test_state().await;
        let (status, _) = must_ok(
            submit_new(&state, "openapp", vec![], vec![], VALID_MESSAGE).await,
            "first application",
        );
        assert_eq!(status, StatusCode::CREATED);
        let (status, body) = must_err(
            submit_existing(&state, "openapp", vec![], vec![], VALID_MESSAGE).await,
            "second application",
        );
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body.error, "You already have an open application (pending)");
    }

    /// Move every `updated_at` of the user's applications further into the
    /// past by `by` (a SurrealQL duration such as `61m` or `31d`), keeping
    /// the distance between rows: the same as that much time passing.
    async fn let_time_pass(state: &AppState, user_id: &str, by: &str) {
        // `by` is a test literal, so it is safe in the query text.
        state
            .db
            .client
            .query(format!(
                "UPDATE application SET updated_at = updated_at - {by} WHERE user_id = $uid"
            ))
            .bind(("uid", user_id.to_string()))
            .await
            .expect("let time pass");
    }

    /// Withdrawn rows of the user inside the rolling window, uncapped.
    async fn withdrawn_in_window(state: &AppState, user_id: &str) -> usize {
        state
            .db
            .withdrawn_application_times_since(
                user_id,
                withdrawn_application_window_start(chrono::Utc::now()),
                1000,
            )
            .await
            .expect("withdrawn in window")
            .len()
    }

    const CAP_REASON: &str = "You have withdrawn 5 applications in the last 30 days";

    #[tokio::test]
    async fn five_withdrawn_applications_in_the_window_refuse_a_new_submit() {
        let state = test_state().await;
        seed_user(&state, "capped", "capped").await;
        seed_withdrawn(&state, "capped", 5).await;
        // Past the cooldown, so the cap is what refuses.
        let_time_pass(&state, "capped", "2h").await;

        let (status, body) = must_err(
            submit_existing(&state, "capped", vec![], vec![], VALID_MESSAGE).await,
            "submit at the withdrawn cap",
        );
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            body.error,
            format!("{CAP_REASON}. You can apply again in 30 days")
        );
        // Nothing added, nothing deleted or rewritten.
        assert_eq!(application_count(&state, "capped").await, 5);
        assert_eq!(withdrawn_in_window(&state, "capped").await, 5);

        // 29 days on, all five are still inside the window.
        let_time_pass(&state, "capped", "29d").await;
        let (status, body) = must_err(
            submit_existing(&state, "capped", vec![], vec![], VALID_MESSAGE).await,
            "submit on day 29",
        );
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            body.error,
            format!("{CAP_REASON}. You can apply again in 22 hours")
        );
        assert_eq!(application_count(&state, "capped").await, 5);
    }

    /// Four withdrawn rows in the window and older ones outside it: the
    /// older rows do not count, so the user can apply after the cooldown.
    #[tokio::test]
    async fn four_withdrawn_in_the_window_plus_older_ones_can_still_apply() {
        let state = test_state().await;
        seed_user(&state, "fourwd", "fourwd").await;
        // Two withdrawn more than 30 days ago.
        seed_withdrawn(&state, "fourwd", 2).await;
        let_time_pass(&state, "fourwd", "31d").await;
        // Four withdrawn just now.
        seed_withdrawn(&state, "fourwd", 4).await;
        assert_eq!(application_count(&state, "fourwd").await, 6);
        assert_eq!(withdrawn_in_window(&state, "fourwd").await, 4);

        // Under the cap, but the last withdraw was just now: cooldown.
        let (status, body) = must_err(
            submit_existing(&state, "fourwd", vec![], vec![], VALID_MESSAGE).await,
            "submit inside the cooldown",
        );
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(body.error, "You can apply again in 60 minutes");

        let_time_pass(&state, "fourwd", "61m").await;
        let (status, _) = must_ok(
            submit_existing(&state, "fourwd", vec![], vec![], VALID_MESSAGE).await,
            "submit under the cap",
        );
        assert_eq!(status, StatusCode::CREATED);

        // The fifth in-window withdraw is allowed and reaches the cap.
        let Json(app) = must_ok(withdraw_existing(&state, "fourwd").await, "fifth withdraw");
        assert_eq!(app.status, ApplicationStatus::Withdrawn);
        assert_eq!(withdrawn_in_window(&state, "fourwd").await, 5);
        let_time_pass(&state, "fourwd", "61m").await;
        let (status, body) = must_err(
            submit_existing(&state, "fourwd", vec![], vec![], VALID_MESSAGE).await,
            "submit at the cap",
        );
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        // The slot opens when the oldest of the five leaves the window.
        assert_eq!(
            body.error,
            format!("{CAP_REASON}. You can apply again in 30 days")
        );
        // The old rows are still stored.
        assert_eq!(application_count(&state, "fourwd").await, 7);
    }

    /// An open application next to five rows withdrawn in the window (stored
    /// before the cap, or withdrawn by an officer) cannot be self-withdrawn
    /// into a sixth.
    #[tokio::test]
    async fn a_sixth_withdraw_in_the_window_is_refused() {
        let state = test_state().await;
        seed_user(&state, "sixth", "sixth").await;
        seed_withdrawn(&state, "sixth", 5).await;
        let open = state
            .db
            .submit_application("sixth", vec![], vec![], Some(VALID_MESSAGE))
            .await
            .expect("open application");

        let (status, body) = must_err(
            withdraw_existing(&state, "sixth").await,
            "withdraw at the cap",
        );
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            body.error,
            format!("{CAP_REASON}. You can withdraw again in 30 days")
        );
        let stored = state
            .db
            .get_application(&open.id)
            .await
            .expect("reload")
            .expect("application");
        assert_eq!(stored.status, ApplicationStatus::Pending);
        assert_eq!(withdrawn_in_window(&state, "sixth").await, 5);
        assert_eq!(application_count(&state, "sixth").await, 6);
    }

    /// The cap is a window, not a lifetime count: once the oldest counted
    /// withdraw is more than 30 days old, a new application is accepted.
    #[tokio::test]
    async fn apply_is_allowed_again_once_the_oldest_withdraw_leaves_the_window() {
        let state = test_state().await;
        seed_user(&state, "rollapply", "rollapply").await;
        // One withdrawn 29 days and 11 hours ago, four withdrawn 11 hours ago.
        seed_withdrawn(&state, "rollapply", 1).await;
        let_time_pass(&state, "rollapply", "29d").await;
        seed_withdrawn(&state, "rollapply", 4).await;
        let_time_pass(&state, "rollapply", "11h").await;

        let (status, body) = must_err(
            submit_existing(&state, "rollapply", vec![], vec![], VALID_MESSAGE).await,
            "submit at the cap",
        );
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        // Under a day left is said in hours, not "0 days" or "1 day".
        assert_eq!(
            body.error,
            format!("{CAP_REASON}. You can apply again in 13 hours")
        );
        assert_eq!(application_count(&state, "rollapply").await, 5);

        // A day and an hour later the oldest is past 30 days.
        let_time_pass(&state, "rollapply", "25h").await;
        assert_eq!(withdrawn_in_window(&state, "rollapply").await, 4);
        let (status, Json(app)) = must_ok(
            submit_existing(&state, "rollapply", vec![], vec![], VALID_MESSAGE).await,
            "submit after the oldest left the window",
        );
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(app.status, ApplicationStatus::Pending);
        // The row that left the window is not deleted or rewritten.
        assert_eq!(application_count(&state, "rollapply").await, 6);

        // The freed slot can be used for one more withdraw, and then the
        // cap is reached again.
        let Json(app) = must_ok(withdraw_existing(&state, "rollapply").await, "withdraw");
        assert_eq!(app.status, ApplicationStatus::Withdrawn);
        assert_eq!(withdrawn_in_window(&state, "rollapply").await, 5);
        let_time_pass(&state, "rollapply", "61m").await;
        let (status, body) = must_err(
            submit_existing(&state, "rollapply", vec![], vec![], VALID_MESSAGE).await,
            "submit at the cap again",
        );
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        // The next to leave is one of the four from 11 hours (now 37 hours)
        // ago.
        assert_eq!(
            body.error,
            format!("{CAP_REASON}. You can apply again in 29 days")
        );
    }

    /// The same for a withdraw: refused while five are in the window, and
    /// allowed once the oldest of them is more than 30 days old.
    #[tokio::test]
    async fn withdraw_is_allowed_again_once_the_oldest_withdraw_leaves_the_window() {
        let state = test_state().await;
        seed_user(&state, "rollwd", "rollwd").await;
        seed_withdrawn(&state, "rollwd", 1).await;
        let_time_pass(&state, "rollwd", "29d12h").await;
        seed_withdrawn(&state, "rollwd", 4).await;
        let open = state
            .db
            .submit_application("rollwd", vec![], vec![], Some(VALID_MESSAGE))
            .await
            .expect("open application");

        let (status, body) = must_err(
            withdraw_existing(&state, "rollwd").await,
            "withdraw at the cap",
        );
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            body.error,
            format!("{CAP_REASON}. You can withdraw again in 12 hours")
        );
        let stored = state
            .db
            .get_application(&open.id)
            .await
            .expect("reload")
            .expect("application");
        assert_eq!(stored.status, ApplicationStatus::Pending);

        // 13 hours later the oldest is past 30 days.
        let_time_pass(&state, "rollwd", "13h").await;
        assert_eq!(withdrawn_in_window(&state, "rollwd").await, 4);
        let Json(app) = must_ok(
            withdraw_existing(&state, "rollwd").await,
            "withdraw after the oldest left the window",
        );
        assert_eq!(app.status, ApplicationStatus::Withdrawn);
        assert_eq!(app.id, open.id);
        assert_eq!(withdrawn_in_window(&state, "rollwd").await, 5);
        assert_eq!(application_count(&state, "rollwd").await, 6);
    }

    /// The query is per user, only of withdrawn rows, only inside the
    /// window, newest first, and stops at the limit.
    #[tokio::test]
    async fn withdrawn_times_ignore_other_users_other_statuses_and_old_rows() {
        let state = test_state().await;
        seed_user(&state, "counta", "counta").await;
        seed_user(&state, "countb", "countb").await;
        // One old row, then two recent ones, then an open application.
        seed_withdrawn(&state, "counta", 1).await;
        let_time_pass(&state, "counta", "30d1h").await;
        seed_withdrawn(&state, "counta", 1).await;
        let_time_pass(&state, "counta", "3d").await;
        seed_withdrawn(&state, "counta", 1).await;
        seed_withdrawn(&state, "countb", 7).await;
        state
            .db
            .submit_application("counta", vec![], vec![], Some(VALID_MESSAGE))
            .await
            .expect("open application");

        let now = chrono::Utc::now();
        let since = withdrawn_application_window_start(now);
        let times = state
            .db
            .withdrawn_application_times_since("counta", since, 5)
            .await
            .expect("times");
        assert_eq!(times.len(), 2);
        assert!(times[0] > times[1], "newest first: {times:?}");
        assert!(now - times[0] < chrono::Duration::hours(1), "{times:?}");
        assert!(now - times[1] >= chrono::Duration::days(3), "{times:?}");
        assert!(now - times[1] < chrono::Duration::days(4), "{times:?}");
        // The limit bounds the answer: the cap is all the handler needs.
        assert_eq!(
            state
                .db
                .withdrawn_application_times_since("countb", since, 5)
                .await
                .expect("times")
                .len(),
            5
        );
        assert_eq!(withdrawn_in_window(&state, "countb").await, 7);
        assert_eq!(withdrawn_in_window(&state, "nobody").await, 0);
    }

    /// Accept one application as an officer and return the member it made.
    async fn accept_application(state: &AppState, app_id: String, user_id: &str) -> Member {
        let Json(accepted) = must_ok(
            update_application(
                State(state.clone()),
                officer(state).await,
                Path(app_id),
                Json(UpdateApplicationRequest {
                    status: ApplicationStatus::Accepted,
                    review_notes: None,
                }),
            )
            .await,
            "accept",
        );
        assert_eq!(accepted.status, ApplicationStatus::Accepted);
        state
            .db
            .get_member_by_user(user_id)
            .await
            .expect("get member")
            .expect("member")
    }

    /// An accepted applicant is an active member: the 409 it always was,
    /// and it comes before the cooldown 429.
    #[tokio::test]
    async fn active_accepted_member_is_still_409_inside_the_cooldown() {
        let state = test_state().await;
        let (_, Json(app)) = must_ok(
            submit_new(&state, "accepted", vec![], vec![], VALID_MESSAGE).await,
            "first application",
        );
        let member = accept_application(&state, app.id, "accepted").await;
        assert!(member.is_active);

        let (status, body) = must_err(
            submit_existing(&state, "accepted", vec![], vec![], VALID_MESSAGE).await,
            "resubmit as an active member",
        );
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body.error, "Already an active org member");
        // Still the 409 after the cooldown, and nothing was stored.
        age_applications(&state, "accepted", 61).await;
        let (status, body) = must_err(
            submit_existing(&state, "accepted", vec![], vec![], VALID_MESSAGE).await,
            "resubmit as an active member after the cooldown",
        );
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body.error, "Already an active org member");
        assert_eq!(application_count(&state, "accepted").await, 1);
    }

    /// An accepted application is closed too. A member who was accepted and
    /// then deactivated (not banned) may apply again, but only after the
    /// cooldown from the accept.
    #[tokio::test]
    async fn resubmit_right_after_accept_waits_for_the_cooldown() {
        let state = test_state().await;
        let (_, Json(app)) = must_ok(
            submit_new(&state, "acceptcd", vec![], vec![], VALID_MESSAGE).await,
            "first application",
        );
        let member = accept_application(&state, app.id, "acceptcd").await;
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
                None,
                None,
                None,
            )
            .await
            .expect("deactivate");

        let (status, body) = must_err(
            submit_existing(&state, "acceptcd", vec![], vec![], VALID_MESSAGE).await,
            "resubmit inside the cooldown",
        );
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(body.error, "You can apply again in 60 minutes");
        assert_eq!(application_count(&state, "acceptcd").await, 1);

        age_applications(&state, "acceptcd", 59).await;
        let (status, body) = must_err(
            submit_existing(&state, "acceptcd", vec![], vec![], VALID_MESSAGE).await,
            "resubmit at 59 minutes",
        );
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(body.error, "You can apply again in 1 minute");

        age_applications(&state, "acceptcd", 61).await;
        let (status, Json(again)) = must_ok(
            submit_existing(&state, "acceptcd", vec![], vec![], VALID_MESSAGE).await,
            "resubmit after the cooldown",
        );
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(again.status, ApplicationStatus::Pending);
        assert_eq!(application_count(&state, "acceptcd").await, 2);
    }

    /// Trial/accept copies the account username into `member.display_name`.
    /// That is the registration and OAuth signup write. A username that keys
    /// the same as an existing member is 409 `display_name_taken`.
    #[tokio::test]
    async fn provisioning_rejects_a_display_name_taken_username() {
        let state = test_state().await;
        seed_user(&state, "holder", "holder").await;
        state
            .db
            .create_member("holder", "Frozen", OrgRole::Member)
            .await
            .expect("holder");
        seed_user(&state, "applicant", "FROZEN").await;

        let (status, Json(body)) = ensure_member_for_application(
            &state,
            "applicant",
            ApplicationStatus::Pending,
            ApplicationStatus::Trial,
            "app1",
            Some(OrgRole::Officer),
        )
        .await
        .expect_err("taken name");
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body.error, "display_name_taken");
        assert!(
            state
                .db
                .get_member_by_user("applicant")
                .await
                .expect("lookup")
                .is_none(),
            "a rejected provision must not create a member"
        );
    }

    /// #812: with no reviewer role (the applicant self-withdraw path passes
    /// `None`) an inactive member is never reactivated, whatever their rank.
    /// 403, nothing written. An active member needs no reactivation, so `None`
    /// does not get in the way there.
    #[tokio::test]
    async fn provisioning_without_a_reviewer_role_never_reactivates() {
        let state = test_state().await;
        seed_user(&state, "lapsed", "Lapsed").await;
        let member = state
            .db
            .create_member("lapsed", "Lapsed", OrgRole::Recruit)
            .await
            .expect("lapsed member");
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
                None,
                None,
                None,
            )
            .await
            .expect("deactivate");

        for to in [ApplicationStatus::Trial, ApplicationStatus::Accepted] {
            let (status, Json(body)) = ensure_member_for_application(
                &state,
                "lapsed",
                ApplicationStatus::Pending,
                to,
                "app1",
                None,
            )
            .await
            .expect_err("no reviewer role");
            assert_eq!(status, StatusCode::FORBIDDEN);
            assert_eq!(
                body.error,
                "Only officers or admins can reactivate a member through an application"
            );
            let after = state
                .db
                .get_member(&member.id)
                .await
                .expect("lookup")
                .expect("member");
            assert!(
                !after.is_active,
                "{to} with no reviewer must not reactivate"
            );
            assert_eq!(after.org_role, OrgRole::Recruit);
        }

        seed_user(&state, "current", "Current").await;
        let active = state
            .db
            .create_member("current", "Current", OrgRole::Recruit)
            .await
            .expect("current member");
        let kept = must_ok(
            ensure_member_for_application(
                &state,
                "current",
                ApplicationStatus::Pending,
                ApplicationStatus::Trial,
                "app2",
                None,
            )
            .await,
            "an active member needs no reactivation",
        );
        assert_eq!(kept.id, active.id);
        assert!(kept.is_active);
        assert_eq!(kept.org_role, OrgRole::Recruit);
    }
}
