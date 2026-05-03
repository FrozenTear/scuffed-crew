use axum::{extract::State, http::StatusCode, Json};
use serde::Deserialize;

use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{AuditAction, AuditTargetType, SiteSettings};

use crate::extractors::AdminUser;
use crate::routes::audit_log::audit;
use crate::state::AppState;

/// GET /api/settings — public
pub async fn get_settings(
    State(state): State<AppState>,
) -> Result<Json<SiteSettings>, (StatusCode, Json<ErrorResponse>)> {
    state
        .db
        .get_settings()
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
pub struct UpdateSettingsRequest {
    pub org_name: Option<String>,
    pub site_description: Option<String>,
    pub recruitment_open: Option<bool>,
    pub recruitment_message: Option<String>,
    pub min_age: Option<u32>,
    pub forum_backend: Option<String>,
}

/// PUT /api/settings — admin only
pub async fn update_settings(
    State(state): State<AppState>,
    admin: AdminUser,
    Json(body): Json<UpdateSettingsRequest>,
) -> Result<Json<SiteSettings>, (StatusCode, Json<ErrorResponse>)> {
    let settings = state
        .db
        .update_settings(
            body.org_name.as_deref(),
            body.site_description.as_deref(),
            body.recruitment_open,
            body.recruitment_message.as_deref(),
            body.min_age,
            body.forum_backend.as_deref(),
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
        &admin.member.id,
        AuditAction::UpdatedSettings,
        AuditTargetType::Settings,
        &settings.id,
        None,
    )
    .await;

    Ok(Json(settings))
}
