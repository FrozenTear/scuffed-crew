use std::collections::{HashMap, HashSet};
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

use scuffed_auth::password::MIN_PASSWORD_LEN;
use scuffed_auth::server::session::ErrorResponse;

use nostr::key::Keys;
use nostr::{FromBech32, SecretKey};
use zeroize::{Zeroize, Zeroizing};

use scuffed_db::NostrKeyMode;

use scuffed_chat::nostr::events::EventBuilder;
use scuffed_chat::nostr::relay::publish_event_oneshot;

use crate::extractors::{OfficerUser, OrgMember};
use crate::nostr_rate_limit::RateClass;
use crate::state::AppState;

/// Enforce the per-member rate limit for a secret-touching Nostr op
/// (DR1-NOSTR-006). Each [`RateClass`] has its own independent per-member
/// bucket, so exhausting one class never throttles the other. Returns `429`
/// when the member's bucket for `class` is empty.
fn enforce_nostr_rate_limit(
    state: &AppState,
    member_id: &str,
    class: RateClass,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if state.nostr_rate_limiter.check(member_id, class) {
        Ok(())
    } else {
        Err((
            StatusCode::TOO_MANY_REQUESTS,
            Json(ErrorResponse {
                error:
                    "Too many Nostr key/message operations — please slow down and retry shortly."
                        .into(),
            }),
        ))
    }
}

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
///
/// Query semantics (NS2-4a):
/// - `?name=<local>` — return that single normalized name if a member matches
///   (display names are lowercased and stripped to `[a-z0-9_]` in Rust; no DB
///   column yet — option (b) is out of scope tonight).
/// - `?name=_` — NIP-05 **root** identifier for the domain itself, **not** a
///   wildcard. We have no configured root identity yet, so this returns an
///   empty `names` map (never enumerates every member).
/// - missing / empty `name` — deliberate empty `names` map (not a bulk dump).
///
/// The 2000-row identity scan remains; rate-limit coverage is NS2-6's job.
pub async fn nostr_json(
    State(state): State<AppState>,
    Query(query): Query<Nip05Query>,
) -> impl IntoResponse {
    let empty = || {
        (
            StatusCode::OK,
            [(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")],
            Json(Nip05Response {
                names: HashMap::new(),
                relays: HashMap::new(),
            }),
        )
    };

    let requested_name = query.name.as_deref().unwrap_or("").trim().to_lowercase();

    // Empty name or NIP-05 root `_` — never enumerate the member table.
    if requested_name.is_empty() || requested_name == "_" {
        return empty();
    }

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

    for member in &members {
        if let Some(ref pubkey) = member.nostr_pubkey {
            let nip05_name = normalize_nip05_name(&member.display_name);
            if nip05_name.is_empty() {
                continue;
            }
            if requested_name == nip05_name {
                names.insert(nip05_name, pubkey.clone());
                if let Some(ref relay_url) = state.relay_url {
                    relays.insert(pubkey.clone(), vec![relay_url.clone()]);
                }
                // Single-name lookup: at most one match under the normalize rule.
                break;
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

pub(crate) const CHALLENGE_TTL_SECS: u64 = 300; // 5 minutes

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
pub(crate) fn sign_challenge_token(
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
pub(crate) fn verify_challenge_token(
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

    // Verify HMAC (data uses colon separator — only the token wire format uses pipes).
    //
    // Compare the raw 32-byte MACs in constant time, NOT the hex strings: a
    // variable-time `&str` compare (`!=`) short-circuits on the first differing
    // byte and leaks, via timing, how many leading bytes an attacker guessed —
    // a MAC-forgery oracle. `blake3::Hash`'s `PartialEq` is documented
    // constant-time, so parsing the provided hex into a `Hash` and comparing
    // `Hash == Hash` closes that oracle. A malformed (non-hex / wrong-length)
    // provided MAC simply fails to parse and is rejected.
    let hmac_data = format!("{challenge}:{member_id}:{expires_ts}");
    let expected = blake3::keyed_hash(key, hmac_data.as_bytes());
    let provided = blake3::Hash::from_hex(provided_hmac).map_err(|_| "Invalid token signature")?;

    if expected != provided {
        return Err("Invalid token signature");
    }

    Ok((challenge.to_string(), member_id.to_string()))
}

/// Max age (seconds) of a signed NIP-42 kind-22242 event, measured from the
/// event's `created_at` to now. nostr 0.44's `Event::verify()` checks only the
/// event id + Schnorr signature and does **not** bound `created_at`, so a
/// victim-signed event otherwise verifies forever. Bounding freshness to the
/// challenge TTL means a legitimate flow (fetch challenge → sign → submit, all
/// within `CHALLENGE_TTL_SECS`) always passes, while a replayed *old* captured
/// event is rejected.
pub(crate) const EVENT_MAX_AGE_SECS: u64 = CHALLENGE_TTL_SECS;

/// Tolerance (seconds) for a client clock running ahead of the server. Matches
/// the 60s relay-drift overlap already used by DM sync. Events dated more than
/// this into the future are rejected as clock-skew / forgery.
pub(crate) const EVENT_FUTURE_SKEW_SECS: u64 = 60;

/// Retention for a consumed-challenge entry in [`crate::challenge_store`]. Must
/// be at least as wide as the largest window in which a token/event could still
/// verify (`EVENT_MAX_AGE_SECS` past + `EVENT_FUTURE_SKEW_SECS` future) so a
/// replay attempt can never outlive its consumed marker.
pub(crate) const CONSUMED_CHALLENGE_TTL: std::time::Duration =
    std::time::Duration::from_secs(EVENT_MAX_AGE_SECS + EVENT_FUTURE_SKEW_SECS);

/// Reject a signed event whose `created_at` is outside the freshness window
/// `[now - EVENT_MAX_AGE_SECS, now + EVENT_FUTURE_SKEW_SECS]`. This is the
/// replay-closer: `Event::verify()` never inspects `created_at`, so without
/// this a single captured event is a permanently-replayable credential.
pub(crate) fn check_event_freshness(event_created_at_secs: u64) -> Result<(), &'static str> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    if event_created_at_secs > now.saturating_add(EVENT_FUTURE_SKEW_SECS) {
        return Err("event timestamp is in the future (outside window)");
    }
    if now.saturating_sub(event_created_at_secs) > EVENT_MAX_AGE_SECS {
        return Err("event too old / timestamp outside window");
    }
    Ok(())
}

/// POST /api/nostr/challenge — generate a challenge for the member to sign.
pub async fn nostr_challenge(
    State(state): State<AppState>,
    caller: OrgMember,
    Json(body): Json<ChallengeRequest>,
) -> Result<Json<ChallengeResponse>, (StatusCode, Json<ErrorResponse>)> {
    enforce_nostr_rate_limit(&state, &caller.member.id, RateClass::Interactive)?;

    let pubkey_hex = resolve_pubkey_hex(&body.pubkey).map_err(|_e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Internal error".into(),
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

    // Generate random challenge (OsRng — not thread_rng)
    let mut challenge_bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut challenge_bytes);
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
    enforce_nostr_rate_limit(&state, &caller.member.id, RateClass::Interactive)?;

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

    // 5b. Reject stale events. `Event::verify()` does not bound `created_at`, so
    // a captured event would otherwise verify forever — enforce a freshness window.
    check_event_freshness(body.signed_event.created_at.as_secs()).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
    })?;

    // 5c. One-time-use: block replay of this challenge within the TTL window.
    if !state
        .consumed_challenges
        .consume(&challenge, CONSUMED_CHALLENGE_TTL)
    {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "challenge already used".into(),
            }),
        ));
    }

    let pubkey_hex = body.signed_event.pubkey.to_hex();

    // 6. Reject if pubkey already linked to a different active member (DB also has UNIQUE index).
    if let Some(existing) = state
        .db
        .get_member_by_nostr_pubkey(&pubkey_hex)
        .await
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })?
        && existing.id != caller.member.id
    {
        return Err((
            StatusCode::CONFLICT,
            Json(ErrorResponse {
                error: "Nostr pubkey is already linked to another member".into(),
            }),
        ));
    }

    // 7. Update member's nostr_pubkey
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
            None,
            None,
            None,
        )
        .await
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })?;

    crate::routes::audit_log::audit(
        &state.db,
        &caller.member.id,
        scuffed_db::AuditAction::NostrKeyLinked,
        scuffed_db::AuditTargetType::Member,
        &caller.member.id,
        Some("Linked verified Nostr identity"),
    )
    .await;

    Ok(Json(updated))
}

// NOSTR-011 follow-up (scoped out of DR1 nostr-polish): the key-mode–changing
// ops below (unlink) and the export/import handlers should require a step-up
// reauth (recent password re-entry / fresh session assertion) so a stolen live
// session cannot silently export the nsec or flip key mode. Implementing this
// cleanly needs a `last_authenticated_at` on the session + a reauth challenge
// endpoint + UI, which is larger than this branch's footgun/rate-limit scope.
// Tracked for a dedicated follow-up; the per-member rate limit (DR1-NOSTR-006)
// and the import server-managed guard (DR1-NOSTR-003) bound the immediate risk.
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
            None,
            None,
            None,
        )
        .await
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })?;

    crate::routes::audit_log::audit(
        &state.db,
        &caller.member.id,
        scuffed_db::AuditAction::NostrKeyUnlinked,
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
    // Expensive NIP-49 (Argon2) op — bound it per member (DR1-NOSTR-006).
    enforce_nostr_rate_limit(&state, &caller.member.id, RateClass::KeyOp)?;

    // Align the backup password floor with the account password policy
    // (DR1-NOSTR-005): the ncryptsec is an offline-brute-forceable wrapper of the
    // live nsec, so a shorter floor than login passwords would understate its
    // strength.
    if body.password.len() < MIN_PASSWORD_LEN {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Password must be at least {MIN_PASSWORD_LEN} characters"),
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

    let mut secret_hex = state
        .db
        .get_nostr_secret_key(&caller.member.id)
        .await
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
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
    secret_hex.zeroize();

    crate::routes::audit_log::audit(
        &state.db,
        &caller.member.id,
        scuffed_db::AuditAction::NostrKeyExported,
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

/// POST /api/nostr/import-key — link an *external* Nostr key from an ncryptsec
/// backup.
///
/// This is **not** a restore-into-server-managed path: it decrypts the nsec only
/// to derive the pubkey, then flips the member to `external` mode (the server no
/// longer holds the secret). Because that is destructive to a server-managed
/// key, it is refused when the member currently has one — they must explicitly
/// `Unlink` first (DR1-NOSTR-003).
pub async fn nostr_import_key(
    State(state): State<AppState>,
    caller: OrgMember,
    Json(body): Json<ImportKeyRequest>,
) -> Result<Json<scuffed_db::Member>, (StatusCode, Json<ErrorResponse>)> {
    // Expensive NIP-49 (Argon2) op — bound it per member (DR1-NOSTR-006).
    enforce_nostr_rate_limit(&state, &caller.member.id, RateClass::KeyOp)?;

    // Refuse to silently destroy a server-managed key (DR1-NOSTR-003). Importing
    // sets external mode and clears the server-held secret; a server-managed
    // member must deliberately unlink first.
    if caller.member.nostr_key_mode == Some(NostrKeyMode::ServerManaged) {
        return Err((
            StatusCode::CONFLICT,
            Json(ErrorResponse {
                error: "You have a server-managed key. Unlink it first before importing an \
                        external key — importing switches you to external mode and discards the \
                        server-managed secret."
                    .into(),
            }),
        ));
    }

    let mut secret_hex = Zeroizing::new(
        scuffed_auth::nip49::decrypt(&body.ncryptsec, &body.password).map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Failed to decrypt: {e}"),
                }),
            )
        })?,
    );

    // Derive pubkey from the decrypted secret
    let keys = Keys::new(SecretKey::from_hex(&secret_hex).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid key in backup: {e}"),
            }),
        )
    })?);
    secret_hex.zeroize();
    let pubkey_hex = keys.public_key().to_hex();

    // Update member to external mode with this key
    state
        .db
        .set_external_nostr_key(&caller.member.id, &pubkey_hex)
        .await
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })?;

    let updated = state
        .db
        .get_member(&caller.member.id)
        .await
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
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
        scuffed_db::AuditAction::NostrKeyImported,
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

    let mut secret_hex = state
        .db
        .get_nostr_secret_key(&caller.member.id)
        .await
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
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
    secret_hex.zeroize();

    let members = state.db.list_nostr_identities().await.map_err(|_e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Internal error".into(),
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

    let mut secret_hex = state
        .db
        .get_nostr_secret_key(&caller.member.id)
        .await
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
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
    secret_hex.zeroize();

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

/// Officer-group ids for the public feed filter.
///
/// `Unknown` is a failed lookup. `Known` empty means the query succeeded and
/// returned nothing. Both hide every `h`-tagged post from non-officers: an
/// empty set cannot prove which group tags are public, and a failed lookup
/// must not fall open.
pub(crate) enum OfficerGroups {
    Known(HashSet<String>),
    Unknown,
}

pub(crate) fn officer_groups_from_lookup(
    result: Result<Vec<String>, impl std::fmt::Display>,
) -> OfficerGroups {
    match result {
        Ok(ids) => OfficerGroups::Known(ids.into_iter().collect()),
        Err(e) => {
            tracing::error!(error = %e, "officer group lookup failed; hiding grouped feed posts");
            OfficerGroups::Unknown
        }
    }
}

/// Active officer or admin who is not suspended or banned.
pub(crate) fn member_is_feed_officer(
    member: &scuffed_db::Member,
    suspended_or_banned: bool,
) -> bool {
    member.is_active && !suspended_or_banned && member.org_role.can_access_officer_channel()
}

fn event_has_h_tag(event: &scuffed_types::nostr::NostrEvent) -> bool {
    event
        .tags
        .iter()
        .any(|t| t.first().map(String::as_str) == Some("h"))
}

fn event_in_officer_group(
    event: &scuffed_types::nostr::NostrEvent,
    groups: &HashSet<String>,
) -> bool {
    event.tags.iter().any(|t| {
        t.first().map(String::as_str) == Some("h") && t.get(1).is_some_and(|g| groups.contains(g))
    })
}

/// Drop officer-group posts for callers who are not acting officers.
pub(crate) fn visible_feed_events(
    events: Vec<scuffed_types::nostr::NostrEvent>,
    groups: &OfficerGroups,
    is_officer: bool,
) -> Vec<scuffed_types::nostr::NostrEvent> {
    if is_officer {
        return events;
    }
    events
        .into_iter()
        .filter(|event| match groups {
            // Fail closed: a missing or empty restricted set hides every grouped post.
            OfficerGroups::Unknown => !event_has_h_tag(event),
            OfficerGroups::Known(set) if set.is_empty() => !event_has_h_tag(event),
            OfficerGroups::Known(set) => !event_in_officer_group(event, set),
        })
        .collect()
}

/// Cookie session counts as an officer only when the member is active and
/// not suspended or banned. Lookup errors are non-officers (fail closed).
async fn feed_caller_is_officer(
    state: &AppState,
    jar: &axum_extra::extract::cookie::CookieJar,
) -> bool {
    feed_caller_is_officer_with(state.db.as_ref(), &state.session_config.cookie_name, jar).await
}

trait FeedCallerLookups {
    async fn session_user(&self, token: &str) -> Result<Option<String>, scuffed_db::DbError>;
    async fn member_by_user(
        &self,
        user_id: &str,
    ) -> Result<Option<scuffed_db::Member>, scuffed_db::DbError>;
    async fn member_suspended(&self, member_id: &str) -> Result<bool, scuffed_db::DbError>;
}

impl FeedCallerLookups for scuffed_db::Database {
    async fn session_user(&self, token: &str) -> Result<Option<String>, scuffed_db::DbError> {
        self.get_session(token).await
    }

    async fn member_by_user(
        &self,
        user_id: &str,
    ) -> Result<Option<scuffed_db::Member>, scuffed_db::DbError> {
        scuffed_db::Database::get_member_by_user(self, user_id).await
    }

    async fn member_suspended(&self, member_id: &str) -> Result<bool, scuffed_db::DbError> {
        self.is_member_suspended_or_banned(member_id).await
    }
}

async fn feed_caller_is_officer_with(
    db: &impl FeedCallerLookups,
    cookie_name: &str,
    jar: &axum_extra::extract::cookie::CookieJar,
) -> bool {
    let Some(cookie) = jar.get(cookie_name) else {
        return false;
    };
    let Ok(Some(uid)) = db.session_user(cookie.value()).await else {
        return false;
    };
    let Ok(Some(member)) = db.member_by_user(&uid).await else {
        return false;
    };
    let suspended = match db.member_suspended(&member.id).await {
        Ok(flag) => flag,
        Err(e) => {
            tracing::error!(
                error = %e,
                "suspension lookup failed; treating feed caller as non-officer"
            );
            return false;
        }
    };
    member_is_feed_officer(&member, suspended)
}

fn group_post_denied(status: StatusCode, error: &str) -> (StatusCode, Json<ErrorResponse>) {
    (
        status,
        Json(ErrorResponse {
            error: error.into(),
        }),
    )
}

trait GroupPostLookups {
    async fn member_suspended(&self, member_id: &str) -> Result<bool, scuffed_db::DbError>;
    async fn channel_by_group_id(
        &self,
        group_id: &str,
    ) -> Result<Option<scuffed_db::TeamChannel>, scuffed_db::DbError>;
    async fn on_team_roster(
        &self,
        member_id: &str,
        team_id: &str,
    ) -> Result<bool, scuffed_db::DbError>;
}

impl GroupPostLookups for scuffed_db::Database {
    async fn member_suspended(&self, member_id: &str) -> Result<bool, scuffed_db::DbError> {
        self.is_member_suspended_or_banned(member_id).await
    }

    async fn channel_by_group_id(
        &self,
        group_id: &str,
    ) -> Result<Option<scuffed_db::TeamChannel>, scuffed_db::DbError> {
        self.get_channel_by_group_id(group_id).await
    }

    async fn on_team_roster(
        &self,
        member_id: &str,
        team_id: &str,
    ) -> Result<bool, scuffed_db::DbError> {
        self.is_on_team_roster(member_id, team_id).await
    }
}

/// `group_id` may be written only by an active, non-banned member of that
/// channel. Officer channels additionally require an officer or admin role.
/// The checked value is trimmed; callers must use that same string as the `h` tag.
async fn ensure_can_post_to_group(
    state: &AppState,
    caller: &OrgMember,
    group_id: &str,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    ensure_can_post_to_group_with(state.db.as_ref(), caller, group_id).await
}

async fn ensure_can_post_to_group_with(
    db: &impl GroupPostLookups,
    caller: &OrgMember,
    group_id: &str,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let group_id = group_id.trim();
    if group_id.is_empty() {
        return Err(group_post_denied(
            StatusCode::BAD_REQUEST,
            "group_id must not be empty",
        ));
    }
    if !caller.member.is_active {
        return Err(group_post_denied(
            StatusCode::FORBIDDEN,
            "Not allowed to post to this group",
        ));
    }
    let suspended = db.member_suspended(&caller.member.id).await.map_err(|e| {
        tracing::error!(error = %e, "suspension lookup failed during nostr post");
        group_post_denied(StatusCode::INTERNAL_SERVER_ERROR, "Internal error")
    })?;
    if suspended {
        return Err(group_post_denied(
            StatusCode::FORBIDDEN,
            "Not allowed to post to this group",
        ));
    }

    let channel = db.channel_by_group_id(group_id).await.map_err(|e| {
        tracing::error!(error = %e, "channel lookup failed during nostr post");
        group_post_denied(StatusCode::INTERNAL_SERVER_ERROR, "Internal error")
    })?;
    let Some(channel) = channel else {
        return Err(group_post_denied(
            StatusCode::NOT_FOUND,
            "Channel not found",
        ));
    };
    if !channel.is_active {
        return Err(group_post_denied(
            StatusCode::NOT_FOUND,
            "Channel not found",
        ));
    }

    let on_roster = db
        .on_team_roster(&caller.member.id, &channel.team_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "roster lookup failed during nostr post");
            group_post_denied(StatusCode::INTERNAL_SERVER_ERROR, "Internal error")
        })?;
    if !on_roster {
        return Err(group_post_denied(
            StatusCode::FORBIDDEN,
            "Not allowed to post to this group",
        ));
    }
    if channel.group_type == scuffed_db::GroupType::Officer
        && !caller.member.org_role.can_access_officer_channel()
    {
        return Err(group_post_denied(
            StatusCode::FORBIDDEN,
            "Not allowed to post to this group",
        ));
    }
    Ok(())
}

/// Trimmed group id for both the membership check and the event `h` tag.
/// `None` means the post is ungrouped. Surrounding whitespace is not stored.
async fn group_id_for_event(
    state: &AppState,
    caller: &OrgMember,
    raw: Option<&str>,
) -> Result<Option<String>, (StatusCode, Json<ErrorResponse>)> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let group_id = raw.trim();
    ensure_can_post_to_group(state, caller, group_id).await?;
    Ok(Some(group_id.to_string()))
}

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

    let is_officer = feed_caller_is_officer(&state, &jar).await;
    let officer_groups = officer_groups_from_lookup(state.db.list_officer_group_ids().await);
    let events = visible_feed_events(events, &officer_groups, is_officer);

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
    let group_id = group_id_for_event(&state, &caller, body.group_id.as_deref()).await?;

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

    let mut secret_hex = state
        .db
        .get_nostr_secret_key(&caller.member.id)
        .await
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
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
    secret_hex.zeroize();

    let event = EventBuilder::build_community_post(
        &keys,
        &body.content,
        &body.hashtags,
        body.community_id.as_deref(),
        group_id.as_deref(),
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
        .or_else(|| group_id.clone())
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

    // Defense in depth: blank `Some("")` from a stale process or mis-set env
    // must not report configured (F-AUI-003). Prefer `relay_url_from_env` at boot.
    let relay_url = match crate::state::normalize_relay_url(state.relay_url.clone()) {
        Some(url) => url,
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
///
/// Loads the encrypted secret via full member fetch (auth extractors omit secrets).
async fn require_server_managed_dm_caller(
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
    let full = state
        .db
        .get_member(&caller.member.id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "get_member for DM secret failed");
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
        })?;
    let blob = full.nostr_secret_key_encrypted.ok_or_else(|| {
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
    Ok(scuffed_chat::EncryptionService::new((*crypto).clone()))
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
    enforce_nostr_rate_limit(&state, &caller.member.id, RateClass::Interactive)?;

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
        require_server_managed_dm_caller(&state, &caller).await?;
    let encryption = require_encryption_service(&state)?;

    if recipient == sender_pubkey {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Cannot DM yourself".into(),
            }),
        ));
    }

    // NIP-17 DMs use a single conversation context as the `h` tag. Use the same
    // canonical, order-independent `conversation_key(a, b)` the DB uses as its
    // conversation identity (DR1-NOSTR-007) so both directions of a thread and
    // every device agree on one context — the previous `dm:{sender}:{recipient}`
    // form was order-dependent and split the two directions into distinct tags.
    let context_id = scuffed_db::conversation_key(&sender_pubkey, &recipient);

    let wraps = encryption
        .build_gift_wraps(
            &sender_blob,
            &sender_pubkey,
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

    // Publish to the relay first — never report success if the peer cannot receive.
    if let Err(e) = publish_event_oneshot(&relay_url, relay_event).await {
        tracing::error!("Failed to publish DM gift wrap: {e}");
        return Err((
            StatusCode::BAD_GATEWAY,
            Json(ErrorResponse {
                error: "Relay rejected or failed to accept the message".into(),
            }),
        ));
    }
    tracing::info!("Published DM gift wrap {gift_wrap_id}");

    // Store the sender's own copy so the UI can render it immediately.
    // Receiver-side dedup against the relay's later resync is handled by the
    // unique index on `gift_wrap_id`.
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

    crate::routes::audit_log::audit(
        &state.db,
        &caller.member.id,
        scuffed_db::AuditAction::SentDirectMessage,
        scuffed_db::AuditTargetType::DirectMessage,
        &gift_wrap_id,
        Some("Sent encrypted direct message"),
    )
    .await;

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
    let (relay_url, my_pubkey, my_blob) = require_server_managed_dm_caller(&state, &caller).await?;
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
            .unwrap_gift_wrap_json(&my_blob, &my_pubkey, &event_json)
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
    let (_, my_pubkey, _) = require_server_managed_dm_caller(&state, &caller).await?;
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
    let (_, my_pubkey, _) = require_server_managed_dm_caller(&state, &caller).await?;
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
    let (_, my_pubkey, _) = require_server_managed_dm_caller(&state, &caller).await?;
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
    let (_, my_pubkey, _) = require_server_managed_dm_caller(&state, &caller).await?;
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
    let (_, _, _) = require_server_managed_dm_caller(&state, &caller).await?;
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

#[cfg(test)]
mod hardening_tests {
    use super::*;

    fn now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    #[test]
    fn freshness_accepts_recent_and_small_skew() {
        assert!(check_event_freshness(now()).is_ok());
        // Within the past window and within the future skew tolerance.
        assert!(check_event_freshness(now() - (EVENT_MAX_AGE_SECS - 1)).is_ok());
        assert!(check_event_freshness(now() + (EVENT_FUTURE_SKEW_SECS - 1)).is_ok());
    }

    #[test]
    fn freshness_rejects_too_old_and_far_future() {
        let old = check_event_freshness(now() - EVENT_MAX_AGE_SECS - 5);
        assert_eq!(old, Err("event too old / timestamp outside window"));

        let future = check_event_freshness(now() + EVENT_FUTURE_SKEW_SECS + 5);
        assert!(future.is_err());
    }

    #[test]
    fn challenge_token_roundtrip_and_tamper_rejected() {
        let key = [7u8; 32];
        let expires = now() + CHALLENGE_TTL_SECS;
        let token = sign_challenge_token(&key, "scuffedclan-login:abc", "@login", expires);

        // Valid token verifies (constant-time MAC compare path).
        let (challenge, subject) = verify_challenge_token(&key, &token).expect("valid token");
        assert_eq!(challenge, "scuffedclan-login:abc");
        assert_eq!(subject, "@login");

        // Wrong key → MAC mismatch rejected.
        let wrong_key = [8u8; 32];
        assert!(verify_challenge_token(&wrong_key, &token).is_err());

        // Non-hex / malformed MAC in the token is rejected by the from_hex parse.
        use base64::Engine;
        let raw = String::from_utf8(
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(&token)
                .unwrap(),
        )
        .unwrap();
        let mut parts: Vec<&str> = raw.splitn(4, '|').collect();
        parts[3] = "not-hex";
        let mangled = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(parts.join("|"));
        assert!(verify_challenge_token(&key, &mangled).is_err());
    }
}

#[cfg(test)]
mod feed_acl_tests {
    use super::*;
    use axum::extract::State;
    use chrono::Utc;
    use scuffed_auth::{AuthProvider, User};
    use scuffed_db::{GroupType, ModerationActionType, OrgRole, TeamRole};
    use scuffed_types::nostr::NostrEvent;

    use crate::extractors::OrgMember;
    use crate::test_support::test_state;

    fn event(tags: Vec<Vec<&str>>) -> NostrEvent {
        NostrEvent {
            id: "evt".into(),
            pubkey: "pk".into(),
            created_at: 1,
            kind: 1,
            tags: tags
                .into_iter()
                .map(|t| t.into_iter().map(str::to_string).collect())
                .collect(),
            content: "secret".into(),
            sig: "sig".into(),
        }
    }

    fn ids_of(events: &[NostrEvent]) -> Vec<&str> {
        events.iter().map(|e| e.id.as_str()).collect()
    }

    fn tagged(id: &str, group: &str) -> NostrEvent {
        let mut ev = event(vec![vec!["h", group]]);
        ev.id = id.into();
        ev
    }

    #[test]
    fn failing_or_empty_group_lookup_hides_officer_posts() {
        let officer = tagged("off", "officers");
        let public = tagged("pub", "team-public");
        let mut plain = event(vec![vec!["t", "lfg"]]);
        plain.id = "plain".into();
        let events = vec![officer, public, plain];

        let hidden = visible_feed_events(events.clone(), &OfficerGroups::Unknown, false);
        assert_eq!(ids_of(&hidden), vec!["plain"]);

        let empty =
            visible_feed_events(events.clone(), &OfficerGroups::Known(HashSet::new()), false);
        assert_eq!(ids_of(&empty), vec!["plain"]);

        let mut known = HashSet::new();
        known.insert("officers".into());
        let filtered = visible_feed_events(events, &OfficerGroups::Known(known), false);
        assert_eq!(ids_of(&filtered), vec!["pub", "plain"]);

        let mut known = HashSet::new();
        known.insert("officers".into());
        let as_officer = visible_feed_events(
            vec![tagged("off", "officers")],
            &OfficerGroups::Known(known),
            true,
        );
        assert_eq!(ids_of(&as_officer), vec!["off"]);
    }

    #[test]
    fn lookup_error_maps_to_unknown() {
        let groups = officer_groups_from_lookup(Err::<Vec<String>, _>("db down"));
        assert!(matches!(groups, OfficerGroups::Unknown));
        let groups = officer_groups_from_lookup(Ok::<_, &str>(vec!["officers".into()]));
        match groups {
            OfficerGroups::Known(set) => assert!(set.contains("officers")),
            OfficerGroups::Unknown => panic!("successful lookup must stay known"),
        }
    }

    fn caller(member: scuffed_db::Member) -> OrgMember {
        OrgMember {
            user: User {
                id: member.user_id.clone(),
                provider: AuthProvider::Local,
                provider_id: member.user_id.clone(),
                username: member.display_name.clone(),
                avatar_url: None,
                created_at: Utc::now(),
            },
            member,
        }
    }

    #[tokio::test]
    async fn deactivated_officer_group_stays_hidden_and_inactive_members_are_not_officers() {
        let state = test_state().await;
        let game = state.db.create_game("Overwatch", Some("OW")).await.unwrap();
        let team = state
            .db
            .create_team("Alpha", &game.id, None, None, None)
            .await
            .unwrap();
        let channel = state
            .db
            .create_team_channel(
                &team.id,
                "alpha-officers",
                GroupType::Officer,
                "ws://relay.test",
            )
            .await
            .unwrap();
        state
            .db
            .deactivate_team_channel(&channel.group_id)
            .await
            .unwrap();
        state
            .db
            .create_team_channel(
                &team.id,
                "alpha-public",
                GroupType::Public,
                "ws://relay.test",
            )
            .await
            .unwrap();

        let ids = state.db.list_officer_group_ids().await.unwrap();
        assert!(
            ids.iter().any(|id| id == "alpha-officers"),
            "deactivated officer groups stay in the restricted set: {ids:?}"
        );
        assert!(
            !ids.iter().any(|id| id == "alpha-public"),
            "public groups are not restricted: {ids:?}"
        );

        let groups = OfficerGroups::Known(ids.into_iter().collect());
        let visible = visible_feed_events(
            vec![
                tagged("hist", "alpha-officers"),
                tagged("pub", "alpha-public"),
            ],
            &groups,
            false,
        );
        assert_eq!(ids_of(&visible), vec!["pub"]);

        let officer = state
            .db
            .create_member("officer-user", "Officer", OrgRole::Officer)
            .await
            .unwrap();
        assert!(member_is_feed_officer(&officer, false));
        assert!(!member_is_feed_officer(&officer, true));

        state
            .db
            .update_member(
                &officer.id,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(false),
                None,
                None,
                None,
            )
            .await
            .unwrap();
        let deactivated = state.db.get_member(&officer.id).await.unwrap().unwrap();
        assert!(!member_is_feed_officer(&deactivated, false));

        let banned = state
            .db
            .create_member("banned-user", "BannedOff", OrgRole::Officer)
            .await
            .unwrap();
        state
            .db
            .create_moderation_action(
                &banned.id,
                ModerationActionType::Ban,
                "test",
                &officer.id,
                None,
            )
            .await
            .unwrap();
        let suspended = state
            .db
            .is_member_suspended_or_banned(&banned.id)
            .await
            .unwrap();
        assert!(suspended);
        assert!(!member_is_feed_officer(&banned, suspended));
    }

    #[tokio::test]
    async fn recruit_post_to_officer_group_is_forbidden() {
        let state = test_state().await;
        let game = state.db.create_game("Overwatch", Some("OW")).await.unwrap();
        let team = state
            .db
            .create_team("Alpha", &game.id, None, None, None)
            .await
            .unwrap();
        state
            .db
            .create_team_channel(
                &team.id,
                "alpha-officers",
                GroupType::Officer,
                "ws://relay.test",
            )
            .await
            .unwrap();
        state
            .db
            .create_team_channel(
                &team.id,
                "alpha-public",
                GroupType::Public,
                "ws://relay.test",
            )
            .await
            .unwrap();

        let recruit = state
            .db
            .create_member("recruit-user", "Recruit", OrgRole::Recruit)
            .await
            .unwrap();
        state
            .db
            .add_to_roster(&recruit.id, &team.id, TeamRole::Player)
            .await
            .unwrap();
        let officer = state
            .db
            .create_member("officer-user", "Officer", OrgRole::Officer)
            .await
            .unwrap();
        state
            .db
            .add_to_roster(&officer.id, &team.id, TeamRole::Player)
            .await
            .unwrap();

        let body = CommunityPostRequest {
            content: "hello".into(),
            hashtags: vec![],
            community_id: None,
            group_id: Some("alpha-officers".into()),
            reply_to: None,
            root: None,
        };
        let err = match nostr_post(State(state.clone()), caller(recruit), Json(body)).await {
            Err(err) => err,
            Ok(_) => panic!("recruit must not post to an officer group"),
        };
        assert_eq!(err.0, StatusCode::FORBIDDEN);

        let body = CommunityPostRequest {
            content: "hello".into(),
            hashtags: vec![],
            community_id: None,
            group_id: Some("alpha-officers".into()),
            reply_to: None,
            root: None,
        };
        // Officer passes the role check. This fixture has no relay configured,
        // so the handler stops at 503 instead of 403.
        let err = match nostr_post(State(state.clone()), caller(officer), Json(body)).await {
            Err(err) => err,
            Ok(_) => panic!("officer has no relay configured in this fixture"),
        };
        assert_eq!(err.0, StatusCode::SERVICE_UNAVAILABLE);

        let stranger = state
            .db
            .create_member("stranger-user", "Stranger", OrgRole::Officer)
            .await
            .unwrap();
        let body = CommunityPostRequest {
            content: "hello".into(),
            hashtags: vec![],
            community_id: None,
            group_id: Some("alpha-public".into()),
            reply_to: None,
            root: None,
        };
        let err = match nostr_post(State(state), caller(stranger), Json(body)).await {
            Err(err) => err,
            Ok(_) => panic!("not a roster member"),
        };
        assert_eq!(err.0, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn padded_officer_group_id_is_checked_and_tagged_trimmed() {
        let state = test_state().await;
        let game = state.db.create_game("Overwatch", Some("OW")).await.unwrap();
        let team = state
            .db
            .create_team("Alpha", &game.id, None, None, None)
            .await
            .unwrap();
        state
            .db
            .create_team_channel(&team.id, "officers", GroupType::Officer, "ws://relay.test")
            .await
            .unwrap();

        let recruit = state
            .db
            .create_member("pad-recruit", "Recruit", OrgRole::Recruit)
            .await
            .unwrap();
        state
            .db
            .add_to_roster(&recruit.id, &team.id, TeamRole::Player)
            .await
            .unwrap();
        let officer = state
            .db
            .create_member("pad-officer", "Officer", OrgRole::Officer)
            .await
            .unwrap();
        state
            .db
            .add_to_roster(&officer.id, &team.id, TeamRole::Player)
            .await
            .unwrap();

        let body = CommunityPostRequest {
            content: "hello".into(),
            hashtags: vec![],
            community_id: None,
            group_id: Some(" officers".into()),
            reply_to: None,
            root: None,
        };
        let err = match nostr_post(State(state.clone()), caller(recruit), Json(body)).await {
            Err(err) => err,
            Ok(_) => panic!("padded officer group id must still be rejected"),
        };
        assert_eq!(err.0, StatusCode::FORBIDDEN);
        assert_eq!(err.1.0.error, "Not allowed to post to this group");

        let h_tag = match group_id_for_event(&state, &caller(officer), Some(" officers")).await {
            Ok(value) => value,
            Err(err) => panic!("officer may post to the trimmed officer group: {}", err.0),
        };
        assert_eq!(h_tag.as_deref(), Some("officers"));

        let mut known = HashSet::new();
        known.insert("officers".into());
        let groups = OfficerGroups::Known(known);
        let leaked = visible_feed_events(vec![tagged("pad", " officers")], &groups, false);
        assert_eq!(
            ids_of(&leaked),
            vec!["pad"],
            "an untrimmed h tag would miss the officer set and show publicly"
        );
        let hidden = visible_feed_events(
            vec![tagged("pad", h_tag.as_deref().unwrap())],
            &groups,
            false,
        );
        assert!(hidden.is_empty(), "the trimmed h tag stays officer-only");
    }

    fn active_officer() -> scuffed_db::Member {
        scuffed_db::Member {
            id: "member-1".into(),
            user_id: "user-1".into(),
            org_role: OrgRole::Officer,
            display_name: "Officer".into(),
            bio: None,
            avatar_url: None,
            timezone: None,
            pronouns: None,
            availability_status: None,
            nostr_pubkey: None,
            nostr_key_mode: None,
            nostr_secret_key_encrypted: None,
            joined_at: Utc::now(),
            is_active: true,
            main_role: None,
            twitch: None,
            twitter: None,
        }
    }

    fn open_channel(group_id: &str) -> scuffed_db::TeamChannel {
        scuffed_db::TeamChannel {
            id: "channel-1".into(),
            team_id: "team-1".into(),
            group_id: group_id.into(),
            group_type: GroupType::Public,
            relay_url: "ws://relay.test".into(),
            is_active: true,
            created_at: Utc::now(),
            synced_at: None,
        }
    }

    fn assert_post_lookup_failed(err: (StatusCode, Json<ErrorResponse>)) {
        assert_eq!(err.0, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(err.1.0.error, "Internal error");
    }

    struct FeedLookup {
        fail_suspended: bool,
    }

    impl FeedCallerLookups for FeedLookup {
        async fn session_user(&self, _token: &str) -> Result<Option<String>, scuffed_db::DbError> {
            Ok(Some("user-1".into()))
        }

        async fn member_by_user(
            &self,
            _user_id: &str,
        ) -> Result<Option<scuffed_db::Member>, scuffed_db::DbError> {
            Ok(Some(active_officer()))
        }

        async fn member_suspended(&self, _member_id: &str) -> Result<bool, scuffed_db::DbError> {
            if self.fail_suspended {
                Err(scuffed_db::DbError::Timeout)
            } else {
                Ok(false)
            }
        }
    }

    #[tokio::test]
    async fn suspension_lookup_failure_treats_feed_caller_as_non_officer() {
        use axum_extra::extract::cookie::{Cookie, CookieJar};

        let jar = CookieJar::new().add(Cookie::new("sid", "token"));
        let failed = feed_caller_is_officer_with(
            &FeedLookup {
                fail_suspended: true,
            },
            "sid",
            &jar,
        )
        .await;
        assert!(!failed, "a suspension lookup error is not an officer");

        let clear = feed_caller_is_officer_with(
            &FeedLookup {
                fail_suspended: false,
            },
            "sid",
            &jar,
        )
        .await;
        assert!(
            clear,
            "the same officer is recognized when the lookup succeeds"
        );
    }

    enum PostLookupFail {
        Suspended,
        Channel,
        Roster,
    }

    struct PostLookup {
        mode: PostLookupFail,
    }

    impl GroupPostLookups for PostLookup {
        async fn member_suspended(&self, _member_id: &str) -> Result<bool, scuffed_db::DbError> {
            match self.mode {
                PostLookupFail::Suspended => Err(scuffed_db::DbError::Timeout),
                PostLookupFail::Channel | PostLookupFail::Roster => Ok(false),
            }
        }

        async fn channel_by_group_id(
            &self,
            group_id: &str,
        ) -> Result<Option<scuffed_db::TeamChannel>, scuffed_db::DbError> {
            match self.mode {
                PostLookupFail::Channel => Err(scuffed_db::DbError::Timeout),
                PostLookupFail::Roster => Ok(Some(open_channel(group_id))),
                PostLookupFail::Suspended => panic!("channel lookup runs after suspension"),
            }
        }

        async fn on_team_roster(
            &self,
            _member_id: &str,
            _team_id: &str,
        ) -> Result<bool, scuffed_db::DbError> {
            match self.mode {
                PostLookupFail::Roster => Err(scuffed_db::DbError::Timeout),
                PostLookupFail::Suspended | PostLookupFail::Channel => {
                    panic!("roster lookup runs after channel")
                }
            }
        }
    }

    #[tokio::test]
    async fn group_post_lookup_failures_are_denied() {
        let caller = caller(active_officer());
        let suspended = match ensure_can_post_to_group_with(
            &PostLookup {
                mode: PostLookupFail::Suspended,
            },
            &caller,
            "officers",
        )
        .await
        {
            Err(err) => err,
            Ok(()) => panic!("suspension lookup failure"),
        };
        assert_post_lookup_failed(suspended);

        let channel = match ensure_can_post_to_group_with(
            &PostLookup {
                mode: PostLookupFail::Channel,
            },
            &caller,
            "officers",
        )
        .await
        {
            Err(err) => err,
            Ok(()) => panic!("channel lookup failure"),
        };
        assert_post_lookup_failed(channel);

        let roster = match ensure_can_post_to_group_with(
            &PostLookup {
                mode: PostLookupFail::Roster,
            },
            &caller,
            "officers",
        )
        .await
        {
            Err(err) => err,
            Ok(()) => panic!("roster lookup failure"),
        };
        assert_post_lookup_failed(roster);
    }
}
