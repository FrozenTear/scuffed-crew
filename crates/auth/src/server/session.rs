use axum::http::StatusCode;
use axum::Json;
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use serde::Serialize;

use crate::SessionConfig;

/// Error response for API endpoints
#[derive(Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

/// Build a secure session cookie with proper security attributes.
pub fn build_session_cookie(config: &SessionConfig, session_token: String) -> Cookie<'static> {
    let is_secure = std::env::var("SECURE_COOKIES").is_ok()
        || crate::is_production_env()
        || !cfg!(debug_assertions);

    Cookie::build((config.cookie_name.clone(), session_token))
        .path("/")
        .http_only(true)
        .secure(is_secure)
        .same_site(SameSite::Lax)
        .max_age(time::Duration::hours(config.duration_hours))
        .build()
}

/// Build a short-lived CSRF cookie for OAuth state validation.
pub fn build_csrf_cookie(config: &SessionConfig, csrf_token: String) -> Cookie<'static> {
    let is_secure = std::env::var("SECURE_COOKIES").is_ok()
        || crate::is_production_env()
        || !cfg!(debug_assertions);

    Cookie::build((config.csrf_cookie_name.clone(), csrf_token))
        .path("/")
        .http_only(true)
        .secure(is_secure)
        .same_site(SameSite::Lax)
        .max_age(time::Duration::minutes(config.csrf_max_age_minutes))
        .build()
}

/// Validate the OAuth state parameter against the stored CSRF cookie.
pub fn validate_csrf_state(
    jar: &CookieJar,
    config: &SessionConfig,
    state_param: Option<&String>,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let stored_state = jar
        .get(&config.csrf_cookie_name)
        .map(|c| c.value().to_string());

    match (stored_state, state_param) {
        (Some(stored), Some(received)) if stored == *received => Ok(()),
        (None, _) => {
            tracing::warn!("CSRF validation failed: no state cookie found");
            Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Invalid OAuth state. Please try logging in again.".into(),
                }),
            ))
        }
        (_, None) => {
            tracing::warn!("CSRF validation failed: no state parameter in callback");
            Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Invalid OAuth state. Please try logging in again.".into(),
                }),
            ))
        }
        (Some(_), Some(_)) => {
            tracing::warn!("CSRF validation failed: state mismatch");
            Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Invalid OAuth state. Please try logging in again.".into(),
                }),
            ))
        }
    }
}

/// Generate a secure random session token (32 bytes, URL-safe base64).
pub fn generate_session_token() -> String {
    use rand::rngs::OsRng;
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, bytes)
}

fn cookie_is_secure() -> bool {
    std::env::var("SECURE_COOKIES").is_ok() || crate::is_production_env() || !cfg!(debug_assertions)
}

fn clear_named_cookie(name: String) -> Cookie<'static> {
    Cookie::build((name, ""))
        .path("/")
        .http_only(true)
        .secure(cookie_is_secure())
        .same_site(SameSite::Lax)
        .max_age(time::Duration::ZERO)
        .build()
}

/// Clear a session cookie by setting max-age to zero.
///
/// Attributes must match those used when setting the cookie, or browsers may
/// keep the old value.
pub fn clear_session_cookie(config: &SessionConfig) -> Cookie<'static> {
    clear_named_cookie(config.cookie_name.clone())
}

/// Clear the OAuth CSRF state cookie (not the session cookie).
///
/// OAuth callback must use this — never `clear_session_cookie` — or
/// `CookieJar::add(session).remove(session_clear)` cancels the new session
/// (cookie 0.18: add+remove same name drops the delta when no original cookie).
pub fn clear_csrf_cookie(config: &SessionConfig) -> Cookie<'static> {
    clear_named_cookie(config.csrf_cookie_name.clone())
}

/// OAuth callback query params
#[derive(serde::Deserialize)]
pub struct OAuthCallback {
    pub code: String,
    pub state: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> SessionConfig {
        SessionConfig {
            cookie_name: "sc_session".into(),
            csrf_cookie_name: "sc_oauth_state".into(),
            duration_hours: 24,
            csrf_max_age_minutes: 10,
        }
    }

    #[test]
    fn clear_csrf_targets_csrf_name_not_session() {
        let cfg = test_config();
        let c = clear_csrf_cookie(&cfg);
        assert_eq!(c.name(), "sc_oauth_state");
        assert_ne!(c.name(), cfg.cookie_name);
        assert_eq!(c.max_age(), Some(time::Duration::ZERO));
        assert!(c.http_only().unwrap_or(false));
    }

    #[test]
    fn clear_session_targets_session_name() {
        let cfg = test_config();
        let c = clear_session_cookie(&cfg);
        assert_eq!(c.name(), "sc_session");
        assert_eq!(c.max_age(), Some(time::Duration::ZERO));
    }

    /// Regression: OAuth callback must not clear the session cookie name.
    /// Using clear_session_cookie for CSRF was the login-breaking bug.
    #[test]
    fn oauth_helpers_use_distinct_cookie_names() {
        let cfg = test_config();
        let session = build_session_cookie(&cfg, "tok-abc".into());
        let csrf_clear = clear_csrf_cookie(&cfg);
        assert_ne!(
            session.name(),
            csrf_clear.name(),
            "session Set-Cookie and CSRF clear must not share a name \
             (cookie jar add+remove same name cancels a fresh session)"
        );
        assert_eq!(session.value(), "tok-abc");
        assert_eq!(csrf_clear.max_age(), Some(time::Duration::ZERO));
    }
}
