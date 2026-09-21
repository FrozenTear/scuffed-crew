use axum::{Json, extract::State, http::StatusCode};
use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{AuditAction, AuditTargetType, OrgRole};
use scuffed_types::api::UpdateSettingsRequest;
use scuffed_types::{HomeShell, HomeSkin, HomepageContent, NavConfig, PublicLayout, SiteSettings};

use crate::extractors::OfficerUser;
use crate::routes::audit_log::audit;
use crate::state::AppState;

/// Officer may set `strategies_enabled` only. Any other Some field is admin-only.
/// Destructure so a new `UpdateSettingsRequest` field fails to compile until classified.
fn officer_has_admin_only_fields(body: &UpdateSettingsRequest) -> bool {
    let UpdateSettingsRequest {
        org_name,
        site_description,
        recruitment_open,
        strategies_enabled: _,
        recruitment_message,
        min_age,
        forum_backend,
        extra_relay_urls,
        home_shell,
        home_skin,
        public_layout,
        homepage,
        nav,
        page_bg_color,
        page_bg_image_url,
        brand_accent_dark,
        brand_accent_light,
    } = body;
    org_name.is_some()
        || site_description.is_some()
        || recruitment_open.is_some()
        || recruitment_message.is_some()
        || min_age.is_some()
        || forum_backend.is_some()
        || extra_relay_urls.is_some()
        || home_shell.is_some()
        || home_skin.is_some()
        || public_layout.is_some()
        || homepage.is_some()
        || nav.is_some()
        || page_bg_color.is_some()
        || page_bg_image_url.is_some()
        || brand_accent_dark.is_some()
        || brand_accent_light.is_some()
}

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

/// Brand accents: empty or `#rgb` / `#rrggbb` only (no alpha).
fn sanitize_brand_accent(raw: &str) -> Result<String, String> {
    let s = raw.trim();
    if s.is_empty() {
        return Ok(String::new());
    }
    let hex = s.strip_prefix('#').unwrap_or(s);
    let ok = matches!(hex.len(), 3 | 6) && hex.chars().all(|c| c.is_ascii_hexdigit());
    if ok {
        let full = if hex.len() == 3 {
            hex.chars().flat_map(|c| [c, c]).collect::<String>()
        } else {
            hex.to_string()
        };
        Ok(format!("#{}", full.to_ascii_lowercase()))
    } else {
        Err("Brand accent must be a hex color like #8f73ff (or empty for default)".into())
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
        strategies_enabled: db.strategies_enabled,
        recruitment_message: db.recruitment_message,
        min_age: db.min_age,
        forum_backend: db.forum_backend,
        extra_relay_urls: db.extra_relay_urls,
        home_shell: HomeShell::from_str_lossy(&db.home_shell),
        home_skin: HomeSkin::from_str_lossy(&db.home_skin),
        public_layout: PublicLayout::from_str_lossy(&db.public_layout),
        homepage: HomepageContent::from_json(&db.homepage_json),
        nav,
        page_bg_color: db.page_bg_color,
        page_bg_image_url: db.page_bg_image_url,
        brand_accent_dark: db.brand_accent_dark,
        brand_accent_light: db.brand_accent_light,
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
        .map(|s| {
            tracing::debug!(
                home_shell = %s.home_shell,
                home_skin = %s.home_skin,
                "GET /api/settings"
            );
            Json(to_api_settings(s))
        })
        .map_err(|e| {
            tracing::error!(error = %e, "GET /api/settings failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })
}

/// PUT /api/settings — Admin: full `UpdateSettingsRequest`.
/// Officer: `strategies_enabled` only; any other set field is 403.
pub async fn update_settings(
    State(state): State<AppState>,
    officer: OfficerUser,
    Json(body): Json<UpdateSettingsRequest>,
) -> Result<Json<SiteSettings>, (StatusCode, Json<ErrorResponse>)> {
    if officer.member.org_role != OrgRole::Admin && officer_has_admin_only_fields(&body) {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ErrorResponse {
                error: "Admin access required".into(),
            }),
        ));
    }
    let homepage_json = body.homepage.as_ref().map(|h| h.to_json());
    let nav_json = body.nav.as_ref().map(|n| {
        let mut n = n.clone();
        n.normalize();
        n.to_json()
    });

    // Prefer home_shell; fall back to public_layout for legacy clients.
    let shell_str = body.home_shell.map(|s| s.as_str().to_string()).or_else(|| {
        body.public_layout
            .map(|l| HomeShell::from_public_layout(l).as_str().into())
    });
    let skin_str = body.home_skin.map(|s| s.as_str().to_string());
    // Still pass public_layout for dual-write path when only layout is sent (shell_str derived above).
    let layout = body.public_layout.map(|l| l.as_str().to_string());

    let page_bg_color = match body.page_bg_color.as_deref() {
        Some(c) => Some(
            sanitize_bg_color(c)
                .map_err(|e| (StatusCode::BAD_REQUEST, Json(ErrorResponse { error: e })))?,
        ),
        None => None,
    };
    let page_bg_image_url = match body.page_bg_image_url.as_deref() {
        Some(u) => Some(
            sanitize_bg_image_url(u)
                .map_err(|e| (StatusCode::BAD_REQUEST, Json(ErrorResponse { error: e })))?,
        ),
        None => None,
    };
    let brand_accent_dark = match body.brand_accent_dark.as_deref() {
        Some(c) => Some(
            sanitize_brand_accent(c)
                .map_err(|e| (StatusCode::BAD_REQUEST, Json(ErrorResponse { error: e })))?,
        ),
        None => None,
    };
    let brand_accent_light = match body.brand_accent_light.as_deref() {
        Some(c) => Some(
            sanitize_brand_accent(c)
                .map_err(|e| (StatusCode::BAD_REQUEST, Json(ErrorResponse { error: e })))?,
        ),
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
            // When shell is provided, layout dual-write is derived in DB layer.
            // When only layout is provided, shell_str is already set above so layout param is unused for mapping.
            layout.as_deref(),
            homepage_json.as_deref(),
            nav_json.as_deref(),
            page_bg_color.as_deref(),
            page_bg_image_url.as_deref(),
            brand_accent_dark.as_deref(),
            brand_accent_light.as_deref(),
            shell_str.as_deref(),
            skin_str.as_deref(),
            body.strategies_enabled,
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

    tracing::info!(
        home_shell = %settings.home_shell,
        home_skin = %settings.home_skin,
        member = %officer.member.id,
        "Updated site settings"
    );

    let details = Some(format!(
        "home_shell={} home_skin={}",
        settings.home_shell, settings.home_skin
    ));
    audit(
        &state.db,
        &officer.member.id,
        AuditAction::UpdatedSettings,
        AuditTargetType::Settings,
        &settings.id,
        details.as_deref(),
    )
    .await;

    Ok(Json(to_api_settings(settings)))
}

#[cfg(test)]
mod officer_settings_gate_tests {
    use super::*;

    #[test]
    fn officer_strategies_only_is_allowed() {
        let body = UpdateSettingsRequest {
            strategies_enabled: Some(false),
            ..Default::default()
        };
        assert!(!officer_has_admin_only_fields(&body));
    }

    #[test]
    fn officer_empty_body_is_not_admin_only() {
        assert!(!officer_has_admin_only_fields(
            &UpdateSettingsRequest::default()
        ));
    }

    #[test]
    fn officer_org_name_is_admin_only() {
        let body = UpdateSettingsRequest {
            org_name: Some("Nope".into()),
            ..Default::default()
        };
        assert!(officer_has_admin_only_fields(&body));
    }

    #[test]
    fn officer_mixed_body_is_admin_only() {
        let body = UpdateSettingsRequest {
            strategies_enabled: Some(false),
            recruitment_open: Some(false),
            ..Default::default()
        };
        assert!(officer_has_admin_only_fields(&body));
    }
}
