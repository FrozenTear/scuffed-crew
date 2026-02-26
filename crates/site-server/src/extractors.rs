use axum::{
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
    Json,
};

use scuffed_auth::server::session::ErrorResponse;
use scuffed_auth::server::AuthUser;
use scuffed_auth::User;
use scuffed_db::{Member, OrgRole};

use crate::state::AppState;

/// Extractor: any authenticated org member.
pub struct OrgMember {
    pub user: User,
    pub member: Member,
}

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

        let member = state
            .db
            .get_member_by_user(&user.id)
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

        Ok(OrgMember { user, member })
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
