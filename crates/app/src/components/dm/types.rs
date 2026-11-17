use bech32::{Bech32, Hrp, primitives::decode::CheckedHrpstring};
use scuffed_api_client::ClientError;
use serde::{Deserialize, Serialize};

/// Why a DM list or thread fetch failed. Status mapping is deliberate:
/// 403 is membership (OrgMember), not the 412 server-managed identity case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DmLoadFailure {
    /// 412: server-managed identity missing or not enabled.
    IdentitySettings,
    /// 403: signed in, but not an active org member.
    Membership,
    /// 503: relay URL is not configured.
    RelayConfig,
    /// Any other failure; `message` is the client error string.
    Other(String),
}

impl DmLoadFailure {
    pub fn message(&self) -> String {
        match self {
            Self::IdentitySettings => {
                "Direct messages require a server-managed Nostr identity.".to_string()
            }
            Self::Membership => "Direct messages require an active clan membership.".to_string(),
            Self::RelayConfig => {
                "Direct messages need a configured chat relay. Ask an admin to set the relay URL."
                    .to_string()
            }
            Self::Other(message) => message.clone(),
        }
    }
}

pub fn classify_dm_client_error(err: &ClientError) -> DmLoadFailure {
    match err {
        ClientError::Http { status: 412, .. } => DmLoadFailure::IdentitySettings,
        ClientError::Http { status: 403, .. } => DmLoadFailure::Membership,
        ClientError::Http { status: 503, .. } => DmLoadFailure::RelayConfig,
        other => DmLoadFailure::Other(other.to_string()),
    }
}

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

/// A crew member as `/api/members` returns them, reduced to what DMs need:
/// the name to show and the Nostr key to match a conversation or a
/// recipient against.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct DmMember {
    pub id: String,
    pub display_name: String,
    #[serde(default)]
    pub nostr_pubkey: Option<String>,
}

/// One page of `/api/members`.
#[derive(Debug, Clone, Deserialize)]
pub struct DmMemberPage {
    pub data: Vec<DmMember>,
    #[serde(default)]
    pub next_cursor: Option<String>,
}

/// The name to show for a peer: the server's display name when it sends one,
/// else the crew member whose Nostr key matches, else a short npub label.
pub fn peer_label(pubkey: &str, server_name: Option<&str>, members: &[DmMember]) -> String {
    if let Some(name) = server_name.map(str::trim).filter(|s| !s.is_empty()) {
        return name.to_string();
    }
    members
        .iter()
        .find(|m| {
            m.nostr_pubkey
                .as_deref()
                .is_some_and(|pk| pk.trim().eq_ignore_ascii_case(pubkey.trim()))
        })
        .map(|m| m.display_name.trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| truncate_pubkey(pubkey))
}

/// Up to two initials for an avatar, the same rule as the article byline and
/// the account avatar: two words give the first letter of each ("Player Two"
/// is "PT"), one word its first two letters or digits ("Player02" is "PL").
pub fn avatar_initials(name: &str) -> String {
    let mut words = name.split_whitespace();
    let (first, second) = (words.next().unwrap_or(""), words.next());
    let picked: Vec<char> = match second {
        Some(second) => first
            .chars()
            .filter(|c| c.is_alphanumeric())
            .take(1)
            .chain(second.chars().filter(|c| c.is_alphanumeric()).take(1))
            .collect(),
        None => first
            .chars()
            .filter(|c| c.is_alphanumeric())
            .take(2)
            .collect(),
    };
    let out: String = picked.into_iter().flat_map(char::to_uppercase).collect();
    if out.is_empty() { "?".to_string() } else { out }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn member(name: &str, pk: Option<&str>) -> DmMember {
        DmMember {
            id: format!("member:{name}"),
            display_name: name.to_string(),
            nostr_pubkey: pk.map(str::to_string),
        }
    }

    #[test]
    fn peers_show_by_name_when_the_app_can_resolve_them() {
        let pk = "ab".repeat(32);
        let members = vec![member("Player02", Some(&pk)), member("Player03", None)];
        // The server's name wins.
        assert_eq!(peer_label(&pk, Some("Player09"), &members), "Player09");
        // A blank server name falls back to the member list, any key case.
        assert_eq!(peer_label(&pk, Some("  "), &members), "Player02");
        assert_eq!(
            peer_label(&pk.to_ascii_uppercase(), None, &members),
            "Player02"
        );
        // Nobody matches: the short npub label, never the full key.
        let stranger = "cd".repeat(32);
        assert_eq!(
            peer_label(&stranger, None, &members),
            truncate_pubkey(&stranger)
        );
    }

    #[test]
    fn avatar_initials_take_two_characters() {
        assert_eq!(avatar_initials("Player02"), "PL");
        assert_eq!(avatar_initials("Player Two"), "PT");
        assert_eq!(avatar_initials("npub1\u{2026}abcdef"), "NP");
        assert_eq!(avatar_initials(""), "?");
    }

    #[test]
    fn dm_errors_map_412_403_503_and_otherwise_keep_the_string() {
        assert_eq!(
            classify_dm_client_error(&ClientError::http(
                412,
                r#"{"error":"Server-managed Nostr key required for DMs"}"#,
                None,
            )),
            DmLoadFailure::IdentitySettings
        );

        let forbidden = classify_dm_client_error(&ClientError::http(
            403,
            r#"{"error":"Not an org member"}"#,
            None,
        ));
        assert_eq!(forbidden, DmLoadFailure::Membership);
        assert!(
            !forbidden.message().contains("server-managed"),
            "403 must not use the identity banner"
        );

        // Old bug: any body mentioning server_managed, including 403, became the identity banner.
        let forbidden_body =
            classify_dm_client_error(&ClientError::http(403, "server_managed", None));
        assert_eq!(forbidden_body, DmLoadFailure::Membership);

        assert_eq!(
            classify_dm_client_error(&ClientError::http(
                503,
                r#"{"error":"Relay not configured"}"#,
                None,
            )),
            DmLoadFailure::RelayConfig
        );
        assert!(
            !DmLoadFailure::RelayConfig
                .message()
                .contains("server-managed")
        );

        let other = classify_dm_client_error(&ClientError::http(
            400,
            r#"{"error":"server_managed"}"#,
            None,
        ));
        match other {
            DmLoadFailure::Other(message) => {
                assert!(message.contains("server_managed"), "{message}");
                assert!(message.contains("400"), "{message}");
            }
            other => panic!("expected the error string, got {other:?}"),
        }

        let network = classify_dm_client_error(&ClientError::Network("offline".into()));
        assert_eq!(
            network,
            DmLoadFailure::Other("Network error: offline".into())
        );
    }
}
