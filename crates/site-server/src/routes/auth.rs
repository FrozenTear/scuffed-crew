use axum::{
    Json,
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Redirect, Response},
};
use axum_extra::extract::cookie::CookieJar;
use serde::Deserialize;

use scuffed_auth::password::{MIN_PASSWORD_LEN, hash_password, verify_dummy, verify_password};
use scuffed_auth::server::AuthUser;
use scuffed_auth::server::OAuthProvider;
use scuffed_auth::server::discord::DiscordProvider;
use scuffed_auth::server::google::GoogleProvider;
use scuffed_auth::server::session::{
    ErrorResponse, build_csrf_cookie, build_session_cookie, clear_csrf_cookie,
    clear_session_cookie, generate_session_token, validate_csrf_state,
};
use scuffed_auth::{AuthProvider, UserInfo};
use scuffed_db::{Member, OrgRole};
use scuffed_types::{
    AuthProvidersResponse, LocalLoginRequest, OkResponse, RegisterRequest, SetupRequest,
    SetupStatusResponse,
};

use crate::state::AppState;

fn validate_local_username(username: &str) -> Result<String, &'static str> {
    let u = scuffed_db::Database::normalize_local_username(username);
    if u.is_empty() || u.len() > 32 {
        return Err("username must be 1–32 characters");
    }
    if !u
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err("username may only contain letters, digits, _ and -");
    }
    Ok(u)
}

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

    // Existing provider ids may log in while recruitment is closed.
    // A never-seen id is created only when recruitment_open is true.
    let user = match upsert_oauth_user_if_allowed(
        &state.db,
        auth_provider,
        provider_id,
        username,
        avatar_url,
    )
    .await
    {
        Ok(u) => u,
        // Closed registration is a browser redirect (no session cookie) so the
        // login page can show a banner. Other callback failures stay plain text.
        Err((status, msg)) => return oauth_callback_refusal(status, msg),
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
    // Clear OAuth CSRF state only — never remove the session cookie we just set.
    // cookie 0.18: add+remove *same name* cancels a fresh session Set-Cookie.
    let csrf_clear = clear_csrf_cookie(&state.session_config);

    (
        jar.add(session_cookie).add(csrf_clear),
        Redirect::temporary("/"),
    )
        .into_response()
}

const OAUTH_REGISTRATION_CLOSED: &str = "registration is currently closed";
const OAUTH_REGISTRATION_CLOSED_LOGIN: &str = "/login?error=registration_closed";

/// Browser response for an OAuth callback that did not establish a session.
///
/// A brand-new Discord/Google id refused because `recruitment_open` is false
/// is a 303 to [`OAUTH_REGISTRATION_CLOSED_LOGIN`] with no `Set-Cookie`.
/// Database and provider failures stay status + plain text. There is no
/// existing callback error that already redirects with a query param.
fn oauth_callback_refusal(status: StatusCode, msg: &'static str) -> Response {
    if status == StatusCode::FORBIDDEN && msg == OAUTH_REGISTRATION_CLOSED {
        return Redirect::to(OAUTH_REGISTRATION_CLOSED_LOGIN).into_response();
    }
    (status, msg).into_response()
}

/// Whether an OAuth callback may create or resume an account.
///
/// Existing accounts always proceed, even when recruitment is closed or the
/// settings row could not be read. A never-seen provider id is created only
/// when `recruitment_open` is `Some(Ok(true))`. `None` and `Err` fail closed
/// for new accounts (settings not loaded, or the read failed).
fn oauth_signup_decision(
    existing_user: bool,
    recruitment_open: Option<Result<bool, ()>>,
) -> Result<(), (StatusCode, &'static str)> {
    if existing_user {
        return Ok(());
    }
    match recruitment_open {
        Some(Ok(true)) => Ok(()),
        Some(Ok(false)) => Err((StatusCode::FORBIDDEN, OAUTH_REGISTRATION_CLOSED)),
        Some(Err(())) | None => Err((StatusCode::INTERNAL_SERVER_ERROR, "Database error")),
    }
}

/// Look up the provider id, refuse a brand-new account when recruitment is
/// closed, then upsert. The callback uses this instead of upserting directly
/// so a closed org cannot mint a session for a never-seen Discord/Google id.
async fn upsert_oauth_user_if_allowed(
    db: &scuffed_db::Database,
    provider: AuthProvider,
    provider_id: String,
    username: String,
    avatar_url: Option<String>,
) -> Result<scuffed_auth::User, (StatusCode, &'static str)> {
    let existing = match db.get_user_by_provider(provider, &provider_id).await {
        Ok(user) => user.is_some(),
        Err(e) => {
            tracing::error!("oauth callback user lookup: {e}");
            return Err((StatusCode::INTERNAL_SERVER_ERROR, "Database error"));
        }
    };
    let recruitment = if existing {
        None
    } else {
        match db.get_settings().await {
            Ok(settings) => Some(Ok(settings.recruitment_open)),
            Err(e) => {
                tracing::error!("oauth callback get_settings: {e}");
                Some(Err(()))
            }
        }
    };
    oauth_signup_decision(existing, recruitment)?;
    db.upsert_user_from_oauth(provider, provider_id, username, avatar_url)
        .await
        .map_err(|e| {
            tracing::error!("Failed to upsert user: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Database error")
        })
}

#[derive(serde::Serialize)]
pub struct MeResponse {
    pub user: UserInfo,
    pub member: Option<Member>,
    /// Full NIP-05 identifier (`name@domain`), or `None` when this deploy has
    /// no valid public NIP-05 domain configured. Computed here so the client
    /// never guesses the domain. Mirrors `scuffed_types::MeResponse::nip05`.
    pub nip05: Option<String>,
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
            let nip05 = member.as_ref().and_then(|m| {
                crate::state::nip05_identifier(&m.display_name, state.nip05_domain.as_deref())
            });
            Json(MeResponse {
                user: info,
                member,
                nip05,
            })
            .into_response()
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

/// GET /api/auth/setup-status — whether first-boot admin creation is required.
pub async fn setup_status(State(state): State<AppState>) -> impl IntoResponse {
    // Match the actual setup gate (POST /api/auth/setup gates on `has_any_member`,
    // DR1-ACCT-003). Using the live actionable-admin count here instead would show
    // "needs setup" while every admin is merely suspended, even though the gated
    // endpoint 403s — a display-vs-gate mismatch. Members are never hard-deleted,
    // so `has_any_member` is the monotonic first-boot signal the gate uses.
    let needs_setup = match state.db.has_any_member().await {
        Ok(has) => !has,
        Err(e) => {
            tracing::error!("setup_status has_any_member: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "database error".into(),
                }),
            )
                .into_response();
        }
    };
    let local_login = match state.db.has_local_login().await {
        Ok(v) => v,
        Err(e) => {
            tracing::error!("setup_status has_local_login: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "database error".into(),
                }),
            )
                .into_response();
        }
    };
    Json(SetupStatusResponse {
        needs_setup,
        local_login,
    })
    .into_response()
}

/// GET /api/auth/providers — which login methods the UI should offer.
pub async fn auth_providers(State(state): State<AppState>) -> impl IntoResponse {
    // Match the actual setup gate (`has_any_member`, DR1-ACCT-003) rather than the
    // live actionable-admin count, so this display signal cannot claim setup is
    // available while POST /api/auth/setup 403s (all-admins-suspended state).
    let needs_setup = state
        .db
        .has_any_member()
        .await
        .ok()
        .map(|h| !h)
        .unwrap_or(false);
    let local_login = state.db.has_local_login().await.unwrap_or(false);
    // Self-registration piggybacks the recruitment toggle: recruitment closed
    // means no new accounts (accounts exist only to apply / become members).
    let (register, min_age) = match state.db.get_settings().await {
        Ok(s) => (s.recruitment_open && !needs_setup, s.min_age),
        Err(_) => (false, 16),
    };
    let config = &state.oauth_config;
    Json(AuthProvidersResponse {
        local: local_login || needs_setup,
        discord: !config.discord_client_id.is_empty() && !config.discord_client_secret.is_empty(),
        google: !config.google_client_id.is_empty() && !config.google_client_secret.is_empty(),
        register,
        min_age,
        nostr: true,
    })
    .into_response()
}

/// POST /api/auth/local/register — self-serve local account (privacy-first: no email).
///
/// Creates a bare user only — membership still goes through the application flow.
/// Gated on `recruitment_open` (admin-togglable kill switch, no deploy needed).
pub async fn local_register(
    State(state): State<AppState>,
    jar: CookieJar,
    Json(body): Json<RegisterRequest>,
) -> impl IntoResponse {
    let (open, min_age) = match state.db.get_settings().await {
        Ok(s) => (s.recruitment_open, s.min_age),
        Err(e) => {
            tracing::error!("register get_settings: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "database error".into(),
                }),
            )
                .into_response();
        }
    };
    if !open {
        return (
            StatusCode::FORBIDDEN,
            Json(ErrorResponse {
                error: "registration is currently closed".into(),
            }),
        )
            .into_response();
    }
    if !body.confirm_min_age {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("you must confirm you are {min_age} or older"),
            }),
        )
            .into_response();
    }

    let username = match validate_local_username(&body.username) {
        Ok(u) => u,
        Err(msg) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse { error: msg.into() }),
            )
                .into_response();
        }
    };
    if body.password.len() < MIN_PASSWORD_LEN {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("password must be at least {MIN_PASSWORD_LEN} characters"),
            }),
        )
            .into_response();
    }
    let password_hash = match hash_password(&body.password) {
        Ok(h) => h,
        Err(e) => {
            tracing::error!("register hash_password: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "password hashing failed".into(),
                }),
            )
                .into_response();
        }
    };

    // create_local_user re-checks uniqueness; surface it as a clean 409.
    let user = match state.db.create_local_user(&username, &password_hash).await {
        Ok(u) => u,
        Err(e) if e.to_string().contains("already taken") => {
            // Same blandness as login: do not say whether this username exists.
            return (
                StatusCode::CONFLICT,
                Json(ErrorResponse {
                    error: "could not create account".into(),
                }),
            )
                .into_response();
        }
        Err(e) => {
            tracing::error!("register create_local_user: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "failed to create account".into(),
                }),
            )
                .into_response();
        }
    };

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
        tracing::error!("register create_session: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "session creation failed".into(),
            }),
        )
            .into_response();
    }

    tracing::info!("Local account registered: {}", user.username);
    let session_cookie = build_session_cookie(&state.session_config, session_token);
    (
        StatusCode::CREATED,
        (jar.add(session_cookie), Json(OkResponse { ok: true })),
    )
        .into_response()
}

/// POST /api/auth/setup — one-time first admin account (local username/password).
///
/// Gate (DR1-ACCT-003): this unauthenticated endpoint proceeds **only when no
/// member row exists at all** (`has_any_member() == false`), a monotonic
/// first-boot signal. It deliberately does NOT gate on the live
/// actionable-admin count: that count drops to zero whenever every admin is
/// merely suspended (a *transient / recoverable* state), which previously
/// reopened this endpoint to any unauthenticated caller — a momentary
/// privilege-escalation window. Members are only ever deactivated, never
/// deleted, so once the first admin is created this endpoint stays closed
/// permanently.
///
/// The `has_any_member` check alone is not atomic: two concurrent first-boot
/// POSTs can both observe an empty member table (F-API-002). After validation,
/// we CAS-claim `bootstrap_lock:setup` (`UPDATE … WHERE claimed = false`) so
/// only one request may call `create_member(..., Admin)`. The lock is never
/// reset, so setup stays closed after the first successful claim.
///
/// Lockout recovery for an already-provisioned org (all admins
/// suspended/banned) is intentionally NOT this endpoint: recovery is an
/// operator action (lift the suspension / reactivate an admin directly against
/// the database), never an unauthenticated web request. A future dedicated,
/// explicitly-authenticated recovery path can replace that manual step.
pub async fn setup(
    State(state): State<AppState>,
    jar: CookieJar,
    Json(body): Json<SetupRequest>,
) -> impl IntoResponse {
    match state.db.has_any_member().await {
        Ok(true) => {
            return (
                StatusCode::FORBIDDEN,
                Json(ErrorResponse {
                    error: "setup already completed".into(),
                }),
            )
                .into_response();
        }
        Ok(false) => {}
        Err(e) => {
            tracing::error!("setup has_any_member: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "database error".into(),
                }),
            )
                .into_response();
        }
    }

    let username = match validate_local_username(&body.username) {
        Ok(u) => u,
        Err(msg) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse { error: msg.into() }),
            )
                .into_response();
        }
    };

    if body.password.len() < MIN_PASSWORD_LEN {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("password must be at least {MIN_PASSWORD_LEN} characters"),
            }),
        )
            .into_response();
    }

    let password_hash = match hash_password(&body.password) {
        Ok(h) => h,
        Err(e) => {
            tracing::error!("setup hash_password: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "password hashing failed".into(),
                }),
            )
                .into_response();
        }
    };

    // F-API-002: claim the singleton before creating user/member so two
    // concurrent POSTs cannot both become admins. Loser → 403 (same as the
    // sequential "already completed" path). Claim stays held if a later
    // create_* fails — fail closed rather than reopen first-boot.
    match state.db.claim_bootstrap_lock().await {
        Ok(()) => {}
        Err(scuffed_db::DbError::Conflict(_)) => {
            return (
                StatusCode::FORBIDDEN,
                Json(ErrorResponse {
                    error: "setup already completed".into(),
                }),
            )
                .into_response();
        }
        Err(e) => {
            tracing::error!("setup claim_bootstrap_lock: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "database error".into(),
                }),
            )
                .into_response();
        }
    }

    let user = match state.db.create_local_user(&username, &password_hash).await {
        Ok(u) => u,
        Err(e) => {
            tracing::error!("setup create_local_user: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "failed to create user".into(),
                }),
            )
                .into_response();
        }
    };

    if let Err(e) = state
        .db
        .create_member(&user.id, &username, OrgRole::Admin)
        .await
    {
        tracing::error!("setup create_member: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "failed to create admin member".into(),
            }),
        )
            .into_response();
    }

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
        tracing::error!("setup create_session: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "session creation failed".into(),
            }),
        )
            .into_response();
    }

    // Optional first-boot branding / homepage template.
    {
        use scuffed_types::homepage_preset_by_id;

        let org = body
            .org_name
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let preset = body
            .homepage_preset
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .and_then(homepage_preset_by_id);

        if org.is_some() || preset.is_some() {
            let (layout, homepage_json, brand_dark, brand_light, shell, skin) =
                if let Some(p) = preset.as_ref() {
                    (
                        Some(p.suggested_layout.as_str().to_string()),
                        Some(p.content.to_json()),
                        if p.suggested_brand.accent_dark.is_empty() {
                            None
                        } else {
                            Some(p.suggested_brand.accent_dark.to_string())
                        },
                        if p.suggested_brand.accent_light.is_empty() {
                            None
                        } else {
                            Some(p.suggested_brand.accent_light.to_string())
                        },
                        Some(p.suggested_shell.as_str().to_string()),
                        Some(p.suggested_skin.as_str().to_string()),
                    )
                } else {
                    (None::<String>, None, None, None, None, None)
                };
            // Invalidate before the write and again after it returns, so a
            // shell read during setup cannot keep the pre-setup row.
            state.public_settings.invalidate();
            let write = state
                .db
                .update_settings(
                    org.as_deref(),
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    layout.as_deref(),
                    homepage_json.as_deref(),
                    None,
                    None,
                    None,
                    brand_dark.as_deref(),
                    brand_light.as_deref(),
                    shell.as_deref(),
                    skin.as_deref(),
                    None,
                    None,
                )
                .await;
            state.public_settings.invalidate();
            if let Err(e) = write {
                tracing::warn!("setup: failed to apply org/template settings: {e}");
            }
        }
    }

    tracing::info!("First-boot admin created: {}", user.username);
    let session_cookie = build_session_cookie(&state.session_config, session_token);
    (jar.add(session_cookie), Json(OkResponse { ok: true })).into_response()
}

/// 429 for a password-login lockout. The body is the same whether or not the
/// username exists; `Retry-After` is the remaining backoff in seconds.
fn too_many_logins(retry_after_secs: u64) -> axum::response::Response {
    let secs = retry_after_secs.max(1).to_string();
    (
        StatusCode::TOO_MANY_REQUESTS,
        [(header::RETRY_AFTER, secs)],
        Json(ErrorResponse {
            error: crate::login_lockout::LOCKOUT_MESSAGE.into(),
        }),
    )
        .into_response()
}

/// POST /api/auth/local/login — username/password login for local accounts.
///
/// Failed attempts share a per-account backoff ([`crate::login_lockout`]).
/// Bearer-token and OAuth logins do not.
pub async fn local_login(
    State(state): State<AppState>,
    jar: CookieJar,
    Json(body): Json<LocalLoginRequest>,
) -> impl IntoResponse {
    let username = scuffed_db::Database::normalize_local_username(&body.username);
    let invalid = || {
        (
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse {
                error: "invalid username or password".into(),
            }),
        )
            .into_response()
    };

    // Already locked: same 429 for a real account and a missing one. Do not
    // touch the database, so the response cannot leak which it was.
    if let Some(secs) = state.login_lockout.retry_after(&username) {
        return too_many_logins(secs);
    }

    let (user, hash) = match state.db.get_local_user_by_username(&username).await {
        Ok(Some(pair)) => pair,
        Ok(None) => {
            // No such user: still spend Argon2 verify time against a fixed dummy
            // hash so this path costs roughly the same as a real verify. Without
            // it, a missing username returns in sub-ms while an existing one pays
            // the Argon2 cost, leaking username existence via timing
            // (DR1-AUTH-002). Count the miss toward the same per-username lock
            // so the 429 does not reveal that the account is absent.
            verify_dummy(&body.password);
            if let Some(secs) = state.login_lockout.record_failure(&username) {
                return too_many_logins(secs);
            }
            return invalid();
        }
        Err(e) => {
            tracing::error!("local_login lookup: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "database error".into(),
                }),
            )
                .into_response();
        }
    };

    match verify_password(&body.password, &hash) {
        Ok(true) => {}
        Ok(false) => {
            if let Some(secs) = state.login_lockout.record_failure(&username) {
                return too_many_logins(secs);
            }
            return invalid();
        }
        Err(e) => {
            tracing::error!("local_login verify: {e}");
            if let Some(secs) = state.login_lockout.record_failure(&username) {
                return too_many_logins(secs);
            }
            return invalid();
        }
    }

    state.login_lockout.record_success(&username);

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
        tracing::error!("local_login create_session: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "session creation failed".into(),
            }),
        )
            .into_response();
    }

    tracing::info!("User {} logged in via local", user.username);
    let session_cookie = build_session_cookie(&state.session_config, session_token);
    (jar.add(session_cookie), Json(OkResponse { ok: true })).into_response()
}

// ─── Nostr NIP-07 login (privacy-first signup path #2) ──────────────────────

/// Subject marker for anonymous login challenge tokens — member ids are
/// record-id strings, so this can never collide with a link-flow token.
const NOSTR_LOGIN_SUBJECT: &str = "@login";

#[derive(serde::Serialize)]
pub struct NostrLoginChallengeResponse {
    pub challenge: String,
    pub token: String,
    pub expires_in_secs: u64,
}

/// GET /api/auth/nostr/challenge — anonymous challenge for NIP-07 login.
pub async fn nostr_login_challenge(State(state): State<AppState>) -> impl IntoResponse {
    use rand::RngCore;
    let mut challenge_bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut challenge_bytes);
    let challenge_hex: String = challenge_bytes.iter().map(|b| format!("{b:02x}")).collect();
    let challenge = format!("scuffedclan-login:{challenge_hex}");

    let expires_ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + crate::routes::nostr::CHALLENGE_TTL_SECS;

    let token = crate::routes::nostr::sign_challenge_token(
        &state.nostr_challenge_key,
        &challenge,
        NOSTR_LOGIN_SUBJECT,
        expires_ts,
    );

    Json(NostrLoginChallengeResponse {
        challenge,
        token,
        expires_in_secs: crate::routes::nostr::CHALLENGE_TTL_SECS,
    })
    .into_response()
}

#[derive(Deserialize)]
pub struct NostrLoginVerifyRequest {
    pub token: String,
    pub signed_event: nostr::Event,
}

/// POST /api/auth/nostr/verify — verify a NIP-07-signed challenge and sign in.
///
/// Known pubkeys log into their account (member-linked first, then
/// nostr-provider users). Unknown pubkeys register a bare user — gated on
/// `recruitment_open`, same as local registration.
pub async fn nostr_login_verify(
    State(state): State<AppState>,
    jar: CookieJar,
    Json(body): Json<NostrLoginVerifyRequest>,
) -> impl IntoResponse {
    let bad =
        |msg: String| (StatusCode::BAD_REQUEST, Json(ErrorResponse { error: msg })).into_response();

    let (challenge, subject) =
        match crate::routes::nostr::verify_challenge_token(&state.nostr_challenge_key, &body.token)
        {
            Ok(v) => v,
            Err(e) => return bad(format!("Token verification failed: {e}")),
        };
    if subject != NOSTR_LOGIN_SUBJECT {
        return bad("Token was not issued for login".into());
    }
    if body.signed_event.kind != nostr::Kind::Custom(22242) {
        return bad("Event must use ephemeral kind 22242".into());
    }
    if body.signed_event.content != challenge {
        return bad("Event content does not match the challenge".into());
    }
    if let Err(e) = body.signed_event.verify() {
        return bad(format!("Event verification failed: {e}"));
    }
    // Reject stale events: `Event::verify()` does not bound `created_at`, so a
    // captured victim-signed event would otherwise be replayable forever. This
    // freshness window is the primary replay-closer.
    if let Err(e) =
        crate::routes::nostr::check_event_freshness(body.signed_event.created_at.as_secs())
    {
        return bad(e.to_string());
    }
    // One-time-use: block replay of this exact challenge within its TTL window
    // (defense-in-depth alongside the freshness check above).
    if !state
        .consumed_challenges
        .consume(&challenge, crate::routes::nostr::CONSUMED_CHALLENGE_TTL)
    {
        return bad("challenge already used".into());
    }
    let pubkey_hex = body.signed_event.pubkey.to_hex();

    // 1. Member-linked pubkey → that member's user.
    let user_id = match state.db.get_member_by_nostr_pubkey(&pubkey_hex).await {
        Ok(Some(member)) => Some(member.user_id),
        Ok(None) => None,
        Err(e) => {
            tracing::error!("nostr login member lookup: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "database error".into(),
                }),
            )
                .into_response();
        }
    };

    // 2. Existing nostr-provider user, else 3. register a bare user (gated).
    let user_id = match user_id {
        Some(id) => id,
        None => {
            match state
                .db
                .get_user_by_provider(scuffed_auth::AuthProvider::Nostr, &pubkey_hex)
                .await
            {
                Ok(Some(u)) => u.id,
                Ok(None) => {
                    let open = state
                        .db
                        .get_settings()
                        .await
                        .map(|s| s.recruitment_open)
                        .unwrap_or(false);
                    if !open {
                        return (
                            StatusCode::FORBIDDEN,
                            Json(ErrorResponse {
                                error: "registration is currently closed".into(),
                            }),
                        )
                            .into_response();
                    }
                    let username = format!("nostr-{}", &pubkey_hex[..12]);
                    match state
                        .db
                        .upsert_user_from_oauth(
                            scuffed_auth::AuthProvider::Nostr,
                            pubkey_hex.clone(),
                            username,
                            None,
                        )
                        .await
                    {
                        Ok(u) => u.id,
                        Err(e) => {
                            tracing::error!("nostr login create user: {e}");
                            return (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                Json(ErrorResponse {
                                    error: "failed to create account".into(),
                                }),
                            )
                                .into_response();
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("nostr login user lookup: {e}");
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse {
                            error: "database error".into(),
                        }),
                    )
                        .into_response();
                }
            }
        }
    };

    let session_token = generate_session_token();
    if let Err(e) = state
        .db
        .create_session(
            &user_id,
            &session_token,
            state.session_config.duration_hours,
        )
        .await
    {
        tracing::error!("nostr login create_session: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "session creation failed".into(),
            }),
        )
            .into_response();
    }

    let session_cookie = build_session_cookie(&state.session_config, session_token);
    (jar.add(session_cookie), Json(OkResponse { ok: true })).into_response()
}

#[cfg(test)]
mod oauth_recruitment_tests {
    use super::{oauth_signup_decision, upsert_oauth_user_if_allowed};
    use axum::http::StatusCode;
    use scuffed_auth::AuthProvider;

    #[test]
    fn closed_registration_callback_redirects_without_session_cookie() {
        use axum::http::header;

        let response = super::oauth_callback_refusal(
            StatusCode::FORBIDDEN,
            "registration is currently closed",
        );
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            response
                .headers()
                .get(header::LOCATION)
                .and_then(|v| v.to_str().ok()),
            Some("/login?error=registration_closed")
        );
        assert!(
            response.headers().get(header::SET_COOKIE).is_none(),
            "a refused new OAuth user must not receive a session cookie"
        );

        let db_err =
            super::oauth_callback_refusal(StatusCode::INTERNAL_SERVER_ERROR, "Database error");
        assert_eq!(db_err.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(db_err.headers().get(header::LOCATION).is_none());
    }

    #[test]
    fn new_oauth_identity_refused_when_closed_or_settings_unreadable() {
        let closed = oauth_signup_decision(false, Some(Ok(false))).unwrap_err();
        assert_eq!(closed.0, StatusCode::FORBIDDEN);
        assert_eq!(closed.1, "registration is currently closed");

        let unread = oauth_signup_decision(false, Some(Err(()))).unwrap_err();
        assert_eq!(unread.0, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(unread.1, "Database error");

        let missing = oauth_signup_decision(false, None).unwrap_err();
        assert_eq!(missing.0, StatusCode::INTERNAL_SERVER_ERROR);

        assert!(oauth_signup_decision(false, Some(Ok(true))).is_ok());
    }

    #[test]
    fn existing_oauth_identity_allowed_when_closed_or_settings_unreadable() {
        assert!(oauth_signup_decision(true, Some(Ok(false))).is_ok());
        assert!(oauth_signup_decision(true, Some(Err(()))).is_ok());
        assert!(oauth_signup_decision(true, None).is_ok());
    }

    async fn set_recruitment(db: &scuffed_db::Database, open: bool) {
        db.update_settings(
            None,
            None,
            Some(open),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("update recruitment_open");
    }

    #[tokio::test]
    async fn closed_recruitment_refuses_new_oauth_user_and_allows_existing() {
        let state = crate::test_support::test_state().await;
        set_recruitment(&state.db, false).await;

        let err = upsert_oauth_user_if_allowed(
            &state.db,
            AuthProvider::Discord,
            "never-seen-discord".into(),
            "newbie".into(),
            None,
        )
        .await
        .expect_err("new discord id must be refused");
        assert_eq!(err.0, StatusCode::FORBIDDEN);
        assert_eq!(err.1, "registration is currently closed");
        assert!(
            state
                .db
                .get_user_by_provider(AuthProvider::Discord, "never-seen-discord")
                .await
                .expect("lookup")
                .is_none(),
            "refused signup must not create a user"
        );

        let seeded = state
            .db
            .upsert_user_from_oauth(
                AuthProvider::Google,
                "existing-google".into(),
                "returning".into(),
                None,
            )
            .await
            .expect("seed existing oauth user");
        let logged_in = upsert_oauth_user_if_allowed(
            &state.db,
            AuthProvider::Google,
            "existing-google".into(),
            "returning".into(),
            None,
        )
        .await
        .expect("existing google id may log in while closed");
        assert_eq!(logged_in.id, seeded.id);

        set_recruitment(&state.db, true).await;
        let created = upsert_oauth_user_if_allowed(
            &state.db,
            AuthProvider::Discord,
            "brand-new-discord".into(),
            "opened".into(),
            None,
        )
        .await
        .expect("new discord id allowed when recruitment is open");
        assert_eq!(created.username, "opened");
        assert_eq!(created.provider, AuthProvider::Discord);
    }
}
