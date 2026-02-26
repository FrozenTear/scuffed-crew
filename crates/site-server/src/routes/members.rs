use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;

use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{Member, OrgRole};

use crate::extractors::{AdminUser, OrgMember};
use crate::state::AppState;

/// GET /api/members — list all active members
pub async fn list_members(
    State(state): State<AppState>,
    _member: OrgMember,
) -> Result<Json<Vec<Member>>, (StatusCode, Json<ErrorResponse>)> {
    state.db.list_members().await.map(Json).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
    })
}

/// GET /api/members/:id — get member profile
pub async fn get_member(
    State(state): State<AppState>,
    _member: OrgMember,
    Path(id): Path<String>,
) -> Result<Json<Member>, (StatusCode, Json<ErrorResponse>)> {
    state
        .db
        .get_member(&id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
        })?
        .map(Json)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Member not found".into(),
                }),
            )
        })
}

#[derive(Deserialize)]
pub struct UpdateMemberRequest {
    pub display_name: Option<String>,
    pub bio: Option<Option<String>>,
}

/// PUT /api/members/:id — update member profile (self or officer+)
pub async fn update_member(
    State(state): State<AppState>,
    caller: OrgMember,
    Path(id): Path<String>,
    Json(body): Json<UpdateMemberRequest>,
) -> Result<Json<Member>, (StatusCode, Json<ErrorResponse>)> {
    // Members can edit themselves; officers+ can edit anyone
    let target = state
        .db
        .get_member(&id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Member not found".into(),
                }),
            )
        })?;

    let is_self = target.user_id == caller.user.id;
    let is_officer = caller.member.org_role.is_at_least(OrgRole::Officer);

    if !is_self && !is_officer {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ErrorResponse {
                error: "Can only edit your own profile".into(),
            }),
        ));
    }

    let updated = state
        .db
        .update_member(
            &id,
            body.display_name.as_deref(),
            body.bio
                .as_ref()
                .map(|b| b.as_deref()),
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
        })?;

    Ok(Json(updated))
}

#[derive(Deserialize)]
pub struct ChangeRoleRequest {
    pub role: OrgRole,
}

/// PATCH /api/members/:id/role — change org role (admin only)
pub async fn change_role(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<String>,
    Json(body): Json<ChangeRoleRequest>,
) -> Result<Json<Member>, (StatusCode, Json<ErrorResponse>)> {
    state
        .db
        .change_member_role(&id, body.role)
        .await
        .map(Json)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
        })
}
