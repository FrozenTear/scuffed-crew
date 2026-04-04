use scuffed_auth::crypto::EncryptedBlob;
use secp256k1::{rand::rngs::OsRng, Secp256k1};

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
pub fn generate_encrypted_keypair(db: &Database) -> DbResult<NostrKeypair> {
    let crypto = db
        .crypto
        .as_ref()
        .ok_or_else(|| DbError::Config("CryptoService not configured — set ENCRYPTION_KEY".into()))?;

    let secp = Secp256k1::new();
    let (secret_key, public_key) = secp.generate_keypair(&mut OsRng);

    // x-only public key (32 bytes) is the Nostr pubkey format
    let (xonly, _parity) = public_key.x_only_public_key();
    let pubkey_hex = hex::encode(xonly.serialize());

    // Encrypt the 32-byte secret key
    let secret_hex = hex::encode(secret_key.secret_bytes());
    let secret_key_encrypted = crypto.encrypt(&secret_hex).map_err(DbError::Crypto)?;

    // secret_hex is on the stack and will be dropped — for additional safety
    // the actual secret_key bytes are in secp256k1's SecretKey which implements
    // zeroize via Drop (when the `std` feature is active on secp256k1)

    Ok(NostrKeypair {
        pubkey: pubkey_hex,
        secret_key_encrypted,
    })
}

/// Decrypt a member's stored Nostr secret key, returning the hex-encoded secret.
///
/// The caller MUST use the result immediately and not hold it in long-lived state.
pub fn decrypt_nostr_secret_key(db: &Database, blob: &EncryptedBlob) -> DbResult<String> {
    let crypto = db
        .crypto
        .as_ref()
        .ok_or_else(|| DbError::Config("CryptoService not configured".into()))?;

    crypto.decrypt(blob).map_err(DbError::Crypto)
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
    pub async fn get_nostr_secret_key(&self, member_id: &str) -> DbResult<Option<String>> {
        let member = self
            .get_member(member_id)
            .await?
            .ok_or_else(|| DbError::NotFound(format!("Member {member_id} not found")))?;

        match (member.nostr_key_mode, member.nostr_secret_key_encrypted) {
            (Some(NostrKeyMode::ServerManaged), Some(ref blob)) => {
                let secret_hex = decrypt_nostr_secret_key(self, blob)?;
                Ok(Some(secret_hex))
            }
            _ => Ok(None),
        }
    }

    /// Transition a member to external key mode.
    ///
    /// Sets the pubkey to the externally-provided one, clears the encrypted
    /// secret key, and sets mode to `external`.
    pub async fn set_external_nostr_key(
        &self,
        member_id: &str,
        pubkey: &str,
    ) -> DbResult<()> {
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
        let decrypted = decrypt_nostr_secret_key(&db, &keypair.secret_key_encrypted).unwrap();

        // Decrypted secret should be 64-char hex (32 bytes)
        assert_eq!(decrypted.len(), 64);
        assert!(decrypted.chars().all(|c| c.is_ascii_hexdigit()));

        // Verify the secret key corresponds to the public key
        let secret_bytes = hex::decode(&decrypted).unwrap();
        let secp = Secp256k1::new();
        let sk = secp256k1::SecretKey::from_slice(&secret_bytes).unwrap();
        let pk = sk.public_key(&secp);
        let (xonly, _) = pk.x_only_public_key();
        assert_eq!(hex::encode(xonly.serialize()), keypair.pubkey);
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
        assert_eq!(member.nostr_key_mode, Some(crate::types::NostrKeyMode::ServerManaged));
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

        // Now provision with a crypto-enabled db (different in-memory instance,
        // so we test provision_nostr_keypair on the first db)
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
        assert_eq!(member.nostr_key_mode, Some(crate::types::NostrKeyMode::ServerManaged));

        // Simulate linking an external key via update_member
        let external_pubkey = "a".repeat(64); // fake 64-char hex
        let updated = db
            .update_member(
                &member.id,
                None, None, None, None, None, None,
                Some(Some(&external_pubkey)),
                None,
            )
            .await
            .unwrap();

        assert_eq!(updated.nostr_key_mode, Some(crate::types::NostrKeyMode::External));
        assert_eq!(updated.nostr_pubkey.as_deref(), Some(external_pubkey.as_str()));
        assert!(updated.nostr_secret_key_encrypted.is_none());
    }
}
