use axum::{Json, extract::State, http::StatusCode};
use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{AuditAction, AuditTargetType};
use scuffed_types::api::UpdateSettingsRequest;
use scuffed_types::{HomepageContent, NavConfig, PublicLayout, SiteSettings};

use crate::extractors::AdminUser;
use crate::routes::audit_log::audit;
use crate::state::AppState;

/// Accept only safe hex colors (`#rgb`, `#rrggbb`, `#rrggbbaa`) or empty.
fn sanitize_bg_color(raw: &str) -> Result<String, String> {
    let s = raw.trim();
    if s.is_empty() {
        return Ok(String::new());
    }
    let hex = s.strip_prefix('#').unwrap_or(s);
    let ok = matches!(hex.len(), 3 | 6 | 8) && hex.chars().all(|c| c.is_ascii_hexdigit());
    if ok {
        Ok(format!("#{}", hex.to_ascii_lowercase()))
    } else {
        Err(
            "Page background color must be a hex value like #17171d (or empty for theme default)"
                .into(),
        )
    }
}

/// Accept https URLs, site-relative paths, or empty. Reject quotes / CSS injection.
fn sanitize_bg_image_url(raw: &str) -> Result<String, String> {
    let s = raw.trim();
    if s.is_empty() {
        return Ok(String::new());
    }
    if s.contains(['"', '\'', '(', ')', ';', '<', '>', '\\', '\n', '\r']) {
        return Err("Background image URL contains invalid characters".into());
    }
    if s.starts_with('/') || s.starts_with("https://") || s.starts_with("http://") {
        Ok(s.to_string())
    } else {
        Err("Background image must be an https/http URL or a path starting with /".into())
    }
}

fn to_api_settings(db: scuffed_db::SiteSettings) -> SiteSettings {
    let mut nav = NavConfig::from_json(&db.nav_json);
    nav.normalize();
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
        nav,
        page_bg_color: db.page_bg_color,
        page_bg_image_url: db.page_bg_image_url,
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
            tracing::error!(error = %e, "GET /api/settings failed");
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
    let nav_json = body.nav.as_ref().map(|n| {
        let mut n = n.clone();
        n.normalize();
        n.to_json()
    });
    let layout = body.public_layout.map(|l| l.as_str().to_string());

    let page_bg_color = match body.page_bg_color.as_deref() {
        Some(c) => Some(sanitize_bg_color(c).map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse { error: e }),
            )
        })?),
        None => None,
    };
    let page_bg_image_url = match body.page_bg_image_url.as_deref() {
        Some(u) => Some(sanitize_bg_image_url(u).map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse { error: e }),
            )
        })?),
        None => None,
    };

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
            nav_json.as_deref(),
            page_bg_color.as_deref(),
            page_bg_image_url.as_deref(),
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
