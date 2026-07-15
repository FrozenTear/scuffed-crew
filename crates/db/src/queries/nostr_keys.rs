use scuffed_auth::crypto::{aad, EncryptedBlob};
use secp256k1::{rand::rngs::OsRng, Secp256k1};
use zeroize::{Zeroize, Zeroizing};

use crate::types::NostrKeyMode;
use crate::{Database, DbError, DbResult};

/// A generated Nostr keypair with the secret key encrypted at rest.
#[derive(Debug)]
pub struct NostrKeypair {
    /// Hex-encoded x-only public key (64 chars).
    pub pubkey: String,
    /// The secret key encrypted via CryptoService (AES-256-GCM).
    pub secret_key_encrypted: EncryptedBlob,
}

/// Generate a random Nostr keypair and encrypt the secret key.
///
/// Uses secp256k1 directly — a Nostr keypair is just a secp256k1 keypair
/// with the public key in x-only (Schnorr) format.
///
/// Stores **raw 32-byte** secret material (not hex) under AAD bound to the pubkey.
pub fn generate_encrypted_keypair(db: &Database) -> DbResult<NostrKeypair> {
    let crypto = db.crypto.as_ref().ok_or_else(|| {
        DbError::Config("CryptoService not configured — set ENCRYPTION_KEY".into())
    })?;

    let secp = Secp256k1::new();
    let (secret_key, public_key) = secp.generate_keypair(&mut OsRng);

    // x-only public key (32 bytes) is the Nostr pubkey format
    let (xonly, _parity) = public_key.x_only_public_key();
    let pubkey_hex = hex::encode(xonly.serialize());

    let mut secret_bytes = secret_key.secret_bytes();
    let aad_s = aad::nostr_secret_key(&pubkey_hex);
    let secret_key_encrypted = crypto
        .encrypt_bytes(&secret_bytes, aad_s.as_bytes())
        .map_err(DbError::Crypto)?;
    secret_bytes.zeroize();

    Ok(NostrKeypair {
        pubkey: pubkey_hex,
        secret_key_encrypted,
    })
}

/// Decrypt a member's stored Nostr secret key, returning the hex-encoded secret.
///
/// Accepts raw 32-byte ciphertext payloads (preferred) and legacy UTF-8 hex strings.
/// Verifies that the secret derives `pubkey_hex` (secp256k1 x-only, case-insensitive).
/// The caller MUST use the result immediately and not hold it in long-lived state.
/// The returned buffer is zeroized on drop.
pub fn decrypt_nostr_secret_key(
    db: &Database,
    blob: &EncryptedBlob,
    pubkey_hex: &str,
) -> DbResult<Zeroizing<String>> {
    let crypto = db
        .crypto
        .as_ref()
        .ok_or_else(|| DbError::Config("CryptoService not configured".into()))?;

    let aad_s = aad::nostr_secret_key(pubkey_hex);
    let pt: Zeroizing<Vec<u8>> = crypto
        .decrypt_bytes(blob, aad_s.as_bytes())
        .map_err(DbError::Crypto)?;

    let secret_hex: Zeroizing<String> = if pt.len() == 32 {
        Zeroizing::new(hex::encode(pt.as_slice()))
    } else {
        // Legacy: UTF-8 hex string inside the blob
        let hex_str = String::from_utf8(pt.to_vec()).map_err(|_| {
            DbError::Config(
                "Invalid Nostr secret key encoding (expected 32 raw bytes or hex)".into(),
            )
        })?;
        if hex_str.len() != 64 || !hex_str.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(DbError::Config(
                "Invalid Nostr secret key encoding (expected 64-char hex)".into(),
            ));
        }
        Zeroizing::new(hex_str)
    };

    verify_secret_derives_pubkey(&secret_hex, pubkey_hex)?;
    Ok(secret_hex)
}

/// Verify a hex-encoded secret derives the given Nostr pubkey (x-only secp256k1).
fn verify_secret_derives_pubkey(secret_hex: &str, pubkey_hex: &str) -> DbResult<()> {
    let mut secret_bytes = hex::decode(secret_hex).map_err(|_| {
        DbError::Config("Invalid Nostr secret key encoding (hex decode failed)".into())
    })?;
    let result = (|| {
        let secp = Secp256k1::new();
        let sk = secp256k1::SecretKey::from_slice(&secret_bytes).map_err(|_| {
            DbError::Config("Invalid Nostr secret key (not a valid secp256k1 scalar)".into())
        })?;
        let pk = sk.public_key(&secp);
        let (xonly, _) = pk.x_only_public_key();
        let derived = hex::encode(xonly.serialize());
        if !derived.eq_ignore_ascii_case(pubkey_hex) {
            return Err(DbError::Config(
                "Nostr secret key does not match owner pubkey".into(),
            ));
        }
        Ok(())
    })();
    secret_bytes.zeroize();
    result
}

impl Database {
    /// Provision a server-managed Nostr keypair for a member.
    ///
    /// Generates a random keypair, encrypts the secret key, and stores both
    /// the public key and encrypted secret on the member record.
    /// Sets `nostr_key_mode` to `server_managed`.
    ///
    /// Returns the hex-encoded public key.
    pub async fn provision_nostr_keypair(&self, member_id: &str) -> DbResult<String> {
        let keypair = generate_encrypted_keypair(self)?;
        let pubkey = keypair.pubkey.clone();

        self.update_member_nostr_keys(
            member_id,
            Some(&keypair.pubkey),
            Some(&NostrKeyMode::ServerManaged.to_string()),
            Some(&keypair.secret_key_encrypted),
        )
        .await?;

        Ok(pubkey)
    }

    /// Decrypt the Nostr secret key for a server-managed member.
    ///
    /// Returns `None` if the member has no server-managed key.
    /// The caller MUST use the result immediately — do not hold in long-lived state.
    /// The returned buffer is zeroized on drop.
    pub async fn get_nostr_secret_key(
        &self,
        member_id: &str,
    ) -> DbResult<Option<Zeroizing<String>>> {
        let member = self
            .get_member(member_id)
            .await?
            .ok_or_else(|| DbError::NotFound(format!("Member {member_id} not found")))?;

        match (
            member.nostr_key_mode,
            member.nostr_secret_key_encrypted,
            member.nostr_pubkey,
        ) {
            (Some(NostrKeyMode::ServerManaged), Some(ref blob), Some(ref pubkey)) => {
                let secret_hex = decrypt_nostr_secret_key(self, blob, pubkey)?;
                Ok(Some(secret_hex))
            }
            _ => Ok(None),
        }
    }

    /// Transition a member to external key mode.
    ///
    /// Sets the pubkey to the externally-provided one, clears the encrypted
    /// secret key, and sets mode to `external`.
    pub async fn set_external_nostr_key(&self, member_id: &str, pubkey: &str) -> DbResult<()> {
        self.update_member_nostr_keys(
            member_id,
            Some(pubkey),
            Some(&NostrKeyMode::External.to_string()),
            None,
        )
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations::run_migrations;
    use scuffed_auth::crypto::CryptoService;
    use std::sync::Arc;

    async fn test_db() -> Database {
        let key = CryptoService::generate_key();
        let crypto = CryptoService::new(&key, 1).unwrap();
        let mut db = Database::connect_memory().await.unwrap();
        db.crypto = Some(Arc::new(crypto));
        run_migrations(&db.client).await.unwrap();
        db
    }

    #[tokio::test]
    async fn test_generate_keypair_produces_valid_pubkey() {
        let db = test_db().await;
        let keypair = generate_encrypted_keypair(&db).unwrap();

        // Pubkey should be 64-char hex (32 bytes x-only)
        assert_eq!(keypair.pubkey.len(), 64);
        assert!(keypair.pubkey.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[tokio::test]
    async fn test_encrypt_decrypt_roundtrip() {
        let db = test_db().await;
        let keypair = generate_encrypted_keypair(&db).unwrap();
        let decrypted =
            decrypt_nostr_secret_key(&db, &keypair.secret_key_encrypted, &keypair.pubkey).unwrap();

        // Decrypted secret should be 64-char hex (32 bytes)
        assert_eq!(decrypted.len(), 64);
        assert!(decrypted.chars().all(|c| c.is_ascii_hexdigit()));

        // Verify the secret key corresponds to the public key
        let secret_bytes = hex::decode(decrypted.as_str()).unwrap();
        let secp = Secp256k1::new();
        let sk = secp256k1::SecretKey::from_slice(&secret_bytes).unwrap();
        let pk = sk.public_key(&secp);
        let (xonly, _) = pk.x_only_public_key();
        assert_eq!(hex::encode(xonly.serialize()), keypair.pubkey);
    }

    #[tokio::test]
    async fn test_decrypt_rejects_pubkey_mismatch() {
        let db = test_db().await;
        let crypto = db.crypto.as_ref().unwrap();

        // Secret for kp1, encrypted under AAD of a different claimed owner pubkey.
        let kp1 = generate_encrypted_keypair(&db).unwrap();
        let secret = decrypt_nostr_secret_key(&db, &kp1.secret_key_encrypted, &kp1.pubkey).unwrap();
        let mut secret_bytes = hex::decode(secret.as_str()).unwrap();
        let claimed_owner = "dd".repeat(32);
        let aad_s = aad::nostr_secret_key(&claimed_owner);
        let mismatched = crypto
            .encrypt_bytes(&secret_bytes, aad_s.as_bytes())
            .unwrap();
        secret_bytes.zeroize();

        let err = decrypt_nostr_secret_key(&db, &mismatched, &claimed_owner).unwrap_err();
        assert!(
            err.to_string().contains("does not match owner pubkey"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn test_different_keypairs_are_unique() {
        let db = test_db().await;
        let kp1 = generate_encrypted_keypair(&db).unwrap();
        let kp2 = generate_encrypted_keypair(&db).unwrap();
        assert_ne!(kp1.pubkey, kp2.pubkey);
    }

    #[tokio::test]
    async fn test_no_crypto_service_errors() {
        let mut db = Database::connect_memory().await.unwrap();
        db.crypto = None;

        let result = generate_encrypted_keypair(&db);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("CryptoService"));
    }

    #[tokio::test]
    async fn test_create_member_gets_server_managed_keypair() {
        let db = test_db().await;
        let member = db
            .create_member("test-user-1", "TestUser", crate::types::OrgRole::Member)
            .await
            .unwrap();

        assert!(member.nostr_pubkey.is_some());
        assert_eq!(
            member.nostr_key_mode,
            Some(crate::types::NostrKeyMode::ServerManaged)
        );
        assert!(member.nostr_secret_key_encrypted.is_some());

        // Pubkey should be valid 64-char hex
        let pubkey = member.nostr_pubkey.unwrap();
        assert_eq!(pubkey.len(), 64);
        assert!(pubkey.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[tokio::test]
    async fn test_provision_keypair_for_existing_member() {
        let db = test_db().await;

        // Create member without crypto first
        let mut db_no_crypto = Database::connect_memory().await.unwrap();
        db_no_crypto.crypto = None;
        run_migrations(&db_no_crypto.client).await.unwrap();
        let member = db_no_crypto
            .create_member("test-user-2", "NoCrypto", crate::types::OrgRole::Member)
            .await
            .unwrap();
        assert!(member.nostr_pubkey.is_none());

        // Now provision with a crypto-enabled db
        let member = db
            .create_member("test-user-3", "LateKeypair", crate::types::OrgRole::Member)
            .await
            .unwrap();
        let pubkey = db.provision_nostr_keypair(&member.id).await.unwrap();
        assert_eq!(pubkey.len(), 64);

        // Verify we can decrypt
        let secret = db.get_nostr_secret_key(&member.id).await.unwrap();
        assert!(secret.is_some());
    }

    #[tokio::test]
    async fn test_external_key_mode_transition() {
        let db = test_db().await;
        let member = db
            .create_member("test-user-4", "ExternalTest", crate::types::OrgRole::Member)
            .await
            .unwrap();

        // Should start as server_managed
        assert_eq!(
            member.nostr_key_mode,
            Some(crate::types::NostrKeyMode::ServerManaged)
        );

        // Simulate linking an external key via update_member
        let external_pubkey = "a".repeat(64); // fake 64-char hex
        let updated = db
            .update_member(
                &member.id,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(Some(&external_pubkey)),
                None,
            )
            .await
            .unwrap();

        assert_eq!(
            updated.nostr_key_mode,
            Some(crate::types::NostrKeyMode::External)
        );
        assert_eq!(
            updated.nostr_pubkey.as_deref(),
            Some(external_pubkey.as_str())
        );
        assert!(updated.nostr_secret_key_encrypted.is_none());
    }
}
