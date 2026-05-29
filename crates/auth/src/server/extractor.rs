use axum::{
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
    Json,
};
use axum_extra::extract::cookie::CookieJar;
use std::future::Future;

use super::session::ErrorResponse;
use crate::{AuthError, SessionConfig, User};

/// Trait that app states implement to enable the generic AuthUser extractor.
///
/// Each app provides its own session lookup logic (different DBs, dev login, etc.)
/// while sharing the same cookie/extraction mechanism.
pub trait HasAuth: Clone + Send + Sync + 'static {
    fn session_config(&self) -> &SessionConfig;

    fn get_session_user(
        &self,
        token: &str,
    ) -> impl Future<Output = Result<Option<User>, AuthError>> + Send;
}

/// Extractor that requires authentication and provides the current user.
///
/// Generic over `S: HasAuth` so any Axum app state that implements `HasAuth`
/// can use this extractor.
pub struct AuthUser<S: HasAuth>(pub User, std::marker::PhantomData<S>);

impl<S: HasAuth> AuthUser<S> {
    pub fn into_inner(self) -> User {
        self.0
    }
}

impl<S: HasAuth> std::ops::Deref for AuthUser<S> {
    type Target = User;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

fn unauthorized() -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(ErrorResponse {
            error: "Authentication required".into(),
        }),
    )
}

impl<S: HasAuth> FromRequestParts<S> for AuthUser<S> {
    type Rejection = (StatusCode, Json<ErrorResponse>);

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        // Try bearer token first (for desktop/API clients)
        let token = if let Some(auth_header) = parts.headers.get(axum::http::header::AUTHORIZATION)
        {
            let header_str = auth_header.to_str().map_err(|_| unauthorized())?;
            if let Some(bearer_token) = header_str.strip_prefix("Bearer ") {
                bearer_token.to_string()
            } else {
                return Err(unauthorized());
            }
        } else {
            // Fall back to session cookie
            let jar = CookieJar::from_request_parts(parts, state)
                .await
                .unwrap_or_default();
            let config = state.session_config();
            jar.get(&config.cookie_name)
                .ok_or_else(unauthorized)?
                .value()
                .to_string()
        };

        let user = state
            .get_session_user(&token)
            .await
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: "Internal error".into(),
                    }),
                )
            })?
            .ok_or_else(|| {
                (
                    StatusCode::UNAUTHORIZED,
                    Json(ErrorResponse {
                        error: "Session expired or invalid".into(),
                    }),
                )
            })?;

        Ok(AuthUser(user, std::marker::PhantomData))
    }
}
