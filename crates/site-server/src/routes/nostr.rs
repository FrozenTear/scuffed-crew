use std::collections::HashMap;

use axum::{
    extract::{Query, State},
    http::{header, StatusCode},
    response::IntoResponse,
    Json,
};
use rand::RngCore;
use serde::{Deserialize, Serialize};

use scuffed_auth::server::session::ErrorResponse;

use nostr::key::Keys;
use nostr::{FromBech32, SecretKey};

use scuffed_db::NostrKeyMode;

use scuffed_chat::nostr::events::EventBuilder;
use scuffed_chat::nostr::relay::publish_event_oneshot;

use crate::extractors::{OfficerUser, OrgMember};
use crate::state::AppState;

// ─── NIP-05 well-known endpoint (Phase 1) ───

#[derive(Deserialize)]
pub struct Nip05Query {
    pub name: Option<String>,
}

#[derive(Serialize)]
pub struct Nip05Response {
    pub names: HashMap<String, String>,
    pub relays: HashMap<String, Vec<String>>,
}

/// Normalize a display name to a NIP-05 local name: lowercase, keep alphanumeric + underscores.
fn normalize_nip05_name(display_name: &str) -> String {
    display_name
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect()
}

/// GET /.well-known/nostr.json — NIP-05 identity verification endpoint.
pub async fn nostr_json(
    State(state): State<AppState>,
    Query(query): Query<Nip05Query>,
) -> impl IntoResponse {
    let members = match state.db.list_nostr_identities().await {
        Ok(m) => m,
        Err(e) => {
            tracing::error!("Failed to list Nostr identities: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")],
                Json(Nip05Response {
                    names: HashMap::new(),
                    relays: HashMap::new(),
                }),
            );
        }
    };

    let mut names = HashMap::new();
    let mut relays: HashMap<String, Vec<String>> = HashMap::new();
    let requested_name = query.name.unwrap_or_default().to_lowercase();

    for member in &members {
        if let Some(ref pubkey) = member.nostr_pubkey {
            let nip05_name = normalize_nip05_name(&member.display_name);
            if nip05_name.is_empty() {
                continue;
            }
            if requested_name == "_" || requested_name == nip05_name {
                names.insert(nip05_name, pubkey.clone());
                // Add relay hints for this pubkey if relay URL is configured
                if let Some(ref relay_url) = state.relay_url {
                    relays.insert(pubkey.clone(), vec![relay_url.clone()]);
                }
            }
        }
    }

    (
        StatusCode::OK,
        [(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")],
        Json(Nip05Response {
            names,
            relays,
        }),
    )
}

// ─── Phase 1.5: Challenge-response Nostr identity verification ───

const CHALLENGE_TTL_SECS: u64 = 300; // 5 minutes

#[derive(Deserialize)]
pub struct ChallengeRequest {
    pub pubkey: String,
}

#[derive(Serialize)]
pub struct ChallengeResponse {
    pub challenge: String,
    pub token: String,
    pub pubkey_hex: String,
    pub expires_in_secs: u64,
}

#[derive(Deserialize)]
pub struct VerifyRequest {
    pub token: String,
    pub signed_event: nostr::Event,
}

/// Resolve a pubkey string: accept 64-char hex or npub1 bech32 format.
fn resolve_pubkey_hex(input: &str) -> Result<String, &'static str> {
    let trimmed = input.trim();

    if trimmed.len() == 64 && trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
        return Ok(trimmed.to_lowercase());
    }

    if trimmed.starts_with("npub1") {
        let pk = nostr::PublicKey::from_bech32(trimmed)
            .map_err(|_| "Invalid npub address")?;
        return Ok(pk.to_hex());
    }

    Err("Pubkey must be a 64-character hex string or npub1... bech32 address")
}

/// Create an HMAC token.
///
/// Token format: `{challenge}|{member_id}|{expires_ts}|{hmac_hex}` (pipe-delimited,
/// because the challenge contains colons). HMAC covers all three fields.
fn sign_challenge_token(
    key: &[u8; 32],
    challenge: &str,
    member_id: &str,
    expires_ts: u64,
) -> String {
    let hmac_data = format!("{challenge}:{member_id}:{expires_ts}");
    let hash = blake3::keyed_hash(key, hmac_data.as_bytes());
    let hmac_hex = hash.to_hex();
    let token_raw = format!("{challenge}|{member_id}|{expires_ts}|{hmac_hex}");
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(token_raw.as_bytes())
}

/// Parse and verify a challenge token. Returns (challenge, member_id).
fn verify_challenge_token(
    key: &[u8; 32],
    token: &str,
) -> Result<(String, String), &'static str> {
    use base64::Engine;
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(token)
        .map_err(|_| "Invalid token encoding")?;
    let token_str = String::from_utf8(decoded).map_err(|_| "Invalid token encoding")?;

    // Split on pipe: challenge|member_id|expires_ts|hmac_hex
    let parts: Vec<&str> = token_str.splitn(4, '|').collect();
    if parts.len() != 4 {
        return Err("Malformed token");
    }

    let challenge = parts[0];
    let member_id = parts[1];
    let expires_ts: u64 = parts[2].parse().map_err(|_| "Invalid expiry")?;
    let provided_hmac = parts[3];

    // Check expiry
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    if now > expires_ts {
        return Err("Challenge expired");
    }

    // Verify HMAC (data uses colon separator — only the token wire format uses pipes)
    let hmac_data = format!("{challenge}:{member_id}:{expires_ts}");
    let expected = blake3::keyed_hash(key, hmac_data.as_bytes());
    let expected_hex = expected.to_hex();

    if expected_hex.as_str() != provided_hmac {
        return Err("Invalid token signature");
    }

    Ok((challenge.to_string(), member_id.to_string()))
}

/// POST /api/nostr/challenge — generate a challenge for the member to sign.
pub async fn nostr_challenge(
    State(state): State<AppState>,
    caller: OrgMember,
    Json(body): Json<ChallengeRequest>,
) -> Result<Json<ChallengeResponse>, (StatusCode, Json<ErrorResponse>)> {
    let pubkey_hex = resolve_pubkey_hex(&body.pubkey).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
    })?;

    nostr::PublicKey::from_hex(&pubkey_hex).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Invalid secp256k1 public key".into(),
            }),
        )
    })?;

    // Generate random challenge
    let mut challenge_bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut challenge_bytes);
    let challenge_hex: String = challenge_bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let challenge = format!("scuffedclan-verify:{challenge_hex}");

    let expires_ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + CHALLENGE_TTL_SECS;

    let token = sign_challenge_token(
        &state.nostr_challenge_key,
        &challenge,
        &caller.member.id,
        expires_ts,
    );

    Ok(Json(ChallengeResponse {
        challenge,
        token,
        pubkey_hex,
        expires_in_secs: CHALLENGE_TTL_SECS,
    }))
}

/// POST /api/nostr/verify — verify a signed Nostr event and link the pubkey.
pub async fn nostr_verify(
    State(state): State<AppState>,
    caller: OrgMember,
    Json(body): Json<VerifyRequest>,
) -> Result<Json<scuffed_db::Member>, (StatusCode, Json<ErrorResponse>)> {
    // 1. Verify the challenge token
    let (challenge, token_member_id) =
        verify_challenge_token(&state.nostr_challenge_key, &body.token).map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Token verification failed: {e}"),
                }),
            )
        })?;

    // 2. Ensure the token was issued for this member
    if token_member_id != caller.member.id {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ErrorResponse {
                error: "Token was not issued for your account".into(),
            }),
        ));
    }

    // 3. Reject non-ephemeral event kinds (must be 22242 / NIP-42 AUTH)
    if body.signed_event.kind != nostr::Kind::Custom(22242) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Event must use ephemeral kind 22242".into(),
            }),
        ));
    }

    // 4. Verify event content matches the challenge
    if body.signed_event.content != challenge {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Event content does not match the challenge".into(),
            }),
        ));
    }

    // 5. Verify event ID and signature
    body.signed_event.verify().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Event verification failed: {e}"),
            }),
        )
    })?;

    let pubkey_hex = body.signed_event.pubkey.to_hex();

    // 6. Update member's nostr_pubkey
    let updated = state
        .db
        .update_member(
            &caller.member.id,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(Some(pubkey_hex.as_str())),
            None,
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

    crate::routes::audit_log::audit(
        &state.db,
        &caller.member.id,
        scuffed_db::AuditAction::UpdatedMember,
        scuffed_db::AuditTargetType::Member,
        &caller.member.id,
        Some("Linked verified Nostr identity"),
    )
    .await;

    Ok(Json(updated))
}

/// DELETE /api/nostr/identity — remove the caller's Nostr pubkey.
pub async fn nostr_unlink(
    State(state): State<AppState>,
    caller: OrgMember,
) -> Result<Json<scuffed_db::Member>, (StatusCode, Json<ErrorResponse>)> {
    let updated = state
        .db
        .update_member(
            &caller.member.id,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(None),
            None,
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

    crate::routes::audit_log::audit(
        &state.db,
        &caller.member.id,
        scuffed_db::AuditAction::UpdatedMember,
        scuffed_db::AuditTargetType::Member,
        &caller.member.id,
        Some("Unlinked Nostr identity"),
    )
    .await;

    Ok(Json(updated))
}

// ─── NIP-49 encrypted key backup ───

#[derive(Deserialize)]
pub struct ExportBackupRequest {
    pub password: String,
}

#[derive(Serialize)]
pub struct ExportBackupResponse {
    pub ncryptsec: String,
}

/// POST /api/nostr/export-backup — export server-managed key as ncryptsec.
pub async fn nostr_export_backup(
    State(state): State<AppState>,
    caller: OrgMember,
    Json(body): Json<ExportBackupRequest>,
) -> Result<Json<ExportBackupResponse>, (StatusCode, Json<ErrorResponse>)> {
    if body.password.len() < 8 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Password must be at least 8 characters".into(),
            }),
        ));
    }

    if caller.member.nostr_key_mode != Some(NostrKeyMode::ServerManaged) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Key backup is only available for server-managed keys".into(),
            }),
        ));
    }

    let secret_hex = state
        .db
        .get_nostr_secret_key(&caller.member.id)
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
                    error: "No server-managed key found".into(),
                }),
            )
        })?;

    let ncryptsec = scuffed_auth::nip49::encrypt(&secret_hex, &body.password).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Encryption failed: {e}"),
            }),
        )
    })?;

    crate::routes::audit_log::audit(
        &state.db,
        &caller.member.id,
        scuffed_db::AuditAction::UpdatedMember,
        scuffed_db::AuditTargetType::Member,
        &caller.member.id,
        Some("Exported Nostr key backup (ncryptsec)"),
    )
    .await;

    Ok(Json(ExportBackupResponse { ncryptsec }))
}

#[derive(Deserialize)]
pub struct ImportKeyRequest {
    pub ncryptsec: String,
    pub password: String,
}

/// POST /api/nostr/import-key — import a key from ncryptsec backup.
pub async fn nostr_import_key(
    State(state): State<AppState>,
    caller: OrgMember,
    Json(body): Json<ImportKeyRequest>,
) -> Result<Json<scuffed_db::Member>, (StatusCode, Json<ErrorResponse>)> {
    let secret_hex =
        scuffed_auth::nip49::decrypt(&body.ncryptsec, &body.password).map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Failed to decrypt: {e}"),
                }),
            )
        })?;

    // Derive pubkey from the decrypted secret
    let keys = Keys::new(
        SecretKey::from_hex(&secret_hex).map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Invalid key in backup: {e}"),
                }),
            )
        })?,
    );
    let pubkey_hex = keys.public_key().to_hex();

    // Update member to external mode with this key
    state
        .db
        .set_external_nostr_key(&caller.member.id, &pubkey_hex)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
        })?;

    let updated = state
        .db
        .get_member(&caller.member.id)
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

    crate::routes::audit_log::audit(
        &state.db,
        &caller.member.id,
        scuffed_db::AuditAction::UpdatedMember,
        scuffed_db::AuditTargetType::Member,
        &caller.member.id,
        Some("Imported Nostr key from ncryptsec backup"),
    )
    .await;

    Ok(Json(updated))
}

// ─── NIP-72 Community Definition (Phase 2) ───

#[derive(Deserialize)]
pub struct CommunityRequest {
    pub community_id: String,
    pub name: String,
    pub description: Option<String>,
    pub rules: Option<String>,
    pub image: Option<String>,
}

#[derive(Serialize)]
pub struct CommunityResponse {
    pub event_id: String,
    pub community_id: String,
}

/// POST /api/nostr/community — publish or update a NIP-72 community definition.
///
/// Officer+ only. Moderator pubkeys are auto-resolved from all Officers and Admins.
pub async fn nostr_community(
    State(state): State<AppState>,
    caller: OfficerUser,
    Json(body): Json<CommunityRequest>,
) -> Result<Json<CommunityResponse>, (StatusCode, Json<ErrorResponse>)> {
    let relay_url = state.relay_url.clone().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "Relay not configured".into(),
            }),
        )
    })?;

    if caller.member.nostr_key_mode != Some(NostrKeyMode::ServerManaged) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Server-managed Nostr key required to publish community events".into(),
            }),
        ));
    }

    let secret_hex = state
        .db
        .get_nostr_secret_key(&caller.member.id)
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
                    error: "No server-managed key found".into(),
                }),
            )
        })?;

    let keys = EventBuilder::keys_from_hex(&secret_hex).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Invalid key: {e}"),
            }),
        )
    })?;

    let members = state.db.list_nostr_identities().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
    })?;

    let moderator_pubkeys: Vec<String> = members
        .iter()
        .filter(|m| m.org_role.is_at_least(scuffed_db::OrgRole::Officer))
        .filter_map(|m| m.nostr_pubkey.clone())
        .collect();

    let event = EventBuilder::build_community_definition(
        &keys,
        &body.community_id,
        &body.name,
        body.description.as_deref(),
        body.rules.as_deref(),
        body.image.as_deref(),
        &moderator_pubkeys,
    )
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to build event: {e}"),
            }),
        )
    })?;

    let event_id = event.id.to_hex();
    let community_id = body.community_id.clone();
    let relay_event = EventBuilder::to_relay_event(&event);

    let db = state.db.clone();
    let member_id = caller.member.id.clone();
    tokio::spawn(async move {
        if let Err(e) = publish_event_oneshot(&relay_url, relay_event).await {
            tracing::error!("Failed to publish community definition: {e}");
        } else {
            tracing::info!("Published NIP-72 community definition for {community_id}");
        }

        crate::routes::audit_log::audit(
            &db,
            &member_id,
            scuffed_db::AuditAction::PublishedCommunity,
            scuffed_db::AuditTargetType::Settings,
            &community_id,
            Some("Published NIP-72 community definition"),
        )
        .await;
    });

    Ok(Json(CommunityResponse {
        event_id,
        community_id: body.community_id,
    }))
}

// ─── NIP-25 Reactions (Phase 2) ───

#[derive(Deserialize)]
pub struct ReactRequest {
    pub event_id: String,
    pub event_author_pubkey: String,
    #[serde(default = "default_reaction")]
    pub content: String,
}

fn default_reaction() -> String {
    "+".to_string()
}

#[derive(Serialize)]
pub struct ReactResponse {
    pub reaction_event_id: String,
}

/// POST /api/nostr/react — publish a NIP-25 reaction to a Nostr event.
pub async fn nostr_react(
    State(state): State<AppState>,
    caller: OrgMember,
    Json(body): Json<ReactRequest>,
) -> Result<Json<ReactResponse>, (StatusCode, Json<ErrorResponse>)> {
    let relay_url = state.relay_url.clone().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "Relay not configured".into(),
            }),
        )
    })?;

    if caller.member.nostr_key_mode != Some(NostrKeyMode::ServerManaged) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Server-managed Nostr key required to publish reactions".into(),
            }),
        ));
    }

    if body.content.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Reaction content must not be empty".into(),
            }),
        ));
    }

    let secret_hex = state
        .db
        .get_nostr_secret_key(&caller.member.id)
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
                    error: "No server-managed key found".into(),
                }),
            )
        })?;

    let keys = EventBuilder::keys_from_hex(&secret_hex).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Invalid key: {e}"),
            }),
        )
    })?;

    let event = EventBuilder::build_reaction(
        &keys,
        &body.event_id,
        &body.event_author_pubkey,
        &body.content,
    )
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to build event: {e}"),
            }),
        )
    })?;

    let reaction_event_id = event.id.to_hex();
    let relay_event = EventBuilder::to_relay_event(&event);
    let target_event_id = body.event_id.clone();

    let db = state.db.clone();
    let member_id = caller.member.id.clone();
    tokio::spawn(async move {
        if let Err(e) = publish_event_oneshot(&relay_url, relay_event).await {
            tracing::error!("Failed to publish reaction: {e}");
        } else {
            tracing::info!("Published NIP-25 reaction to event {target_event_id}");
        }

        crate::routes::audit_log::audit(
            &db,
            &member_id,
            scuffed_db::AuditAction::PublishedReaction,
            scuffed_db::AuditTargetType::Member,
            &target_event_id,
            Some("Published NIP-25 reaction"),
        )
        .await;
    });

    Ok(Json(ReactResponse { reaction_event_id }))
}

// ─── Feed endpoint (Phase 4: relay read path) ───

#[derive(Deserialize)]
pub struct FeedQuery {
    pub limit: Option<usize>,
    pub since: Option<u64>,
    pub hashtag: Option<String>,
}

#[derive(Serialize)]
pub struct FeedPostResponse {
    pub id: String,
    pub pubkey: String,
    pub author_name: Option<String>,
    pub content: String,
    pub hashtags: Vec<String>,
    pub created_at: i64,
    pub reactions: Vec<serde_json::Value>,
    pub reply_count: u32,
}

/// GET /api/nostr/feed — query community posts from the Nostr relay.
///
/// Applies per-group read ACLs: events tagged with officer-only groups
/// are filtered out for unauthenticated or non-officer callers.
pub async fn nostr_feed(
    State(state): State<AppState>,
    jar: axum_extra::extract::cookie::CookieJar,
    Query(query): Query<FeedQuery>,
) -> Result<Json<Vec<FeedPostResponse>>, (StatusCode, Json<ErrorResponse>)> {
    let relay_url = match &state.relay_url {
        Some(url) => url.clone(),
        None => return Ok(Json(vec![])),
    };

    let limit = query.limit.unwrap_or(50).min(200);

    let mut filter = if let Some(ref tag) = query.hashtag {
        scuffed_types::nostr::NostrFilter::by_hashtag(tag)
    } else {
        scuffed_types::nostr::NostrFilter::community_posts()
    };
    filter.limit = Some(limit);
    if let Some(since) = query.since {
        filter.since = Some(since);
    }

    let events = scuffed_chat::nostr::relay::query_events_oneshot(
        &relay_url,
        vec![filter],
        5,
    )
    .await
    .map_err(|e| {
        tracing::error!("Failed to query relay feed: {e}");
        (
            StatusCode::BAD_GATEWAY,
            Json(ErrorResponse {
                error: "Failed to query relay".into(),
            }),
        )
    })?;

    let is_officer = {
        let cookie_name = &state.session_config.cookie_name;
        match jar.get(cookie_name) {
            Some(cookie) => {
                let token = cookie.value();
                match state.db.get_session(token).await {
                    Ok(Some(uid)) => state
                        .db
                        .get_member_by_user(&uid)
                        .await
                        .ok()
                        .flatten()
                        .map(|m| m.org_role.can_access_officer_channel())
                        .unwrap_or(false),
                    _ => false,
                }
            }
            None => false,
        }
    };

    let officer_groups: std::collections::HashSet<String> = state
        .db
        .list_officer_group_ids()
        .await
        .unwrap_or_default()
        .into_iter()
        .collect();

    let events: Vec<_> = if officer_groups.is_empty() || is_officer {
        events
    } else {
        events
            .into_iter()
            .filter(|e| {
                !e.tags.iter().any(|t| {
                    t.first().map(|s| s.as_str()) == Some("h")
                        && t.get(1).map(|g| officer_groups.contains(g)).unwrap_or(false)
                })
            })
            .collect()
    };

    let members = state.db.list_nostr_identities().await.unwrap_or_default();
    let pubkey_names: HashMap<String, String> = members
        .into_iter()
        .filter_map(|m| {
            m.nostr_pubkey.map(|pk| (pk, m.display_name))
        })
        .collect();

    let mut posts: Vec<FeedPostResponse> = events
        .into_iter()
        .filter(|e| e.kind == 1)
        .map(|e| {
            let hashtags: Vec<String> = e
                .tags
                .iter()
                .filter(|t| t.first().map(|s| s.as_str()) == Some("t"))
                .filter_map(|t| t.get(1).cloned())
                .collect();

            FeedPostResponse {
                id: e.id.clone(),
                pubkey: e.pubkey.clone(),
                author_name: pubkey_names.get(&e.pubkey).cloned(),
                content: e.content,
                hashtags,
                created_at: e.created_at as i64,
                reactions: vec![],
                reply_count: 0,
            }
        })
        .collect();

    posts.sort_by(|a, b| b.created_at.cmp(&a.created_at));

    Ok(Json(posts))
}

#[derive(Deserialize)]
pub struct CommunityPostRequest {
    pub content: String,
    #[serde(default)]
    pub hashtags: Vec<String>,
    pub community_id: Option<String>,
    pub group_id: Option<String>,
    pub reply_to: Option<String>,
    pub root: Option<String>,
}

#[derive(Serialize)]
pub struct CommunityPostResponse {
    pub event_id: String,
}

/// POST /api/nostr/post — publish a kind 1 community post.
pub async fn nostr_post(
    State(state): State<AppState>,
    caller: OrgMember,
    Json(body): Json<CommunityPostRequest>,
) -> Result<Json<CommunityPostResponse>, (StatusCode, Json<ErrorResponse>)> {
    let relay_url = state.relay_url.clone().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "Relay not configured".into(),
            }),
        )
    })?;

    if caller.member.nostr_key_mode != Some(NostrKeyMode::ServerManaged) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Server-managed Nostr key required to publish posts".into(),
            }),
        ));
    }

    if body.content.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Post content must not be empty".into(),
            }),
        ));
    }

    let secret_hex = state
        .db
        .get_nostr_secret_key(&caller.member.id)
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
                    error: "No server-managed key found".into(),
                }),
            )
        })?;

    let keys = EventBuilder::keys_from_hex(&secret_hex).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Invalid key: {e}"),
            }),
        )
    })?;

    let event = EventBuilder::build_community_post(
        &keys,
        &body.content,
        &body.hashtags,
        body.community_id.as_deref(),
        body.group_id.as_deref(),
        body.reply_to.as_deref(),
        body.root.as_deref(),
    )
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to build event: {e}"),
            }),
        )
    })?;

    let event_id = event.id.to_hex();
    let relay_event = EventBuilder::to_relay_event(&event);
    let post_context_id = body
        .community_id
        .clone()
        .or_else(|| body.group_id.clone())
        .unwrap_or_else(|| caller.member.id.clone());

    let db = state.db.clone();
    let member_id = caller.member.id.clone();
    let log_event_id = event_id.clone();
    tokio::spawn(async move {
        if let Err(e) = publish_event_oneshot(&relay_url, relay_event).await {
            tracing::error!("Failed to publish community post: {e}");
        } else {
            tracing::info!("Published kind 1 community post {log_event_id}");
        }

        crate::routes::audit_log::audit(
            &db,
            &member_id,
            scuffed_db::AuditAction::PublishedPost,
            scuffed_db::AuditTargetType::Member,
            &post_context_id,
            Some("Published kind 1 community post"),
        )
        .await;
    });

    Ok(Json(CommunityPostResponse { event_id }))
}

// ─── Relay health endpoint (Phase 4) ───

#[derive(Serialize)]
pub struct RelayHealthResponse {
    pub configured: bool,
    pub reachable: bool,
    pub relay_url: Option<String>,
    pub extra_relay_urls: Vec<String>,
    pub relay_info: Option<RelayInfoResponse>,
    pub forum_backend: String,
}

#[derive(Serialize)]
pub struct RelayInfoResponse {
    pub name: Option<String>,
    pub description: Option<String>,
}

/// GET /api/nostr/health — relay connectivity and configuration status.
pub async fn nostr_health(
    State(state): State<AppState>,
) -> Json<RelayHealthResponse> {
    let settings = state.db.get_settings().await.ok();
    let forum_backend = settings
        .as_ref()
        .map(|s| s.forum_backend.clone())
        .unwrap_or_else(|| "local".into());
    let extra_relay_urls: Vec<String> = settings
        .as_ref()
        .map(|s| {
            s.extra_relay_urls
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty() && (l.starts_with("ws://") || l.starts_with("wss://")))
                .collect()
        })
        .unwrap_or_default();

    let relay_url = match &state.relay_url {
        Some(url) => url.clone(),
        None => {
            return Json(RelayHealthResponse {
                configured: false,
                reachable: false,
                relay_url: None,
                extra_relay_urls,
                relay_info: None,
                forum_backend,
            });
        }
    };

    let http_url = relay_url
        .replace("ws://", "http://")
        .replace("wss://", "https://");

    let reachable = match reqwest::Client::new()
        .get(&http_url)
        .header("Accept", "application/nostr+json")
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await
    {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    };

    Json(RelayHealthResponse {
        configured: true,
        reachable,
        relay_url: Some(relay_url),
        extra_relay_urls,
        relay_info: None,
        forum_backend,
    })
}
