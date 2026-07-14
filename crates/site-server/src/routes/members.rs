use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;

use scuffed_auth::server::session::ErrorResponse;
use scuffed_chat::{EventBuilder, publish_event_oneshot};
use scuffed_db::{AuditAction, AuditTargetType, GameAccount, Member, NostrKeyMode, OrgRole};
use scuffed_types::api::{CursorResponse, PaginationParams};

use crate::extractors::{AdminUser, OrgMember};
use crate::routes::audit_log::audit;
use crate::state::AppState;

/// GET /api/members — list active members (cursor-paginated)
pub async fn list_members(
    State(state): State<AppState>,
    _member: OrgMember,
    axum::extract::Query(pagination): axum::extract::Query<PaginationParams>,
) -> Result<Json<CursorResponse<Member>>, (StatusCode, Json<ErrorResponse>)> {
    let (limit, offset) = pagination.resolve();
    let items = state
        .db
        .list_members_paginated(limit, offset)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
        })?;
    Ok(Json(CursorResponse::from_oversized(items, limit, offset)))
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
    pub avatar_url: Option<Option<String>>,
    pub timezone: Option<Option<String>>,
    pub pronouns: Option<Option<String>>,
    pub availability_status: Option<Option<String>>,
    pub nostr_pubkey: Option<Option<String>>,
    pub is_active: Option<bool>,
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
            tracing::error!(error = %e, "get_member failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal server error".into(),
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

    // nostr_pubkey must go through /api/nostr/challenge + /api/nostr/verify
    // (signature proof). Reject arbitrary sets/clears here to prevent NIP-05
    // impersonation bypass. Field ignored if client still sends it.
    if body.nostr_pubkey.is_some() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Nostr pubkey can only be set via the challenge/verify flow".into(),
            }),
        ));
    }

    // is_active is officer+ only, with last-admin and hierarchy guards
    let mut is_active = body.is_active;
    if let Some(new_active) = is_active {
        if new_active == target.is_active {
            is_active = None; // no-op
        } else {
            let admin_count = state.db.count_actionable_admins().await.map_err(|e| {
                tracing::error!(error = %e, "count_actionable_admins failed");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: "Internal server error".into(),
                    }),
                )
            })?;
            let target_suspended = state
                .db
                .is_member_suspended_or_banned(&target.id)
                .await
                .map_err(|e| {
                    tracing::error!(error = %e, "suspension check for is_active");
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse {
                            error: "Internal server error".into(),
                        }),
                    )
                })?;
            let target_is_actionable_admin = target.is_active
                && target.org_role == OrgRole::Admin
                && !target_suspended;
            if let Err(msg) = crate::membership_policy::can_set_is_active(
                &caller.member.id,
                caller.member.org_role,
                &target.id,
                target.org_role,
                target.is_active,
                new_active,
                admin_count,
                target_is_actionable_admin,
            ) {
                return Err((
                    StatusCode::FORBIDDEN,
                    Json(ErrorResponse {
                        error: msg.into(),
                    }),
                ));
            }
        }
    }

    let updated = state
        .db
        .update_member(
            &id,
            body.display_name.as_deref(),
            body.bio.as_ref().map(|b| b.as_deref()),
            body.avatar_url.as_ref().map(|a| a.as_deref()),
            body.timezone.as_ref().map(|t| t.as_deref()),
            body.pronouns.as_ref().map(|p| p.as_deref()),
            body.availability_status.as_ref().map(|a| a.as_deref()),
            None, // never set pubkey from this route
            is_active,
        )
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "update_member failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal server error".into(),
                }),
            )
        })?;

    if crate::membership_policy::deactivation_revokes_sessions(target.is_active, updated.is_active)
    {
        if let Err(e) = state.db.delete_sessions_for_user(&target.user_id).await {
            tracing::error!(error = %e, user_id = %target.user_id, "failed to revoke sessions on deactivate");
        }
        audit(
            &state.db,
            &caller.member.id,
            AuditAction::DeactivatedMember,
            AuditTargetType::Member,
            &id,
            Some("Member deactivated; sessions revoked"),
        )
        .await;
    } else {
        audit(
            &state.db,
            &caller.member.id,
            AuditAction::UpdatedMember,
            AuditTargetType::Member,
            &id,
            None,
        )
        .await;
    }

    // Fire-and-forget: publish NIP-01 kind 0 profile metadata to relay
    publish_profile_metadata(&state, &updated);

    Ok(Json(updated))
}

/// Spawn a fire-and-forget task to publish a NIP-01 kind 0 profile metadata event
/// for a member with a server-managed Nostr key.
fn publish_profile_metadata(state: &AppState, member: &Member) {
    let relay_url = match state.relay_url {
        Some(ref url) => url.clone(),
        None => return,
    };

    // Only publish for members with server-managed keys
    if member.nostr_key_mode != Some(NostrKeyMode::ServerManaged) || member.nostr_pubkey.is_none() {
        return;
    }

    let db = state.db.clone();
    let member_id = member.id.clone();
    let display_name = member.display_name.clone();
    let bio = member.bio.clone();
    let avatar_url = member.avatar_url.clone();

    tokio::spawn(async move {
        let secret_hex = match db.get_nostr_secret_key(&member_id).await {
            Ok(Some(s)) => s,
            Ok(None) => {
                tracing::debug!("No server-managed secret key for member {member_id}");
                return;
            }
            Err(e) => {
                tracing::error!("Failed to decrypt Nostr secret key for profile metadata: {e}");
                return;
            }
        };

        let keys = match EventBuilder::keys_from_hex(&secret_hex) {
            Ok(k) => k,
            Err(e) => {
                tracing::error!("Invalid Nostr secret key for profile metadata: {e}");
                return;
            }
        };

        // Build NIP-05 identifier from display name
        let nip05_name: String = display_name
            .to_lowercase()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        let nip05 = if nip05_name.is_empty() {
            None
        } else {
            Some(format!("{nip05_name}@scuffed.gg"))
        };

        let event = match EventBuilder::build_profile_metadata(
            &keys,
            &display_name,
            bio.as_deref(),
            avatar_url.as_deref(),
            nip05.as_deref(),
            None,
        ) {
            Ok(e) => e,
            Err(e) => {
                tracing::error!("Failed to build kind 0 profile metadata event: {e}");
                return;
            }
        };

        let relay_event = EventBuilder::to_relay_event(&event);
        if let Err(e) = publish_event_oneshot(&relay_url, relay_event).await {
            tracing::error!("Failed to publish kind 0 profile metadata: {e}");
        } else {
            tracing::info!("Published kind 0 profile metadata for member {member_id}");
        }
    });
}

#[derive(Deserialize)]
pub struct ChangeRoleRequest {
    pub role: OrgRole,
}

/// PATCH /api/members/:id/role — change org role (admin only)
pub async fn change_role(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<String>,
    Json(body): Json<ChangeRoleRequest>,
) -> Result<Json<Member>, (StatusCode, Json<ErrorResponse>)> {
    let target = state
        .db
        .get_member(&id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "get_member for change_role");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal server error".into(),
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

    let admin_count = state.db.count_actionable_admins().await.map_err(|e| {
        tracing::error!(error = %e, "count_actionable_admins");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Internal server error".into(),
            }),
        )
    })?;
    let target_suspended = state
        .db
        .is_member_suspended_or_banned(&target.id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "suspension check for change_role");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal server error".into(),
                }),
            )
        })?;
    let target_is_actionable_admin =
        target.is_active && target.org_role == OrgRole::Admin && !target_suspended;

    if let Err(msg) = crate::membership_policy::can_change_role(
        &admin.member.id,
        &target.id,
        target.org_role,
        target.is_active,
        body.role,
        admin_count,
        target_is_actionable_admin,
    ) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: msg.into(),
            }),
        ));
    }

    let member = state
        .db
        .change_member_role(&id, body.role)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "change_member_role");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal server error".into(),
                }),
            )
        })?;

    audit(
        &state.db,
        &admin.member.id,
        AuditAction::ChangedRole,
        AuditTargetType::Member,
        &id,
        Some(&format!("{} → {}", target.org_role, body.role)),
    )
    .await;

    Ok(Json(member))
}

/// GET /api/members/:id/game-accounts — list game accounts for a member
pub async fn list_game_accounts(
    State(state): State<AppState>,
    _member: OrgMember,
    Path(member_id): Path<String>,
) -> Result<Json<Vec<GameAccount>>, (StatusCode, Json<ErrorResponse>)> {
    state
        .db
        .list_member_game_accounts(&member_id)
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

#[derive(Deserialize)]
pub struct UpsertGameAccountRequest {
    pub game_id: String,
    pub account_name: String,
    pub account_id: Option<String>,
}

/// PUT /api/members/:id/game-accounts — upsert game account (self or officer+)
pub async fn upsert_game_account(
    State(state): State<AppState>,
    caller: OrgMember,
    Path(member_id): Path<String>,
    Json(body): Json<UpsertGameAccountRequest>,
) -> Result<Json<GameAccount>, (StatusCode, Json<ErrorResponse>)> {
    let is_self = caller.member.id == member_id;
    let is_officer = caller.member.org_role.is_at_least(OrgRole::Officer);

    if !is_self && !is_officer {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ErrorResponse {
                error: "Can only edit your own game accounts".into(),
            }),
        ));
    }

    state
        .db
        .upsert_game_account(
            &member_id,
            &body.game_id,
            &body.account_name,
            body.account_id.as_deref(),
        )
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

/// DELETE /api/members/:member_id/game-accounts/:id — delete game account (self or officer+)
pub async fn delete_game_account(
    State(state): State<AppState>,
    caller: OrgMember,
    Path((member_id, account_id)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let is_self = caller.member.id == member_id;
    let is_officer = caller.member.org_role.is_at_least(OrgRole::Officer);

    if !is_self && !is_officer {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ErrorResponse {
                error: "Can only delete your own game accounts".into(),
            }),
        ));
    }

    state
        .db
        .delete_game_account(&member_id, &account_id)
        .await
        .map_err(|e| match e {
            scuffed_db::DbError::NotFound(msg) => (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse { error: msg }),
            ),
            other => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: other.to_string(),
                }),
            ),
        })?;

    Ok(StatusCode::OK)
}
