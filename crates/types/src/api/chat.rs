//! Site chat API types (F-API-003/004).
//!
//! Mirrors `GET /api/teams/:id/channels`, `POST /api/chat/send-encrypted`,
//! `POST /api/chat/decrypt`, and `POST /api/chat/auth-token`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::nostr::NostrEvent;

/// Type of NIP-29 group channel returned by team channel discovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GroupType {
    Public,
    Officer,
}

impl GroupType {
    pub fn is_officer(self) -> bool {
        matches!(self, Self::Officer)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Public => "Team",
            Self::Officer => "Officers",
        }
    }
}

impl std::fmt::Display for GroupType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// A team's provisioned NIP-29 channel. Use `group_id` as-is — do not guess slugs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TeamChannel {
    pub id: String,
    pub team_id: String,
    pub group_id: String,
    pub group_type: GroupType,
    pub relay_url: String,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub synced_at: Option<DateTime<Utc>>,
}

/// POST /api/chat/send-encrypted
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SendEncryptedRequest {
    pub group_id: String,
    pub content: String,
    #[serde(default)]
    pub reply_to: Option<String>,
}

/// Response from POST /api/chat/send-encrypted
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SendEncryptedResponse {
    pub recipients_count: usize,
    pub sender_pubkey: String,
}

/// POST /api/chat/decrypt
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DecryptMessageRequest {
    pub event_json: String,
}

/// Response from POST /api/chat/decrypt
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DecryptMessageResponse {
    pub sender_pubkey: String,
    pub content: String,
    pub kind: u32,
    #[serde(default)]
    pub tags: Vec<Vec<String>>,
    pub created_at: u64,
}

/// POST /api/chat/auth-token
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuthTokenRequest {
    pub relay_url: String,
    #[serde(default)]
    pub challenge: Option<String>,
}

/// Response from POST /api/chat/auth-token
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuthTokenResponse {
    pub auth_event: NostrEvent,
    pub pubkey: String,
    pub relay_url: String,
}

/// User-facing copy for chat HTTP failures. Never leave the UI on eternal Loading.
pub fn chat_api_error_copy(status: u16, body: &str) -> String {
    match status {
        401 => "Sign in to use team chat.".into(),
        403 => "You don't have access to this channel.".into(),
        404 => "This channel isn't provisioned yet.".into(),
        400 if body_mentions(body, &["External key", "NIP-07"]) => {
            "Your Nostr key is external — use a NIP-07 extension to send.".into()
        }
        400 if body_mentions(
            body,
            &["auth-token", "No Nostr keys", "Nostr keys provisioned"],
        ) =>
        {
            "Nostr keys aren't set up yet. Try again after provisioning.".into()
        }
        422 => "No officer or admin on the roster has a Nostr key yet.".into(),
        502 => "The relay rejected the encrypted message.".into(),
        503 => "Can't reach the chat relay.".into(),
        _ => serde_json::from_str::<serde_json::Value>(body)
            .ok()
            .and_then(|v| v.get("error")?.as_str().map(str::to_owned))
            .unwrap_or_else(|| format!("Chat request failed (HTTP {status})")),
    }
}

fn body_mentions(body: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| body.contains(n))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_type_roundtrip() {
        assert_eq!(
            serde_json::from_str::<GroupType>("\"public\"").unwrap(),
            GroupType::Public
        );
        assert_eq!(
            serde_json::from_str::<GroupType>("\"officer\"").unwrap(),
            GroupType::Officer
        );
        assert_eq!(
            serde_json::to_string(&GroupType::Officer).unwrap(),
            "\"officer\""
        );
    }

    #[test]
    fn team_channel_deserializes_api_shape() {
        let json = r#"{
            "id": "ch1",
            "team_id": "t1",
            "group_id": "alpha-core",
            "group_type": "public",
            "relay_url": "wss://relay.example/relay",
            "is_active": true,
            "created_at": "2026-02-27T12:00:00Z"
        }"#;
        let ch: TeamChannel = serde_json::from_str(json).unwrap();
        assert_eq!(ch.group_id, "alpha-core");
        assert_eq!(ch.group_type, GroupType::Public);
        assert!(ch.is_active);
        assert!(ch.synced_at.is_none());
    }

    #[test]
    fn send_encrypted_serializes_null_reply() {
        let req = SendEncryptedRequest {
            group_id: "g1".into(),
            content: "hello".into(),
            reply_to: None,
        };
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(v["group_id"], "g1");
        assert_eq!(v["content"], "hello");
        assert!(v["reply_to"].is_null());
    }

    #[test]
    fn error_copy_maps_contract_statuses() {
        assert!(chat_api_error_copy(404, "").contains("isn't provisioned"));
        assert!(chat_api_error_copy(422, "").contains("Nostr key"));
        assert!(chat_api_error_copy(503, "").contains("Can't reach"));
        assert!(chat_api_error_copy(502, "").contains("rejected"));
        assert!(
            chat_api_error_copy(
                400,
                r#"{"error":"No Nostr keys provisioned. Call /api/chat/auth-token first."}"#
            )
            .contains("provisioning")
        );
        assert!(
            chat_api_error_copy(
                400,
                "External key users must encrypt client-side (NIP-07 + NIP-44)"
            )
            .contains("NIP-07")
        );
        assert_eq!(
            chat_api_error_copy(409, r#"{"error":"duplicate"}"#),
            "duplicate"
        );
    }
}
