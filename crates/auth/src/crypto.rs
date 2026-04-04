//! Cryptographic utilities for secure data storage.
//!
//! Provides:
//! - BLAKE3 hashing for session tokens
//! - AES-256-GCM encryption for sensitive fields (e.g., OAuth provider IDs)

use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use zeroize::Zeroize;

/// Hash a session token for storage using BLAKE3.
///
/// BLAKE3 is used because session tokens are already high-entropy random
/// values — no need for slow password-hashing algorithms like Argon2.
pub fn hash_session_token(token: &str) -> String {
    let hash = blake3::hash(token.as_bytes());
    hash.to_hex().to_string()
}

/// Verify a session token against its stored hash.
///
/// Uses constant-time comparison to prevent timing attacks.
pub fn verify_session_token(token: &str, stored_hash: &str) -> bool {
    let computed = hash_session_token(token);
    constant_time_eq(computed.as_bytes(), stored_hash.as_bytes())
}

/// Constant-time byte comparison to prevent timing attacks.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut result = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        result |= x ^ y;
    }
    result == 0
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

/// Errors that can occur during cryptographic operations.
#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("Invalid encryption key: must be 32 bytes base64-encoded")]
    InvalidKey,
    #[error("Encryption failed")]
    EncryptionFailed,
    #[error("Decryption failed: data may be corrupted or key mismatch")]
    DecryptionFailed,
    #[error("Base64 decoding failed")]
    Base64Error,
    #[error("Invalid UTF-8 in decrypted data")]
    Utf8Error,
}

/// Encryption service for sensitive fields using AES-256-GCM.
///
/// Provides authenticated encryption — data cannot be read OR tampered
/// with without the correct key.
#[derive(Clone)]
pub struct CryptoService {
    cipher: Aes256Gcm,
    key_version: u32,
}

impl CryptoService {
    /// Create a new CryptoService from a base64-encoded 256-bit key.
    pub fn new(key_base64: &str, key_version: u32) -> Result<Self, CryptoError> {
        let key_bytes = BASE64
            .decode(key_base64)
            .map_err(|_| CryptoError::InvalidKey)?;

        if key_bytes.len() != 32 {
            return Err(CryptoError::InvalidKey);
        }

        let cipher =
            Aes256Gcm::new_from_slice(&key_bytes).map_err(|_| CryptoError::InvalidKey)?;

        Ok(Self {
            cipher,
            key_version,
        })
    }

    /// Create a CryptoService from environment variables.
    ///
    /// Reads `ENCRYPTION_KEY` (base64-encoded 32 bytes) and optionally
    /// `ENCRYPTION_KEY_VERSION` (defaults to 1).
    ///
    /// Returns `None` if `ENCRYPTION_KEY` is not set (encryption disabled).
    pub fn from_env() -> Result<Option<Self>, CryptoError> {
        let key = match std::env::var("ENCRYPTION_KEY") {
            Ok(k) if !k.is_empty() => k,
            _ => return Ok(None),
        };

        let version: u32 = std::env::var("ENCRYPTION_KEY_VERSION")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1);

        Ok(Some(Self::new(&key, version)?))
    }

    /// Generate a new random 256-bit encryption key (base64-encoded).
    pub fn generate_key() -> String {
        let mut key = [0u8; 32];
        OsRng.fill_bytes(&mut key);
        let encoded = BASE64.encode(&key);
        key.zeroize();
        encoded
    }

    /// Encrypt a string value using AES-256-GCM.
    ///
    /// Each encryption uses a unique random nonce, so encrypting the same
    /// value twice produces different ciphertext (semantic security).
    pub fn encrypt(&self, plaintext: &str) -> Result<EncryptedBlob, CryptoError> {
        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = self
            .cipher
            .encrypt(nonce, plaintext.as_bytes())
            .map_err(|_| CryptoError::EncryptionFailed)?;

        Ok(EncryptedBlob {
            ciphertext: BASE64.encode(&ciphertext),
            nonce: BASE64.encode(&nonce_bytes),
            key_version: self.key_version,
        })
    }

    /// Decrypt an encrypted blob back to the original string.
    pub fn decrypt(&self, blob: &EncryptedBlob) -> Result<String, CryptoError> {
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

        let plaintext = self
            .cipher
            .decrypt(nonce, ciphertext.as_ref())
            .map_err(|_| CryptoError::DecryptionFailed)?;

        String::from_utf8(plaintext).map_err(|_| CryptoError::Utf8Error)
    }

    /// Get the current key version.
    pub fn key_version(&self) -> u32 {
        self.key_version
    }
}

/// Hash a provider ID for database lookups.
///
/// Deterministic (same input = same output) but one-way. Includes
/// provider name to prevent cross-provider collisions.
pub fn hash_provider_id(provider: &str, provider_id: &str) -> String {
    let input = format!("{}:{}", provider, provider_id);
    let hash = blake3::hash(input.as_bytes());
    hash.to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_session_token_deterministic() {
        let token = "test_token_12345";
        let hash1 = hash_session_token(token);
        let hash2 = hash_session_token(token);
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_hash_session_token_different_inputs() {
        let hash1 = hash_session_token("token_a");
        let hash2 = hash_session_token("token_b");
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_hash_output_length() {
        let hash = hash_session_token("any_token");
        // BLAKE3 produces 256-bit (32-byte) hash, hex-encoded = 64 chars
        assert_eq!(hash.len(), 64);
    }

    #[test]
    fn test_verify_session_token_valid() {
        let token = "my_secret_token";
        let hash = hash_session_token(token);
        assert!(verify_session_token(token, &hash));
    }

    #[test]
    fn test_verify_session_token_invalid() {
        let token = "my_secret_token";
        let hash = hash_session_token(token);
        assert!(!verify_session_token("wrong_token", &hash));
    }

    #[test]
    fn test_constant_time_eq_equal() {
        assert!(constant_time_eq(b"hello", b"hello"));
    }

    #[test]
    fn test_constant_time_eq_not_equal() {
        assert!(!constant_time_eq(b"hello", b"world"));
    }

    #[test]
    fn test_constant_time_eq_different_lengths() {
        assert!(!constant_time_eq(b"short", b"longer"));
    }
}

#[cfg(test)]
mod encryption_tests {
    use super::*;

    fn test_key() -> String {
        "dGhpc19pc19leGFjdGx5XzMyX2J5dGVzX2tleSEhISE=".to_string()
    }

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let service = CryptoService::new(&test_key(), 1).unwrap();
        let plaintext = "discord_user_12345";

        let encrypted = service.encrypt(plaintext).unwrap();
        let decrypted = service.decrypt(&encrypted).unwrap();

        assert_eq!(plaintext, decrypted);
    }

    #[test]
    fn test_encrypt_produces_different_ciphertext() {
        let service = CryptoService::new(&test_key(), 1).unwrap();
        let plaintext = "same_value";

        let encrypted1 = service.encrypt(plaintext).unwrap();
        let encrypted2 = service.encrypt(plaintext).unwrap();

        assert_ne!(encrypted1.ciphertext, encrypted2.ciphertext);
        assert_ne!(encrypted1.nonce, encrypted2.nonce);

        assert_eq!(service.decrypt(&encrypted1).unwrap(), plaintext);
        assert_eq!(service.decrypt(&encrypted2).unwrap(), plaintext);
    }

    #[test]
    fn test_wrong_key_fails_decryption() {
        let service1 = CryptoService::new(&test_key(), 1).unwrap();
        let service2 = CryptoService::new(&CryptoService::generate_key(), 1).unwrap();

        let encrypted = service1.encrypt("secret").unwrap();
        assert!(service2.decrypt(&encrypted).is_err());
    }

    #[test]
    fn test_tampered_ciphertext_fails() {
        let service = CryptoService::new(&test_key(), 1).unwrap();
        let mut encrypted = service.encrypt("secret").unwrap();

        encrypted.ciphertext = BASE64.encode(b"tampered_data");
        assert!(service.decrypt(&encrypted).is_err());
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
        let encrypted = service.encrypt("test").unwrap();
        assert_eq!(encrypted.key_version, 42);
    }

    #[test]
    fn test_hash_provider_id_deterministic() {
        let hash1 = hash_provider_id("discord", "123456");
        let hash2 = hash_provider_id("discord", "123456");
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_hash_provider_id_different_providers() {
        let discord_hash = hash_provider_id("discord", "123456");
        let google_hash = hash_provider_id("google", "123456");
        assert_ne!(discord_hash, google_hash);
    }
}
