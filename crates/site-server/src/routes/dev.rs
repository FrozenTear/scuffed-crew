use axum::{extract::State, response::Redirect};
use axum_extra::extract::CookieJar;
use axum_extra::extract::cookie::Cookie;

use crate::state::AppState;

const DEV_SESSION_TOKEN: &str = "dev-session-token-do-not-use-in-production";

/// GET /api/dev/login — sets the dev session cookie and redirects to admin
pub async fn dev_login(
    State(state): State<AppState>,
    jar: CookieJar,
) -> (CookieJar, Redirect) {
    let cookie = Cookie::build((state.session_config.cookie_name.clone(), DEV_SESSION_TOKEN))
        .path("/")
        .http_only(true);

    let jar = jar.add(cookie);
    (jar, Redirect::to("/admin/"))
}
