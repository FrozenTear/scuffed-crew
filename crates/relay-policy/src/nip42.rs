//! NIP-42 AUTH checks for the read gate.
//!
//! The gate issues the challenge. A valid AUTH event is kind 22242, signed by
//! the pubkey it claims, and tagged with that challenge. The relay URL tag is
//! not compared to a single configured string: browsers present the
//! same-origin `wss://<host>/relay` URL, while the server's publish URL is
//! `ws://strfry:7777`. The fresh challenge is what binds the signature to this
//! connection.

use nostr::JsonUtil;
use serde_json::Value;

use crate::session::AuthVerifier;

pub struct NostrAuthVerifier;

impl AuthVerifier for NostrAuthVerifier {
    fn verify_auth(&self, event: &Value, challenge: &str) -> Result<String, String> {
        verify_nip42(event, challenge)
    }
}

pub fn verify_nip42(event: &Value, challenge: &str) -> Result<String, String> {
    if challenge.is_empty() {
        return Err("empty challenge".into());
    }
    let raw = serde_json::to_string(event).map_err(|e| format!("event json: {e}"))?;
    let parsed = nostr::Event::from_json(raw).map_err(|e| format!("invalid AUTH event: {e}"))?;
    parsed.verify().map_err(|_| "bad signature".to_string())?;
    if parsed.kind.as_u16() != 22242 {
        return Err("kind must be 22242".into());
    }
    let matches_challenge = parsed.tags.iter().any(|tag| {
        let parts = tag.as_slice();
        parts.len() >= 2 && parts[0] == "challenge" && parts[1] == challenge
    });
    if !matches_challenge {
        return Err("challenge mismatch".into());
    }
    Ok(parsed.pubkey.to_hex())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Tag, TagKind};

    fn auth_event(keys: &Keys, challenge: &str, kind: u16) -> Value {
        let event = EventBuilder::new(nostr::Kind::Custom(kind), "")
            .tag(Tag::custom(
                TagKind::custom("challenge"),
                vec![challenge.to_string()],
            ))
            .tag(Tag::custom(
                TagKind::custom("relay"),
                vec!["wss://ow.example/relay".to_string()],
            ))
            .sign_with_keys(keys)
            .expect("sign");
        serde_json::to_value(&event).expect("json")
    }

    #[test]
    fn valid_auth_returns_the_signing_pubkey() {
        let keys = Keys::generate();
        let event = auth_event(&keys, "chal-1", 22242);
        let pubkey = verify_nip42(&event, "chal-1").unwrap();
        assert_eq!(pubkey, keys.public_key().to_hex());
    }

    #[test]
    fn wrong_challenge_is_rejected() {
        let keys = Keys::generate();
        let event = auth_event(&keys, "chal-1", 22242);
        let err = verify_nip42(&event, "other").unwrap_err();
        assert!(err.contains("challenge"));
    }

    #[test]
    fn non_auth_kind_is_rejected_even_with_a_valid_signature() {
        let keys = Keys::generate();
        let event = auth_event(&keys, "chal-1", 1);
        assert!(verify_nip42(&event, "chal-1")
            .unwrap_err()
            .contains("22242"));
    }

    #[test]
    fn tampered_signature_is_rejected() {
        let keys = Keys::generate();
        let mut event = auth_event(&keys, "chal-1", 22242);
        event["sig"] = Value::String("00".repeat(64));
        assert!(verify_nip42(&event, "chal-1").is_err());
    }
}
