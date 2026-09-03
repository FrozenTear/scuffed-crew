//! Chat endpoints: NIP-42 auth provisioning + NIP-44/NIP-59 encrypted messaging.
//!
//! Provides server-managed key generation, relay auth event signing,
//! and encrypted group messaging for officer/private channels.

use axum::{Json, extract::State, http::StatusCode};
use scuffed_auth::server::session::ErrorResponse;
use scuffed_chat::{AuthTokenRequest, AuthTokenResponse, EncryptionService, NostrAuthService};
use scuffed_db::NostrKeyMode;
use serde::{Deserialize, Serialize};

use scuffed_site_server::extractors::{OfficerUser, OrgMember};
use scuffed_site_server::state::AppState;

fn internal_err(e: impl std::fmt::Display, ctx: &str) -> (StatusCode, Json<ErrorResponse>) {
    tracing::error!(error = %e, "{ctx}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse {
            error: "Internal error".into(),
        }),
    )
}

fn bad_request_logged(
    e: impl std::fmt::Display,
    ctx: &str,
    client_msg: &str,
) -> (StatusCode, Json<ErrorResponse>) {
    tracing::error!(error = %e, "{ctx}");
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse {
            error: client_msg.into(),
        }),
    )
}

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
    let crypto = require_crypto(&state)?;
    let auth_service = NostrAuthService::new(crypto);
    let member_id = caller.member.id.clone();

    // Check key mode (auth extractor omits secrets — load full row when signing)
    match caller.member.nostr_key_mode {
        Some(NostrKeyMode::External) => Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "External key users must sign AUTH events client-side (NIP-07)".into(),
            }),
        )),
        Some(NostrKeyMode::ServerManaged) => {
            let full = load_member_with_secret(&state, &member_id).await?;
            let encrypted = full.nostr_secret_key_encrypted.as_ref().ok_or_else(|| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: "Server-managed key missing encrypted secret".into(),
                    }),
                )
            })?;
            let pubkey = full.nostr_pubkey.as_deref().ok_or_else(|| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: "Server-managed key missing public key".into(),
                    }),
                )
            })?;

            let challenge = body.challenge.as_deref().unwrap_or("");
            let response = auth_service
                .provision_auth_event(encrypted, pubkey, &body.relay_url, challenge)
                .map_err(|e| internal_err(e, "Auth event provisioning failed"))?;

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
                    &member_id,
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

            tracing::info!(member_id = %member_id, "Generated server-managed Nostr keypair");

            // Now provision the auth event with the newly created key
            let challenge = body.challenge.as_deref().unwrap_or("");
            let response = auth_service
                .provision_auth_event(&encrypted, &pubkey, &body.relay_url, challenge)
                .map_err(|e| internal_err(e, "Auth event provisioning failed after keygen"))?;

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
    pub kind: u32,
    /// Tags from the inner rumor.
    pub tags: Vec<Vec<String>>,
    /// Timestamp from the inner rumor.
    pub created_at: u64,
}

/// Extract CryptoService from AppState.
fn require_crypto(
    state: &AppState,
) -> Result<scuffed_auth::crypto::CryptoService, (StatusCode, Json<ErrorResponse>)> {
    state.crypto.as_ref().map(|c| (**c).clone()).ok_or_else(|| {
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
///
/// Site chat mount: `group_id` comes from `GET /api/teams/:id/channels`
/// (`group_type: officer`). Channels are provisioned on team create/update
/// and via `POST /api/admin/teams/provision-channels` (F-API-003).
pub async fn send_encrypted(
    State(state): State<AppState>,
    caller: OfficerUser,
    Json(body): Json<SendEncryptedRequest>,
) -> Result<Json<SendEncryptedResponse>, (StatusCode, Json<ErrorResponse>)> {
    let crypto = require_crypto(&state)?;

    // Verify sender has server-managed keys (secret loaded via full member fetch).
    let full = match caller.member.nostr_key_mode {
        Some(NostrKeyMode::ServerManaged) => {
            load_member_with_secret(&state, &caller.member.id).await?
        }
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
    let sender_encrypted_key = full.nostr_secret_key_encrypted.as_ref().ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Server-managed key missing encrypted secret".into(),
            }),
        )
    })?;

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

    // Collect pubkeys of officer-channel-eligible members only (admin/officer).
    // Recruits/members must not receive gift-wraps for officer traffic.
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
        if let Some(m) = m
            && m.org_role.can_access_officer_channel()
            && let Some(pubkey) = &m.nostr_pubkey
        {
            recipient_pubkeys.push(pubkey.clone());
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

    let sender_pubkey = full.nostr_pubkey.clone().unwrap_or_default();

    // Build gift wraps
    let enc_service = EncryptionService::new(crypto);
    let gift_wraps = enc_service
        .build_gift_wraps(
            sender_encrypted_key,
            &sender_pubkey,
            &recipient_pubkeys,
            &body.content,
            &body.group_id,
            body.reply_to.as_deref(),
        )
        .await
        .map_err(|e| internal_err(e, "Gift wrap construction failed"))?;

    let recipients_count = gift_wraps.len();

    // Publish each gift wrap to the relay and require success
    // TODO(Phase 2c): Use the RelayClient from shared state instead of creating per-request
    let relay_url = &channel.relay_url;
    let relay_client = scuffed_chat::RelayClient::new(relay_url);
    match relay_client.connect().await {
        Ok(_rx) => {
            let mut failed = 0usize;
            for gw in &gift_wraps {
                let relay_event = scuffed_chat::EventBuilder::to_relay_event(&gw.event);
                if let Err(e) = relay_client.publish_event(relay_event).await {
                    failed += 1;
                    tracing::warn!(
                        recipient = %gw.recipient_pubkey,
                        "Failed to publish gift wrap: {e}"
                    );
                }
            }
            relay_client.disconnect().await;
            if failed > 0 {
                return Err((
                    StatusCode::BAD_GATEWAY,
                    Json(ErrorResponse {
                        error: format!(
                            "Relay rejected {failed}/{recipients_count} gift wrap(s); message not fully delivered"
                        ),
                    }),
                ));
            }
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
        member_id = %full.id,
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
    State(state): State<AppState>,
    caller: OrgMember,
    Json(body): Json<DecryptMessageRequest>,
) -> Result<Json<DecryptMessageResponse>, (StatusCode, Json<ErrorResponse>)> {
    let crypto = require_crypto(&state)?;

    let full = match caller.member.nostr_key_mode {
        Some(NostrKeyMode::ServerManaged) => {
            load_member_with_secret(&state, &caller.member.id).await?
        }
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
    let encrypted_key = full.nostr_secret_key_encrypted.as_ref().ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Server-managed key missing encrypted secret".into(),
            }),
        )
    })?;

    let recipient_pubkey = full.nostr_pubkey.as_deref().ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Server-managed key missing public key".into(),
            }),
        )
    })?;

    let enc_service = EncryptionService::new(crypto);
    let msg = enc_service
        .unwrap_gift_wrap_json(encrypted_key, recipient_pubkey, &body.event_json)
        .await
        .map_err(|e| bad_request_logged(e, "Gift wrap decryption failed", "Decryption failed"))?;

    Ok(Json(DecryptMessageResponse {
        sender_pubkey: msg.sender_pubkey,
        content: msg.content,
        kind: msg.kind,
        tags: msg.tags,
        created_at: msg.created_at,
    }))
}

/// Auth extractors omit secrets — load full member only when signing/decrypting.
async fn load_member_with_secret(
    state: &AppState,
    member_id: &str,
) -> Result<scuffed_db::Member, (StatusCode, Json<ErrorResponse>)> {
    state
        .db
        .get_member(member_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "get_member for secret failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Member not found".into(),
                }),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode, header};
    use axum::routing::post;
    use http_body_util::BodyExt;
    use scuffed_auth::SessionConfig;
    use scuffed_auth::crypto::{CryptoService, hash_session_token};
    use scuffed_chat::{NostrAuthService, officer_group_id};
    use scuffed_db::Database;
    use scuffed_db::migrations::run_migrations;
    use scuffed_site_server::state::{AppState, OAuthConfig};
    use serde_json::json;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tower::ServiceExt;

    const OFFICER_TOKEN: &str = "test-officer-token";

    fn test_crypto() -> CryptoService {
        CryptoService::new("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=", 1)
            .expect("32-byte zero key")
    }

    async fn test_state() -> AppState {
        let db = Database::connect_memory().await.expect("mem db");
        run_migrations(&db.client).await.expect("migrations");
        AppState {
            db: Arc::new(db),
            session_config: SessionConfig::default(),
            oauth_config: OAuthConfig {
                discord_client_id: String::new(),
                discord_client_secret: String::new(),
                google_client_id: String::new(),
                google_client_secret: String::new(),
                redirect_base_url: "http://localhost:3000".into(),
                allowed_origins: vec!["http://localhost:3000".into()],
            },
            upload_dir: PathBuf::from("/tmp/scuffed-test-uploads"),
            notifier: None,
            nostr_challenge_key: [0u8; 32],
            consumed_challenges: scuffed_site_server::challenge_store::ConsumedChallengeStore::new(
            ),
            nostr_rate_limiter: scuffed_site_server::nostr_rate_limit::NostrRateLimiter::new(),
            crypto: Some(Arc::new(test_crypto())),
            relay_url: None,
            dm_events: None,
            nip05_domain: None,
            nip05_republish_enabled: false,
        }
    }

    async fn seed_officer(db: &Database) {
        let token_hash = hash_session_token(OFFICER_TOKEN);
        db.client
            .query(
                r#"CREATE user:officeruser SET
                    provider = 'discord',
                    username = 'TestOfficer',
                    avatar_url = NONE,
                    provider_id = 'officer-pid',
                    provider_id_hash = 'officer-pidh',
                    provider_id_encrypted = NONE,
                    created_at = time::now()"#,
            )
            .await
            .expect("seed user");
        db.client
            .query(
                r#"CREATE member:officermember SET
                    user_id = 'officeruser',
                    org_role = 'officer',
                    display_name = 'TestOfficer',
                    bio = NONE,
                    avatar_url = NONE,
                    timezone = NONE,
                    pronouns = NONE,
                    availability_status = NONE,
                    joined_at = time::now(),
                    is_active = true"#,
            )
            .await
            .expect("seed member");
        db.client
            .query(
                r#"CREATE session:sess_officer SET
                    user_id = 'officeruser',
                    token = $tok,
                    expires_at = time::now() + 365d,
                    created_at = time::now()"#,
            )
            .bind(("tok", token_hash))
            .await
            .expect("seed session");

        // send_encrypted checks key mode before channel lookup — provision
        // vs 404 is only observable after this gate.
        let (pubkey, encrypted) = NostrAuthService::new(test_crypto())
            .generate_keypair()
            .expect("keypair");
        db.update_member_nostr_keys(
            "officermember",
            Some(&pubkey),
            Some("server_managed"),
            Some(&encrypted),
        )
        .await
        .expect("store keys");
    }

    fn send_req(group_id: &str) -> Request<Body> {
        Request::builder()
            .method(Method::POST)
            .uri("/api/chat/send-encrypted")
            .header(header::AUTHORIZATION, format!("Bearer {OFFICER_TOKEN}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::to_vec(&json!({
                    "group_id": group_id,
                    "content": "hello officers"
                }))
                .unwrap(),
            ))
            .unwrap()
    }

    #[tokio::test]
    async fn send_encrypted_finds_provisioned_officer_channel() {
        let state = test_state().await;
        seed_officer(&state.db).await;
        let team = state
            .db
            .create_team("Alpha", "ow2", None, None, None)
            .await
            .unwrap();
        let group_id = officer_group_id(&team.id);

        let app = Router::new()
            .route("/api/chat/send-encrypted", post(send_encrypted))
            .with_state(state.clone());
        let resp = app.oneshot(send_req(&group_id)).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "unprovisioned team must 404"
        );

        scuffed_chat::ensure_team_channel_rows(&state.db, &team.id, "")
            .await
            .unwrap();

        let app = Router::new()
            .route("/api/chat/send-encrypted", post(send_encrypted))
            .with_state(state);
        let resp = app.oneshot(send_req(&group_id)).await.unwrap();
        let status = resp.status();
        let body = String::from_utf8(
            resp.into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .to_vec(),
        )
        .unwrap();
        assert_ne!(
            status,
            StatusCode::NOT_FOUND,
            "provisioned officer channel must be found: {body}"
        );
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "channel found; empty roster has no recipient pubkeys: {body}"
        );
    }
}
