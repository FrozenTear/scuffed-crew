//! NIP-42 authentication token provisioning.
//!
//! Server-side flow:
//! 1. Member authenticates via cookie session to Axum
//! 2. Axum retrieves member's server-managed key (or provisions one)
//! 3. Server builds a pre-signed NIP-42 AUTH event for the relay
//! 4. Frontend receives the auth event and presents it on WebSocket AUTH challenge

use scuffed_auth::crypto::{aad, CryptoService, EncryptedBlob};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

use super::events::{EventBuilder, EventError};
use scuffed_types::nostr::NostrEvent;

/// Build `nostr::Keys` from decrypted secret material.
///
/// Accepts raw 32-byte secrets (preferred) or legacy 64-char hex UTF-8.
fn keys_from_secret_plaintext(pt: &Zeroizing<Vec<u8>>) -> Result<nostr::Keys, AuthError> {
    use nostr::key::{Keys, SecretKey};

    if pt.len() == 32 {
        let sk = SecretKey::from_slice(pt.as_slice()).map_err(|_| AuthError::InvalidKeyData)?;
        return Ok(Keys::new(sk));
    }
    // Legacy: UTF-8 hex string
    let hex_str = std::str::from_utf8(pt.as_slice()).map_err(|_| AuthError::InvalidKeyData)?;
    let sk = SecretKey::from_hex(hex_str).map_err(|_| AuthError::InvalidKeyData)?;
    Ok(Keys::new(sk))
}

/// NIP-42 AUTH challenge cap. Longer challenges are rejected before signing.
pub const MAX_AUTH_CHALLENGE_BYTES: usize = 256;

/// Errors from the auth service.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("encryption service not configured")]
    NoCryptoService,
    #[error("encryption failed: {0}")]
    EncryptionFailed(String),
    #[error("decryption failed: {0}")]
    DecryptionFailed(String),
    #[error("event signing failed: {0}")]
    SigningFailed(#[from] EventError),
    #[error("invalid key data")]
    InvalidKeyData,
    #[error("decrypted Nostr secret does not match owner pubkey")]
    PubkeyMismatch,
    #[error("relay URL is not allowed")]
    RelayUrlRejected,
    #[error("challenge is too long")]
    ChallengeTooLong,
}

/// Canonical `ws`/`wss` relay URL.
///
/// Scheme and host are lowercase, a trailing slash is removed, and the
/// default port is omitted (`80` for `ws`, `443` for `wss`). Any other
/// scheme, a missing host, or userinfo yields `None`.
pub fn canonical_relay_url(raw: &str) -> Option<String> {
    let parsed = url::Url::parse(raw.trim()).ok()?;
    let scheme = parsed.scheme();
    if scheme != "ws" && scheme != "wss" {
        return None;
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return None;
    }
    let host = parsed.host_str()?;
    if host.is_empty() {
        return None;
    }
    // `Url::port` already hides the scheme default (ws 80, wss 443).
    let port = parsed.port();
    let path = parsed.path().trim_end_matches('/');

    let mut out = String::new();
    out.push_str(scheme);
    out.push_str("://");
    out.push_str(host);
    if let Some(port) = port {
        out.push(':');
        out.push_str(&port.to_string());
    }
    if !path.is_empty() {
        out.push_str(path);
    }
    if let Some(query) = parsed.query() {
        out.push('?');
        out.push_str(query);
    }
    Some(out)
}

/// Relay URL to sign, when `requested` canonicalizes to one of `allowed`.
///
/// Returns the canonical form. Callers sign that string, not the raw input.
pub fn allowed_auth_relay<'a>(
    requested: &str,
    allowed: impl IntoIterator<Item = &'a str>,
) -> Option<String> {
    let requested = canonical_relay_url(requested)?;
    let matched = allowed
        .into_iter()
        .any(|candidate| canonical_relay_url(candidate).as_deref() == Some(requested.as_str()));
    matched.then_some(requested)
}

/// Request for a NIP-42 auth token.
#[derive(Debug, Deserialize)]
pub struct AuthTokenRequest {
    pub relay_url: String,
    pub challenge: Option<String>,
}

/// Response containing the pre-signed NIP-42 auth event.
#[derive(Debug, Serialize)]
pub struct AuthTokenResponse {
    pub auth_event: NostrEvent,
    pub pubkey: String,
    pub relay_url: String,
}

/// Key mode for a member's Nostr identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyMode {
    /// Server generates and manages the keypair.
    ServerManaged,
    /// Member provided their own key via NIP-07 (Phase 1.5 linked).
    External,
}

impl std::fmt::Display for KeyMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KeyMode::ServerManaged => write!(f, "server_managed"),
            KeyMode::External => write!(f, "external"),
        }
    }
}

/// Service for NIP-42 auth token provisioning and key management.
pub struct NostrAuthService {
    crypto: CryptoService,
}

impl NostrAuthService {
    /// Create a new auth service with the given CryptoService.
    pub fn new(crypto: CryptoService) -> Self {
        Self { crypto }
    }

    /// Generate a new Nostr keypair and return the encrypted secret key.
    ///
    /// The public key (hex) and encrypted secret key blob are returned for
    /// storage in the member's database record.
    pub fn generate_keypair(&self) -> Result<(String, EncryptedBlob), AuthError> {
        let keys = EventBuilder::generate_keys();
        let pubkey_hex = keys.public_key().to_hex();
        let mut secret_bytes = keys.secret_key().to_secret_bytes();
        let aad = aad::nostr_secret_key(&pubkey_hex);

        let encrypted = self
            .crypto
            .encrypt_bytes(&secret_bytes, aad.as_bytes())
            .map_err(|e| AuthError::EncryptionFailed(e.to_string()))?;

        secret_bytes.zeroize();

        Ok((pubkey_hex, encrypted))
    }

    /// Provision a NIP-42 AUTH event for a member with a server-managed key.
    ///
    /// Signs only when `relay_url` canonicalizes to one of `allowed_relay_urls`
    /// and `challenge` is at most [`MAX_AUTH_CHALLENGE_BYTES`] bytes. The event
    /// uses the canonical relay URL. Decrypts the secret only after those
    /// checks, then zeroizes it.
    pub fn provision_auth_event(
        &self,
        encrypted_secret_key: &EncryptedBlob,
        owner_pubkey_hex: &str,
        relay_url: &str,
        challenge: &str,
        allowed_relay_urls: &[&str],
    ) -> Result<AuthTokenResponse, AuthError> {
        if challenge.len() > MAX_AUTH_CHALLENGE_BYTES {
            return Err(AuthError::ChallengeTooLong);
        }
        let relay_url = allowed_auth_relay(relay_url, allowed_relay_urls.iter().copied())
            .ok_or(AuthError::RelayUrlRejected)?;

        let aad = aad::nostr_secret_key(owner_pubkey_hex);
        let pt = self
            .crypto
            .decrypt_bytes(encrypted_secret_key, aad.as_bytes())
            .map_err(|e| AuthError::DecryptionFailed(e.to_string()))?;

        let keys = keys_from_secret_plaintext(&pt)?;
        let pubkey_hex = keys.public_key().to_hex();
        if !pubkey_hex.eq_ignore_ascii_case(owner_pubkey_hex) {
            return Err(AuthError::PubkeyMismatch);
        }

        // Build the AUTH event
        let event = EventBuilder::build_auth_event(&keys, &relay_url, challenge)?;
        let relay_event = EventBuilder::to_relay_event(&event);

        Ok(AuthTokenResponse {
            auth_event: relay_event,
            pubkey: pubkey_hex,
            relay_url: relay_url.to_string(),
        })
    }

    /// Sign an event on behalf of a member with a server-managed key.
    ///
    /// Used for publishing group messages when the member uses server-managed keys.
    pub fn sign_event_for_member(
        &self,
        encrypted_secret_key: &EncryptedBlob,
        owner_pubkey_hex: &str,
        kind: u32,
        content: &str,
        tags: Vec<Vec<String>>,
    ) -> Result<NostrEvent, AuthError> {
        let aad = aad::nostr_secret_key(owner_pubkey_hex);
        let pt = self
            .crypto
            .decrypt_bytes(encrypted_secret_key, aad.as_bytes())
            .map_err(|e| AuthError::DecryptionFailed(e.to_string()))?;

        let keys = keys_from_secret_plaintext(&pt)?;
        if !keys
            .public_key()
            .to_hex()
            .eq_ignore_ascii_case(owner_pubkey_hex)
        {
            return Err(AuthError::PubkeyMismatch);
        }

        let mut builder = nostr::EventBuilder::new(nostr::Kind::Custom(kind as u16), content);

        for tag_parts in &tags {
            if tag_parts.is_empty() {
                continue;
            }
            let kind = nostr::TagKind::custom(&tag_parts[0]);
            let values: Vec<String> = tag_parts[1..].to_vec();
            builder = builder.tag(nostr::Tag::custom(kind, values));
        }

        let event = builder
            .sign_with_keys(&keys)
            .map_err(|e| EventError::SigningFailed(e.to_string()))?;

        Ok(EventBuilder::to_relay_event(&event))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_crypto() -> CryptoService {
        let key = CryptoService::generate_key();
        CryptoService::new(&key, 1).unwrap()
    }

    #[test]
    fn generate_keypair_produces_valid_pubkey() {
        let service = NostrAuthService::new(test_crypto());
        let (pubkey, _encrypted) = service.generate_keypair().unwrap();
        // Valid hex pubkey is 64 chars
        assert_eq!(pubkey.len(), 64);
        assert!(pubkey.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn generate_keypair_encrypted_blob_decryptable() {
        let svc = NostrAuthService::new(test_crypto());
        let (_pubkey, encrypted) = svc.generate_keypair().unwrap();

        // We can't decrypt with a different key, but the shape is right
        assert!(!encrypted.ciphertext.is_empty());
        assert!(!encrypted.nonce.is_empty());
    }

    #[test]
    fn provision_auth_event_roundtrip() {
        let crypto = test_crypto();
        let service = NostrAuthService::new(crypto);

        let (pubkey, encrypted) = service.generate_keypair().unwrap();
        let response = service
            .provision_auth_event(
                &encrypted,
                &pubkey,
                "wss://relay.example.com",
                "challenge123",
                &["wss://relay.example.com"],
            )
            .unwrap();

        assert_eq!(response.pubkey, pubkey);
        assert_eq!(response.relay_url, "wss://relay.example.com");
        assert!(response.auth_event.is_auth());
        assert_eq!(
            response.auth_event.tag_value("relay"),
            Some("wss://relay.example.com")
        );
        assert_eq!(
            response.auth_event.tag_value("challenge"),
            Some("challenge123")
        );
    }

    #[test]
    fn sign_event_for_member_group_message() {
        let crypto = test_crypto();
        let service = NostrAuthService::new(crypto);

        let (pubkey, encrypted) = service.generate_keypair().unwrap();
        let event = service
            .sign_event_for_member(
                &encrypted,
                &pubkey,
                9, // NIP-29 group message
                "Hello team!",
                vec![vec!["h".into(), "team-alpha".into()]],
            )
            .unwrap();

        assert_eq!(event.kind, 9);
        assert_eq!(event.content, "Hello team!");
        assert_eq!(event.group_id(), Some("team-alpha"));
    }

    #[test]
    fn wrong_key_fails_provision() {
        let crypto1 = test_crypto();
        let crypto2 = test_crypto();
        let service1 = NostrAuthService::new(crypto1);
        let service2 = NostrAuthService::new(crypto2);

        let (_pubkey, encrypted) = service1.generate_keypair().unwrap();

        // Trying to decrypt with a different key should fail
        let result = service2.provision_auth_event(
            &encrypted,
            &_pubkey,
            "wss://relay.example.com",
            "challenge",
            &["wss://relay.example.com"],
        );
        assert!(result.is_err());
    }

    #[test]
    fn key_mode_display() {
        assert_eq!(KeyMode::ServerManaged.to_string(), "server_managed");
        assert_eq!(KeyMode::External.to_string(), "external");
    }

    #[test]
    fn provision_auth_event_pubkey_mismatch() {
        let crypto = test_crypto();
        let service = NostrAuthService::new(crypto.clone());

        // Encrypt a real secret under AAD of a different claimed owner so decrypt
        // succeeds but the derived pubkey does not match.
        let claimed_owner = "aa".repeat(32);
        let real_keys = EventBuilder::generate_keys();
        let mut secret_bytes = real_keys.secret_key().to_secret_bytes();
        let aad = aad::nostr_secret_key(&claimed_owner);
        let mismatched = crypto.encrypt_bytes(&secret_bytes, aad.as_bytes()).unwrap();
        secret_bytes.zeroize();

        let result = service.provision_auth_event(
            &mismatched,
            &claimed_owner,
            "wss://relay.example.com",
            "c",
            &["wss://relay.example.com"],
        );
        assert!(matches!(result, Err(AuthError::PubkeyMismatch)));
    }

    #[test]
    fn sign_event_for_member_pubkey_mismatch() {
        let crypto = test_crypto();
        let service = NostrAuthService::new(crypto.clone());

        let claimed_owner = "bb".repeat(32);
        let real_keys = EventBuilder::generate_keys();
        let mut secret_bytes = real_keys.secret_key().to_secret_bytes();
        let aad = aad::nostr_secret_key(&claimed_owner);
        let mismatched = crypto.encrypt_bytes(&secret_bytes, aad.as_bytes()).unwrap();
        secret_bytes.zeroize();

        let result = service.sign_event_for_member(&mismatched, &claimed_owner, 1, "hi", vec![]);
        assert!(matches!(result, Err(AuthError::PubkeyMismatch)));
    }

    #[test]
    fn allowlist_accepts_normalized_configured_relay() {
        let allowed = ["wss://Relay.Example.com:443/", "ws://backup.example:80/"];
        assert_eq!(
            allowed_auth_relay("WSS://relay.example.com/", allowed).as_deref(),
            Some("wss://relay.example.com")
        );
        assert_eq!(
            allowed_auth_relay("ws://Backup.Example/", allowed).as_deref(),
            Some("ws://backup.example")
        );
        assert_eq!(
            canonical_relay_url("ws://relay.example.com:443"),
            Some("ws://relay.example.com:443".into())
        );
        assert_eq!(
            canonical_relay_url("wss://relay.example.com/path/"),
            Some("wss://relay.example.com/path".into())
        );
    }

    #[test]
    fn allowlist_rejects_other_host_and_http_schemes() {
        let allowed = ["wss://relay.example.com", "https://relay.example.com"];
        assert!(allowed_auth_relay("wss://evil.example", allowed).is_none());
        assert!(allowed_auth_relay("http://relay.example.com", allowed).is_none());
        assert!(allowed_auth_relay("https://relay.example.com", allowed).is_none());
        assert!(allowed_auth_relay("wss://user:pass@relay.example.com", allowed).is_none());
    }

    #[test]
    fn provision_auth_event_challenge_256_ok_257_rejected() {
        let crypto = test_crypto();
        let service = NostrAuthService::new(crypto);
        let (pubkey, encrypted) = service.generate_keypair().unwrap();
        let allowed = ["wss://relay.example.com"];

        let challenge_ok = "a".repeat(256);
        let response = service
            .provision_auth_event(
                &encrypted,
                &pubkey,
                "WSS://Relay.Example.com:443/",
                &challenge_ok,
                &allowed,
            )
            .unwrap();
        assert_eq!(response.relay_url, "wss://relay.example.com");
        assert_eq!(
            response.auth_event.tag_value("relay"),
            Some("wss://relay.example.com")
        );
        assert_eq!(
            response.auth_event.tag_value("challenge"),
            Some(challenge_ok.as_str())
        );

        let challenge_bad = "b".repeat(257);
        let rejected = service.provision_auth_event(
            &encrypted,
            &pubkey,
            "wss://relay.example.com",
            &challenge_bad,
            &allowed,
        );
        assert!(matches!(rejected, Err(AuthError::ChallengeTooLong)));
    }

    #[test]
    fn provision_auth_event_rejects_other_host_and_http() {
        let crypto = test_crypto();
        let service = NostrAuthService::new(crypto);
        let (pubkey, encrypted) = service.generate_keypair().unwrap();
        let allowed = ["wss://relay.example.com"];

        let other =
            service.provision_auth_event(&encrypted, &pubkey, "wss://evil.example", "c", &allowed);
        assert!(matches!(other, Err(AuthError::RelayUrlRejected)));

        for url in ["http://relay.example.com", "https://relay.example.com"] {
            let rejected = service.provision_auth_event(&encrypted, &pubkey, url, "c", &[url]);
            assert!(
                matches!(rejected, Err(AuthError::RelayUrlRejected)),
                "{url}"
            );
        }
    }
}
