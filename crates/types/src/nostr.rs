//! Lightweight Nostr relay protocol types (NIP-01, NIP-29, NIP-42).
//!
//! These are minimal types for relay WebSocket communication — not a full Nostr
//! library. Designed for WASM frontend use in the Dioxus chat widget.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// =============================================================================
// Nostr Event (NIP-01)
// =============================================================================

/// A Nostr event as defined by NIP-01.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct NostrEvent {
    pub id: String,
    pub pubkey: String,
    pub created_at: u64,
    pub kind: u32,
    pub tags: Vec<Vec<String>>,
    pub content: String,
    pub sig: String,
}

impl NostrEvent {
    /// Get the first value for a given single-letter tag (e.g., "e", "p", "h", "d").
    pub fn tag_value(&self, tag_name: &str) -> Option<&str> {
        self.tags
            .iter()
            .find(|t| t.first().map(|s| s.as_str()) == Some(tag_name))
            .and_then(|t| t.get(1).map(|s| s.as_str()))
    }

    /// Get all values for a given single-letter tag.
    pub fn tag_values(&self, tag_name: &str) -> Vec<&str> {
        self.tags
            .iter()
            .filter(|t| t.first().map(|s| s.as_str()) == Some(tag_name))
            .filter_map(|t| t.get(1).map(|s| s.as_str()))
            .collect()
    }

    /// Check if this is a NIP-29 group chat message (kind 9).
    pub fn is_group_chat(&self) -> bool {
        self.kind == 9
    }

    /// Check if this is a NIP-42 AUTH event (kind 22242).
    pub fn is_auth(&self) -> bool {
        self.kind == 22242
    }

    /// Get the group ID from an "h" tag (NIP-29).
    pub fn group_id(&self) -> Option<&str> {
        self.tag_value("h")
    }

    /// Get referenced pubkeys from "p" tags.
    pub fn referenced_pubkeys(&self) -> Vec<&str> {
        self.tag_values("p")
    }
}

// =============================================================================
// NIP-01 Subscription Filter
// =============================================================================

/// A NIP-01 subscription filter.
///
/// Serializes with `#[serde(skip_serializing_if = "Option::is_none")]` to produce
/// compact JSON that relays expect.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct NostrFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authors: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kinds: Option<Vec<u32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    /// Generic tag filters: key is the tag letter (without #), value is the list
    /// of accepted values. Serialized as `#e`, `#p`, `#h`, etc.
    #[serde(flatten)]
    pub tags: HashMap<String, Vec<String>>,
}

impl NostrFilter {
    /// Create a filter for NIP-29 group chat messages in a specific group.
    pub fn group_chat(group_id: &str, limit: Option<usize>) -> Self {
        let mut tags = HashMap::new();
        tags.insert(format!("#{}", "h"), vec![group_id.to_string()]);
        Self {
            kinds: Some(vec![9]),     // NIP-29 group chat message
            limit,
            tags,
            ..Default::default()
        }
    }

    /// Create a filter for NIP-29 group metadata events.
    pub fn group_metadata(group_id: &str) -> Self {
        let mut tags = HashMap::new();
        tags.insert(format!("#{}", "d"), vec![group_id.to_string()]);
        Self {
            kinds: Some(vec![39000, 39001, 39002]), // group metadata, admins, members
            tags,
            ..Default::default()
        }
    }

    /// Create a filter for multiple event kinds (e.g., presence, voice status).
    pub fn by_kinds(kinds: Vec<u32>) -> Self {
        Self {
            kinds: Some(kinds),
            ..Default::default()
        }
    }
}

// =============================================================================
// Relay Protocol Messages (NIP-01 JSON arrays)
// =============================================================================

/// Messages sent from client to relay (NIP-01 + NIP-42).
///
/// These serialize as JSON arrays per the Nostr protocol:
/// - `["EVENT", <event>]`
/// - `["REQ", <sub_id>, <filter>, ...]`
/// - `["CLOSE", <sub_id>]`
/// - `["AUTH", <event>]`
#[derive(Clone, Debug)]
pub enum ClientRelayMessage {
    /// Publish an event to the relay.
    Event(NostrEvent),
    /// Subscribe to events matching the given filters.
    Req {
        subscription_id: String,
        filters: Vec<NostrFilter>,
    },
    /// Close a subscription.
    Close(String),
    /// NIP-42 AUTH response.
    Auth(NostrEvent),
}

impl ClientRelayMessage {
    /// Serialize to JSON string (NIP-01 array format).
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        match self {
            ClientRelayMessage::Event(event) => {
                serde_json::to_string(&serde_json::json!(["EVENT", event]))
            }
            ClientRelayMessage::Req {
                subscription_id,
                filters,
            } => {
                let mut arr: Vec<serde_json::Value> = vec![
                    serde_json::Value::String("REQ".into()),
                    serde_json::Value::String(subscription_id.clone()),
                ];
                for f in filters {
                    arr.push(serde_json::to_value(f)?);
                }
                serde_json::to_string(&arr)
            }
            ClientRelayMessage::Close(sub_id) => {
                serde_json::to_string(&serde_json::json!(["CLOSE", sub_id]))
            }
            ClientRelayMessage::Auth(event) => {
                serde_json::to_string(&serde_json::json!(["AUTH", event]))
            }
        }
    }
}

/// Messages received from relay to client (NIP-01 + NIP-42).
#[derive(Clone, Debug)]
pub enum RelayMessage {
    /// An event matching a subscription.
    Event {
        subscription_id: String,
        event: NostrEvent,
    },
    /// Acknowledgement of a published event.
    Ok {
        event_id: String,
        accepted: bool,
        message: String,
    },
    /// End of stored events for a subscription.
    Eose {
        subscription_id: String,
    },
    /// NIP-42 AUTH challenge from relay.
    Auth {
        challenge: String,
    },
    /// Human-readable notice from relay.
    Notice(String),
}

impl RelayMessage {
    /// Parse a relay message from a JSON string (NIP-01 array format).
    pub fn from_json(json: &str) -> Result<Self, String> {
        let arr: Vec<serde_json::Value> =
            serde_json::from_str(json).map_err(|e| format!("invalid JSON: {e}"))?;

        let msg_type = arr
            .first()
            .and_then(|v| v.as_str())
            .ok_or("missing message type")?;

        match msg_type {
            "EVENT" => {
                let sub_id = arr
                    .get(1)
                    .and_then(|v| v.as_str())
                    .ok_or("EVENT: missing subscription_id")?
                    .to_string();
                let event: NostrEvent = serde_json::from_value(
                    arr.get(2).cloned().ok_or("EVENT: missing event object")?,
                )
                .map_err(|e| format!("EVENT: invalid event: {e}"))?;
                Ok(RelayMessage::Event {
                    subscription_id: sub_id,
                    event,
                })
            }
            "OK" => {
                let event_id = arr
                    .get(1)
                    .and_then(|v| v.as_str())
                    .ok_or("OK: missing event_id")?
                    .to_string();
                let accepted = arr.get(2).and_then(|v| v.as_bool()).unwrap_or(false);
                let message = arr
                    .get(3)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                Ok(RelayMessage::Ok {
                    event_id,
                    accepted,
                    message,
                })
            }
            "EOSE" => {
                let sub_id = arr
                    .get(1)
                    .and_then(|v| v.as_str())
                    .ok_or("EOSE: missing subscription_id")?
                    .to_string();
                Ok(RelayMessage::Eose {
                    subscription_id: sub_id,
                })
            }
            "AUTH" => {
                let challenge = arr
                    .get(1)
                    .and_then(|v| v.as_str())
                    .ok_or("AUTH: missing challenge")?
                    .to_string();
                Ok(RelayMessage::Auth { challenge })
            }
            "NOTICE" => {
                let msg = arr
                    .get(1)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                Ok(RelayMessage::Notice(msg))
            }
            other => Err(format!("unknown relay message type: {other}")),
        }
    }
}

// =============================================================================
// NIP-29 Group Types
// =============================================================================

/// A NIP-29 relay-based group.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct NostrGroup {
    /// Group identifier (from the "d" tag on group metadata events).
    pub id: String,
    /// Human-readable group name.
    pub name: String,
    /// Group description/about.
    pub about: Option<String>,
    /// Whether the group is public or private.
    pub is_public: bool,
    /// Whether the group is open (anyone can join) or closed (invite-only).
    pub is_open: bool,
}

/// A chat message parsed from a NIP-29 kind-9 event.
#[derive(Clone, Debug, PartialEq)]
pub struct ChatMessage {
    /// Nostr event ID.
    pub id: String,
    /// Author pubkey.
    pub pubkey: String,
    /// Display name (resolved from profile or member data).
    pub display_name: Option<String>,
    /// Avatar URL (resolved from profile or member data).
    pub avatar_url: Option<String>,
    /// Message content (plaintext or decrypted).
    pub content: String,
    /// Unix timestamp.
    pub created_at: u64,
    /// Group ID this message belongs to.
    pub group_id: String,
    /// Whether this message is encrypted (NIP-44).
    pub encrypted: bool,
    /// Referenced event IDs (replies).
    pub reply_to: Option<String>,
}

impl ChatMessage {
    /// Parse a ChatMessage from a NIP-29 group chat event.
    pub fn from_event(event: &NostrEvent) -> Option<Self> {
        let group_id = event.group_id()?.to_string();
        let reply_to = event.tag_value("e").map(|s| s.to_string());

        Some(ChatMessage {
            id: event.id.clone(),
            pubkey: event.pubkey.clone(),
            display_name: None,
            avatar_url: None,
            content: event.content.clone(),
            created_at: event.created_at,
            group_id,
            encrypted: false,
            reply_to,
        })
    }
}

// =============================================================================
// NIP-29 Event Kinds (for reference)
// =============================================================================

/// Well-known Nostr event kinds used in the chat system.
pub mod event_kinds {
    /// NIP-01 kind 0: user profile metadata.
    pub const PROFILE_METADATA: u32 = 0;
    /// NIP-29 group chat message.
    pub const GROUP_CHAT_MESSAGE: u32 = 9;
    /// NIP-29 group chat reply (threaded).
    pub const GROUP_CHAT_REPLY: u32 = 10;
    /// NIP-29 join request.
    pub const GROUP_JOIN_REQUEST: u32 = 9021;
    /// NIP-29 group metadata.
    pub const GROUP_METADATA: u32 = 39000;
    /// NIP-29 group admins.
    pub const GROUP_ADMINS: u32 = 39001;
    /// NIP-29 group members.
    pub const GROUP_MEMBERS: u32 = 39002;
    /// NIP-42 AUTH event.
    pub const AUTH: u32 = 22242;
    /// NIP-44 encrypted direct message (NIP-17 private).
    pub const PRIVATE_DIRECT_MESSAGE: u32 = 14;
    /// NIP-59 gift wrap.
    pub const GIFT_WRAP: u32 = 1059;
    /// NIP-59 seal.
    pub const SEAL: u32 = 13;
    /// NIP-25 reaction (like, emoji).
    pub const REACTION: u32 = 7;
    /// NIP-72 community definition (replaceable).
    pub const COMMUNITY_DEFINITION: u32 = 34550;
}
