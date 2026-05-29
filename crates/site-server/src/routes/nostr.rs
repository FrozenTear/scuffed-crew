use std::collections::HashMap;
use std::convert::Infallible;

use axum::{
    Json,
    extract::{Query, State},
    http::{StatusCode, header},
    response::IntoResponse,
    response::sse::{Event, KeepAlive, Sse},
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
        Json(Nip05Response { names, relays }),
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
        let pk = nostr::PublicKey::from_bech32(trimmed).map_err(|_| "Invalid npub address")?;
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
fn verify_challenge_token(key: &[u8; 32], token: &str) -> Result<(String, String), &'static str> {
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
    let challenge_hex: String = challenge_bytes.iter().map(|b| format!("{b:02x}")).collect();
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
    let keys = Keys::new(SecretKey::from_hex(&secret_hex).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid key in backup: {e}"),
            }),
        )
    })?);
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

    let events = scuffed_chat::nostr::relay::query_events_oneshot(&relay_url, vec![filter], 5)
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
                        && t.get(1)
                            .map(|g| officer_groups.contains(g))
                            .unwrap_or(false)
                })
            })
            .collect()
    };

    let members = state.db.list_nostr_identities().await.unwrap_or_default();
    let pubkey_names: HashMap<String, String> = members
        .into_iter()
        .filter_map(|m| m.nostr_pubkey.map(|pk| (pk, m.display_name)))
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

    posts.sort_by_key(|b| std::cmp::Reverse(b.created_at));

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
pub async fn nostr_health(State(state): State<AppState>) -> Json<RelayHealthResponse> {
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

// ─── Phase 5: Encrypted Direct Messages (NIP-44 + NIP-59) ───
//
// All DM routes require server-managed Nostr keys: the server holds the
// member's encrypted secret key, decrypts on demand to encrypt/decrypt the
// gift wrap, and stores the decrypted plaintext in `dm_message`. External-key
// (NIP-07) members cannot send/receive DMs through this path — that requires
// client-side encryption and is out of scope for Phase 5 v1.

/// Pubkeys are 32-byte secp256k1 x-only keys serialized as 64 lowercase hex chars.
fn validate_pubkey_hex(pk: &str) -> Result<(), &'static str> {
    if pk.len() != 64 {
        return Err("pubkey must be 64 hex characters");
    }
    if !pk
        .chars()
        .all(|c| c.is_ascii_hexdigit() && (c.is_numeric() || c.is_ascii_lowercase()))
    {
        return Err("pubkey must be lowercase hex");
    }
    Ok(())
}

#[derive(Deserialize)]
pub struct DmSendRequest {
    pub recipient_pubkey: String,
    pub content: String,
    #[serde(default)]
    pub reply_to_event_id: Option<String>,
}

#[derive(Serialize)]
pub struct DmSendResponse {
    /// The kind 1059 gift-wrap event id published to the relay (and stored).
    pub gift_wrap_id: String,
}

#[derive(Serialize, Clone)]
pub struct DmMessageResponse {
    pub id: String,
    pub gift_wrap_id: String,
    pub sender_pubkey: String,
    pub recipient_pubkey: String,
    pub content: String,
    pub reply_to_event_id: Option<String>,
    pub created_at: String,
}

impl From<scuffed_db::DmMessage> for DmMessageResponse {
    fn from(m: scuffed_db::DmMessage) -> Self {
        Self {
            id: m.id,
            gift_wrap_id: m.gift_wrap_id,
            sender_pubkey: m.sender_pubkey,
            recipient_pubkey: m.recipient_pubkey,
            content: m.content,
            reply_to_event_id: m.reply_to_event_id,
            created_at: m.created_at.to_rfc3339(),
        }
    }
}

#[derive(Serialize)]
pub struct DmConversationResponse {
    pub peer_pubkey: String,
    pub last_message_preview: String,
    pub last_message_at: String,
    pub last_sender_pubkey: String,
    pub unread_count: u32,
}

impl From<scuffed_db::DmConversation> for DmConversationResponse {
    fn from(c: scuffed_db::DmConversation) -> Self {
        Self {
            peer_pubkey: c.peer_pubkey,
            last_message_preview: c.last_message_preview,
            last_message_at: c.last_message_at.to_rfc3339(),
            last_sender_pubkey: c.last_sender_pubkey,
            unread_count: c.unread_count,
        }
    }
}

#[derive(Serialize)]
pub struct DmSyncResponse {
    /// Number of gift-wrap events fetched from the relay.
    pub fetched: u32,
    /// Number of new messages newly stored (excluding dedup hits).
    pub stored: u32,
}

#[derive(Deserialize)]
pub struct DmInboxQuery {
    /// RFC3339 timestamp; only return messages strictly newer than this.
    #[serde(default)]
    pub since_ts: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Deserialize)]
pub struct DmThreadQuery {
    #[serde(default)]
    pub before_ts: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Deserialize)]
pub struct DmMarkReadRequest {
    pub peer_pubkey: String,
    /// RFC3339 timestamp; mark all messages from `peer_pubkey` up to and
    /// including this timestamp as read.
    pub until_ts: String,
}

/// Resolve the caller's server-managed Nostr identity, or return the right
/// HTTP error for an external-key / unconfigured-relay caller.
fn require_server_managed_dm_caller(
    state: &AppState,
    caller: &OrgMember,
) -> Result<(String, String, scuffed_auth::crypto::EncryptedBlob), (StatusCode, Json<ErrorResponse>)>
{
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
            StatusCode::PRECONDITION_FAILED,
            Json(ErrorResponse {
                error: "Server-managed Nostr key required for DMs".into(),
            }),
        ));
    }
    let pubkey = caller.member.nostr_pubkey.clone().ok_or_else(|| {
        (
            StatusCode::PRECONDITION_FAILED,
            Json(ErrorResponse {
                error: "Member has no Nostr pubkey".into(),
            }),
        )
    })?;
    let blob = caller
        .member
        .nostr_secret_key_encrypted
        .clone()
        .ok_or_else(|| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Member has no encrypted secret key".into(),
                }),
            )
        })?;
    Ok((relay_url, pubkey, blob))
}

fn require_encryption_service(
    state: &AppState,
) -> Result<scuffed_chat::EncryptionService, (StatusCode, Json<ErrorResponse>)> {
    let crypto = state.crypto.clone().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "Encryption service not configured (set ENCRYPTION_KEY)".into(),
            }),
        )
    })?;
    Ok(scuffed_chat::EncryptionService::new(crypto))
}

fn parse_rfc3339(
    value: &str,
) -> Result<chrono::DateTime<chrono::Utc>, (StatusCode, Json<ErrorResponse>)> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Invalid RFC3339 timestamp: {e}"),
                }),
            )
        })
}

/// POST /api/nostr/dm/send — send an encrypted DM via NIP-59 gift wrap.
pub async fn dm_send(
    State(state): State<AppState>,
    caller: OrgMember,
    Json(body): Json<DmSendRequest>,
) -> Result<Json<DmSendResponse>, (StatusCode, Json<ErrorResponse>)> {
    let recipient = body.recipient_pubkey.trim().to_lowercase();
    validate_pubkey_hex(&recipient).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid recipient_pubkey: {e}"),
            }),
        )
    })?;
    let content = body.content.trim();
    if content.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Message content must not be empty".into(),
            }),
        ));
    }

    let (relay_url, sender_pubkey, sender_blob) =
        require_server_managed_dm_caller(&state, &caller)?;
    let encryption = require_encryption_service(&state)?;

    if recipient == sender_pubkey {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Cannot DM yourself".into(),
            }),
        ));
    }

    // NIP-17 DMs use a single conversation context; we reuse the recipient
    // pubkey as the `h` tag context so unrelated group chatter is not pulled
    // in by relay-side filters keyed on it.
    let context_id = format!("dm:{}:{}", sender_pubkey, recipient);

    let wraps = encryption
        .build_gift_wraps(
            &sender_blob,
            std::slice::from_ref(&recipient),
            content,
            &context_id,
            body.reply_to_event_id.as_deref(),
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to build gift wrap: {e}"),
                }),
            )
        })?;

    let wrap = wraps.into_iter().next().ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Encryption produced no gift wrap".into(),
            }),
        )
    })?;
    let gift_wrap_id = wrap.event.id.to_hex();
    let relay_event = EventBuilder::to_relay_event(&wrap.event);

    // Store the sender's own copy synchronously so the UI can render it
    // immediately. Receiver-side dedup against the relay's later resync is
    // handled by the unique index on `gift_wrap_id`.
    let now = chrono::Utc::now();
    let _ = state
        .db
        .insert_dm_message(
            &gift_wrap_id,
            &sender_pubkey,
            &recipient,
            content,
            body.reply_to_event_id.as_deref(),
            now,
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to store sent message: {e}"),
                }),
            )
        })?;

    let db = state.db.clone();
    let log_event_id = gift_wrap_id.clone();
    let member_id = caller.member.id.clone();
    tokio::spawn(async move {
        if let Err(e) = publish_event_oneshot(&relay_url, relay_event).await {
            tracing::error!("Failed to publish DM gift wrap: {e}");
        } else {
            tracing::info!("Published DM gift wrap {log_event_id}");
        }
        crate::routes::audit_log::audit(
            &db,
            &member_id,
            scuffed_db::AuditAction::SentDirectMessage,
            scuffed_db::AuditTargetType::DirectMessage,
            &log_event_id,
            Some("Sent encrypted direct message"),
        )
        .await;
    });

    Ok(Json(DmSendResponse { gift_wrap_id }))
}

/// POST /api/nostr/dm/sync — pull new gift wraps from the relay, decrypt, store.
///
/// Used by the frontend on page mount (until real-time subscription wiring
/// lands — see [THE-878]). Always idempotent: dedup is enforced via the
/// unique index on `gift_wrap_id`.
pub async fn dm_sync(
    State(state): State<AppState>,
    caller: OrgMember,
) -> Result<Json<DmSyncResponse>, (StatusCode, Json<ErrorResponse>)> {
    let (relay_url, my_pubkey, my_blob) = require_server_managed_dm_caller(&state, &caller)?;
    let encryption = require_encryption_service(&state)?;

    // Use the inbox high-water mark as the relay `since` filter so we don't
    // refetch every gift wrap on each sync. Subtract a small overlap window
    // to forgive minor relay clock drift.
    let high_water = state
        .db
        .dm_inbox_high_water(&my_pubkey)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to read inbox high-water: {e}"),
                }),
            )
        })?;
    let since_secs: Option<u64> = high_water.map(|dt| {
        let secs = dt.timestamp();
        // 60s overlap window
        let lower = secs - 60;
        if lower < 0 { 0 } else { lower as u64 }
    });

    let mut tags = std::collections::HashMap::new();
    tags.insert("#p".to_string(), vec![my_pubkey.clone()]);
    let filter = scuffed_types::nostr::NostrFilter {
        kinds: Some(vec![scuffed_types::nostr::event_kinds::GIFT_WRAP]),
        since: since_secs,
        limit: Some(500),
        tags,
        ..Default::default()
    };

    let events = scuffed_chat::nostr::relay::query_events_oneshot(&relay_url, vec![filter], 10)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                Json(ErrorResponse {
                    error: format!("Relay query failed: {e}"),
                }),
            )
        })?;
    let fetched = events.len() as u32;

    let mut stored: u32 = 0;
    for relay_event in events {
        let event_json = match serde_json::to_string(&relay_event) {
            Ok(j) => j,
            Err(e) => {
                tracing::warn!("Skipping unserializable relay event: {e}");
                continue;
            }
        };
        let unwrapped = match encryption
            .unwrap_gift_wrap_json(&my_blob, &event_json)
            .await
        {
            Ok(u) => u,
            Err(e) => {
                tracing::debug!("Skipping unwrap failure: {e}");
                continue;
            }
        };
        if unwrapped.kind != scuffed_types::nostr::event_kinds::PRIVATE_DIRECT_MESSAGE {
            tracing::debug!(
                "Skipping non-DM rumor inside gift wrap (kind={})",
                unwrapped.kind
            );
            continue;
        }
        let reply_to = unwrapped.tags.iter().find_map(|tag| {
            if tag.first().map(|s| s.as_str()) == Some("e") {
                tag.get(1).cloned()
            } else {
                None
            }
        });
        let created_at =
            chrono::DateTime::<chrono::Utc>::from_timestamp(unwrapped.created_at as i64, 0)
                .unwrap_or_else(chrono::Utc::now);

        match state
            .db
            .insert_dm_message(
                &relay_event.id,
                &unwrapped.sender_pubkey,
                &my_pubkey,
                &unwrapped.content,
                reply_to.as_deref(),
                created_at,
            )
            .await
        {
            Ok((_, was_new)) => {
                if was_new {
                    stored += 1;
                }
            }
            Err(e) => {
                tracing::warn!("Failed to store DM {}: {e}", relay_event.id);
            }
        }
    }

    if stored > 0 {
        let db = state.db.clone();
        let member_id = caller.member.id.clone();
        let stored_count = stored;
        tokio::spawn(async move {
            crate::routes::audit_log::audit(
                &db,
                &member_id,
                scuffed_db::AuditAction::SyncedDirectMessages,
                scuffed_db::AuditTargetType::DirectMessage,
                &member_id,
                Some(&format!("Synced {stored_count} new DM(s) from relay")),
            )
            .await;
        });
    }

    Ok(Json(DmSyncResponse { fetched, stored }))
}

/// GET /api/nostr/dm/inbox?since_ts=&limit= — flat inbox for the caller.
pub async fn dm_inbox(
    State(state): State<AppState>,
    caller: OrgMember,
    Query(query): Query<DmInboxQuery>,
) -> Result<Json<Vec<DmMessageResponse>>, (StatusCode, Json<ErrorResponse>)> {
    let (_, my_pubkey, _) = require_server_managed_dm_caller(&state, &caller)?;
    let limit = query.limit.unwrap_or(100).min(500);
    let since = query.since_ts.as_deref().map(parse_rfc3339).transpose()?;

    let messages = state
        .db
        .list_dm_inbox(&my_pubkey, limit, since)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to list inbox: {e}"),
                }),
            )
        })?;
    Ok(Json(messages.into_iter().map(Into::into).collect()))
}

/// GET /api/nostr/dm/conversations — distinct peer summaries with unread counts.
pub async fn dm_conversations(
    State(state): State<AppState>,
    caller: OrgMember,
) -> Result<Json<Vec<DmConversationResponse>>, (StatusCode, Json<ErrorResponse>)> {
    let (_, my_pubkey, _) = require_server_managed_dm_caller(&state, &caller)?;
    let convs = state
        .db
        .list_dm_conversations(&caller.member.id, &my_pubkey)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to list conversations: {e}"),
                }),
            )
        })?;
    Ok(Json(convs.into_iter().map(Into::into).collect()))
}

/// GET /api/nostr/dm/thread/:peer_pubkey?before_ts=&limit= — paginated thread.
///
/// Path param is taken via Query (`peer_pubkey=`) to match the codebase's
/// existing query-extractor pattern; the route is registered as
/// `GET /api/nostr/dm/thread`.
#[derive(Deserialize)]
pub struct DmThreadPeerQuery {
    pub peer_pubkey: String,
    #[serde(default)]
    pub before_ts: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

pub async fn dm_thread(
    State(state): State<AppState>,
    caller: OrgMember,
    Query(query): Query<DmThreadPeerQuery>,
) -> Result<Json<Vec<DmMessageResponse>>, (StatusCode, Json<ErrorResponse>)> {
    let (_, my_pubkey, _) = require_server_managed_dm_caller(&state, &caller)?;
    let peer = query.peer_pubkey.trim().to_lowercase();
    validate_pubkey_hex(&peer).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid peer_pubkey: {e}"),
            }),
        )
    })?;
    let limit = query.limit.unwrap_or(50).min(200);
    let before = query.before_ts.as_deref().map(parse_rfc3339).transpose()?;
    let messages = state
        .db
        .list_dm_thread(&my_pubkey, &peer, limit, before)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to list thread: {e}"),
                }),
            )
        })?;
    Ok(Json(messages.into_iter().map(Into::into).collect()))
}

/// GET /api/nostr/dm/stream — Server-Sent Events stream of new DMs for the caller.
///
/// Backed by [`crate::dm_subscriber`]. Each event is a JSON-encoded
/// [`DmEvent`] with `event: "dm"`. Clients should treat each event as a hint
/// to refetch (`/api/nostr/dm/inbox` or `/api/nostr/dm/conversations`) — the
/// server already inserted the message before publishing.
///
/// Returns 503 if the subscriber is not running (e.g. relay or encryption
/// not configured), in which case clients should fall back to polling
/// `POST /api/nostr/dm/sync`.
pub async fn dm_stream(
    State(state): State<AppState>,
    caller: OrgMember,
) -> Result<
    Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>>,
    (StatusCode, Json<ErrorResponse>),
> {
    let (_, my_pubkey, _) = require_server_managed_dm_caller(&state, &caller)?;
    let bus = state.dm_events.clone().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "Real-time DM delivery not configured".into(),
            }),
        )
    })?;

    let mut rx = bus.subscribe();
    let (out_tx, out_rx) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(64);

    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(dm) => {
                    if dm.recipient_pubkey != my_pubkey {
                        continue;
                    }
                    let event = match Event::default().event("dm").json_data(&dm) {
                        Ok(e) => e,
                        Err(e) => {
                            tracing::warn!("Failed to encode DM SSE event: {e}");
                            continue;
                        }
                    };
                    if out_tx.send(Ok(event)).await.is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(
                        "DM SSE consumer lagged by {n} events; client will refetch on next event"
                    );
                    let event = Event::default().event("lagged").data(n.to_string());
                    if out_tx.send(Ok(event)).await.is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(out_rx);
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

/// POST /api/nostr/dm/mark-read — advance the read marker for a peer.
pub async fn dm_mark_read(
    State(state): State<AppState>,
    caller: OrgMember,
    Json(body): Json<DmMarkReadRequest>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let (_, _, _) = require_server_managed_dm_caller(&state, &caller)?;
    let peer = body.peer_pubkey.trim().to_lowercase();
    validate_pubkey_hex(&peer).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid peer_pubkey: {e}"),
            }),
        )
    })?;
    let until = parse_rfc3339(&body.until_ts)?;
    state
        .db
        .upsert_dm_read_marker(&caller.member.id, &peer, until)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to mark read: {e}"),
                }),
            )
        })?;
    Ok(StatusCode::NO_CONTENT)
}
