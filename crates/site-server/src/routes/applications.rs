use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;

use scuffed_auth::server::AuthUser;
use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{Application, ApplicationStatus, AuditAction, AuditTargetType, OrgRole};

use crate::extractors::OfficerUser;
use crate::routes::audit_log::audit;
use crate::state::AppState;

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
    let app = state
        .db
        .submit_application(
            &user.id,
            body.preferred_games,
            body.preferred_roles,
            body.message.as_deref(),
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

    // Notify officers about new application
    if let Some(ref notifier) = state.notifier {
        notifier.notify_officers(format!(
            "New application received from user {}",
            user.username
        ));
    }

    Ok((StatusCode::CREATED, Json(app)))
}

/// GET /api/applications — list all applications (officer+)
pub async fn list_applications(
    State(state): State<AppState>,
    _officer: OfficerUser,
) -> Result<Json<Vec<Application>>, (StatusCode, Json<ErrorResponse>)> {
    state.db.list_applications().await.map(Json).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
    })
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
    let app = state
        .db
        .update_application_status(
            &id,
            body.status,
            &officer.member.id,
            body.review_notes.as_deref(),
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

    // Auto-create member record when application is accepted
    if body.status == ApplicationStatus::Accepted {
        let existing_member = state
            .db
            .get_member_by_user(&app.user_id)
            .await
            .ok()
            .flatten();

        if existing_member.is_none() {
            // Look up the user to get their display name
            let user = state.db.get_user(&app.user_id).await.ok().flatten();
            let display_name = user
                .map(|u| u.username)
                .unwrap_or_else(|| "New Member".to_string());

            match state
                .db
                .create_member(&app.user_id, &display_name, OrgRole::Recruit)
                .await
            {
                Ok(member) => {
                    tracing::info!(
                        "Auto-created member {} for accepted application {}",
                        member.id,
                        id
                    );
                    // Notify general room about new member
                    if let Some(ref notifier) = state.notifier {
                        let org = state
                            .db
                            .get_settings()
                            .await
                            .map(|s| s.org_name)
                            .unwrap_or_else(|_| "the clan".into());
                        notifier.notify_general(format!("Welcome {display_name} to {org}!"));
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to auto-create member for application {}: {}", id, e);
                }
            }
        }
    }

    let action = match body.status {
        ApplicationStatus::Accepted => AuditAction::AcceptedApplication,
        ApplicationStatus::Rejected => AuditAction::RejectedApplication,
        _ => AuditAction::AcceptedApplication, // fallback
    };

    audit(
        &state.db,
        &officer.member.id,
        action,
        AuditTargetType::Application,
        &id,
        Some(&format!("Status: {}", body.status)),
    )
    .await;

    Ok(Json(app))
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
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
        })
}
