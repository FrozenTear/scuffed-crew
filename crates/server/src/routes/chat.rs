//! Chat endpoints: NIP-42 auth provisioning, NIP-29 public channel sends,
//! and NIP-44/NIP-59 encrypted messaging.
//!
//! Provides server-managed key generation, relay auth event signing,
//! public kind 9 publishes, and encrypted group messaging for officer channels.

use axum::response::{IntoResponse, Response};
use axum::{Json, extract::State, http::StatusCode};
use scuffed_auth::server::session::ErrorResponse;
use scuffed_chat::{AuthTokenRequest, AuthTokenResponse, EncryptionService, NostrAuthService};
use scuffed_db::NostrKeyMode;
use scuffed_site_server::nostr_rate_limit::RateClass;
use scuffed_types::{MAX_CHAT_MESSAGE_CHARS, SendPublicRequest, SendPublicResponse};
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

fn map_auth_provision_error(
    e: scuffed_chat::AuthError,
    ctx: &str,
) -> (StatusCode, Json<ErrorResponse>) {
    match e {
        scuffed_chat::AuthError::ChallengeTooLong => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Challenge is too long".into(),
            }),
        ),
        scuffed_chat::AuthError::RelayUrlRejected => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Relay URL is not allowed".into(),
            }),
        ),
        other => internal_err(other, ctx),
    }
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

/// Outcome for `POST /api/chat/send-public`.
///
/// Ordinary failures stay `(status, {"error"})`. The shared nostr limiter
/// uses `{"error":"rate_limited","retry_after":N}`.
pub enum SendPublicError {
    Status(StatusCode, Json<ErrorResponse>),
    Limited(Box<Response>),
}

impl From<(StatusCode, Json<ErrorResponse>)> for SendPublicError {
    fn from(value: (StatusCode, Json<ErrorResponse>)) -> Self {
        Self::Status(value.0, value.1)
    }
}

impl IntoResponse for SendPublicError {
    fn into_response(self) -> Response {
        match self {
            Self::Status(status, body) => (status, body).into_response(),
            Self::Limited(response) => *response,
        }
    }
}

/// Charge the shared per-member interactive bucket used by other
/// secret-touching publishes (challenge, verify, dm-send).
fn charge_interactive_send(state: &AppState, member_id: &str) -> Result<(), SendPublicError> {
    match state
        .nostr_rate_limiter
        .check(member_id, RateClass::Interactive)
    {
        Ok(()) => Ok(()),
        Err(secs) => Err(SendPublicError::Limited(Box::new(
            scuffed_site_server::rate_limit::rate_limited_response(secs),
        ))),
    }
}

/// Primary relay, `site_settings.extra_relay_urls` (one URL per line), and
/// this site's own `/relay` when `redirect_base` is set.
///
/// Blank lines are dropped. Configured and extra entries must be `ws` or
/// `wss`; an `http` or `https` line is ignored. `redirect_base` is
/// [`AppState::auth_redirect_base`], the raw `REDIRECT_BASE_URL` from
/// startup, with no localhost default. An https base adds `wss://host/relay`
/// and `https://host/relay`, and an http base adds the `ws` and `http` pair.
/// A path or query on the base is dropped. A missing or blank base adds nothing.
fn auth_relay_candidates(
    configured: Option<&str>,
    extra_lines: &str,
    redirect_base: Option<&str>,
) -> Vec<String> {
    let mut allowed = Vec::new();
    if let Some(url) = configured.map(str::trim).filter(|url| !url.is_empty()) {
        push_websocket_relay(&mut allowed, url);
    }
    for line in extra_lines.lines() {
        let line = line.trim();
        if !line.is_empty() {
            push_websocket_relay(&mut allowed, line);
        }
    }
    if let Some(base) = redirect_base.map(str::trim).filter(|base| !base.is_empty()) {
        allowed.extend(scuffed_chat::site_auth_relay_urls(base));
    }
    allowed
}

/// Keep a configured relay only when it canonicalizes to `ws` or `wss`.
fn push_websocket_relay(allowed: &mut Vec<String>, raw: &str) {
    let Some(canonical) = scuffed_chat::canonical_relay_url(raw) else {
        return;
    };
    if canonical.starts_with("ws://") || canonical.starts_with("wss://") {
        allowed.push(raw.to_string());
    }
}

async fn load_auth_relay_candidates(state: &AppState) -> Vec<String> {
    let extras = match state.db.get_settings().await {
        Ok(settings) => settings.extra_relay_urls,
        Err(e) => {
            tracing::error!(error = %e, "auth-token allow-list settings lookup failed");
            String::new()
        }
    };
    auth_relay_candidates(
        state.relay_url.as_deref(),
        &extras,
        state.auth_redirect_base.as_deref(),
    )
}

/// POST /api/chat/auth-token — provision a NIP-42 auth event for relay authentication.
///
/// Flow:
/// 1. Reject a challenge over 256 bytes, or a relay URL that does not
///    canonicalize to `state.relay_url`, a `ws`/`wss` line of
///    `site_settings.extra_relay_urls`, or this site's `/relay` built from
///    `REDIRECT_BASE_URL`
/// 2. If the member has no Nostr keys, generate server-managed keys
/// 3. If external keys (NIP-07), return an error. The client must sign client-side.
/// 4. Decrypt member's server-managed key, sign AUTH event, return it
pub async fn provision_auth_token(
    State(state): State<AppState>,
    caller: OrgMember,
    Json(body): Json<AuthTokenRequest>,
) -> Result<Json<AuthTokenResponse>, (StatusCode, Json<ErrorResponse>)> {
    let challenge = body.challenge.as_deref().unwrap_or("");
    if challenge.len() > scuffed_chat::MAX_AUTH_CHALLENGE_BYTES {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Challenge is too long".into(),
            }),
        ));
    }
    let allowed = load_auth_relay_candidates(&state).await;
    let allowed_refs: Vec<&str> = allowed.iter().map(String::as_str).collect();
    if scuffed_chat::allowed_auth_relay(&body.relay_url, allowed_refs.iter().copied()).is_none() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Relay URL is not allowed".into(),
            }),
        ));
    }

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

            let response = auth_service
                .provision_auth_event(encrypted, pubkey, &body.relay_url, challenge, &allowed_refs)
                .map_err(|e| map_auth_provision_error(e, "Auth event provisioning failed"))?;

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
            let response = auth_service
                .provision_auth_event(
                    &encrypted,
                    &pubkey,
                    &body.relay_url,
                    challenge,
                    &allowed_refs,
                )
                .map_err(|e| {
                    map_auth_provision_error(e, "Auth event provisioning failed after keygen")
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

/// Dial URL for a team channel on the server.
///
/// `channel_relay_url` is the `team_channel.relay_url` column the browser
/// also reads. An absolute `ws://` or `wss://` value is used as stored
/// (trimmed). Blank, whitespace, and browser paths such as `/relay` are not
/// dialable from this process, so `configured` is used instead.
/// `configured` is `AppState.relay_url`, loaded once from `NOSTR_RELAY_URL`.
/// It must also be `ws://` or `wss://`. Returns `None` when neither side is
/// a websocket URL. Callers answer that with 503. This does not rewrite the
/// stored column: browsers still map a blank or relative value to the public
/// relay.
/// Trim chat text and enforce [`MAX_CHAT_MESSAGE_CHARS`].
///
/// Length is Unicode scalar values of the trimmed text. Blank input is
/// returned as an empty string; callers that reject blank content do that
/// themselves.
fn trimmed_chat_content(raw: &str) -> Result<String, (StatusCode, Json<ErrorResponse>)> {
    let content = raw.trim();
    if content.chars().count() > MAX_CHAT_MESSAGE_CHARS {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Message is too long (max {MAX_CHAT_MESSAGE_CHARS} characters)"),
            }),
        ));
    }
    Ok(content.to_string())
}

fn resolve_server_relay_url(channel_relay_url: &str, configured: Option<&str>) -> Option<String> {
    let channel = channel_relay_url.trim();
    if channel.starts_with("ws://") || channel.starts_with("wss://") {
        return Some(channel.to_string());
    }
    let configured = configured?.trim();
    if configured.starts_with("ws://") || configured.starts_with("wss://") {
        Some(configured.to_string())
    } else {
        None
    }
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
    let content = trimmed_chat_content(&body.content)?;
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
        if let Some(m) = m {
            // Same rule as `get_team_roster`: active and not banned. The query
            // already drops them; this skips a row that is still on the edge.
            let banned = state.db.is_member_banned(&m.id).await.map_err(|e| {
                tracing::error!("Failed to check ban for {}: {e}", m.id);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: "Failed to resolve member keys".into(),
                    }),
                )
            })?;
            if m.is_active
                && !banned
                && m.org_role.can_access_officer_channel()
                && let Some(pubkey) = &m.nostr_pubkey
            {
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

    let sender_pubkey = full.nostr_pubkey.clone().unwrap_or_default();

    // Build gift wraps
    let enc_service = EncryptionService::new(crypto);
    let gift_wraps = enc_service
        .build_gift_wraps(
            sender_encrypted_key,
            &sender_pubkey,
            &recipient_pubkeys,
            &content,
            &body.group_id,
            body.reply_to.as_deref(),
        )
        .await
        .map_err(|e| internal_err(e, "Gift wrap construction failed"))?;

    let recipients_count = gift_wraps.len();

    // Publish each gift wrap to the relay and require success.
    // TODO(Phase 2c): Use the RelayClient from shared state instead of creating per-request.
    // Do not dial `channel.relay_url` raw: live rows are blank, and a path
    // like `/relay` is a browser hint, not a server socket.
    let relay_url = match resolve_server_relay_url(&channel.relay_url, state.relay_url.as_deref()) {
        Some(url) => url,
        None => {
            tracing::error!(
                group_id = %body.group_id,
                channel_relay = %channel.relay_url,
                configured = state.relay_url.as_deref().unwrap_or(""),
                "No usable relay URL for encrypted send"
            );
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorResponse {
                    error: "No usable relay URL for publishing. Set a ws:// or wss:// channel relay, or set NOSTR_RELAY_URL to a ws:// or wss:// URL.".into(),
                }),
            ));
        }
    };
    let relay_client = scuffed_chat::RelayClient::new(&relay_url);
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

/// POST /api/chat/send-public. Publish one NIP-29 kind 9 event to a public team channel.
///
/// Body: `{ "group_id": string, "content": string }`.
/// Success: `{ "event_id", "sender_pubkey", "created_at" }`.
///
/// The signed event is kind 9 with a single `["h", group_id]` tag and the
/// trimmed plaintext as content. That is the event `nip07_sign_kind9` builds,
/// so relay readers render it the same way.
///
/// Caller must be an org member and on the roster of the team that owns the
/// channel. Officer channels are rejected with 400 pointing at
/// `POST /api/chat/send-encrypted`. External (NIP-07) keys must sign
/// client-side. Content is trimmed. Blank content is rejected. After trim,
/// more than 2000 characters (`MAX_CHAT_MESSAGE_CHARS`) is 400, same as
/// `send_encrypted`. This route charges the shared per-member interactive
/// nostr limiter.
///
/// `group_id` comes from `GET /api/teams/:id/channels` (`group_type: public`).
pub async fn send_public(
    State(state): State<AppState>,
    caller: OrgMember,
    Json(body): Json<SendPublicRequest>,
) -> Result<Json<SendPublicResponse>, SendPublicError> {
    let content = trimmed_chat_content(&body.content)?;
    if content.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Message content must not be empty".into(),
            }),
        )
            .into());
    }
    charge_interactive_send(&state, &caller.member.id)?;
    let crypto = require_crypto(&state)?;

    let group_id = body.group_id.trim();
    if group_id.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "group_id must not be empty".into(),
            }),
        )
            .into());
    }

    // Key mode before channel lookup, matching send_encrypted.
    let full = match caller.member.nostr_key_mode {
        Some(NostrKeyMode::ServerManaged) => {
            load_member_with_secret(&state, &caller.member.id).await?
        }
        Some(NostrKeyMode::External) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "External key users must sign client-side (NIP-07)".into(),
                }),
            )
                .into());
        }
        None => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "No Nostr keys provisioned. Call /api/chat/auth-token first.".into(),
                }),
            )
                .into());
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
    let sender_pubkey = full.nostr_pubkey.clone().ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Server-managed key missing public key".into(),
            }),
        )
    })?;

    let channel = state
        .db
        .get_channel_by_group_id(group_id)
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
                    error: format!("Channel '{group_id}' not found"),
                }),
            )
        })?;

    match channel.group_type {
        scuffed_db::GroupType::Officer => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Officer channels use POST /api/chat/send-encrypted.".into(),
                }),
            )
                .into());
        }
        scuffed_db::GroupType::Public => {}
    }

    if !channel.is_active {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Channel '{group_id}' not found"),
            }),
        )
            .into());
    }

    let on_roster = state
        .db
        .is_on_team_roster(&full.id, &channel.team_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to check team roster: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Failed to check team membership".into(),
                }),
            )
        })?;
    if !on_roster {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ErrorResponse {
                error: "Not a member of this team".into(),
            }),
        )
            .into());
    }

    let auth_service = NostrAuthService::new(crypto);
    let event = auth_service
        .sign_event_for_member(
            sender_encrypted_key,
            &sender_pubkey,
            scuffed_types::nostr::event_kinds::GROUP_CHAT_MESSAGE,
            &content,
            vec![vec!["h".to_string(), channel.group_id.clone()]],
        )
        .map_err(|e| internal_err(e, "Public channel event signing failed"))?;

    // Do not dial `channel.relay_url` raw: live rows are blank, and a path
    // like `/relay` is a browser hint, not a server socket.
    let relay_url = match resolve_server_relay_url(&channel.relay_url, state.relay_url.as_deref()) {
        Some(url) => url,
        None => {
            tracing::error!(
                group_id = %channel.group_id,
                channel_relay = %channel.relay_url,
                configured = state.relay_url.as_deref().unwrap_or(""),
                "No usable relay URL for public send"
            );
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorResponse {
                    error: "No usable relay URL for publishing. Set a ws:// or wss:// channel relay, or set NOSTR_RELAY_URL to a ws:// or wss:// URL.".into(),
                }),
            )
                .into());
        }
    };

    if let Err(e) = scuffed_chat::publish_event_oneshot(&relay_url, event.clone()).await {
        tracing::error!(error = %e, "Failed to publish public channel event");
        let status = if matches!(e, scuffed_chat::RelayError::ConnectionFailed(_)) {
            StatusCode::SERVICE_UNAVAILABLE
        } else {
            StatusCode::BAD_GATEWAY
        };
        let message = if status == StatusCode::SERVICE_UNAVAILABLE {
            "Cannot connect to relay for publishing"
        } else {
            "Relay rejected or failed to accept the message"
        };
        return Err((
            status,
            Json(ErrorResponse {
                error: message.into(),
            }),
        )
            .into());
    }

    tracing::info!(
        member_id = %full.id,
        group_id = %channel.group_id,
        event_id = %event.id,
        "Published public channel message"
    );

    Ok(Json(SendPublicResponse {
        event_id: event.id,
        sender_pubkey,
        created_at: event.created_at,
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
            auth_redirect_base: None,
            upload_dir: PathBuf::from("/tmp/scuffed-test-uploads"),
            reports_dir: PathBuf::from("/tmp/scuffed-test-reports"),
            reports_enabled: true,
            packs_dir: None,
            notifier: None,
            nostr_challenge_key: [0u8; 32],
            consumed_challenges: scuffed_site_server::challenge_store::ConsumedChallengeStore::new(
            ),
            nostr_rate_limiter: scuffed_site_server::nostr_rate_limit::NostrRateLimiter::new(),
            login_lockout: scuffed_site_server::login_lockout::LoginLockout::new(),
            link_code_attempts: scuffed_site_server::link_attempts::LinkCodeAttempts::new(),
            link_poll: scuffed_site_server::link_poll::LinkPollGate::system(),
            crypto: Some(Arc::new(test_crypto())),
            relay_url: None,
            dm_events: None,
            nip05_domain: None,
            nip05_republish_enabled: false,
            public_settings: scuffed_site_server::state::PublicSettingsCache::new(),
            leaderboard_cache: scuffed_site_server::leaderboard_cache::LeaderboardCache::from_env(),
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

    async fn seed_keyed_officer(db: &Database, id: &str, user_id: &str, active: bool) {
        db.client
            .query(format!(
                r#"CREATE user:{user_id} SET
                    provider = 'discord',
                    username = '{user_id}',
                    avatar_url = NONE,
                    provider_id = '{user_id}-pid',
                    provider_id_hash = '{user_id}-pidh',
                    provider_id_encrypted = NONE,
                    created_at = time::now();
                   CREATE member:{id} SET
                    user_id = '{user_id}',
                    org_role = 'officer',
                    display_name = '{user_id}',
                    bio = NONE,
                    avatar_url = NONE,
                    timezone = NONE,
                    pronouns = NONE,
                    availability_status = NONE,
                    joined_at = time::now(),
                    is_active = {active}"#
            ))
            .await
            .expect("seed officer");
        let (pubkey, encrypted) = NostrAuthService::new(test_crypto())
            .generate_keypair()
            .expect("keypair");
        db.update_member_nostr_keys(id, Some(&pubkey), Some("server_managed"), Some(&encrypted))
            .await
            .expect("keys");
    }

    #[tokio::test]
    async fn send_encrypted_skips_banned_and_deactivated_officers() {
        let state = test_state().await;
        seed_officer(&state.db).await;
        let team = state
            .db
            .create_team("Alpha", "ow2", None, None, None)
            .await
            .unwrap();
        scuffed_chat::ensure_team_channel_rows(&state.db, &team.id, "")
            .await
            .unwrap();
        seed_keyed_officer(&state.db, "deadofficer", "deaduser", false).await;
        seed_keyed_officer(&state.db, "bannedofficer", "banneduser", true).await;
        state
            .db
            .add_to_roster("deadofficer", &team.id, scuffed_db::TeamRole::Player)
            .await
            .unwrap();
        state
            .db
            .add_to_roster("bannedofficer", &team.id, scuffed_db::TeamRole::Player)
            .await
            .unwrap();
        state
            .db
            .create_moderation_action(
                "bannedofficer",
                scuffed_db::ModerationActionType::Ban,
                "chat",
                "officermember",
                None,
            )
            .await
            .unwrap();

        let group_id = officer_group_id(&team.id);
        let app = Router::new()
            .route("/api/chat/send-encrypted", post(send_encrypted))
            .with_state(state.clone());
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
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "banned and deactivated officers must not be gift-wrap recipients: {body}"
        );
        assert!(
            body.contains("No channel members"),
            "expected the empty-recipient error, got {body}"
        );

        seed_keyed_officer(&state.db, "liveofficer", "liveuser", true).await;
        state
            .db
            .add_to_roster("liveofficer", &team.id, scuffed_db::TeamRole::Player)
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
        assert_eq!(
            status,
            StatusCode::SERVICE_UNAVAILABLE,
            "an active officer is still a recipient; with no dialable relay the send is 503: {body}"
        );
        assert!(
            body.contains("No usable relay URL"),
            "blank channel and unset NOSTR_RELAY_URL must 503 before dialing, got {body}"
        );
    }

    #[test]
    fn auth_token_allowlist_accepts_configured_rejects_other_host_and_http() {
        let allowed = auth_relay_candidates(
            Some("wss://Relay.Example.com:443/"),
            "\nws://backup.example:80/\nhttps://nope.example\n",
            None,
        );
        let refs: Vec<&str> = allowed.iter().map(String::as_str).collect();

        assert_eq!(
            scuffed_chat::allowed_auth_relay("WSS://relay.example.com/", refs.iter().copied())
                .as_deref(),
            Some("wss://relay.example.com")
        );
        assert_eq!(
            scuffed_chat::allowed_auth_relay("ws://Backup.Example/", refs.iter().copied())
                .as_deref(),
            Some("ws://backup.example")
        );
        assert!(
            scuffed_chat::allowed_auth_relay("wss://evil.example", refs.iter().copied()).is_none()
        );
        assert!(
            scuffed_chat::allowed_auth_relay("http://relay.example.com", refs.iter().copied())
                .is_none()
        );
        assert!(
            scuffed_chat::allowed_auth_relay("https://relay.example.com", refs.iter().copied())
                .is_none()
        );
        assert!(
            scuffed_chat::allowed_auth_relay("https://nope.example", refs.iter().copied())
                .is_none()
        );
    }

    #[test]
    fn auth_token_allowlist_accepts_own_relay_from_redirect_base() {
        let allowed = auth_relay_candidates(
            Some("ws://strfry:7777"),
            "wss://extra.example/\nhttps://ignored.example\n",
            Some("HTTPS://OW.Example.com:443/"),
        );
        let refs: Vec<&str> = allowed.iter().map(String::as_str).collect();

        for requested in [
            "wss://ow.example.com/relay",
            "wss://ow.example.com/relay/",
            "WSS://OW.Example.com:443/relay/",
            "https://ow.example.com/relay",
            "https://ow.example.com/relay/",
            "HTTPS://ow.example.com/relay",
        ] {
            let expected = if requested.to_ascii_lowercase().starts_with("wss://") {
                "wss://ow.example.com/relay"
            } else {
                "https://ow.example.com/relay"
            };
            assert_eq!(
                scuffed_chat::allowed_auth_relay(requested, refs.iter().copied()).as_deref(),
                Some(expected),
                "{requested}"
            );
        }

        assert!(
            scuffed_chat::allowed_auth_relay("wss://evil.example/relay", refs.iter().copied())
                .is_none()
        );
        assert!(
            scuffed_chat::allowed_auth_relay("https://evil.example/relay", refs.iter().copied())
                .is_none()
        );
        assert_eq!(
            scuffed_chat::allowed_auth_relay("ws://strfry:7777", refs.iter().copied()).as_deref(),
            Some("ws://strfry:7777")
        );
        assert_eq!(
            scuffed_chat::allowed_auth_relay("wss://extra.example/", refs.iter().copied())
                .as_deref(),
            Some("wss://extra.example")
        );
        assert!(
            scuffed_chat::allowed_auth_relay("https://ignored.example", refs.iter().copied())
                .is_none()
        );
    }

    #[test]
    fn auth_token_allowlist_accepts_own_relay_from_http_base() {
        let allowed = auth_relay_candidates(
            Some("ws://strfry:7777"),
            "wss://extra.example/\n",
            Some("http://127.0.0.1:3000/app?x=1"),
        );
        let refs: Vec<&str> = allowed.iter().map(String::as_str).collect();

        assert_eq!(
            scuffed_chat::allowed_auth_relay("ws://127.0.0.1:3000/relay", refs.iter().copied())
                .as_deref(),
            Some("ws://127.0.0.1:3000/relay")
        );
        assert_eq!(
            scuffed_chat::allowed_auth_relay("http://127.0.0.1:3000/relay/", refs.iter().copied())
                .as_deref(),
            Some("http://127.0.0.1:3000/relay")
        );
        assert_eq!(
            scuffed_chat::allowed_auth_relay("ws://strfry:7777", refs.iter().copied()).as_deref(),
            Some("ws://strfry:7777")
        );
        assert_eq!(
            scuffed_chat::allowed_auth_relay("wss://extra.example/", refs.iter().copied())
                .as_deref(),
            Some("wss://extra.example")
        );
        assert!(
            scuffed_chat::allowed_auth_relay("wss://evil.example/relay", refs.iter().copied())
                .is_none()
        );
    }

    #[tokio::test]
    async fn load_auth_relay_candidates_reads_redirect_base_from_state() {
        let mut state = test_state().await;
        state.relay_url = Some("ws://strfry:7777".into());
        state.auth_redirect_base = Some("http://127.0.0.1:3000".into());

        let allowed = load_auth_relay_candidates(&state).await;
        let refs: Vec<&str> = allowed.iter().map(String::as_str).collect();
        assert_eq!(
            scuffed_chat::allowed_auth_relay("ws://127.0.0.1:3000/relay/", refs.iter().copied())
                .as_deref(),
            Some("ws://127.0.0.1:3000/relay")
        );
        assert_eq!(
            scuffed_chat::allowed_auth_relay("http://127.0.0.1:3000/relay", refs.iter().copied())
                .as_deref(),
            Some("http://127.0.0.1:3000/relay")
        );

        state.auth_redirect_base = None;
        let allowed = load_auth_relay_candidates(&state).await;
        let refs: Vec<&str> = allowed.iter().map(String::as_str).collect();
        assert!(
            scuffed_chat::allowed_auth_relay("ws://127.0.0.1:3000/relay", refs.iter().copied())
                .is_none()
        );
        assert_eq!(
            scuffed_chat::allowed_auth_relay("ws://strfry:7777", refs.iter().copied()).as_deref(),
            Some("ws://strfry:7777")
        );
    }

    #[test]
    fn auth_token_allowlist_missing_redirect_base_uses_configured_only() {
        for base in [None, Some("   "), Some("not a url")] {
            let allowed = auth_relay_candidates(
                Some("wss://relay.example.com/"),
                "wss://extra.example\nhttps://ignored.example\n",
                base,
            );
            let refs: Vec<&str> = allowed.iter().map(String::as_str).collect();

            assert_eq!(
                scuffed_chat::allowed_auth_relay("wss://relay.example.com", refs.iter().copied())
                    .as_deref(),
                Some("wss://relay.example.com"),
                "{base:?}"
            );
            assert_eq!(
                scuffed_chat::allowed_auth_relay("wss://extra.example/", refs.iter().copied())
                    .as_deref(),
                Some("wss://extra.example"),
                "{base:?}"
            );
            assert!(
                scuffed_chat::allowed_auth_relay("wss://ow.example/relay", refs.iter().copied())
                    .is_none(),
                "{base:?}"
            );
            assert!(
                scuffed_chat::allowed_auth_relay("https://ow.example/relay", refs.iter().copied())
                    .is_none(),
                "{base:?}"
            );
            assert!(
                scuffed_chat::allowed_auth_relay("https://ignored.example", refs.iter().copied())
                    .is_none(),
                "{base:?}"
            );
        }
    }

    #[test]
    fn resolve_server_relay_url_blank_uses_configured() {
        assert_eq!(
            resolve_server_relay_url("", Some("ws://strfry:7777")).as_deref(),
            Some("ws://strfry:7777")
        );
    }

    #[test]
    fn resolve_server_relay_url_relative_path_uses_configured() {
        assert_eq!(
            resolve_server_relay_url("/relay", Some("wss://relay.example")).as_deref(),
            Some("wss://relay.example")
        );
    }

    #[test]
    fn resolve_server_relay_url_ws_channel_wins() {
        assert_eq!(
            resolve_server_relay_url("ws://x", Some("wss://other")).as_deref(),
            Some("ws://x")
        );
    }

    #[test]
    fn resolve_server_relay_url_wss_channel_wins() {
        assert_eq!(
            resolve_server_relay_url("wss://x", Some("ws://other")).as_deref(),
            Some("wss://x")
        );
    }

    #[test]
    fn resolve_server_relay_url_whitespace_uses_configured() {
        assert_eq!(
            resolve_server_relay_url("  \t\n  ", Some("  ws://strfry:7777  ")).as_deref(),
            Some("ws://strfry:7777")
        );
    }

    #[test]
    fn resolve_server_relay_url_both_missing() {
        assert_eq!(resolve_server_relay_url("", None), None);
        assert_eq!(resolve_server_relay_url("   ", None), None);
        assert_eq!(resolve_server_relay_url("/relay", None), None);
        assert_eq!(resolve_server_relay_url("", Some("   ")), None);
        assert_eq!(resolve_server_relay_url("/relay", Some("")), None);
        assert_eq!(
            resolve_server_relay_url("", Some("https://relay.example")),
            None
        );
    }

    fn public_app(state: AppState) -> Router {
        Router::new()
            .route("/api/chat/send-public", post(send_public))
            .with_state(state)
    }

    fn public_req(token: Option<&str>, group_id: &str, content: &str) -> Request<Body> {
        let mut builder = Request::builder()
            .method(Method::POST)
            .uri("/api/chat/send-public")
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(token) = token {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        builder
            .body(Body::from(
                serde_json::to_vec(&json!({
                    "group_id": group_id,
                    "content": content
                }))
                .unwrap(),
            ))
            .unwrap()
    }

    async fn response_text(resp: axum::response::Response) -> (StatusCode, String) {
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
        (status, body)
    }

    /// Returns the member pubkey (real for server-managed, placeholder for external).
    async fn seed_chat_member(
        db: &Database,
        member_id: &str,
        user_id: &str,
        token: &str,
        org_role: &str,
        external: bool,
    ) -> String {
        let token_hash = hash_session_token(token);
        db.client
            .query(format!(
                r#"CREATE user:{user_id} SET
                    provider = 'discord',
                    username = '{user_id}',
                    avatar_url = NONE,
                    provider_id = '{user_id}-pid',
                    provider_id_hash = '{user_id}-pidh',
                    provider_id_encrypted = NONE,
                    created_at = time::now();
                   CREATE member:{member_id} SET
                    user_id = '{user_id}',
                    org_role = '{org_role}',
                    display_name = '{user_id}',
                    bio = NONE,
                    avatar_url = NONE,
                    timezone = NONE,
                    pronouns = NONE,
                    availability_status = NONE,
                    joined_at = time::now(),
                    is_active = true;
                   CREATE session:sess_{member_id} SET
                    user_id = '{user_id}',
                    token = $tok,
                    expires_at = time::now() + 365d,
                    created_at = time::now()"#
            ))
            .bind(("tok", token_hash))
            .await
            .expect("seed member");

        if external {
            let pubkey = "ab".repeat(32);
            db.update_member_nostr_keys(member_id, Some(&pubkey), Some("external"), None)
                .await
                .expect("external key");
            pubkey
        } else {
            let (pubkey, encrypted) = NostrAuthService::new(test_crypto())
                .generate_keypair()
                .expect("keypair");
            db.update_member_nostr_keys(
                member_id,
                Some(&pubkey),
                Some("server_managed"),
                Some(&encrypted),
            )
            .await
            .expect("store keys");
            pubkey
        }
    }

    struct Provisioned {
        team_id: String,
        public_id: String,
        officer_id: String,
    }

    async fn provision_team(state: &AppState) -> Provisioned {
        let team = state
            .db
            .create_team("Alpha", "ow2", None, None, None)
            .await
            .unwrap();
        scuffed_chat::ensure_team_channel_rows(&state.db, &team.id, "")
            .await
            .unwrap();
        Provisioned {
            public_id: scuffed_chat::public_group_id(&team.id),
            officer_id: scuffed_chat::officer_group_id(&team.id),
            team_id: team.id,
        }
    }

    async fn spawn_ok_relay() -> (
        String,
        tokio::sync::mpsc::UnboundedReceiver<serde_json::Value>,
    ) {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind relay");
        let addr = listener.local_addr().expect("relay addr");
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let tx = tx.clone();
                tokio::spawn(async move {
                    let Ok(ws) = tokio_tungstenite::accept_async(stream).await else {
                        return;
                    };
                    let (mut sink, mut incoming) = ws.split();
                    while let Some(Ok(msg)) = incoming.next().await {
                        let Message::Text(text) = msg else {
                            continue;
                        };
                        let Ok(frame) = serde_json::from_str::<serde_json::Value>(&text) else {
                            continue;
                        };
                        if frame.get(0).and_then(|v| v.as_str()) != Some("EVENT") {
                            continue;
                        }
                        let Some(event) = frame.get(1).cloned() else {
                            continue;
                        };
                        let id = event.get("id").and_then(|v| v.as_str()).unwrap_or("");
                        let ok = serde_json::json!(["OK", id, true, ""]).to_string();
                        if sink.send(Message::Text(ok.into())).await.is_err() {
                            break;
                        }
                        let _ = tx.send(event);
                    }
                });
            }
        });
        (format!("ws://{addr}"), rx)
    }

    #[tokio::test]
    async fn send_public_member_ok() {
        let (relay_url, mut published) = spawn_ok_relay().await;
        let mut state = test_state().await;
        state.relay_url = Some(relay_url);
        let pubkey = seed_chat_member(
            &state.db,
            "publicmember",
            "publicuser",
            "public-token",
            "member",
            false,
        )
        .await;
        let channels = provision_team(&state).await;
        state
            .db
            .add_to_roster(
                "publicmember",
                &channels.team_id,
                scuffed_db::TeamRole::Player,
            )
            .await
            .unwrap();

        let resp = public_app(state)
            .oneshot(public_req(
                Some("public-token"),
                &channels.public_id,
                "  hello team  ",
            ))
            .await
            .unwrap();
        let (status, body) = response_text(resp).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["sender_pubkey"], pubkey);
        assert!(json["created_at"].as_u64().is_some_and(|ts| ts > 0));
        let event_id = json["event_id"].as_str().expect("event_id");
        assert_eq!(event_id.len(), 64);

        let event = published.recv().await.expect("relay accepted the event");
        assert_eq!(event["id"], event_id);
        assert_eq!(event["kind"], 9);
        assert_eq!(event["content"], "hello team");
        assert_eq!(event["pubkey"], pubkey);
        let tags = event["tags"].as_array().expect("tags");
        assert_eq!(
            tags.len(),
            1,
            "kind 9 should carry only the h tag: {tags:?}"
        );
        assert_eq!(tags[0][0], "h");
        assert_eq!(tags[0][1], channels.public_id);
        assert!(!event["sig"].as_str().unwrap_or("").is_empty());
    }

    #[tokio::test]
    async fn send_public_non_team_member_forbidden() {
        let state = test_state().await;
        seed_chat_member(
            &state.db,
            "outsider",
            "outuser",
            "out-token",
            "member",
            false,
        )
        .await;
        let channels = provision_team(&state).await;
        let resp = public_app(state)
            .oneshot(public_req(Some("out-token"), &channels.public_id, "hello"))
            .await
            .unwrap();
        let (status, body) = response_text(resp).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert!(
            body.contains("Not a member of this team"),
            "non-roster org member must be forbidden: {body}"
        );
    }

    #[tokio::test]
    async fn send_public_officer_channel_rejected() {
        let state = test_state().await;
        seed_chat_member(
            &state.db,
            "pubmember2",
            "pubuser2",
            "pub-token-2",
            "member",
            false,
        )
        .await;
        let channels = provision_team(&state).await;
        state
            .db
            .add_to_roster(
                "pubmember2",
                &channels.team_id,
                scuffed_db::TeamRole::Player,
            )
            .await
            .unwrap();
        let resp = public_app(state)
            .oneshot(public_req(
                Some("pub-token-2"),
                &channels.officer_id,
                "hello officers",
            ))
            .await
            .unwrap();
        let (status, body) = response_text(resp).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(
            body.contains("send-encrypted"),
            "officer channels must point at send-encrypted: {body}"
        );
    }

    #[tokio::test]
    async fn send_public_external_key_rejected() {
        let state = test_state().await;
        seed_chat_member(
            &state.db,
            "extmember",
            "extuser",
            "ext-token",
            "member",
            true,
        )
        .await;
        let channels = provision_team(&state).await;
        state
            .db
            .add_to_roster("extmember", &channels.team_id, scuffed_db::TeamRole::Player)
            .await
            .unwrap();
        let resp = public_app(state)
            .oneshot(public_req(Some("ext-token"), &channels.public_id, "hello"))
            .await
            .unwrap();
        let (status, body) = response_text(resp).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(
            body.contains("sign client-side"),
            "external keys must sign client-side: {body}"
        );
    }

    #[tokio::test]
    async fn send_public_empty_content_rejected() {
        let state = test_state().await;
        seed_chat_member(
            &state.db,
            "emptymember",
            "emptyuser",
            "empty-token",
            "member",
            false,
        )
        .await;
        let channels = provision_team(&state).await;
        state
            .db
            .add_to_roster(
                "emptymember",
                &channels.team_id,
                scuffed_db::TeamRole::Player,
            )
            .await
            .unwrap();
        for content in ["", "   \n\t  "] {
            let resp = public_app(state.clone())
                .oneshot(public_req(
                    Some("empty-token"),
                    &channels.public_id,
                    content,
                ))
                .await
                .unwrap();
            let (status, body) = response_text(resp).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
            assert!(
                body.contains("must not be empty"),
                "blank content must be rejected: {body}"
            );
        }
    }

    #[tokio::test]
    async fn send_public_unauthenticated() {
        let state = test_state().await;
        let resp = public_app(state)
            .oneshot(public_req(None, "anywhere", "hello"))
            .await
            .unwrap();
        let (status, body) = response_text(resp).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    }

    fn assert_too_long(status: StatusCode, body: &str) {
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        let value: serde_json::Value = serde_json::from_str(body).expect("json");
        assert_eq!(
            value["error"], "Message is too long (max 2000 characters)",
            "{body}"
        );
    }

    fn encrypted_app(state: AppState) -> Router {
        Router::new()
            .route("/api/chat/send-encrypted", post(send_encrypted))
            .with_state(state)
    }

    fn encrypted_req(group_id: &str, content: &str) -> Request<Body> {
        Request::builder()
            .method(Method::POST)
            .uri("/api/chat/send-encrypted")
            .header(header::AUTHORIZATION, format!("Bearer {OFFICER_TOKEN}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::to_vec(&json!({
                    "group_id": group_id,
                    "content": content
                }))
                .unwrap(),
            ))
            .unwrap()
    }

    #[tokio::test]
    async fn send_public_over_limit_rejected() {
        let state = test_state().await;
        seed_chat_member(
            &state.db,
            "longmember",
            "longuser",
            "long-token",
            "member",
            false,
        )
        .await;
        let over = "a".repeat(MAX_CHAT_MESSAGE_CHARS + 1);
        let resp = public_app(state)
            .oneshot(public_req(Some("long-token"), "missing", &over))
            .await
            .unwrap();
        let (status, body) = response_text(resp).await;
        assert_too_long(status, &body);
    }

    #[tokio::test]
    async fn send_encrypted_over_limit_rejected() {
        let state = test_state().await;
        seed_officer(&state.db).await;
        let over = "a".repeat(MAX_CHAT_MESSAGE_CHARS + 1);
        let resp = encrypted_app(state)
            .oneshot(encrypted_req("missing", &over))
            .await
            .unwrap();
        let (status, body) = response_text(resp).await;
        assert_too_long(status, &body);
    }

    #[tokio::test]
    async fn send_public_exactly_2000_ok() {
        let (relay_url, mut published) = spawn_ok_relay().await;
        let mut state = test_state().await;
        state.relay_url = Some(relay_url);
        seed_chat_member(
            &state.db,
            "capmember",
            "capuser",
            "cap-token",
            "member",
            false,
        )
        .await;
        let channels = provision_team(&state).await;
        state
            .db
            .add_to_roster("capmember", &channels.team_id, scuffed_db::TeamRole::Player)
            .await
            .unwrap();
        let exact = "a".repeat(MAX_CHAT_MESSAGE_CHARS);
        let resp = public_app(state.clone())
            .oneshot(public_req(Some("cap-token"), &channels.public_id, &exact))
            .await
            .unwrap();
        let (status, body) = response_text(resp).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let event = published.recv().await.expect("relay event");
        assert_eq!(
            event["content"].as_str().unwrap().chars().count(),
            exact.chars().count()
        );

        // Surrounding whitespace must not count toward the cap.
        let padded = format!("  {exact}  ");
        let resp = public_app(state)
            .oneshot(public_req(Some("cap-token"), &channels.public_id, &padded))
            .await
            .unwrap();
        let (status, body) = response_text(resp).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let event = published.recv().await.expect("relay event");
        assert_eq!(event["content"], exact);
    }

    #[tokio::test]
    async fn send_encrypted_exactly_2000_ok() {
        let (relay_url, _published) = spawn_ok_relay().await;
        let mut state = test_state().await;
        state.relay_url = Some(relay_url);
        seed_officer(&state.db).await;
        let channels = provision_team(&state).await;
        state
            .db
            .add_to_roster(
                "officermember",
                &channels.team_id,
                scuffed_db::TeamRole::Player,
            )
            .await
            .unwrap();
        let exact = "a".repeat(MAX_CHAT_MESSAGE_CHARS);
        let resp = encrypted_app(state.clone())
            .oneshot(encrypted_req(&channels.officer_id, &exact))
            .await
            .unwrap();
        let (status, body) = response_text(resp).await;
        assert_eq!(status, StatusCode::OK, "{body}");

        let padded = format!("\n{exact}\n");
        let resp = encrypted_app(state)
            .oneshot(encrypted_req(&channels.officer_id, &padded))
            .await
            .unwrap();
        let (status, body) = response_text(resp).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "2000 characters after trim must be accepted: {body}"
        );
    }

    #[tokio::test]
    async fn send_public_rate_limited() {
        let state = test_state().await;
        seed_chat_member(
            &state.db,
            "ratemember",
            "rateuser",
            "rate-token",
            "member",
            false,
        )
        .await;
        let mut limited = None;
        for _ in 0..40 {
            let resp = public_app(state.clone())
                .oneshot(public_req(Some("rate-token"), "missing-group", "hi"))
                .await
                .unwrap();
            let (status, body) = response_text(resp).await;
            if status == StatusCode::TOO_MANY_REQUESTS {
                limited = Some(body);
                break;
            }
        }
        let body = limited.expect("interactive limiter should return 429");
        let value: serde_json::Value = serde_json::from_str(&body).expect("json");
        assert_eq!(value["error"], "rate_limited", "{body}");
        assert!(
            value["retry_after"].as_u64().is_some(),
            "retry_after must be a number: {body}"
        );
    }
}
