use axum::{
    Json,
    extract::FromRequestParts,
    http::{StatusCode, header, header::AUTHORIZATION, request::Parts},
    response::{IntoResponse, Response},
};

use scuffed_auth::User;
use scuffed_auth::server::AuthUser;
use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{Member, OrgRole};

use crate::state::AppState;

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
    /// Upload and daemon-config keep their existing distinct messages.
    Distinct,
    /// Token-check: one body for every token failure.
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

/// Shared daemon-token auth for upload, daemon-config, and token-check.
///
/// `validate_daemon_token` sets `last_used_at` when the token is accepted.
/// That write already happens on the upload path. Rejected tokens, including
/// revoked and expired ones, do not update `last_used_at`.
async fn authenticate_daemon(
    parts: &mut Parts,
    state: &AppState,
    unauthorized_mode: DaemonUnauthorized,
) -> Result<DaemonUser, Box<Response>> {
    let auth_header = parts
        .headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| unauthorized(unauthorized_mode.missing()))?;

    let member_id = state
        .db
        .validate_daemon_token(auth_header)
        .await
        .map_err(|_e| internal_error())?
        .ok_or_else(|| unauthorized(unauthorized_mode.rejected()))?;

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
        authenticate_daemon(parts, state, DaemonUnauthorized::Distinct)
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
        let DaemonUser { member } = authenticate_daemon(parts, state, DaemonUnauthorized::Opaque)
            .await
            .map_err(|err| *err)?;
        Ok(OpaqueDaemonUser { member })
    }
}
