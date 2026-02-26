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
        || std::env::var("PRODUCTION").is_ok()
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
        || std::env::var("PRODUCTION").is_ok()
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
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let bytes: [u8; 32] = rng.r#gen();
    base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, bytes)
}

/// Clear a session cookie by setting max-age to zero.
pub fn clear_session_cookie(config: &SessionConfig) -> Cookie<'static> {
    Cookie::build((config.cookie_name.clone(), ""))
        .path("/")
        .max_age(time::Duration::ZERO)
        .build()
}

/// OAuth callback query params
#[derive(serde::Deserialize)]
pub struct OAuthCallback {
    pub code: String,
    pub state: Option<String>,
}
