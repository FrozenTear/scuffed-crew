use axum::{
    Json,
    extract::FromRequestParts,
    http::{HeaderMap, StatusCode, header, header::AUTHORIZATION, request::Parts},
    response::{IntoResponse, Response},
};
use axum_extra::extract::cookie::CookieJar;

use scuffed_auth::User;
use scuffed_auth::server::AuthUser;
use scuffed_auth::server::HasAuth;
use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{DaemonTokenAuth, Member, OrgRole};

use crate::state::AppState;

/// Session user from the same credentials as [`AuthUser`].
///
/// `Authorization: Bearer` is tried first. A present `Authorization` header
/// does not fall back to the session cookie. With no header, the session
/// cookie is used. Missing, malformed, or unknown credentials are `None` so
/// public reads stay anonymous. Routes that require a user still use
/// [`AuthUser`], which answers 401 for those cases.
pub async fn optional_session_user(
    state: &AppState,
    headers: &HeaderMap,
    jar: &CookieJar,
) -> Option<User> {
    let token = presented_session_token(&state.session_config.cookie_name, headers, jar)?;
    state.get_session_user(&token).await.ok().flatten()
}

/// Bearer token when `Authorization` is present, otherwise the session cookie.
fn presented_session_token(
    cookie_name: &str,
    headers: &HeaderMap,
    jar: &CookieJar,
) -> Option<String> {
    if let Some(value) = headers.get(AUTHORIZATION) {
        let header_str = value.to_str().ok()?;
        return header_str.strip_prefix("Bearer ").map(ToOwned::to_owned);
    }
    jar.get(cookie_name).map(|cookie| cookie.value().to_owned())
}

/// Extractor: any authenticated org member.
pub struct OrgMember {
    pub user: User,
    pub member: Member,
}

/// Extractor: optional org member (anonymous → `None`, does not fail the request).
pub struct OptionalOrgMember(pub Option<Member>);

/// Extractor: officer or admin.
pub struct OfficerUser {
    pub user: User,
    pub member: Member,
}

/// Extractor: admin only.
pub struct AdminUser {
    pub user: User,
    pub member: Member,
}

impl FromRequestParts<AppState> for OrgMember {
    type Rejection = (StatusCode, Json<ErrorResponse>);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let auth_user = AuthUser::<AppState>::from_request_parts(parts, state).await?;
        let user = auth_user.into_inner();

        // Single helper: member lookup + suspension check (fail closed on DB errors).
        let (member, suspended) = state
            .db
            .get_member_auth_by_user(&user.id)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, user_id = %user.id, "member auth lookup failed");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: "Internal error".into(),
                    }),
                )
            })?
            .ok_or_else(|| {
                (
                    StatusCode::FORBIDDEN,
                    Json(ErrorResponse {
                        error: "Not an org member".into(),
                    }),
                )
            })?;

        if !member.is_active {
            return Err((
                StatusCode::FORBIDDEN,
                Json(ErrorResponse {
                    error: "Membership inactive".into(),
                }),
            ));
        }

        if suspended {
            return Err((
                StatusCode::FORBIDDEN,
                Json(ErrorResponse {
                    error: "Account suspended".into(),
                }),
            ));
        }

        Ok(OrgMember { user, member })
    }
}

impl FromRequestParts<AppState> for OptionalOrgMember {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        match OrgMember::from_request_parts(parts, state).await {
            Ok(m) => Ok(OptionalOrgMember(Some(m.member))),
            Err(_) => Ok(OptionalOrgMember(None)),
        }
    }
}

impl FromRequestParts<AppState> for OfficerUser {
    type Rejection = (StatusCode, Json<ErrorResponse>);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let org_member = OrgMember::from_request_parts(parts, state).await?;

        if !org_member.member.org_role.is_at_least(OrgRole::Officer) {
            return Err((
                StatusCode::FORBIDDEN,
                Json(ErrorResponse {
                    error: "Officer access required".into(),
                }),
            ));
        }

        Ok(OfficerUser {
            user: org_member.user,
            member: org_member.member,
        })
    }
}

impl FromRequestParts<AppState> for AdminUser {
    type Rejection = (StatusCode, Json<ErrorResponse>);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let org_member = OrgMember::from_request_parts(parts, state).await?;

        if org_member.member.org_role != OrgRole::Admin {
            return Err((
                StatusCode::FORBIDDEN,
                Json(ErrorResponse {
                    error: "Admin access required".into(),
                }),
            ));
        }

        Ok(AdminUser {
            user: org_member.user,
            member: org_member.member,
        })
    }
}

/// Extractor: daemon token authentication (stat-tracker uploads).
pub struct DaemonUser {
    pub member: Member,
}

/// 401 body for `GET /api/stats/token-check`.
///
/// Missing header, bad token, revoked token, and expired token all use this
/// string so the response does not say which check failed.
pub const TOKEN_CHECK_UNAUTHORIZED: &str = "Unauthorized";

/// How a daemon-token 401 is worded.
#[derive(Clone, Copy)]
enum DaemonUnauthorized {
    /// Upload and daemon-config keep distinct messages. An expired token
    /// uses `{"code":"token_expired"}` instead of the revoked text.
    Distinct,
    /// Token-check and pack downloads: one body for every token failure,
    /// including expiry.
    Opaque,
}

impl DaemonUnauthorized {
    fn missing(self) -> &'static str {
        match self {
            Self::Distinct => "Missing Bearer token",
            Self::Opaque => TOKEN_CHECK_UNAUTHORIZED,
        }
    }

    fn rejected(self) -> &'static str {
        match self {
            Self::Distinct => "Invalid or revoked daemon token",
            Self::Opaque => TOKEN_CHECK_UNAUTHORIZED,
        }
    }
}

/// Daemon-token 401 for upload, daemon-config, and token-check.
///
/// `Cache-Control: no-store` so a shared cache does not keep the failure.
fn unauthorized(error: &str) -> Box<Response> {
    Box::new(
        (
            StatusCode::UNAUTHORIZED,
            [(header::CACHE_CONTROL, "no-store")],
            Json(ErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response(),
    )
}

fn token_expired() -> Box<Response> {
    Box::new(
        Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::CACHE_CONTROL, "no-store")
            .body(axum::body::Body::from(r#"{"code":"token_expired"}"#))
            .expect("token expired response"),
    )
}

fn expired_unauthorized(mode: DaemonUnauthorized) -> Box<Response> {
    match mode {
        DaemonUnauthorized::Distinct => token_expired(),
        DaemonUnauthorized::Opaque => unauthorized(mode.rejected()),
    }
}

fn internal_error() -> Box<Response> {
    Box::new(
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Internal error".into(),
            }),
        )
            .into_response(),
    )
}

fn forbidden(error: &str) -> Box<Response> {
    Box::new(
        (
            StatusCode::FORBIDDEN,
            Json(ErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response(),
    )
}

/// Whether a successful daemon-token check stamps `last_used_at`.
#[derive(Clone, Copy)]
enum DaemonLookup {
    /// Upload, daemon-config, and token-check.
    StampLastUsed,
    /// Pack downloads. The lookup is a read.
    ReadOnly,
}

/// Shared daemon-token auth for upload, daemon-config, token-check, and packs.
///
/// `validate_daemon_token` sets `last_used_at` when the token is accepted.
/// That write already happens on the upload path. Pack downloads pass
/// [`DaemonLookup::ReadOnly`] and do not write. Rejected tokens, including
/// revoked and expired ones, do not update `last_used_at`. An expired token
/// on upload and daemon-config is 401 `{"code":"token_expired"}`.
/// Token-check and pack downloads keep one shared 401 body.
///
/// The raw bearer value is hashed and then dropped. It is not logged.
async fn authenticate_daemon(
    parts: &mut Parts,
    state: &AppState,
    unauthorized_mode: DaemonUnauthorized,
    lookup: DaemonLookup,
) -> Result<DaemonUser, Box<Response>> {
    let auth_header = parts
        .headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| unauthorized(unauthorized_mode.missing()))?;

    let member_id = match lookup {
        DaemonLookup::StampLastUsed => match state.db.validate_daemon_token(auth_header).await {
            Ok(DaemonTokenAuth::Active(id)) => id,
            Ok(DaemonTokenAuth::Expired) => return Err(expired_unauthorized(unauthorized_mode)),
            Ok(DaemonTokenAuth::Missing) => {
                return Err(unauthorized(unauthorized_mode.rejected()));
            }
            Err(_e) => return Err(internal_error()),
        },
        DaemonLookup::ReadOnly => state
            .db
            .lookup_daemon_token(auth_header)
            .await
            .map_err(|_e| internal_error())?
            .ok_or_else(|| unauthorized(unauthorized_mode.rejected()))?,
    };

    let member = state
        .db
        .get_member(&member_id)
        .await
        .map_err(|_e| internal_error())?
        .ok_or_else(|| forbidden("Member not found"))?;

    if !member.is_active {
        return Err(forbidden("Membership inactive"));
    }

    let suspended_or_banned = state
        .db
        .is_member_suspended_or_banned(&member.id)
        .await
        .map_err(|_e| internal_error())?;

    if suspended_or_banned {
        return Err(forbidden("Member is suspended or banned"));
    }

    Ok(DaemonUser { member })
}

impl FromRequestParts<AppState> for DaemonUser {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        authenticate_daemon(
            parts,
            state,
            DaemonUnauthorized::Distinct,
            DaemonLookup::StampLastUsed,
        )
        .await
        .map_err(|err| *err)
    }
}

/// Daemon token auth for `GET /api/stats/token-check`.
///
/// Same checks as [`DaemonUser`]. Token failures share one 401 body.
pub struct OpaqueDaemonUser {
    pub member: Member,
}

impl FromRequestParts<AppState> for OpaqueDaemonUser {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let DaemonUser { member } = authenticate_daemon(
            parts,
            state,
            DaemonUnauthorized::Opaque,
            DaemonLookup::StampLastUsed,
        )
        .await
        .map_err(|err| *err)?;
        Ok(OpaqueDaemonUser { member })
    }
}

/// Daemon token auth for recognizer pack downloads.
///
/// Same member checks as [`DaemonUser`]. Missing, unknown, revoked, and
/// expired tokens share the token-check 401 body. The lookup does not write.
pub struct PackDaemonUser;

impl FromRequestParts<AppState> for PackDaemonUser {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        authenticate_daemon(
            parts,
            state,
            DaemonUnauthorized::Opaque,
            DaemonLookup::ReadOnly,
        )
        .await
        .map(|_| PackDaemonUser)
        .map_err(|err| *err)
    }
}

/// Which credential a [`ReportCreator`] presented.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReportCredential {
    /// Site session (cookie, or a session token as `Authorization: Bearer`).
    Session,
    /// Tracker sync token as `Authorization: Bearer`.
    SyncToken,
}

impl ReportCredential {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::SyncToken => "sync_token",
        }
    }
}

/// Auth for `POST /api/stat-reports` only (report-bundle spec section 8).
///
/// Tries the site session first, with the same checks as [`OrgMember`]. If
/// that is a 401 and the request carries `Authorization: Bearer`, the value
/// is tried as the tracker sync token through the same path stats sync uses
/// ([`authenticate_daemon`]): hashed, matched against active unexpired
/// `daemon_token` rows, then the owner must be an active member who is not
/// suspended or banned (403 otherwise). A non-401 session failure (403 not a
/// member, inactive, suspended; 500) is returned as is.
///
/// Every other report route keeps [`OrgMember`] / [`OfficerUser`], so a sync
/// token there fails the session lookup and is a 401.
pub struct ReportCreator {
    pub member: Member,
    pub credential: ReportCredential,
}

impl FromRequestParts<AppState> for ReportCreator {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let session_reject = match OrgMember::from_request_parts(parts, state).await {
            Ok(org) => {
                return Ok(ReportCreator {
                    member: org.member,
                    credential: ReportCredential::Session,
                });
            }
            Err(reject) => reject,
        };
        let has_bearer = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("Bearer "));
        if session_reject.0 != StatusCode::UNAUTHORIZED || !has_bearer {
            return Err(session_reject.into_response());
        }
        let DaemonUser { member } = authenticate_daemon(
            parts,
            state,
            DaemonUnauthorized::Opaque,
            DaemonLookup::StampLastUsed,
        )
        .await
        .map_err(|err| *err)?;
        Ok(ReportCreator {
            member,
            credential: ReportCredential::SyncToken,
        })
    }
}

#[cfg(test)]
mod presented_token_tests {
    use super::presented_session_token;
    use axum::http::{HeaderMap, header::AUTHORIZATION};
    use axum_extra::extract::cookie::{Cookie, CookieJar};

    fn jar_with_session() -> CookieJar {
        CookieJar::new().add(Cookie::new("sc_session", "cookie-token"))
    }

    #[test]
    fn bearer_is_used_when_authorization_is_present() {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, "Bearer desk-token".parse().unwrap());
        let token = presented_session_token("sc_session", &headers, &jar_with_session());
        assert_eq!(token.as_deref(), Some("desk-token"));
    }

    #[test]
    fn malformed_authorization_does_not_fall_back_to_the_cookie() {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, "Basic nope".parse().unwrap());
        let token = presented_session_token("sc_session", &headers, &jar_with_session());
        assert_eq!(token, None);
    }

    #[test]
    fn cookie_is_used_when_authorization_is_absent() {
        let headers = HeaderMap::new();
        let token = presented_session_token("sc_session", &headers, &jar_with_session());
        assert_eq!(token.as_deref(), Some("cookie-token"));
    }
}
