//! Chat endpoints: NIP-42 auth provisioning + NIP-44/NIP-59 encrypted messaging.
//!
//! Provides server-managed key generation, relay auth event signing,
//! and encrypted group messaging for officer/private channels.

use axum::{
    extract::State,
    http::StatusCode,
    Json,
};
use scuffed_auth::crypto::CryptoService;
use scuffed_auth::server::session::ErrorResponse;
use scuffed_chat::{AuthTokenRequest, AuthTokenResponse, EncryptionService, NostrAuthService};
use scuffed_db::NostrKeyMode;
use serde::{Deserialize, Serialize};

use scuffed_site_server::extractors::{OfficerUser, OrgMember};
use scuffed_site_server::state::AppState;

/// POST /api/chat/auth-token — provision a NIP-42 auth event for relay authentication.
///
/// Flow:
/// 1. If the member has no Nostr keys, generate server-managed keys
/// 2. If external keys (NIP-07), return error — client must sign client-side
/// 3. Decrypt member's server-managed key, sign AUTH event, return it
pub async fn provision_auth_token(
    State(state): State<AppState>,
    caller: OrgMember,
    Json(body): Json<AuthTokenRequest>,
) -> Result<Json<AuthTokenResponse>, (StatusCode, Json<ErrorResponse>)> {
    let crypto = CryptoService::from_env().map_err(|e| {
        tracing::error!("CryptoService init failed: {e}");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Encryption service not available".into(),
            }),
        )
    })?;

    let crypto = crypto.ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "Encryption not configured (ENCRYPTION_KEY required)".into(),
            }),
        )
    })?;

    let auth_service = NostrAuthService::new(crypto);
    let member = &caller.member;

    // Check key mode
    match member.nostr_key_mode {
        Some(NostrKeyMode::External) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "External key users must sign AUTH events client-side (NIP-07)".into(),
                }),
            ));
        }
        Some(NostrKeyMode::ServerManaged) => {
            // Use existing encrypted key
            let encrypted = member.nostr_secret_key_encrypted.as_ref().ok_or_else(|| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: "Server-managed key missing encrypted secret".into(),
                    }),
                )
            })?;

            let challenge = body.challenge.as_deref().unwrap_or("");
            let response = auth_service
                .provision_auth_event(encrypted, &body.relay_url, challenge)
                .map_err(|e| {
                    tracing::error!("Auth event provisioning failed: {e}");
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse {
                            error: format!("Failed to provision auth event: {e}"),
                        }),
                    )
                })?;

            Ok(Json(response))
        }
        None => {
            // No key exists — generate server-managed keys
            let (pubkey, encrypted) = auth_service.generate_keypair().map_err(|e| {
                tracing::error!("Keypair generation failed: {e}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: "Failed to generate Nostr keypair".into(),
                    }),
                )
            })?;

            // Store in DB
            state
                .db
                .update_member_nostr_keys(
                    &member.id,
                    Some(&pubkey),
                    Some("server_managed"),
                    Some(&encrypted),
                )
                .await
                .map_err(|e| {
                    tracing::error!("Failed to store keypair: {e}");
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse {
                            error: "Failed to store keypair".into(),
                        }),
                    )
                })?;

            tracing::info!(member_id = %member.id, "Generated server-managed Nostr keypair");

            // Now provision the auth event with the newly created key
            let challenge = body.challenge.as_deref().unwrap_or("");
            let response = auth_service
                .provision_auth_event(&encrypted, &body.relay_url, challenge)
                .map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse {
                            error: format!("Failed to provision auth event: {e}"),
                        }),
                    )
                })?;

            Ok(Json(response))
        }
    }
}

// =============================================================================
// NIP-44/NIP-59 Encrypted Messaging
// =============================================================================

/// Request to send an encrypted message to an officer channel.
#[derive(Debug, Deserialize)]
pub struct SendEncryptedRequest {
    /// The NIP-29 group ID for the officer channel.
    pub group_id: String,
    /// Plaintext message content.
    pub content: String,
    /// Optional event ID to reply to.
    pub reply_to: Option<String>,
}

/// Response from sending encrypted messages.
#[derive(Debug, Serialize)]
pub struct SendEncryptedResponse {
    /// Number of gift-wrapped events published.
    pub recipients_count: usize,
    /// The sender's public key.
    pub sender_pubkey: String,
}

/// Request to decrypt a gift-wrapped event.
#[derive(Debug, Deserialize)]
pub struct DecryptMessageRequest {
    /// The gift wrap event JSON (kind 1059).
    pub event_json: String,
}

/// Response with the decrypted message.
#[derive(Debug, Serialize)]
pub struct DecryptMessageResponse {
    /// The sender's public key (hex).
    pub sender_pubkey: String,
    /// The decrypted plaintext content.
    pub content: String,
    /// Event kind from the inner rumor.
    pub kind: u16,
    /// Tags from the inner rumor.
    pub tags: Vec<Vec<String>>,
    /// Timestamp from the inner rumor.
    pub created_at: u64,
}

/// Helper to initialize CryptoService from env.
fn init_crypto() -> Result<CryptoService, (StatusCode, Json<ErrorResponse>)> {
    let crypto = CryptoService::from_env().map_err(|e| {
        tracing::error!("CryptoService init failed: {e}");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Encryption service not available".into(),
            }),
        )
    })?;

    crypto.ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "Encryption not configured (ENCRYPTION_KEY required)".into(),
            }),
        )
    })
}

/// POST /api/chat/send-encrypted — encrypt and publish a message to an officer channel.
///
/// Flow:
/// 1. Verify caller is officer+ (required for officer channels)
/// 2. Look up channel by group_id, verify it's an officer channel
/// 3. Get all channel members' pubkeys from the team roster
/// 4. Build NIP-59 gift wraps (one per member)
/// 5. Publish all gift wraps to the relay
///
/// The caller's server-managed key is used for signing.
pub async fn send_encrypted(
    State(state): State<AppState>,
    caller: OfficerUser,
    Json(body): Json<SendEncryptedRequest>,
) -> Result<Json<SendEncryptedResponse>, (StatusCode, Json<ErrorResponse>)> {
    let crypto = init_crypto()?;
    let member = &caller.member;

    // Verify sender has server-managed keys
    let sender_encrypted_key = match member.nostr_key_mode {
        Some(NostrKeyMode::ServerManaged) => member
            .nostr_secret_key_encrypted
            .as_ref()
            .ok_or_else(|| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: "Server-managed key missing encrypted secret".into(),
                    }),
                )
            })?,
        Some(NostrKeyMode::External) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "External key users must encrypt client-side (NIP-07 + NIP-44)".into(),
                }),
            ));
        }
        None => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "No Nostr keys provisioned. Call /api/chat/auth-token first.".into(),
                }),
            ));
        }
    };

    // Look up the channel and verify it's an officer channel
    let channel = state
        .db
        .get_channel_by_group_id(&body.group_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to look up channel: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Failed to look up channel".into(),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!("Channel '{}' not found", body.group_id),
                }),
            )
        })?;

    if channel.group_type != scuffed_db::GroupType::Officer {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Gift wrap encryption is only used for officer channels. Use kind 9 for public channels.".into(),
            }),
        ));
    }

    // Get team members to find all recipient pubkeys
    let roster = state
        .db
        .get_team_roster(&channel.team_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get team roster: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Failed to get channel members".into(),
                }),
            )
        })?;

    // Collect pubkeys of all team members who have Nostr keys
    let mut recipient_pubkeys = Vec::new();
    for entry in &roster {
        let m = state.db.get_member(&entry.member_id).await.map_err(|e| {
            tracing::error!("Failed to get member {}: {e}", entry.member_id);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Failed to resolve member keys".into(),
                }),
            )
        })?;
        if let Some(m) = m {
            if let Some(pubkey) = &m.nostr_pubkey {
                recipient_pubkeys.push(pubkey.clone());
            }
        }
    }

    if recipient_pubkeys.is_empty() {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorResponse {
                error: "No channel members have Nostr keys provisioned".into(),
            }),
        ));
    }

    let sender_pubkey = member.nostr_pubkey.clone().unwrap_or_default();

    // Build gift wraps
    let enc_service = EncryptionService::new(crypto);
    let gift_wraps = enc_service
        .build_gift_wraps(
            sender_encrypted_key,
            &recipient_pubkeys,
            &body.content,
            &body.group_id,
            body.reply_to.as_deref(),
        )
        .await
        .map_err(|e| {
            tracing::error!("Gift wrap construction failed: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Encryption failed: {e}"),
                }),
            )
        })?;

    let recipients_count = gift_wraps.len();

    // Publish each gift wrap to the relay
    // TODO(Phase 2c): Use the RelayClient from shared state instead of creating per-request
    let relay_url = &channel.relay_url;
    let relay_client = scuffed_chat::RelayClient::new(relay_url);
    match relay_client.connect().await {
        Ok(_rx) => {
            for gw in &gift_wraps {
                let relay_event = scuffed_chat::EventBuilder::to_relay_event(&gw.event);
                if let Err(e) = relay_client.publish_event(relay_event).await {
                    tracing::warn!(
                        recipient = %gw.recipient_pubkey,
                        "Failed to publish gift wrap: {e}"
                    );
                }
            }
            relay_client.disconnect().await;
        }
        Err(e) => {
            tracing::error!("Failed to connect to relay {relay_url}: {e}");
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorResponse {
                    error: "Cannot connect to relay for publishing".into(),
                }),
            ));
        }
    }

    tracing::info!(
        member_id = %member.id,
        group_id = %body.group_id,
        recipients = recipients_count,
        "Published encrypted message to officer channel"
    );

    Ok(Json(SendEncryptedResponse {
        recipients_count,
        sender_pubkey,
    }))
}

/// POST /api/chat/decrypt — decrypt a NIP-59 gift-wrapped event.
///
/// The caller's server-managed key is used to unwrap the gift wrap.
/// Returns the decrypted message content and sender info.
pub async fn decrypt_message(
    caller: OrgMember,
    Json(body): Json<DecryptMessageRequest>,
) -> Result<Json<DecryptMessageResponse>, (StatusCode, Json<ErrorResponse>)> {
    let crypto = init_crypto()?;
    let member = &caller.member;

    let encrypted_key = match member.nostr_key_mode {
        Some(NostrKeyMode::ServerManaged) => member
            .nostr_secret_key_encrypted
            .as_ref()
            .ok_or_else(|| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: "Server-managed key missing encrypted secret".into(),
                    }),
                )
            })?,
        Some(NostrKeyMode::External) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "External key users must decrypt client-side (NIP-07 + NIP-44)".into(),
                }),
            ));
        }
        None => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "No Nostr keys provisioned".into(),
                }),
            ));
        }
    };

    let enc_service = EncryptionService::new(crypto);
    let msg = enc_service
        .unwrap_gift_wrap_json(encrypted_key, &body.event_json)
        .await
        .map_err(|e| {
            tracing::error!("Gift wrap decryption failed: {e}");
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Decryption failed: {e}"),
                }),
            )
        })?;

    Ok(Json(DecryptMessageResponse {
        sender_pubkey: msg.sender_pubkey,
        content: msg.content,
        kind: msg.kind,
        tags: msg.tags,
        created_at: msg.created_at,
    }))
}
