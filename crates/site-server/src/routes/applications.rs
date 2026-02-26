use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;

use scuffed_auth::server::session::ErrorResponse;
use scuffed_auth::server::AuthUser;
use scuffed_db::{Application, ApplicationStatus};

use crate::extractors::OfficerUser;
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
    Ok((StatusCode::CREATED, Json(app)))
}

/// GET /api/applications — list all applications (officer+)
pub async fn list_applications(
    State(state): State<AppState>,
    _officer: OfficerUser,
) -> Result<Json<Vec<Application>>, (StatusCode, Json<ErrorResponse>)> {
    state
        .db
        .list_applications()
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
    state
        .db
        .update_application_status(
            &id,
            body.status,
            &officer.member.id,
            body.review_notes.as_deref(),
        )
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
