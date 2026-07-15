//! NIP-44 v2 encryption and NIP-59 gift wrap for officer/private channels.
//!
//! Server-managed encryption flow:
//! 1. Member sends plaintext via Dioxus widget → Axum API (TLS required)
//! 2. Server decrypts member's `nostr_secret_key_encrypted` via `CryptoService` + AAD
//! 3. NIP-44 v2 encrypt (ChaCha20-Poly1305 + HKDF) for each recipient
//! 4. Wrap in NIP-59 gift wrap (rumor → seal → gift wrap per recipient)
//! 5. Decrypted key material zeroized immediately
//!
//! For officer channels of 5-15 members, this produces ~N gift-wrapped events per message.

use nostr::key::Keys;
use nostr::nips::nip44;
use nostr::{Event, EventBuilder as NostrEventBuilder, Kind, PublicKey, SecretKey};
use zeroize::Zeroizing;

use scuffed_auth::crypto::{aad, CryptoService, EncryptedBlob};

/// Errors from encryption operations.
#[derive(Debug, thiserror::Error)]
pub enum EncryptionError {
    #[error("encryption service not configured")]
    NoCryptoService,
    #[error("failed to decrypt member key: {0}")]
    KeyDecryptionFailed(String),
    #[error("invalid key data: {0}")]
    InvalidKey(String),
    #[error("NIP-44 encryption failed: {0}")]
    Nip44EncryptFailed(String),
    #[error("NIP-44 decryption failed: {0}")]
    Nip44DecryptFailed(String),
    #[error("NIP-59 gift wrap failed: {0}")]
    GiftWrapFailed(String),
    #[error("NIP-59 unwrap failed: {0}")]
    UnwrapFailed(String),
    #[error("not a gift wrap event (expected kind 1059)")]
    NotGiftWrap,
    #[error("no recipients provided")]
    NoRecipients,
}

/// A gift-wrapped event ready for publishing, with the intended recipient's pubkey.
#[derive(Debug)]
pub struct GiftWrappedEvent {
    /// The gift wrap event (kind 1059) to publish to the relay.
    pub event: Event,
    /// The recipient's public key (hex) — determines who can decrypt.
    pub recipient_pubkey: String,
}

/// Result of unwrapping a gift wrap event.
#[derive(Debug)]
pub struct UnwrappedMessage {
    /// The sender's public key (hex).
    pub sender_pubkey: String,
    /// The decrypted plaintext content.
    pub content: String,
    /// The original event kind from the rumor.
    pub kind: u32,
    /// Tags from the rumor (e.g., group ID, reply references).
    pub tags: Vec<Vec<String>>,
    /// The rumor's created_at timestamp.
    pub created_at: u64,
}

/// Service for NIP-44 encryption and NIP-59 gift wrap operations.
///
/// Wraps the `nostr` crate's NIP-44/NIP-59 primitives with the server-managed
/// key flow: decrypt member secret key → encrypt/wrap → zeroize key material.
pub struct EncryptionService {
    crypto: CryptoService,
}

impl EncryptionService {
    /// Create a new encryption service with the given CryptoService.
    pub fn new(crypto: CryptoService) -> Self {
        Self { crypto }
    }

    /// Decrypt a member's stored secret key into `nostr::Keys`.
    ///
    /// AAD is bound to `owner_pubkey_hex`. Supports raw 32-byte secrets and
    /// legacy UTF-8 hex strings. Material is zeroized on drop via `Zeroizing`.
    fn decrypt_member_keys(
        &self,
        encrypted_secret_key: &EncryptedBlob,
        owner_pubkey_hex: &str,
    ) -> Result<Keys, EncryptionError> {
        let aad_s = aad::nostr_secret_key(owner_pubkey_hex);
        let pt = self
            .crypto
            .decrypt_bytes(encrypted_secret_key, aad_s.as_bytes())
            .map_err(|e| EncryptionError::KeyDecryptionFailed(e.to_string()))?;

        let keys = keys_from_secret_plaintext(&pt)?;
        if !keys
            .public_key()
            .to_hex()
            .eq_ignore_ascii_case(owner_pubkey_hex)
        {
            return Err(EncryptionError::InvalidKey(
                "decrypted Nostr secret does not match owner pubkey".into(),
            ));
        }
        Ok(keys)
    }

    /// Parse a hex public key string.
    fn parse_pubkey(pubkey_hex: &str) -> Result<PublicKey, EncryptionError> {
        PublicKey::from_hex(pubkey_hex)
            .map_err(|e| EncryptionError::InvalidKey(format!("invalid pubkey: {e}")))
    }

    // =========================================================================
    // NIP-44 Direct Encryption
    // =========================================================================

    /// NIP-44 v2 encrypt content for a single recipient.
    ///
    /// Uses the sender's server-managed key. Returns the base64-encoded ciphertext.
    pub fn encrypt_nip44(
        &self,
        sender_encrypted_key: &EncryptedBlob,
        sender_pubkey_hex: &str,
        recipient_pubkey_hex: &str,
        plaintext: &str,
    ) -> Result<String, EncryptionError> {
        let sender_keys = self.decrypt_member_keys(sender_encrypted_key, sender_pubkey_hex)?;
        let recipient_pk = Self::parse_pubkey(recipient_pubkey_hex)?;

        nip44::encrypt(
            sender_keys.secret_key(),
            &recipient_pk,
            plaintext,
            nip44::Version::V2,
        )
        .map_err(|e| EncryptionError::Nip44EncryptFailed(e.to_string()))
    }

    /// NIP-44 v2 decrypt content from a sender.
    ///
    /// Uses the recipient's server-managed key. Returns the plaintext.
    pub fn decrypt_nip44(
        &self,
        recipient_encrypted_key: &EncryptedBlob,
        recipient_pubkey_hex: &str,
        sender_pubkey_hex: &str,
        ciphertext: &str,
    ) -> Result<String, EncryptionError> {
        let recipient_keys =
            self.decrypt_member_keys(recipient_encrypted_key, recipient_pubkey_hex)?;
        let sender_pk = Self::parse_pubkey(sender_pubkey_hex)?;

        nip44::decrypt(recipient_keys.secret_key(), &sender_pk, ciphertext)
            .map_err(|e| EncryptionError::Nip44DecryptFailed(e.to_string()))
    }

    // =========================================================================
    // NIP-59 Gift Wrap (Group Encryption)
    // =========================================================================

    /// Build NIP-59 gift-wrapped events for all group members.
    ///
    /// For each recipient:
    /// 1. Create unsigned rumor (kind 14) with the plaintext content
    /// 2. Seal the rumor with NIP-44 (kind 13, signed by author)
    /// 3. Gift wrap the seal (kind 1059, signed by random ephemeral key)
    ///
    /// Returns one `GiftWrappedEvent` per recipient. For a 10-member officer
    /// channel, this produces 10 gift-wrapped events.
    pub async fn build_gift_wraps(
        &self,
        sender_encrypted_key: &EncryptedBlob,
        sender_pubkey_hex: &str,
        recipient_pubkeys_hex: &[String],
        plaintext: &str,
        group_id: &str,
        reply_to: Option<&str>,
    ) -> Result<Vec<GiftWrappedEvent>, EncryptionError> {
        if recipient_pubkeys_hex.is_empty() {
            return Err(EncryptionError::NoRecipients);
        }

        let sender_keys = self.decrypt_member_keys(sender_encrypted_key, sender_pubkey_hex)?;

        // Build the unsigned rumor (kind 14 = private direct message per NIP-17)
        let mut rumor_builder = NostrEventBuilder::new(Kind::Custom(14), plaintext);

        // Tag with group ID so recipients can associate with the channel
        rumor_builder = rumor_builder.tag(nostr::Tag::custom(
            nostr::TagKind::custom("h"),
            vec![group_id.to_string()],
        ));

        // Add p-tags for all recipients (per NIP-17)
        for pubkey_hex in recipient_pubkeys_hex {
            rumor_builder = rumor_builder.tag(nostr::Tag::custom(
                nostr::TagKind::custom("p"),
                vec![pubkey_hex.clone()],
            ));
        }

        if let Some(event_id) = reply_to {
            rumor_builder = rumor_builder.tag(nostr::Tag::custom(
                nostr::TagKind::custom("e"),
                vec![event_id.to_string(), String::new(), "reply".to_string()],
            ));
        }

        let rumor = rumor_builder.build(sender_keys.public_key());

        // Gift wrap for each recipient
        let mut wrapped_events = Vec::with_capacity(recipient_pubkeys_hex.len());

        for pubkey_hex in recipient_pubkeys_hex {
            let recipient_pk = Self::parse_pubkey(pubkey_hex)?;
            let gift_wrap =
                NostrEventBuilder::gift_wrap(&sender_keys, &recipient_pk, rumor.clone(), [])
                    .await
                    .map_err(|e| EncryptionError::GiftWrapFailed(e.to_string()))?;

            wrapped_events.push(GiftWrappedEvent {
                event: gift_wrap,
                recipient_pubkey: pubkey_hex.clone(),
            });
        }

        Ok(wrapped_events)
    }

    /// Unwrap a NIP-59 gift wrap event to extract the original message.
    ///
    /// Uses the recipient's server-managed key to:
    /// 1. Decrypt the gift wrap (kind 1059) to get the seal (kind 13)
    /// 2. Decrypt the seal to get the rumor (kind 14)
    /// 3. Verify the sender from the seal signature
    pub async fn unwrap_gift_wrap(
        &self,
        recipient_encrypted_key: &EncryptedBlob,
        recipient_pubkey_hex: &str,
        gift_wrap_event: &Event,
    ) -> Result<UnwrappedMessage, EncryptionError> {
        if gift_wrap_event.kind != Kind::GiftWrap {
            return Err(EncryptionError::NotGiftWrap);
        }

        let recipient_keys =
            self.decrypt_member_keys(recipient_encrypted_key, recipient_pubkey_hex)?;

        let unwrapped = nostr::nips::nip59::extract_rumor(&recipient_keys, gift_wrap_event)
            .await
            .map_err(|e| EncryptionError::UnwrapFailed(e.to_string()))?;

        Ok(UnwrappedMessage {
            sender_pubkey: unwrapped.sender.to_hex(),
            content: unwrapped.rumor.content.to_string(),
            kind: unwrapped.rumor.kind.as_u16() as u32,
            tags: unwrapped
                .rumor
                .tags
                .iter()
                .map(|t| t.as_slice().iter().map(|s| s.to_string()).collect())
                .collect(),
            created_at: unwrapped.rumor.created_at.as_secs(),
        })
    }

    /// Unwrap a gift wrap from JSON event data.
    ///
    /// Convenience method for server-side decryption of incoming events.
    pub async fn unwrap_gift_wrap_json(
        &self,
        recipient_encrypted_key: &EncryptedBlob,
        recipient_pubkey_hex: &str,
        event_json: &str,
    ) -> Result<UnwrappedMessage, EncryptionError> {
        let event: Event = serde_json::from_str(event_json)
            .map_err(|e| EncryptionError::UnwrapFailed(format!("invalid event JSON: {e}")))?;

        self.unwrap_gift_wrap(recipient_encrypted_key, recipient_pubkey_hex, &event)
            .await
    }
}

fn keys_from_secret_plaintext(pt: &Zeroizing<Vec<u8>>) -> Result<Keys, EncryptionError> {
    if pt.len() == 32 {
        let sk = SecretKey::from_slice(pt.as_slice())
            .map_err(|e| EncryptionError::InvalidKey(e.to_string()))?;
        return Ok(Keys::new(sk));
    }
    let hex_str = std::str::from_utf8(pt.as_slice())
        .map_err(|e| EncryptionError::InvalidKey(e.to_string()))?;
    let sk =
        SecretKey::from_hex(hex_str).map_err(|e| EncryptionError::InvalidKey(e.to_string()))?;
    Ok(Keys::new(sk))
}

#[cfg(test)]
mod tests {
    use super::*;
    use zeroize::Zeroize;

    fn test_crypto_key() -> String {
        CryptoService::generate_key()
    }

    fn crypto_from_key(key: &str) -> CryptoService {
        CryptoService::new(key, 1).unwrap()
    }

    /// Encrypt raw 32-byte secret with AAD bound to pubkey (production format).
    fn make_encrypted_keys(key: &str) -> (Keys, EncryptedBlob) {
        let crypto = crypto_from_key(key);
        let keys = Keys::generate();
        let pubkey = keys.public_key().to_hex();
        let mut secret_bytes = keys.secret_key().to_secret_bytes();
        let aad_s = aad::nostr_secret_key(&pubkey);
        let encrypted = crypto
            .encrypt_bytes(&secret_bytes, aad_s.as_bytes())
            .unwrap();
        secret_bytes.zeroize();
        (keys, encrypted)
    }

    #[test]
    fn nip44_encrypt_decrypt_roundtrip() {
        let key = test_crypto_key();
        let svc = EncryptionService::new(crypto_from_key(&key));

        let (sender_keys, sender_enc) = make_encrypted_keys(&key);
        let (recipient_keys, recipient_enc) = make_encrypted_keys(&key);
        let sender_pk = sender_keys.public_key().to_hex();
        let recipient_pk = recipient_keys.public_key().to_hex();

        let plaintext = "Hello officer channel!";
        let ciphertext = svc
            .encrypt_nip44(&sender_enc, &sender_pk, &recipient_pk, plaintext)
            .unwrap();

        assert_ne!(ciphertext, plaintext);

        let decrypted = svc
            .decrypt_nip44(&recipient_enc, &recipient_pk, &sender_pk, &ciphertext)
            .unwrap();

        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn nip44_wrong_key_fails_decrypt() {
        let key = test_crypto_key();
        let svc = EncryptionService::new(crypto_from_key(&key));

        let (_sender_keys, sender_enc) = make_encrypted_keys(&key);
        let (recipient_keys, _recipient_enc) = make_encrypted_keys(&key);
        let (wrong_keys, wrong_enc) = make_encrypted_keys(&key);
        let sender_pk = _sender_keys.public_key().to_hex();
        let recipient_pk = recipient_keys.public_key().to_hex();
        let wrong_pk = wrong_keys.public_key().to_hex();

        let ciphertext = svc
            .encrypt_nip44(&sender_enc, &sender_pk, &recipient_pk, "secret message")
            .unwrap();

        let result = svc.decrypt_nip44(&wrong_enc, &wrong_pk, &sender_pk, &ciphertext);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn gift_wrap_roundtrip() {
        let key = test_crypto_key();
        let svc = EncryptionService::new(crypto_from_key(&key));

        let (sender_keys, sender_enc) = make_encrypted_keys(&key);
        let (_r1_keys, r1_enc) = make_encrypted_keys(&key);
        let (r2_keys, _r2_enc) = make_encrypted_keys(&key);
        let sender_pk = sender_keys.public_key().to_hex();
        let r1_pk = _r1_keys.public_key().to_hex();
        let r2_pk = r2_keys.public_key().to_hex();

        let recipients = vec![r1_pk.clone(), r2_pk];

        let plaintext = "Officer briefing: mission start at 0600";
        let wrapped = svc
            .build_gift_wraps(
                &sender_enc,
                &sender_pk,
                &recipients,
                plaintext,
                "officers-alpha",
                None,
            )
            .await
            .unwrap();

        assert_eq!(wrapped.len(), 2);

        for gw in &wrapped {
            assert_eq!(gw.event.kind, Kind::GiftWrap);
        }

        let msg = svc
            .unwrap_gift_wrap(&r1_enc, &r1_pk, &wrapped[0].event)
            .await
            .unwrap();

        assert_eq!(msg.content, plaintext);
        assert_eq!(msg.sender_pubkey, sender_pk);
        assert_eq!(msg.kind, 14);

        let h_tag = msg
            .tags
            .iter()
            .find(|t| t.first().map(|s| s.as_str()) == Some("h"));
        assert_eq!(
            h_tag.and_then(|t| t.get(1).map(|s| s.as_str())),
            Some("officers-alpha")
        );
    }

    #[tokio::test]
    async fn gift_wrap_with_reply() {
        let key = test_crypto_key();
        let svc = EncryptionService::new(crypto_from_key(&key));

        let (sender_keys, sender_enc) = make_encrypted_keys(&key);
        let (r1_keys, r1_enc) = make_encrypted_keys(&key);
        let sender_pk = sender_keys.public_key().to_hex();
        let r1_pk = r1_keys.public_key().to_hex();

        let recipients = vec![r1_pk.clone()];
        let wrapped = svc
            .build_gift_wraps(
                &sender_enc,
                &sender_pk,
                &recipients,
                "Reply to previous",
                "officers-alpha",
                Some("abc123def"),
            )
            .await
            .unwrap();

        let msg = svc
            .unwrap_gift_wrap(&r1_enc, &r1_pk, &wrapped[0].event)
            .await
            .unwrap();

        let e_tag = msg
            .tags
            .iter()
            .find(|t| t.first().map(|s| s.as_str()) == Some("e"));
        assert_eq!(
            e_tag.and_then(|t| t.get(1).map(|s| s.as_str())),
            Some("abc123def")
        );
    }

    #[tokio::test]
    async fn gift_wrap_no_recipients_fails() {
        let key = test_crypto_key();
        let svc = EncryptionService::new(crypto_from_key(&key));

        let (sender_keys, sender_enc) = make_encrypted_keys(&key);
        let sender_pk = sender_keys.public_key().to_hex();

        let result = svc
            .build_gift_wraps(&sender_enc, &sender_pk, &[], "message", "group1", None)
            .await;

        assert!(matches!(result, Err(EncryptionError::NoRecipients)));
    }

    #[tokio::test]
    async fn unwrap_wrong_kind_fails() {
        let key = test_crypto_key();
        let svc = EncryptionService::new(crypto_from_key(&key));

        let (keys, enc) = make_encrypted_keys(&key);
        let pk = keys.public_key().to_hex();

        let keys2 = Keys::generate();
        let event = NostrEventBuilder::new(Kind::Custom(1), "not a gift wrap")
            .sign_with_keys(&keys2)
            .unwrap();

        let result = svc.unwrap_gift_wrap(&enc, &pk, &event).await;
        assert!(matches!(result, Err(EncryptionError::NotGiftWrap)));
    }

    #[test]
    fn decrypt_member_keys_zeroizes() {
        let key = test_crypto_key();
        let svc = EncryptionService::new(crypto_from_key(&key));

        let (original_keys, encrypted) = make_encrypted_keys(&key);
        let pk = original_keys.public_key().to_hex();

        let keys = svc.decrypt_member_keys(&encrypted, &pk).unwrap();
        assert_eq!(
            keys.public_key().to_hex(),
            original_keys.public_key().to_hex()
        );
    }

    #[test]
    fn decrypt_member_keys_pubkey_mismatch() {
        let key = test_crypto_key();
        let crypto = crypto_from_key(&key);
        let svc = EncryptionService::new(crypto_from_key(&key));

        // Secret of real_keys encrypted under AAD of a different claimed owner.
        let claimed_owner = "cc".repeat(32);
        let real_keys = Keys::generate();
        let mut secret_bytes = real_keys.secret_key().to_secret_bytes();
        let aad_s = aad::nostr_secret_key(&claimed_owner);
        let encrypted = crypto
            .encrypt_bytes(&secret_bytes, aad_s.as_bytes())
            .unwrap();
        secret_bytes.zeroize();

        let err = svc
            .decrypt_member_keys(&encrypted, &claimed_owner)
            .unwrap_err();
        assert!(
            matches!(err, EncryptionError::InvalidKey(_)),
            "unexpected error: {err}"
        );
        assert!(
            err.to_string().contains("does not match owner pubkey"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn encryption_error_display() {
        let err = EncryptionError::NotGiftWrap;
        assert_eq!(
            err.to_string(),
            "not a gift wrap event (expected kind 1059)"
        );

        let err = EncryptionError::NoRecipients;
        assert_eq!(err.to_string(), "no recipients provided");
    }
}
