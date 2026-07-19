//! Cryptographic utilities for secure data storage.
//!
//! - BLAKE3 hashing for high-entropy session tokens
//! - AES-256-GCM with **AAD** (domain-separated) field encryption
//! - Multi-version keyring for rotation (`ENCRYPTION_KEY` + optional previous keys)

use std::collections::HashMap;
use std::sync::Arc;

use aes_gcm::{
    aead::{Aead, KeyInit, OsRng, Payload},
    Aes256Gcm, Nonce,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use zeroize::{Zeroize, Zeroizing};

/// Hash a session token for storage using BLAKE3.
///
/// Session tokens are high-entropy random values — fast keyed hash is appropriate.
pub fn hash_session_token(token: &str) -> String {
    let hash = blake3::hash(token.as_bytes());
    hash.to_hex().to_string()
}

/// Verify a session token against its stored hash (constant-time compare).
pub fn verify_session_token(token: &str, stored_hash: &str) -> bool {
    let computed = hash_session_token(token);
    constant_time_eq(computed.as_bytes(), stored_hash.as_bytes())
}

/// Constant-time byte-slice equality for secret/token/MAC comparisons.
///
/// Unequal lengths short-circuit to `false` (a length difference is not itself
/// secret for our fixed-width hex hashes and tokens); equal-length slices are
/// compared in constant time via [`subtle::ConstantTimeEq`], so no early-out
/// timing oracle leaks how many leading bytes matched.
pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    use subtle::ConstantTimeEq;
    if a.len() != b.len() {
        return false;
    }
    a.ct_eq(b).into()
}

/// Encrypted blob with metadata for key rotation support.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EncryptedBlob {
    /// Base64-encoded ciphertext (includes GCM auth tag)
    pub ciphertext: String,
    /// Base64-encoded 12-byte nonce
    pub nonce: String,
    /// Key version for rotation support
    pub key_version: u32,
}

/// Domain-separated AAD helpers — bind ciphertext to purpose + identifiers.
pub mod aad {
    /// OAuth provider subject id at rest.
    pub fn oauth_provider_id(user_id: &str, provider: &str) -> String {
        format!("oauth_pid|user:{user_id}|provider:{provider}")
    }

    /// Server-managed Nostr secret key (bound to public key).
    pub fn nostr_secret_key(pubkey_hex: &str) -> String {
        format!("nostr_sk|pk:{pubkey_hex}")
    }

    /// Direct message body at rest.
    pub fn dm_content(gift_wrap_id: &str, conversation_key: &str) -> String {
        format!("dm|gw:{gift_wrap_id}|ck:{conversation_key}")
    }
}

/// Errors that can occur during cryptographic operations.
#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("Invalid encryption key: must be 32 bytes base64-encoded")]
    InvalidKey,
    #[error("Encryption failed")]
    EncryptionFailed,
    #[error("Decryption failed: data may be corrupted, wrong key, or wrong AAD")]
    DecryptionFailed,
    #[error("Unknown key version: {0}")]
    UnknownKeyVersion(u32),
    #[error("Base64 decoding failed")]
    Base64Error,
    #[error("Invalid UTF-8 in decrypted data")]
    Utf8Error,
    #[error("Invalid keyring configuration: {0}")]
    KeyringConfig(String),
}

struct KeyEntry {
    cipher: Aes256Gcm,
}

/// Encryption service: AES-256-GCM + multi-version keyring + AAD.
///
/// Cheap to clone (`Arc` inner).
#[derive(Clone)]
pub struct CryptoService {
    inner: Arc<CryptoServiceInner>,
}

struct CryptoServiceInner {
    current_version: u32,
    keys: HashMap<u32, KeyEntry>,
    /// When true, decrypt may fall back to empty AAD (pre-AAD ciphertext).
    allow_legacy_empty_aad: bool,
}

impl CryptoService {
    /// Build from a single base64 32-byte key.
    ///
    /// Strict AAD by default (no empty-AAD legacy decrypt fallback).
    /// Use [`Self::new_allowing_legacy_empty_aad`] when reading pre-AAD ciphertext in tests.
    pub fn new(key_base64: &str, key_version: u32) -> Result<Self, CryptoError> {
        Self::from_keyring(key_base64, key_version, &[], false)
    }

    /// Like [`Self::new`], but allows empty-AAD fallback on decrypt (legacy ciphertext).
    pub fn new_allowing_legacy_empty_aad(
        key_base64: &str,
        key_version: u32,
    ) -> Result<Self, CryptoError> {
        Self::from_keyring(key_base64, key_version, &[], true)
    }

    /// Build a multi-version keyring without reading environment variables.
    ///
    /// `previous` entries must not collide with `current_version`.
    pub fn from_keyring(
        current_key_base64: &str,
        current_version: u32,
        previous: &[(u32, &str)],
        allow_legacy_empty_aad: bool,
    ) -> Result<Self, CryptoError> {
        let mut keys = HashMap::new();
        keys.insert(
            current_version,
            KeyEntry {
                cipher: parse_aes_key(current_key_base64)?,
            },
        );

        for &(ver, key_b64) in previous {
            if ver == current_version {
                return Err(CryptoError::KeyringConfig(format!(
                    "previous key version {ver} collides with current version"
                )));
            }
            keys.insert(
                ver,
                KeyEntry {
                    cipher: parse_aes_key(key_b64)?,
                },
            );
        }

        Ok(Self {
            inner: Arc::new(CryptoServiceInner {
                current_version,
                keys,
                allow_legacy_empty_aad,
            }),
        })
    }

    /// Load keyring from environment.
    ///
    /// - `ENCRYPTION_KEY` — current key (base64 32 bytes)
    /// - `ENCRYPTION_KEY_VERSION` — current version (default 1)
    /// - `ENCRYPTION_KEY_PREVIOUS` — optional `ver:base64,ver:base64` for older versions
    /// - `CRYPTO_STRICT_AAD=1` — disable empty-AAD legacy decrypt fallback
    ///
    /// Production (`PRODUCTION` truthy) also disables empty-AAD fallback.
    /// A previous entry whose version equals the current version is rejected
    /// (`KeyringConfig`) so it cannot overwrite the current key.
    pub fn from_env() -> Result<Option<Self>, CryptoError> {
        let key = match std::env::var("ENCRYPTION_KEY") {
            Ok(k) if !k.is_empty() => k,
            _ => return Ok(None),
        };

        let current_version: u32 = std::env::var("ENCRYPTION_KEY_VERSION")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1);

        let mut previous: Vec<(u32, String)> = Vec::new();
        if let Ok(prev) = std::env::var("ENCRYPTION_KEY_PREVIOUS") {
            for part in prev.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                let (ver_s, key_b64) = part.split_once(':').ok_or_else(|| {
                    CryptoError::KeyringConfig(
                        "ENCRYPTION_KEY_PREVIOUS entries must be ver:base64".into(),
                    )
                })?;
                let ver: u32 = ver_s.parse().map_err(|_| {
                    CryptoError::KeyringConfig(format!("invalid key version '{ver_s}'"))
                })?;
                if ver == current_version {
                    return Err(CryptoError::KeyringConfig(format!(
                        "ENCRYPTION_KEY_PREVIOUS version {ver} collides with current version"
                    )));
                }
                previous.push((ver, key_b64.to_string()));
            }
        }

        let previous_refs: Vec<(u32, &str)> =
            previous.iter().map(|(v, k)| (*v, k.as_str())).collect();

        let allow_legacy_empty_aad = std::env::var("CRYPTO_STRICT_AAD").ok().as_deref()
            != Some("1")
            && !is_strict_production_crypto();

        Ok(Some(Self::from_keyring(
            &key,
            current_version,
            &previous_refs,
            allow_legacy_empty_aad,
        )?))
    }

    /// Generate a new random 256-bit encryption key (base64-encoded).
    pub fn generate_key() -> String {
        let mut key = [0u8; 32];
        OsRng.fill_bytes(&mut key);
        let encoded = BASE64.encode(key);
        key.zeroize();
        encoded
    }

    pub fn key_version(&self) -> u32 {
        self.inner.current_version
    }

    /// Encrypt raw bytes with domain-separated AAD.
    pub fn encrypt_bytes(
        &self,
        plaintext: &[u8],
        aad: &[u8],
    ) -> Result<EncryptedBlob, CryptoError> {
        let entry = self
            .inner
            .keys
            .get(&self.inner.current_version)
            .ok_or(CryptoError::UnknownKeyVersion(self.inner.current_version))?;

        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = entry
            .cipher
            .encrypt(
                nonce,
                Payload {
                    msg: plaintext,
                    aad,
                },
            )
            .map_err(|_| CryptoError::EncryptionFailed)?;

        Ok(EncryptedBlob {
            ciphertext: BASE64.encode(&ciphertext),
            nonce: BASE64.encode(nonce_bytes),
            key_version: self.inner.current_version,
        })
    }

    /// Encrypt a UTF-8 string with AAD (string form of AAD).
    pub fn encrypt(&self, plaintext: &str, aad: &str) -> Result<EncryptedBlob, CryptoError> {
        self.encrypt_bytes(plaintext.as_bytes(), aad.as_bytes())
    }

    /// Decrypt raw bytes; uses `blob.key_version` from the keyring.
    ///
    /// Tries AAD first; optionally falls back to empty AAD for pre-AAD ciphertext
    /// when not in strict mode.
    pub fn decrypt_bytes(
        &self,
        blob: &EncryptedBlob,
        aad: &[u8],
    ) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
        let entry = self
            .inner
            .keys
            .get(&blob.key_version)
            .ok_or(CryptoError::UnknownKeyVersion(blob.key_version))?;

        let ciphertext = BASE64
            .decode(&blob.ciphertext)
            .map_err(|_| CryptoError::Base64Error)?;
        let nonce_bytes = BASE64
            .decode(&blob.nonce)
            .map_err(|_| CryptoError::Base64Error)?;

        if nonce_bytes.len() != 12 {
            return Err(CryptoError::DecryptionFailed);
        }

        let nonce = Nonce::from_slice(&nonce_bytes);

        match entry.cipher.decrypt(
            nonce,
            Payload {
                msg: ciphertext.as_ref(),
                aad,
            },
        ) {
            Ok(pt) => Ok(Zeroizing::new(pt)),
            Err(_) if self.inner.allow_legacy_empty_aad && !aad.is_empty() => {
                // Legacy blobs encrypted without AAD.
                let pt = entry
                    .cipher
                    .decrypt(
                        nonce,
                        Payload {
                            msg: ciphertext.as_ref(),
                            aad: b"",
                        },
                    )
                    .map_err(|_| CryptoError::DecryptionFailed)?;
                Ok(Zeroizing::new(pt))
            }
            Err(_) => Err(CryptoError::DecryptionFailed),
        }
    }

    /// Decrypt to UTF-8 string with AAD.
    pub fn decrypt(&self, blob: &EncryptedBlob, aad: &str) -> Result<String, CryptoError> {
        let pt = self.decrypt_bytes(blob, aad.as_bytes())?;
        String::from_utf8(pt.to_vec()).map_err(|_| CryptoError::Utf8Error)
    }

    /// Re-encrypt a blob under the current key version (for rotation).
    ///
    /// Decrypts with the blob's version + AAD, then encrypts with current key.
    pub fn rewrap(&self, blob: &EncryptedBlob, aad: &str) -> Result<EncryptedBlob, CryptoError> {
        let pt = self.decrypt_bytes(blob, aad.as_bytes())?;
        self.encrypt_bytes(pt.as_ref(), aad.as_bytes())
    }
}

fn is_strict_production_crypto() -> bool {
    match std::env::var("PRODUCTION") {
        Ok(v) => {
            let t = v.trim();
            !t.is_empty()
                && !matches!(
                    t.to_ascii_lowercase().as_str(),
                    "0" | "false" | "no" | "off"
                )
        }
        Err(_) => false,
    }
}

fn parse_aes_key(key_base64: &str) -> Result<Aes256Gcm, CryptoError> {
    let mut key_bytes = BASE64
        .decode(key_base64)
        .map_err(|_| CryptoError::InvalidKey)?;

    if key_bytes.len() != 32 {
        key_bytes.zeroize();
        return Err(CryptoError::InvalidKey);
    }

    let cipher = Aes256Gcm::new_from_slice(&key_bytes).map_err(|_| CryptoError::InvalidKey)?;
    key_bytes.zeroize();
    Ok(cipher)
}

/// Hash a provider ID for database lookups (deterministic, one-way).
pub fn hash_provider_id(provider: &str, provider_id: &str) -> String {
    let input = format!("{provider}:{provider_id}");
    let hash = blake3::hash(input.as_bytes());
    hash.to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_session_token_deterministic() {
        let token = "test_token_12345";
        assert_eq!(hash_session_token(token), hash_session_token(token));
    }

    #[test]
    fn test_hash_session_token_different_inputs() {
        assert_ne!(hash_session_token("token_a"), hash_session_token("token_b"));
    }

    #[test]
    fn test_hash_output_length() {
        assert_eq!(hash_session_token("any_token").len(), 64);
    }

    #[test]
    fn test_verify_session_token_valid() {
        let token = "my_secret_token";
        let hash = hash_session_token(token);
        assert!(verify_session_token(token, &hash));
    }

    #[test]
    fn test_verify_session_token_invalid() {
        let hash = hash_session_token("my_secret_token");
        assert!(!verify_session_token("wrong_token", &hash));
    }

    #[test]
    fn test_constant_time_eq() {
        assert!(constant_time_eq(b"hello", b"hello"));
        assert!(!constant_time_eq(b"hello", b"world"));
        assert!(!constant_time_eq(b"short", b"longer"));
    }
}

#[cfg(test)]
mod encryption_tests {
    use super::*;

    fn test_key() -> String {
        // exactly 32 bytes when decoded
        "dGhpc19pc19leGFjdGx5XzMyX2J5dGVzX2tleSEhISE=".to_string()
    }

    #[test]
    fn test_encrypt_decrypt_roundtrip_with_aad() {
        let service = CryptoService::new(&test_key(), 1).unwrap();
        let aad = aad::oauth_provider_id("user1", "discord");
        let encrypted = service.encrypt("discord_user_12345", &aad).unwrap();
        let decrypted = service.decrypt(&encrypted, &aad).unwrap();
        assert_eq!(decrypted, "discord_user_12345");
    }

    #[test]
    fn test_wrong_aad_fails() {
        let service = CryptoService::new(&test_key(), 1).unwrap();
        let encrypted = service
            .encrypt("secret", &aad::oauth_provider_id("u1", "discord"))
            .unwrap();
        assert!(service
            .decrypt(&encrypted, &aad::oauth_provider_id("u2", "discord"))
            .is_err());
    }

    #[test]
    fn test_encrypt_produces_different_ciphertext() {
        let service = CryptoService::new(&test_key(), 1).unwrap();
        let aad = "ctx";
        let e1 = service.encrypt("same_value", aad).unwrap();
        let e2 = service.encrypt("same_value", aad).unwrap();
        assert_ne!(e1.ciphertext, e2.ciphertext);
        assert_ne!(e1.nonce, e2.nonce);
    }

    #[test]
    fn test_wrong_key_fails_decryption() {
        let s1 = CryptoService::new(&test_key(), 1).unwrap();
        let s2 = CryptoService::new(&CryptoService::generate_key(), 1).unwrap();
        let encrypted = s1.encrypt("secret", "aad").unwrap();
        assert!(s2.decrypt(&encrypted, "aad").is_err());
    }

    #[test]
    fn test_tampered_ciphertext_fails() {
        let service = CryptoService::new(&test_key(), 1).unwrap();
        let mut encrypted = service.encrypt("secret", "aad").unwrap();
        encrypted.ciphertext = BASE64.encode(b"tampered_data");
        assert!(service.decrypt(&encrypted, "aad").is_err());
    }

    #[test]
    fn test_invalid_key_rejected() {
        assert!(CryptoService::new("too_short", 1).is_err());
        assert!(CryptoService::new("not_base64!!!", 1).is_err());
    }

    #[test]
    fn test_generate_key_is_valid() {
        let key = CryptoService::generate_key();
        assert!(CryptoService::new(&key, 1).is_ok());
    }

    #[test]
    fn test_key_version_preserved() {
        let service = CryptoService::new(&test_key(), 42).unwrap();
        let encrypted = service.encrypt("test", "aad").unwrap();
        assert_eq!(encrypted.key_version, 42);
    }

    #[test]
    fn test_rewrap_upgrades_version() {
        let key_v1 = test_key();
        let key_v2 = CryptoService::generate_key();
        // Multi-version keyring without process-global env mutation
        let service =
            CryptoService::from_keyring(&key_v2, 2, &[(1, key_v1.as_str())], false).unwrap();

        let v1 = CryptoService::new(&key_v1, 1).unwrap();
        let blob_v1 = v1.encrypt("rotate-me", "aad").unwrap();
        assert_eq!(blob_v1.key_version, 1);

        let blob_v2 = service.rewrap(&blob_v1, "aad").unwrap();
        assert_eq!(blob_v2.key_version, 2);
        assert_eq!(service.decrypt(&blob_v2, "aad").unwrap(), "rotate-me");
    }

    #[test]
    fn test_previous_key_version_collision_rejected() {
        let key = test_key();
        match CryptoService::from_keyring(&key, 1, &[(1, key.as_str())], false) {
            Err(CryptoError::KeyringConfig(_)) => {}
            Ok(_) => panic!("same version in previous must fail"),
            Err(other) => panic!("expected KeyringConfig, got {other:?}"),
        }
    }

    #[test]
    fn test_strict_new_rejects_empty_aad_legacy() {
        // Encrypt with empty AAD using a service that can still decrypt it...
        let legacy = CryptoService::new_allowing_legacy_empty_aad(&test_key(), 1).unwrap();
        // ...by encrypting with empty AAD via encrypt_bytes
        let blob = legacy.encrypt_bytes(b"secret", b"").unwrap();

        // Strict default must not fall back to empty AAD when a non-empty AAD is provided
        let strict = CryptoService::new(&test_key(), 1).unwrap();
        assert!(strict.decrypt_bytes(&blob, b"some-aad").is_err());

        // Legacy constructor can still recover empty-AAD ciphertext
        assert_eq!(
            legacy.decrypt_bytes(&blob, b"some-aad").unwrap().as_slice(),
            b"secret"
        );
    }

    #[test]
    fn test_hash_provider_id() {
        assert_eq!(
            hash_provider_id("discord", "123456"),
            hash_provider_id("discord", "123456")
        );
        assert_ne!(
            hash_provider_id("discord", "123456"),
            hash_provider_id("google", "123456")
        );
    }

    #[test]
    fn test_encrypt_bytes_roundtrip() {
        let service = CryptoService::new(&test_key(), 1).unwrap();
        let raw = [7u8; 32];
        let aad = aad::nostr_secret_key("abcd");
        let blob = service.encrypt_bytes(&raw, aad.as_bytes()).unwrap();
        let out = service.decrypt_bytes(&blob, aad.as_bytes()).unwrap();
        assert_eq!(out.as_slice(), &raw);
    }
}
