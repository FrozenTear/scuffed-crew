use bech32::{Bech32, Hrp, primitives::decode::CheckedHrpstring};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ConversationSummary {
    pub peer_pubkey: String,
    /// Optional display name. Backend currently does not populate this; we
    /// fall back to a truncated pubkey at render time. See issue THE-874 step 5
    /// for the planned member-listing / NIP-05 reverse lookup.
    #[serde(default)]
    pub peer_display_name: Option<String>,
    #[serde(default)]
    pub last_message_preview: String,
    pub last_message_at: String,
    #[serde(default)]
    pub last_sender_pubkey: String,
    #[serde(default)]
    pub unread_count: u64,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct DmMessage {
    pub id: String,
    #[serde(default)]
    pub gift_wrap_id: String,
    pub sender_pubkey: String,
    pub recipient_pubkey: String,
    pub content: String,
    pub created_at: String,
    #[serde(default)]
    pub reply_to_event_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SyncResponse {
    #[serde(default)]
    pub fetched: u64,
    #[serde(default)]
    pub stored: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct MarkReadBody {
    pub peer_pubkey: String,
    pub until_ts: String,
}

/// Normalize a pubkey string to a 64-char lowercase hex pubkey.
///
/// Accepts either a 64-char hex string (any case) or an `npub1…` bech32 string
/// (NIP-19). Returns `Err` with a human-readable reason otherwise. The backend
/// `/api/nostr/dm/send` only accepts hex, so the frontend must do this
/// conversion.
pub fn normalize_recipient_pubkey(input: &str) -> Result<String, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("Recipient pubkey is empty".into());
    }
    if trimmed.len() == 64 && trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
        return Ok(trimmed.to_ascii_lowercase());
    }
    if trimmed.starts_with("npub1") {
        let parsed =
            CheckedHrpstring::new::<Bech32>(trimmed).map_err(|e| format!("Invalid npub: {e}"))?;
        if parsed.hrp() != Hrp::parse_unchecked("npub") {
            return Err(format!("Expected `npub` prefix, got `{}`", parsed.hrp()));
        }
        let bytes: Vec<u8> = parsed.byte_iter().collect();
        if bytes.len() != 32 {
            return Err(format!(
                "npub decoded to {} bytes; expected 32",
                bytes.len()
            ));
        }
        let mut hex = String::with_capacity(64);
        for b in bytes {
            use std::fmt::Write;
            let _ = write!(hex, "{:02x}", b);
        }
        return Ok(hex);
    }
    Err("Recipient must be a 64-char hex pubkey or npub1… address".into())
}

/// Format a pubkey as a short truncated label when no display name is known.
pub fn truncate_pubkey(pubkey: &str) -> String {
    if pubkey.starts_with("npub1") {
        if pubkey.len() > 16 {
            format!("{}…{}", &pubkey[..10], &pubkey[pubkey.len() - 4..])
        } else {
            pubkey.to_string()
        }
    } else if pubkey.len() > 16 {
        format!("npub1…{}", &pubkey[pubkey.len() - 6..])
    } else {
        pubkey.to_string()
    }
}

/// Render an RFC3339 timestamp as a relative label ("2m", "3h", "5d").
pub fn relative_time(rfc3339: &str) -> String {
    let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(rfc3339) else {
        return rfc3339.chars().take(10).collect();
    };
    let now = chrono::Utc::now();
    let diff = now.signed_duration_since(parsed.with_timezone(&chrono::Utc));
    let secs = diff.num_seconds().max(0);
    if secs < 45 {
        "now".into()
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3600)
    } else if secs < 86_400 * 7 {
        format!("{}d", secs / 86_400)
    } else {
        rfc3339.chars().take(10).collect()
    }
}
