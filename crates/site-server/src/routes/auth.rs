use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect},
};
use axum_extra::extract::cookie::CookieJar;
use serde::Deserialize;

use scuffed_auth::server::AuthUser;
use scuffed_auth::server::OAuthProvider;
use scuffed_auth::server::discord::DiscordProvider;
use scuffed_auth::server::google::GoogleProvider;
use scuffed_auth::server::session::{
    build_csrf_cookie, build_session_cookie, clear_session_cookie, generate_session_token,
    validate_csrf_state,
};
use scuffed_auth::{AuthProvider, UserInfo};
use scuffed_db::Member;

use crate::state::AppState;

#[derive(Deserialize)]
pub struct CallbackParams {
    pub code: String,
    pub state: Option<String>,
}

/// GET /api/auth/:provider/login — redirect to OAuth provider
pub async fn login(
    Path(provider): Path<String>,
    State(state): State<AppState>,
    jar: CookieJar,
) -> impl IntoResponse {
    let config = &state.oauth_config;

    match provider.as_str() {
        "discord" => {
            if config.discord_client_id.is_empty() {
                return (StatusCode::SERVICE_UNAVAILABLE, "OAuth not configured").into_response();
            }
            let discord = DiscordProvider {
                client_id: config.discord_client_id.clone(),
                client_secret: config.discord_client_secret.clone(),
                redirect_base_url: config.redirect_base_url.clone(),
            };
            let (auth_url, csrf_token) = discord.get_auth_url();
            let csrf_cookie = build_csrf_cookie(&state.session_config, csrf_token.secret().clone());
            (jar.add(csrf_cookie), Redirect::temporary(&auth_url)).into_response()
        }
        "google" => {
            if config.google_client_id.is_empty() {
                return (StatusCode::SERVICE_UNAVAILABLE, "OAuth not configured").into_response();
            }
            let google = GoogleProvider {
                client_id: config.google_client_id.clone(),
                client_secret: config.google_client_secret.clone(),
                redirect_base_url: config.redirect_base_url.clone(),
            };
            let (auth_url, csrf_token) = google.get_auth_url();
            let csrf_cookie = build_csrf_cookie(&state.session_config, csrf_token.secret().clone());
            (jar.add(csrf_cookie), Redirect::temporary(&auth_url)).into_response()
        }
        _ => (StatusCode::BAD_REQUEST, "Unknown provider").into_response(),
    }
}

/// GET /api/auth/:provider/callback — exchange code for session
pub async fn callback(
    Path(provider): Path<String>,
    Query(params): Query<CallbackParams>,
    State(state): State<AppState>,
    jar: CookieJar,
) -> impl IntoResponse {
    let auth_provider = match provider.as_str() {
        "discord" => AuthProvider::Discord,
        "google" => AuthProvider::Google,
        _ => return (StatusCode::BAD_REQUEST, "Unknown provider").into_response(),
    };

    // Validate CSRF
    if let Err(rejection) = validate_csrf_state(&jar, &state.session_config, params.state.as_ref())
    {
        return rejection.into_response();
    }

    let config = &state.oauth_config;

    // Exchange code and fetch user info based on provider
    let (provider_id, username, avatar_url) = match auth_provider {
        AuthProvider::Discord => {
            if config.discord_client_id.is_empty() {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Discord OAuth not configured",
                )
                    .into_response();
            }
            let discord = DiscordProvider {
                client_id: config.discord_client_id.clone(),
                client_secret: config.discord_client_secret.clone(),
                redirect_base_url: config.redirect_base_url.clone(),
            };
            let access_token = match discord.exchange_code(&params.code).await {
                Ok(token) => token,
                Err(e) => {
                    tracing::error!("Discord token exchange failed: {e}");
                    return (StatusCode::BAD_GATEWAY, "Authentication failed").into_response();
                }
            };
            let user_info = match discord.get_user_info(&access_token).await {
                Ok(info) => info,
                Err(e) => {
                    tracing::error!("Failed to fetch Discord user info: {e}");
                    return (StatusCode::BAD_GATEWAY, "Failed to fetch user info").into_response();
                }
            };
            (
                DiscordProvider::provider_id(&user_info),
                DiscordProvider::username(&user_info),
                DiscordProvider::avatar_url(&user_info),
            )
        }
        AuthProvider::Google => {
            if config.google_client_id.is_empty() {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Google OAuth not configured",
                )
                    .into_response();
            }
            let google = GoogleProvider {
                client_id: config.google_client_id.clone(),
                client_secret: config.google_client_secret.clone(),
                redirect_base_url: config.redirect_base_url.clone(),
            };
            let access_token = match google.exchange_code(&params.code).await {
                Ok(token) => token,
                Err(e) => {
                    tracing::error!("Google token exchange failed: {e}");
                    return (StatusCode::BAD_GATEWAY, "Authentication failed").into_response();
                }
            };
            let user_info = match google.get_user_info(&access_token).await {
                Ok(info) => info,
                Err(e) => {
                    tracing::error!("Failed to fetch Google user info: {e}");
                    return (StatusCode::BAD_GATEWAY, "Failed to fetch user info").into_response();
                }
            };
            (
                GoogleProvider::provider_id(&user_info),
                GoogleProvider::username(&user_info),
                GoogleProvider::avatar_url(&user_info),
            )
        }
        _ => {
            return (StatusCode::BAD_REQUEST, "Unsupported provider").into_response();
        }
    };

    // Upsert user in database
    let user = match state
        .db
        .upsert_user_from_oauth(auth_provider, provider_id, username, avatar_url)
        .await
    {
        Ok(u) => u,
        Err(e) => {
            tracing::error!("Failed to upsert user: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    // Create session
    let session_token = generate_session_token();
    if let Err(e) = state
        .db
        .create_session(
            &user.id,
            &session_token,
            state.session_config.duration_hours,
        )
        .await
    {
        tracing::error!("Failed to create session: {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, "Session creation failed").into_response();
    }

    tracing::info!("User {} logged in via {}", user.username, auth_provider);

    let session_cookie = build_session_cookie(&state.session_config, session_token);
    let csrf_clear = clear_session_cookie(&state.session_config);

    (
        jar.add(session_cookie).remove(csrf_clear),
        Redirect::temporary("/"),
    )
        .into_response()
}

#[derive(serde::Serialize)]
pub struct MeResponse {
    pub user: UserInfo,
    pub member: Option<Member>,
}

/// GET /api/auth/me — return current user + member info
pub async fn me(
    State(state): State<AppState>,
    user: Result<
        AuthUser<AppState>,
        <AuthUser<AppState> as axum::extract::FromRequestParts<AppState>>::Rejection,
    >,
) -> impl IntoResponse {
    match user {
        Ok(auth_user) => {
            let info: UserInfo = (&*auth_user).into();
            let member = state
                .db
                .get_member_by_user(&auth_user.id)
                .await
                .ok()
                .flatten();
            Json(MeResponse { user: info, member }).into_response()
        }
        Err(rejection) => rejection.into_response(),
    }
}

/// POST /api/auth/logout — clear session
pub async fn logout(State(state): State<AppState>, jar: CookieJar) -> impl IntoResponse {
    if let Some(cookie) = jar.get(&state.session_config.cookie_name)
        && let Err(e) = state.db.delete_session(cookie.value()).await
    {
        tracing::warn!("Failed to delete session from DB: {e}");
    }
    let clear_cookie = clear_session_cookie(&state.session_config);
    (jar.add(clear_cookie), StatusCode::OK).into_response()
}
