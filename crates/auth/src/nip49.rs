//! NIP-49 encrypted secret key backup (ncryptsec).
//!
//! Encrypts a Nostr secret key with a user-provided password using scrypt KDF
//! and XChaCha20-Poly1305. The output is a bech32-encoded string with HRP "ncryptsec".

use chacha20poly1305::{
    aead::{Aead, KeyInit},
    XChaCha20Poly1305, XNonce,
};
use rand::rngs::OsRng;
use rand::RngCore;
use scrypt::{scrypt, Params as ScryptParams};
use zeroize::Zeroize;

const NIP49_VERSION: u8 = 0x02;
const DEFAULT_LOG_N: u8 = 16;
/// Minimum accepted scrypt `log_n` on decrypt (matches encrypt default).
const MIN_LOG_N: u8 = 16;
/// Maximum accepted scrypt `log_n` on decrypt — higher values are a DoS vector.
const MAX_LOG_N: u8 = 20;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
const KEY_LEN: usize = 32;

#[derive(Debug, thiserror::Error)]
pub enum Nip49Error {
    #[error("invalid secret key: must be 64-char hex (32 bytes)")]
    InvalidSecretKey,
    #[error("invalid ncryptsec: {0}")]
    InvalidFormat(String),
    #[error("unsupported version: {0}")]
    UnsupportedVersion(u8),
    #[error("unsupported log_n: {0} (allowed range 16..=20)")]
    UnsupportedLogN(u8),
    #[error("decryption failed — wrong password or corrupted data")]
    DecryptionFailed,
    #[error("scrypt KDF failed: {0}")]
    KdfFailed(String),
    #[error("bech32 encoding failed: {0}")]
    EncodingFailed(String),
}

/// Encrypt a hex-encoded secret key with a password, returning an ncryptsec bech32 string.
pub fn encrypt(secret_key_hex: &str, password: &str) -> Result<String, Nip49Error> {
    let mut key_bytes = hex_to_bytes(secret_key_hex)?;

    let mut salt = [0u8; SALT_LEN];
    OsRng.fill_bytes(&mut salt);

    let mut nonce_bytes = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce_bytes);

    let mut derived_key = [0u8; KEY_LEN];
    derive_key(password, &salt, DEFAULT_LOG_N, &mut derived_key)?;

    let cipher = XChaCha20Poly1305::new_from_slice(&derived_key)
        .map_err(|e| Nip49Error::KdfFailed(e.to_string()))?;
    derived_key.zeroize();

    let nonce = XNonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, key_bytes.as_ref())
        .map_err(|_| Nip49Error::DecryptionFailed)?;
    key_bytes.zeroize();

    // Assemble payload: version (1) + log_n (1) + salt (16) + nonce (24) + ciphertext (48)
    let mut payload = Vec::with_capacity(1 + 1 + SALT_LEN + NONCE_LEN + ciphertext.len());
    payload.push(NIP49_VERSION);
    payload.push(DEFAULT_LOG_N);
    payload.extend_from_slice(&salt);
    payload.extend_from_slice(&nonce_bytes);
    payload.extend_from_slice(&ciphertext);

    let encoded =
        bech32::encode::<bech32::Bech32>(bech32::Hrp::parse("ncryptsec").unwrap(), &payload)
            .map_err(|e| Nip49Error::EncodingFailed(e.to_string()))?;

    Ok(encoded)
}

/// Decrypt an ncryptsec bech32 string with a password, returning the hex-encoded secret key.
pub fn decrypt(ncryptsec: &str, password: &str) -> Result<String, Nip49Error> {
    let (hrp, payload) =
        bech32::decode(ncryptsec).map_err(|e| Nip49Error::InvalidFormat(e.to_string()))?;

    if hrp.as_str() != "ncryptsec" {
        return Err(Nip49Error::InvalidFormat(format!(
            "expected HRP 'ncryptsec', got '{}'",
            hrp
        )));
    }

    // Minimum payload: version (1) + log_n (1) + salt (16) + nonce (24) + ciphertext (48) = 90
    if payload.len() < 90 {
        return Err(Nip49Error::InvalidFormat("payload too short".into()));
    }

    let version = payload[0];
    if version != NIP49_VERSION {
        return Err(Nip49Error::UnsupportedVersion(version));
    }

    let log_n = payload[1];
    // Reject before scrypt — high log_n is a CPU/memory DoS vector.
    if !(MIN_LOG_N..=MAX_LOG_N).contains(&log_n) {
        return Err(Nip49Error::UnsupportedLogN(log_n));
    }

    let salt = &payload[2..2 + SALT_LEN];
    let nonce_bytes = &payload[2 + SALT_LEN..2 + SALT_LEN + NONCE_LEN];
    let ciphertext = &payload[2 + SALT_LEN + NONCE_LEN..];

    let mut derived_key = [0u8; KEY_LEN];
    derive_key(password, salt, log_n, &mut derived_key)?;

    let cipher = XChaCha20Poly1305::new_from_slice(&derived_key)
        .map_err(|e| Nip49Error::KdfFailed(e.to_string()))?;
    derived_key.zeroize();

    let nonce = XNonce::from_slice(nonce_bytes);
    let mut plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| Nip49Error::DecryptionFailed)?;

    if plaintext.len() != KEY_LEN {
        plaintext.zeroize();
        return Err(Nip49Error::DecryptionFailed);
    }

    let hex = hex::encode(&plaintext);
    plaintext.zeroize();
    Ok(hex)
}

fn derive_key(
    password: &str,
    salt: &[u8],
    log_n: u8,
    output: &mut [u8; KEY_LEN],
) -> Result<(), Nip49Error> {
    let params = ScryptParams::new(log_n, 8, 1, KEY_LEN)
        .map_err(|e| Nip49Error::KdfFailed(e.to_string()))?;
    scrypt(password.as_bytes(), salt, &params, output)
        .map_err(|e| Nip49Error::KdfFailed(e.to_string()))?;
    Ok(())
}

fn hex_to_bytes(hex_str: &str) -> Result<[u8; KEY_LEN], Nip49Error> {
    if hex_str.len() != 64 || !hex_str.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(Nip49Error::InvalidSecretKey);
    }
    let bytes = hex::decode(hex_str).map_err(|_| Nip49Error::InvalidSecretKey)?;
    let mut arr = [0u8; KEY_LEN];
    arr.copy_from_slice(&bytes);
    Ok(arr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let secret = "a".repeat(64);
        let password = "correct horse battery staple";

        let ncryptsec = encrypt(&secret, password).unwrap();
        assert!(ncryptsec.starts_with("ncryptsec1"));

        let decrypted = decrypt(&ncryptsec, password).unwrap();
        assert_eq!(decrypted, secret);
    }

    #[test]
    fn wrong_password_fails() {
        let secret = "b".repeat(64);
        let ncryptsec = encrypt(&secret, "right").unwrap();
        let result = decrypt(&ncryptsec, "wrong");
        assert!(result.is_err());
    }

    #[test]
    fn invalid_secret_key_rejected() {
        assert!(encrypt("tooshort", "pass").is_err());
        assert!(encrypt("zz".repeat(32).as_str(), "pass").is_err());
    }

    #[test]
    fn invalid_ncryptsec_rejected() {
        assert!(decrypt("not_bech32", "pass").is_err());
        assert!(decrypt("bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4", "pass").is_err());
    }

    #[test]
    fn different_encryptions_differ() {
        let secret = "c".repeat(64);
        let enc1 = encrypt(&secret, "pass").unwrap();
        let enc2 = encrypt(&secret, "pass").unwrap();
        assert_ne!(enc1, enc2); // random salt + nonce
    }

    /// High log_n must be rejected before scrypt runs (would hang/DoS otherwise).
    #[test]
    fn high_log_n_rejected_without_scrypt() {
        let secret = "d".repeat(64);
        let ncryptsec = encrypt(&secret, "pass").unwrap();

        let (hrp, mut payload) = bech32::decode(&ncryptsec).unwrap();
        assert_eq!(payload[1], DEFAULT_LOG_N);
        // log_n=31 would make scrypt effectively unbounded for a unit test
        payload[1] = 31;
        let tampered =
            bech32::encode::<bech32::Bech32>(hrp, &payload).expect("re-encode tampered payload");

        let result = decrypt(&tampered, "pass");
        match result {
            Err(Nip49Error::UnsupportedLogN(31)) => {}
            other => panic!("expected UnsupportedLogN(31), got {other:?}"),
        }
    }

    #[test]
    fn low_log_n_rejected() {
        let secret = "e".repeat(64);
        let ncryptsec = encrypt(&secret, "pass").unwrap();

        let (hrp, mut payload) = bech32::decode(&ncryptsec).unwrap();
        payload[1] = 10; // below MIN_LOG_N
        let tampered =
            bech32::encode::<bech32::Bech32>(hrp, &payload).expect("re-encode tampered payload");

        let result = decrypt(&tampered, "pass");
        match result {
            Err(Nip49Error::UnsupportedLogN(10)) => {}
            other => panic!("expected UnsupportedLogN(10), got {other:?}"),
        }
    }
}
