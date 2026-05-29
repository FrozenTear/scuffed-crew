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

/// Validate a Nostr pubkey: must be a 64-character lowercase hex string.
fn validate_nostr_pubkey(pubkey: &str) -> bool {
    pubkey.len() == 64 && pubkey.chars().all(|c| c.is_ascii_hexdigit())
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

    // Validate nostr_pubkey if provided
    if let Some(Some(ref pubkey)) = body.nostr_pubkey
        && !validate_nostr_pubkey(pubkey)
    {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Invalid Nostr pubkey: must be a 64-character hex string".into(),
            }),
        ));
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
            body.nostr_pubkey.as_ref().map(|n| n.as_deref()),
            body.is_active,
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

    audit(
        &state.db,
        &caller.member.id,
        AuditAction::UpdatedMember,
        AuditTargetType::Member,
        &id,
        None,
    )
    .await;

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
    let member = state
        .db
        .change_member_role(&id, body.role)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
        })?;

    audit(
        &state.db,
        &admin.member.id,
        AuditAction::ChangedRole,
        AuditTargetType::Member,
        &id,
        Some(&format!("Changed role to {}", body.role)),
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
        .delete_game_account(&account_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
        })?;

    Ok(StatusCode::OK)
}
