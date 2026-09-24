use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
};
use axum_extra::extract::CookieJar;
use axum_extra::extract::cookie::Cookie;

use crate::state::AppState;

const DEV_SESSION_TOKEN: &str = "dev-session-token-do-not-use-in-production";
const DEV_USER_ID: &str = "devadmin";

/// GET /api/dev/login — ensures the seeded dev session exists, sets the cookie,
/// and redirects to admin.
///
/// Only available for local in-memory dev (`PRODUCTION` off and `SURREALDB_URL`
/// unset or blank). Logout deletes the session row; this re-creates it so Dev
/// login keeps working. Production never reaches this handler: the route is not
/// registered, and the process refuses to boot without a remote URL.
pub async fn dev_login(State(state): State<AppState>, jar: CookieJar) -> Response {
    if !scuffed_db::in_memory_dev_from_env() {
        return (
            StatusCode::NOT_FOUND,
            "Dev login is only available for local in-memory development",
        )
            .into_response();
    }

    // Re-create the session if logout (or expiry cleanup) removed it.
    match state.db.get_session(DEV_SESSION_TOKEN).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            if let Err(e) = state
                .db
                .create_session(DEV_USER_ID, DEV_SESSION_TOKEN, 24 * 365)
                .await
            {
                tracing::error!("Failed to recreate dev session: {e}");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to create dev session",
                )
                    .into_response();
            }
            tracing::info!("Recreated dev session for {DEV_USER_ID}");
        }
        Err(e) => {
            tracing::error!("Dev session lookup failed: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to look up dev session",
            )
                .into_response();
        }
    }

    let cookie = Cookie::build((
        state.session_config.cookie_name.clone(),
        DEV_SESSION_TOKEN.to_string(),
    ))
    .path("/")
    .http_only(true);

    let jar = jar.add(cookie);
    (jar, Redirect::to("/admin/")).into_response()
}
