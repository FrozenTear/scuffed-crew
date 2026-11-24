use axum::{Json, extract::State, http::StatusCode};
use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{AuditAction, AuditTargetType, OrgRole};
use scuffed_types::api::UpdateSettingsRequest;
use scuffed_types::brand::{AccentCheck, BrandConfig};
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
        officers_can_edit_teams,
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
        brand_font_display,
    } = body;
    org_name.is_some()
        || site_description.is_some()
        || recruitment_open.is_some()
        || officers_can_edit_teams.is_some()
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
        || brand_font_display.is_some()
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

/// Which themes a brand save changes, as `(dark, light)`. `dark` and `light`
/// are the accents in the request (`None` when absent). A dark save also
/// changes light when the stored light is empty, because the light theme
/// then depends on dark: an empty light follows a set dark accent, and when
/// the sent dark is empty too both themes take the product defaults (light
/// `#6d4aff`, see `BrandConfig::from_settings`).
fn changed_accent_themes(
    dark: Option<&str>,
    light: Option<&str>,
    stored_light: &str,
) -> (bool, bool) {
    let light_changed = light.is_some() || (dark.is_some() && stored_light.trim().is_empty());
    (dark.is_some(), light_changed)
}

/// The brand settings contrast lock, run on the accents a save would leave
/// stored. `dark` and `light` are the sanitized accents in the request
/// (`None` when absent); `stored_*` fill in the absent side. Only the themes
/// [`changed_accent_themes`] reports are checked, so an accent already stored
/// that fails the lock does not block a save that leaves that theme's key out.
///
/// The light check cannot fail today: `AccentRoles::derive` for the light
/// scheme darkens until every light pair passes, so every refusal this
/// returns names the dark theme. The light check stays as a guard in case
/// the derivation changes; its three sentences are pinned in `scuffed-types`
/// (`light_fail_reasons_are_pinned_on_a_constructed_check`). Reasons and
/// rules for API callers: `docs/api/site-settings.md`.
fn brand_contrast_error(
    dark: Option<&str>,
    light: Option<&str>,
    stored_dark: &str,
    stored_light: &str,
) -> Option<String> {
    if dark.is_none() && light.is_none() {
        return None;
    }
    let eff_dark = dark.unwrap_or(stored_dark);
    let eff_light = light.unwrap_or(stored_light);
    let check = AccentCheck::for_brand(&BrandConfig::from_settings(eff_dark, eff_light));
    let (dark_changed, light_changed) = changed_accent_themes(dark, light, stored_light);
    let dark_reason = if dark_changed {
        check.dark.fail_reason()
    } else {
        None
    };
    dark_reason.or_else(|| {
        if light_changed {
            check.light.fail_reason()
        } else {
            None
        }
    })
}

/// Display font: one of `BRAND_FONT_DISPLAY_ALLOWED`, compared exactly.
/// Never a free font name or URL, so nothing here is trimmed or normalized.
/// See `docs/api/site-settings.md`.
fn validate_brand_font_display(raw: &str) -> Result<String, String> {
    if scuffed_types::is_allowed_brand_font_display(raw) {
        Ok(raw.to_string())
    } else {
        Err("Display font must be urbanist or ibm-plex-sans (or empty for default)".into())
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
        officers_can_edit_teams: db.officers_can_edit_teams,
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
        brand_font_display: db.brand_font_display,
        updated_at: db.updated_at,
    }
}

/// Anonymous `GET /api/settings` value.
///
/// The handler takes no session extractor. Officers and anonymous callers
/// both receive [`to_api_settings`] — there is no second field list. The SPA
/// shell embeds the JSON from this same mapping.
pub(crate) async fn load_anonymous_settings(
    db: &scuffed_db::Database,
) -> Result<SiteSettings, scuffed_db::DbError> {
    db.get_settings().await.map(to_api_settings)
}

/// Serialized body of [`load_anonymous_settings`], using `serde_json` the same
/// way Axum's [`Json`] response does (`serde_json::to_vec` / `to_string`).
///
/// `org_name` and `site_description` are the fields the shell writes into
/// `<title>` and the description / Open Graph meta tags.
pub(crate) struct AnonymousSettingsJson {
    pub json: String,
    pub org_name: String,
    pub site_description: String,
}

pub(crate) async fn anonymous_settings_json(
    db: &scuffed_db::Database,
) -> Result<AnonymousSettingsJson, String> {
    let settings = load_anonymous_settings(db)
        .await
        .map_err(|e| e.to_string())?;
    let json = serde_json::to_string(&settings).map_err(|e| e.to_string())?;
    Ok(AnonymousSettingsJson {
        json,
        org_name: settings.org_name,
        site_description: settings.site_description,
    })
}

/// GET /api/settings — public (anonymous and signed-in callers share this body)
pub async fn get_settings(
    State(state): State<AppState>,
) -> Result<Json<SiteSettings>, (StatusCode, Json<ErrorResponse>)> {
    match load_anonymous_settings(&state.db).await {
        Ok(settings) => {
            tracing::debug!(
                home_shell = %settings.home_shell,
                home_skin = %settings.home_skin,
                "GET /api/settings"
            );
            Ok(Json(settings))
        }
        Err(e) => {
            tracing::error!(error = %e, "GET /api/settings failed");
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            ))
        }
    }
}

/// PUT /api/settings — Admin: full `UpdateSettingsRequest`.
/// Officer: `strategies_enabled` only; any other set field, including
/// `officers_can_edit_teams`, is 403.
///
/// `brand_font_display` (values, who may set it, every status code, upgrade
/// and rollback) and the brand accent contrast 400: `docs/api/site-settings.md`.
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
    // Invalidate on the attempt, then again when this handler returns
    // (success, validation error, or a failed database update). The second
    // bump drops anything a shell read cached while the save was in flight.
    state.public_settings.invalidate();
    let _invalidate_after_write = state.public_settings.invalidate_on_drop();
    // The hook sits in the window a shell read can re-cache the pre-write row.
    // The drop guard above runs after this function returns and drops that row.
    #[cfg(test)]
    state.public_settings.run_write_hook().await;

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
    let brand_font_display = match body.brand_font_display.as_deref() {
        Some(f) => Some(
            validate_brand_font_display(f)
                .map_err(|e| (StatusCode::BAD_REQUEST, Json(ErrorResponse { error: e })))?,
        ),
        None => None,
    };

    // Same lock as the brand settings page; refuse before anything is written.
    if brand_accent_dark.is_some() || brand_accent_light.is_some() {
        let (stored_dark, stored_light) =
            if brand_accent_dark.is_some() && brand_accent_light.is_some() {
                (String::new(), String::new())
            } else {
                let stored = state.db.get_settings().await.map_err(|e| {
                    tracing::error!(error = %e, "PUT /api/settings: read stored accents failed");
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse {
                            error: "Internal error".into(),
                        }),
                    )
                })?;
                (stored.brand_accent_dark, stored.brand_accent_light)
            };
        if let Some(reason) = brand_contrast_error(
            brand_accent_dark.as_deref(),
            brand_accent_light.as_deref(),
            &stored_dark,
            &stored_light,
        ) {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse { error: reason }),
            ));
        }
    }

    let written = state
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
            body.officers_can_edit_teams,
            brand_font_display.as_deref(),
        )
        .await;
    // After the write returns, still before this handler returns. The drop
    // guard above has not run yet, so a read released here can store, and
    // that store is cleared when the guard drops.
    #[cfg(test)]
    state.public_settings.run_after_write_hook().await;
    let settings = written.map_err(|_e| {
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

    #[test]
    fn officer_officers_can_edit_teams_is_admin_only() {
        let body = UpdateSettingsRequest {
            officers_can_edit_teams: Some(true),
            ..Default::default()
        };
        assert!(officer_has_admin_only_fields(&body));
    }

    #[test]
    fn officer_brand_font_display_is_admin_only() {
        for value in ["", "urbanist", "ibm-plex-sans"] {
            let body = UpdateSettingsRequest {
                brand_font_display: Some(value.into()),
                ..Default::default()
            };
            assert!(officer_has_admin_only_fields(&body), "{value:?}");
        }
    }

    #[test]
    fn brand_font_display_validation_is_exact() {
        for ok in scuffed_types::BRAND_FONT_DISPLAY_ALLOWED {
            assert_eq!(validate_brand_font_display(ok).as_deref(), Ok(*ok));
        }
        for bad in [
            "comic-sans",
            " urbanist",
            "Urbanist",
            "https://x.test/f.woff2",
        ] {
            assert!(validate_brand_font_display(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn officer_strategies_plus_team_flag_is_admin_only() {
        let body = UpdateSettingsRequest {
            strategies_enabled: Some(false),
            officers_can_edit_teams: Some(true),
            ..Default::default()
        };
        assert!(officer_has_admin_only_fields(&body));
    }
}

#[cfg(test)]
mod brand_contrast_tests {
    use super::{brand_contrast_error, changed_accent_themes};

    /// Blocked by the client lock: no ink reaches 4.5:1 on it in dark.
    const BLOCKED: &str = "#d35400";

    #[test]
    fn no_accent_in_the_request_is_not_checked() {
        assert!(brand_contrast_error(None, None, BLOCKED, BLOCKED).is_none());
    }

    #[test]
    fn product_default_and_empty_pass() {
        assert!(brand_contrast_error(Some("#8f73ff"), Some("#6d4aff"), "", "").is_none());
        assert!(brand_contrast_error(Some(""), Some(""), "", "").is_none());
    }

    #[test]
    fn blocked_accent_fails_with_the_client_reason() {
        assert_eq!(
            brand_contrast_error(Some(BLOCKED), Some(BLOCKED), "", "").as_deref(),
            Some("Ink on this accent does not reach 4.5:1 in the dark theme.")
        );
    }

    #[test]
    fn edgeless_accent_fails_the_fill_pair() {
        let reason = brand_contrast_error(Some("#000000"), Some("#000000"), "", "");
        assert_eq!(
            reason.as_deref(),
            Some(
                "Buttons in this accent do not reach 3:1 against the dark page and cards, so they lose their edge."
            )
        );
    }

    #[test]
    fn dark_only_save_checks_the_requested_dark_accent() {
        assert!(brand_contrast_error(Some(BLOCKED), None, "", "#6d4aff").is_some());
        assert!(brand_contrast_error(Some("#46d8a4"), None, BLOCKED, "#6d4aff").is_none());
    }

    /// Issue 797: a stored failing accent is grandfathered only while its key
    /// stays out of the request. Sending the same value again is checked.
    #[test]
    fn resending_a_stored_failing_dark_is_refused() {
        assert_eq!(
            brand_contrast_error(Some(BLOCKED), None, BLOCKED, "#6d4aff").as_deref(),
            Some("Ink on this accent does not reach 4.5:1 in the dark theme.")
        );
    }

    /// Issue 796: the light default is refused as a dark accent.
    #[test]
    fn light_default_is_refused_as_a_dark_accent() {
        assert_eq!(
            brand_contrast_error(Some("#6d4aff"), None, "", "#6d4aff").as_deref(),
            Some(
                "Buttons in this accent do not reach 3:1 against the dark page and cards, so they lose their edge."
            )
        );
    }

    #[test]
    fn stored_failing_dark_does_not_block_a_light_save() {
        assert!(brand_contrast_error(None, Some("#6d4aff"), BLOCKED, "").is_none());
    }

    #[test]
    fn dark_only_save_changes_light_only_when_stored_light_is_empty() {
        assert_eq!(
            changed_accent_themes(Some("#8f73ff"), None, ""),
            (true, true)
        );
        assert_eq!(
            changed_accent_themes(Some("#8f73ff"), None, "  "),
            (true, true)
        );
        assert_eq!(
            changed_accent_themes(Some("#8f73ff"), None, "#6d4aff"),
            (true, false)
        );
    }

    #[test]
    fn light_only_save_changes_only_light() {
        assert_eq!(
            changed_accent_themes(None, Some("#6d4aff"), ""),
            (false, true)
        );
        assert_eq!(
            changed_accent_themes(None, Some(""), "#6d4aff"),
            (false, true)
        );
    }

    #[test]
    fn save_with_both_or_neither_accent() {
        assert_eq!(
            changed_accent_themes(Some("#8f73ff"), Some("#6d4aff"), "#6d4aff"),
            (true, true)
        );
        assert_eq!(changed_accent_themes(None, None, ""), (false, false));
    }
}
