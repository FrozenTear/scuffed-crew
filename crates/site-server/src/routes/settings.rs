use axum::{Json, extract::State, http::StatusCode};
use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{AuditAction, AuditTargetType};
use scuffed_types::api::UpdateSettingsRequest;
use scuffed_types::{HomepageContent, PublicLayout, SiteSettings};

use crate::extractors::AdminUser;
use crate::routes::audit_log::audit;
use crate::state::AppState;

fn to_api_settings(db: scuffed_db::SiteSettings) -> SiteSettings {
    SiteSettings {
        id: db.id,
        org_name: db.org_name,
        site_description: db.site_description,
        recruitment_open: db.recruitment_open,
        recruitment_message: db.recruitment_message,
        min_age: db.min_age,
        forum_backend: db.forum_backend,
        extra_relay_urls: db.extra_relay_urls,
        public_layout: PublicLayout::from_str_lossy(&db.public_layout),
        homepage: HomepageContent::from_json(&db.homepage_json),
        updated_at: db.updated_at,
    }
}

/// GET /api/settings — public
pub async fn get_settings(
    State(state): State<AppState>,
) -> Result<Json<SiteSettings>, (StatusCode, Json<ErrorResponse>)> {
    state
        .db
        .get_settings()
        .await
        .map(|s| Json(to_api_settings(s)))
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
        })
}

/// PUT /api/settings — admin only
pub async fn update_settings(
    State(state): State<AppState>,
    admin: AdminUser,
    Json(body): Json<UpdateSettingsRequest>,
) -> Result<Json<SiteSettings>, (StatusCode, Json<ErrorResponse>)> {
    let homepage_json = body.homepage.as_ref().map(|h| h.to_json());
    let layout = body.public_layout.map(|l| l.as_str().to_string());

    let settings = state
        .db
        .update_settings(
            body.org_name.as_deref(),
            body.site_description.as_deref(),
            body.recruitment_open,
            body.recruitment_message.as_deref(),
            body.min_age,
            body.forum_backend.as_deref(),
            body.extra_relay_urls.as_deref(),
            layout.as_deref(),
            homepage_json.as_deref(),
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

    Ok(Json(to_api_settings(settings)))
}
