use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
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
use scuffed_db::{AuditAction, AuditTargetType, Member, OrgRole};
use scuffed_types::{
    AuthProvidersResponse, LocalLoginRequest, OkResponse, RegisterRequest, SetupRequest,
    SetupStatusResponse,
};

use crate::extractors::presented_session_token;
use crate::routes::audit_log::audit;
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
    if scuffed_db::is_reserved_username(&u) {
        return Err("reserved_name");
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
    if scuffed_db::is_tombstone_user(&user.id) {
        return oauth_callback_refusal(StatusCode::FORBIDDEN, "tombstone");
    }

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
            if matches!(e, scuffed_db::DbError::ReservedName) {
                return (StatusCode::BAD_REQUEST, "reserved_name");
            }
            if matches!(e, scuffed_db::DbError::Conflict(ref code) if code == "tombstone") {
                return (StatusCode::FORBIDDEN, "tombstone");
            }
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
    /// `Some` for every signed-in response. `true` when a local password is
    /// set, so account delete requires `password`. OAuth-only accounts are
    /// `Some(false)`. Never set on `user`. The hash is not returned.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_password: Option<bool>,
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
            let has_password = match state.db.password_hash_for_user(&auth_user.id).await {
                Ok(hash) => hash.is_some(),
                Err(error) => {
                    tracing::error!(
                        error_kind = account_db_kind(&error),
                        "password presence lookup"
                    );
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse {
                            error: "Internal error".into(),
                        }),
                    )
                        .into_response();
                }
            };
            Json(MeResponse {
                user: info,
                member,
                nip05,
                has_password: Some(has_password),
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

#[derive(Deserialize)]
pub struct DeleteAccountBody {
    #[serde(default)]
    password: Option<String>,
    confirm: String,
}

fn account_code(status: StatusCode, code: &'static str) -> Response {
    (status, Json(serde_json::json!({ "code": code }))).into_response()
}

struct AccountCaller {
    user: scuffed_auth::User,
    member: Member,
}

fn account_unauthorized(error: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(ErrorResponse {
            error: error.into(),
        }),
    )
        .into_response()
}

/// Session for account delete. A tombstone session is 403 and is not a login.
#[allow(clippy::result_large_err)]
async fn load_account_caller(
    state: &AppState,
    headers: &HeaderMap,
    jar: &CookieJar,
) -> Result<AccountCaller, Response> {
    let Some(token) = presented_session_token(&state.session_config.cookie_name, headers, jar)
    else {
        return Err(account_unauthorized("Authentication required"));
    };
    let user_id = match state.db.get_session(&token).await {
        Ok(Some(id)) => id,
        Ok(None) => return Err(account_unauthorized("Session expired or invalid")),
        Err(error) => {
            tracing::error!(error_kind = account_db_kind(&error), "session lookup");
            return Err(account_internal());
        }
    };
    if scuffed_db::is_tombstone_user(&user_id) {
        return Err(account_code(StatusCode::FORBIDDEN, "tombstone"));
    }
    let user = match state.db.get_user(&user_id).await {
        Ok(Some(user)) => user,
        Ok(None) => return Err(account_unauthorized("Session expired or invalid")),
        Err(error) => {
            tracing::error!(error_kind = account_db_kind(&error), "user lookup");
            return Err(account_internal());
        }
    };
    if scuffed_db::is_tombstone_user(&user.id) {
        return Err(account_code(StatusCode::FORBIDDEN, "tombstone"));
    }
    let (member, suspended) = match state.db.get_member_auth_by_user(&user.id).await {
        Ok(Some(pair)) => pair,
        Ok(None) => {
            return Err((
                StatusCode::FORBIDDEN,
                Json(ErrorResponse {
                    error: "Not an org member".into(),
                }),
            )
                .into_response());
        }
        Err(error) => {
            tracing::error!(error_kind = account_db_kind(&error), "member auth lookup");
            return Err(account_internal());
        }
    };
    if scuffed_db::is_tombstone_member(&member.id) {
        return Err(account_code(StatusCode::FORBIDDEN, "tombstone"));
    }
    if !member.is_active {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ErrorResponse {
                error: "Membership inactive".into(),
            }),
        )
            .into_response());
    }
    if suspended {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ErrorResponse {
                error: "Account suspended".into(),
            }),
        )
            .into_response());
    }
    Ok(AccountCaller { user, member })
}

/// DELETE /api/auth/me. Erase the signed-in account. Session only.
///
/// `confirm` must equal the display name or the username after trimming.
/// The match is case-sensitive. A local account (password hash present)
/// must send that password. An OAuth-only account must present a session
/// created within the last 10 minutes. Tracker daemon tokens are 401.
/// The tombstone account is 403.
pub async fn delete_account(
    State(state): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Json(body): Json<DeleteAccountBody>,
) -> Response {
    let member = match load_account_caller(&state, &headers, &jar).await {
        Ok(caller) => caller,
        Err(response) => return response,
    };
    let confirm = body.confirm.trim();
    let name_ok =
        confirm == member.member.display_name.trim() || confirm == member.user.username.trim();
    if !name_ok {
        return account_code(StatusCode::BAD_REQUEST, "confirm_mismatch");
    }

    let password_hash = match state.db.password_hash_for_user(&member.user.id).await {
        Ok(hash) => hash,
        Err(error) => {
            tracing::error!(error_kind = account_db_kind(&error), "password lookup");
            return account_internal();
        }
    };
    if let Some(hash) = password_hash {
        let password = body.password.as_deref().unwrap_or("");
        match verify_password(password, &hash) {
            Ok(true) => {}
            Ok(false) | Err(_) => {
                return account_code(StatusCode::UNAUTHORIZED, "invalid_password");
            }
        }
    } else {
        let fresh = session_is_fresh(&state, &headers, &jar).await;
        match fresh {
            Ok(true) => {}
            Ok(false) => return account_code(StatusCode::UNAUTHORIZED, "reauth_required"),
            Err(()) => return account_internal(),
        }
    }

    match state
        .db
        .member_is_last_actionable_admin(&member.member.id)
        .await
    {
        Ok(true) => return account_code(StatusCode::CONFLICT, "last_admin"),
        Ok(false) => {}
        Err(error) => {
            tracing::error!(error_kind = account_db_kind(&error), "last admin check");
            return account_internal();
        }
    }

    let signed = match sign_account_deletion(&state, &member.member.id).await {
        Ok(signed) => signed,
        Err(()) => return account_internal(),
    };

    let report = match state
        .db
        .erase_account(
            &member.member.id,
            &member.user.id,
            member.member.nostr_pubkey.as_deref(),
            signed.as_ref().map(|(json, _)| json.as_str()),
        )
        .await
    {
        Ok(report) => report,
        Err(scuffed_db::DbError::Conflict(code)) if code == "last_admin" => {
            return account_code(StatusCode::CONFLICT, "last_admin");
        }
        Err(error) => {
            tracing::error!(error_kind = account_db_kind(&error), "erase account");
            return account_internal();
        }
    };

    state.leaderboard_cache.invalidate_all();
    for id in &report.stat_report_ids {
        if let Err(error) = crate::stat_reports::remove_report_files(&state.reports_dir, id).await {
            tracing::warn!(
                error_kind = ?error.kind(),
                "stat report file remove after erase"
            );
        }
    }
    if let (Some(url), Some((_, event))) = (&state.relay_url, signed)
        && let Err(error) = scuffed_chat::publish_event_oneshot(url, event).await
    {
        tracing::warn!(error = %error, "nip-09 publish failed");
    }

    audit(
        &state.db,
        &member.member.id,
        AuditAction::DeletedAccount,
        AuditTargetType::Member,
        &member.member.id,
        Some(&format!(
            "personal_match={} stat_report={}",
            report.personal_match,
            report.stat_report_ids.len()
        )),
    )
    .await;

    let clear_cookie = clear_session_cookie(&state.session_config);
    (
        jar.add(clear_cookie),
        Json(serde_json::json!({ "deleted": true })),
    )
        .into_response()
}

fn account_internal() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse {
            error: "Internal error".into(),
        }),
    )
        .into_response()
}

fn account_db_kind(error: &scuffed_db::DbError) -> &'static str {
    match error {
        scuffed_db::DbError::Surreal(_) => "surreal",
        scuffed_db::DbError::NotFound(_) => "not_found",
        scuffed_db::DbError::Conflict(_) => "conflict",
        scuffed_db::DbError::Config(_) => "config",
        scuffed_db::DbError::Timeout => "timeout",
        scuffed_db::DbError::Crypto(_) => "crypto",
        scuffed_db::DbError::DisplayNameInvalid => "display_name_invalid",
        scuffed_db::DbError::DisplayNameTaken => "display_name_taken",
        scuffed_db::DbError::ReservedName => "reserved_name",
    }
}

async fn session_is_fresh(
    state: &AppState,
    headers: &HeaderMap,
    jar: &CookieJar,
) -> Result<bool, ()> {
    let Some(token) = presented_session_token(&state.session_config.cookie_name, headers, jar)
    else {
        return Ok(false);
    };
    let created = state.db.session_created_at(&token).await.map_err(|error| {
        tracing::error!(error_kind = account_db_kind(&error), "session age lookup");
    })?;
    let Some(created) = created else {
        return Ok(false);
    };
    Ok(chrono::Utc::now().signed_duration_since(created) <= chrono::Duration::minutes(10))
}

/// Sign a NIP-09 kind 5 when this member has stored forum event ids and a
/// server-held key. No key, or no stored ids: skip. Articles are not included.
async fn sign_account_deletion(
    state: &AppState,
    member_id: &str,
) -> Result<Option<(String, scuffed_types::NostrEvent)>, ()> {
    let ids = state
        .db
        .member_stored_nostr_event_ids(member_id)
        .await
        .map_err(|error| {
            tracing::error!(
                error_kind = account_db_kind(&error),
                "list stored nostr ids"
            );
        })?;
    if ids.is_empty() {
        return Ok(None);
    }
    let Some(secret) = state
        .db
        .get_nostr_secret_key(member_id)
        .await
        .map_err(|error| {
            tracing::error!(error_kind = account_db_kind(&error), "nostr secret lookup");
        })?
    else {
        return Ok(None);
    };
    let keys = scuffed_chat::EventBuilder::keys_from_hex(secret.as_str()).map_err(|_| ())?;
    let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    let event =
        scuffed_chat::EventBuilder::build_delete_event(&keys, &refs, Some("account deleted"))
            .map_err(|_| ())?;
    let relay_event = scuffed_chat::EventBuilder::to_relay_event(&event);
    let json = serde_json::to_string(&relay_event).map_err(|_| ())?;
    drop(secret);
    Ok(Some((json, relay_event)))
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
        Err(scuffed_db::DbError::ReservedName) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "reserved_name".into(),
                }),
            )
                .into_response();
        }
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

/// Drop a failed first-boot claim so a later POST can succeed (W1019-14).
///
/// Deletes `created_user_id` only when no member row exists, then clears the
/// lock under the same condition. A member row keeps both the user and the claim.
async fn abandon_failed_setup(state: &AppState, created_user_id: Option<&str>) {
    if let Some(user_id) = created_user_id {
        match state.db.delete_local_user_if_no_member(user_id).await {
            Ok(true) => {
                tracing::warn!("setup rolled back the local user after member create failed")
            }
            Ok(false) => {
                tracing::error!("setup left the local user in place because a member row exists")
            }
            Err(e) => tracing::error!("setup rollback delete user: {e}"),
        }
    }
    match state.db.release_bootstrap_lock().await {
        Ok(true) => tracing::warn!("setup released the first-boot lock after a failed create"),
        Ok(false) => {}
        Err(e) => tracing::error!("setup release_bootstrap_lock: {e}"),
    }
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
/// only one request may call `create_member(..., Admin)`. If that create fails
/// and no member row exists, the claim is released so a later POST can retry.
/// Once a member exists the lock stays claimed, including when session
/// creation fails after the member row is stored.
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
    // concurrent POSTs cannot both become admins. The loser gets 403 (same as
    // the sequential "already completed" path). If user or member creation
    // fails and no member row exists, the claim is released so setup can be
    // retried (W1019-14). A member row keeps the claim closed.
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
        Err(scuffed_db::DbError::ReservedName) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "reserved_name".into(),
                }),
            )
                .into_response();
        }
        Err(e) => {
            tracing::error!("setup create_local_user: {e}");
            abandon_failed_setup(&state, None).await;
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
        abandon_failed_setup(&state, Some(&user.id)).await;
        if let Some(response) = crate::routes::members::display_name_write_error(&e) {
            return response.into_response();
        }
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
            // Invalidate before the write and again when this scope ends,
            // including when the request is dropped. A shell read during
            // setup cannot keep the pre-setup row.
            state.public_settings.invalidate();
            let _invalidate_after_write = state.public_settings.invalidate_on_drop();
            #[cfg(test)]
            state.public_settings.run_write_hook().await;
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
            #[cfg(test)]
            state.public_settings.run_after_write_hook().await;
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
    let invalid = || {
        (
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse {
                error: "invalid username or password".into(),
            }),
        )
            .into_response()
    };

    // Same trim, case fold, length, and character rules as register and setup.
    // A string that could never be stored is not looked up and is not a lockout
    // key (that map is only for names that pass the rules). The body matches a
    // wrong password. `verify_dummy` keeps this branch from returning faster
    // than a missing account. The per-IP login governor still covers the request.
    let username = match validate_local_username(&body.username) {
        Ok(username) => username,
        Err(_) => {
            verify_dummy(&body.password);
            return invalid();
        }
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
        Ok(true) => {
            if scuffed_db::is_tombstone_user(&user.id) {
                return invalid();
            }
        }
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

    if scuffed_db::is_tombstone_user(&user_id) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse {
                error: "invalid credentials".into(),
            }),
        )
            .into_response();
    }

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
    use axum::http::{StatusCode, header};
    use scuffed_auth::AuthProvider;
    use tower::ServiceExt;

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

    fn no_session_cookie(response: &axum::response::Response) -> bool {
        response
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .all(|value| {
                value
                    .to_str()
                    .ok()
                    .is_none_or(|raw| !raw.starts_with("sc_session="))
            })
    }

    async fn login(app: &axum::Router, username: &str, password: &str) -> axum::response::Response {
        app.clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/auth/local/login")
                    .header(header::CONTENT_TYPE, "application/json")
                    .extension(axum::extract::ConnectInfo(std::net::SocketAddr::from((
                        [127, 0, 0, 1],
                        40000,
                    ))))
                    .body(axum::body::Body::from(
                        serde_json::json!({ "username": username, "password": password })
                            .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    /// Password, OAuth link, and daemon token all fail closed for Deleted member.
    #[tokio::test]
    async fn tombstone_cannot_log_in_by_password_oauth_or_token() {
        use http_body_util::BodyExt;
        use scuffed_auth::crypto::hash_session_token;
        use scuffed_auth::password::hash_password;
        use scuffed_db::{DELETED_USER_ID, DbError};

        let state = crate::test_support::test_state().await;
        scuffed_db::queries::erasure::ensure_deleted_member(&state.db.client)
            .await
            .unwrap();
        let app = crate::create_router(state.clone());

        let reserved = login(&app, "deleteduser", "correct-horse").await;
        assert_eq!(reserved.status(), StatusCode::UNAUTHORIZED);
        assert!(no_session_cookie(&reserved));

        let hash = hash_password("correct-horse").unwrap();
        state
            .db
            .client
            .query("UPDATE user:deleteduser SET username = 'ghostmember', password_hash = $hash")
            .bind(("hash", hash))
            .await
            .unwrap()
            .check()
            .unwrap();
        let matched = login(&app, "ghostmember", "correct-horse").await;
        assert_eq!(matched.status(), StatusCode::UNAUTHORIZED);
        assert!(
            no_session_cookie(&matched),
            "a matching password on the tombstone user must not set a session"
        );

        state
            .db
            .client
            .query(
                "UPDATE user:deleteduser SET provider = 'discord', provider_id = 'discord-tomb', \
                 provider_id_hash = NONE",
            )
            .await
            .unwrap()
            .check()
            .unwrap();
        let linked = upsert_oauth_user_if_allowed(
            &state.db,
            AuthProvider::Discord,
            "discord-tomb".into(),
            "someone".into(),
            None,
        )
        .await
        .expect_err("oauth link of the tombstone provider id");
        assert_eq!(linked.0, StatusCode::FORBIDDEN);
        assert_eq!(linked.1, "tombstone");
        let refusal = super::oauth_callback_refusal(linked.0, linked.1);
        assert_eq!(refusal.status(), StatusCode::FORBIDDEN);
        assert!(no_session_cookie(&refusal));
        let session_err = state
            .db
            .create_session(DELETED_USER_ID, "should-not-mint", 1)
            .await
            .unwrap_err();
        assert!(
            matches!(session_err, DbError::Conflict(ref code) if code == "tombstone"),
            "{session_err}"
        );

        let reserved_name = upsert_oauth_user_if_allowed(
            &state.db,
            AuthProvider::Discord,
            "brand-new-tomb".into(),
            " DeletedUser ".into(),
            None,
        )
        .await
        .expect_err("reserved oauth username");
        assert_eq!(reserved_name.0, StatusCode::BAD_REQUEST);
        assert_eq!(reserved_name.1, "reserved_name");

        let token_hash = hash_session_token("tomb-daemon");
        state
            .db
            .client
            .query(
                "CREATE daemon_token SET member_id = 'deleted', token_hash = $hash, \
                 label = 'forged', is_active = true, created_at = time::now(), expires_at = NONE",
            )
            .bind(("hash", token_hash))
            .await
            .unwrap()
            .check()
            .unwrap();
        let upload = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/stats/upload")
                    .header(header::AUTHORIZATION, "Bearer tomb-daemon")
                    .header(header::CONTENT_TYPE, "application/json")
                    .extension(axum::extract::ConnectInfo(std::net::SocketAddr::from((
                        [127, 0, 0, 1],
                        40000,
                    ))))
                    .body(axum::body::Body::from(r#"{"matches":[]}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(upload.status(), StatusCode::UNAUTHORIZED);
        assert!(no_session_cookie(&upload));
        let bytes = upload.into_body().collect().await.unwrap().to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error"], "Invalid or revoked daemon token");
        assert!(body.get("code").is_none(), "{body}");

        let session_hash = hash_session_token("forged-session");
        state
            .db
            .client
            .query(
                "CREATE session SET user_id = 'deleteduser', token = $hash, \
                 expires_at = time::now() + 1d, created_at = time::now()",
            )
            .bind(("hash", session_hash))
            .await
            .unwrap()
            .check()
            .unwrap();
        assert_eq!(
            state
                .db
                .get_session("forged-session")
                .await
                .unwrap()
                .as_deref(),
            Some(DELETED_USER_ID)
        );
        let me = app
            .oneshot(
                axum::http::Request::builder()
                    .method("GET")
                    .uri("/api/auth/me")
                    .header(header::COOKIE, "sc_session=forged-session")
                    .extension(axum::extract::ConnectInfo(std::net::SocketAddr::from((
                        [127, 0, 0, 1],
                        40000,
                    ))))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(me.status(), StatusCode::UNAUTHORIZED);
    }
}
