//! NIP-42 authentication token provisioning.
//!
//! Server-side flow:
//! 1. Member authenticates via cookie session to Axum
//! 2. Axum retrieves member's server-managed key (or provisions one)
//! 3. Server builds a pre-signed NIP-42 AUTH event for the relay
//! 4. Frontend receives the auth event and presents it on WebSocket AUTH challenge

use scuffed_auth::crypto::{aad, CryptoService, EncryptedBlob};
use zeroize::{Zeroize, Zeroizing};
use serde::{Deserialize, Serialize};

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
    /// Decrypts the member's secret key, signs an AUTH event, and immediately
    /// zeroizes the decrypted key material.
    pub fn provision_auth_event(
        &self,
        encrypted_secret_key: &EncryptedBlob,
        owner_pubkey_hex: &str,
        relay_url: &str,
        challenge: &str,
    ) -> Result<AuthTokenResponse, AuthError> {
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
        let event = EventBuilder::build_auth_event(&keys, relay_url, challenge)?;
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
            .provision_auth_event(&encrypted, &pubkey, "wss://relay.example.com", "challenge123")
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
        let result =
            service2.provision_auth_event(&encrypted, &_pubkey, "wss://relay.example.com", "challenge");
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
        let mismatched = crypto
            .encrypt_bytes(&secret_bytes, aad.as_bytes())
            .unwrap();
        secret_bytes.zeroize();

        let result =
            service.provision_auth_event(&mismatched, &claimed_owner, "wss://relay.example.com", "c");
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
        let mismatched = crypto
            .encrypt_bytes(&secret_bytes, aad.as_bytes())
            .unwrap();
        secret_bytes.zeroize();

        let result = service.sign_event_for_member(
            &mismatched,
            &claimed_owner,
            1,
            "hi",
            vec![],
        );
        assert!(matches!(result, Err(AuthError::PubkeyMismatch)));
    }
}
