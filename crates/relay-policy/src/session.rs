//! Per-connection Nostr session for the read gate.
//!
//! `REQ` and client `EVENT` are not forwarded until NIP-42 AUTH succeeds and
//! the pubkey is an active member. After that, `EVENT` frames from strfry are
//! still filtered by [`crate::read_policy::allow_event`]. `COUNT` and
//! negentropy are not forwarded: a count or a sync would leak events the
//! member is not allowed to read. An authenticated member's `EVENT` is
//! forwarded so the write-policy plugin can accept or reject it. Kind 1059
//! is also limited per authed pubkey and recipient before forward. The site
//! server publishes without AUTH on the compose-network strfry port, which
//! does not pass through this gate.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::read_policy::{allow_event, EventView, ReadDirectory, GIFT_WRAP_KIND};

pub const AUTH_REQUIRED: &str = "auth-required: NIP-42 AUTH required before REQ";
pub const PUBLISH_AUTH_REQUIRED: &str = "auth-required: NIP-42 AUTH required before publishing";
pub const COUNT_DISABLED: &str = "restricted: COUNT is disabled";
pub const NEG_DISABLED: &str = "restricted: negentropy sync is disabled";
pub const RATE_LIMITED: &str = "rate-limited: too many events";

/// Per-sender gift-wrap budget for publishes that come through the gate.
///
/// Keyed `gift:{authed pubkey}:{recipient}`. One signed-in member cannot
/// spend another member's inbox budget. Shared across connections in the
/// process. site-server does not pass through this gate.
pub struct GiftBudget {
    limit: u32,
    window: Duration,
    buckets: Mutex<HashMap<String, Vec<Instant>>>,
}

impl GiftBudget {
    pub fn new(limit: u32, window_secs: u64) -> Self {
        Self {
            limit,
            window: Duration::from_secs(window_secs),
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Same knobs as the write plugin: `RELAY_POLICY_RATE_LIMIT` (default 30)
    /// and `RELAY_POLICY_RATE_WINDOW` (default 60).
    pub fn from_env() -> Self {
        let limit = std::env::var("RELAY_POLICY_RATE_LIMIT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30);
        let window_secs = std::env::var("RELAY_POLICY_RATE_WINDOW")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(60);
        Self::new(limit, window_secs)
    }

    /// Charge one wrap from `authed` to each recipient `p` tag.
    ///
    /// Returns false when any pair is already at the limit. Nothing is
    /// recorded in that case. A wrap with no 64-hex `p` tag is not counted
    /// here; the write plugin rejects it.
    pub fn allow(&self, authed: &str, frame: &Value) -> bool {
        let recipients = recipient_pubkeys(frame);
        if recipients.is_empty() {
            return true;
        }
        let authed = authed.trim().to_ascii_lowercase();
        let now = Instant::now();
        let keys: Vec<String> = recipients
            .iter()
            .map(|recipient| format!("gift:{authed}:{recipient}"))
            .collect();
        let mut buckets = self.buckets.lock().unwrap_or_else(|err| err.into_inner());
        for key in &keys {
            let count = buckets
                .get(key)
                .map(|stamps| {
                    stamps
                        .iter()
                        .filter(|stamp| now.duration_since(**stamp) < self.window)
                        .count()
                })
                .unwrap_or(0);
            if count >= self.limit as usize {
                return false;
            }
        }
        for key in keys {
            let stamps = buckets.entry(key).or_default();
            stamps.retain(|stamp| now.duration_since(*stamp) < self.window);
            stamps.push(now);
        }
        true
    }
}

fn recipient_pubkeys(frame: &Value) -> Vec<String> {
    let Some(tags) = frame
        .as_array()
        .and_then(|items| items.get(1))
        .and_then(|event| event.get("tags"))
        .and_then(|tags| tags.as_array())
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for tag in tags {
        let Some(parts) = tag.as_array() else {
            continue;
        };
        if parts.first().and_then(|part| part.as_str()) != Some("p") {
            continue;
        }
        let Some(pubkey) = parts.get(1).and_then(|part| part.as_str()) else {
            continue;
        };
        let pubkey = pubkey.trim().to_ascii_lowercase();
        if pubkey.len() == 64 && pubkey.chars().all(|c| c.is_ascii_hexdigit()) {
            out.push(pubkey);
        }
    }
    out
}

pub trait AuthVerifier: Send + Sync {
    fn verify_auth(&self, event: &Value, challenge: &str) -> Result<String, String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientOutcome {
    pub to_client: Vec<String>,
    pub to_relay: Vec<String>,
}

pub struct GateSession {
    challenge: String,
    authed_pubkey: Option<String>,
}

impl GateSession {
    pub fn new(challenge: impl Into<String>) -> Self {
        Self {
            challenge: challenge.into(),
            authed_pubkey: None,
        }
    }

    pub fn authed_pubkey(&self) -> Option<&str> {
        self.authed_pubkey.as_deref()
    }

    pub fn challenge_message(&self) -> String {
        json!(["AUTH", self.challenge]).to_string()
    }

    pub fn on_client_text<V: AuthVerifier + ?Sized>(
        &mut self,
        text: &str,
        verifier: &V,
        directory: &ReadDirectory,
        gifts: &GiftBudget,
    ) -> ClientOutcome {
        let Ok(value) = serde_json::from_str::<Value>(text) else {
            return ClientOutcome {
                to_client: vec![],
                to_relay: vec![],
            };
        };
        let Some(kind) = value
            .as_array()
            .and_then(|a| a.first())
            .and_then(|v| v.as_str())
        else {
            return ClientOutcome {
                to_client: vec![],
                to_relay: vec![],
            };
        };
        let sub_id = value
            .as_array()
            .and_then(|a| a.get(1))
            .and_then(|v| v.as_str())
            .unwrap_or("");

        match kind {
            "REQ" => self.gate_req(text, sub_id),
            "COUNT" => ClientOutcome {
                to_client: vec![closed(sub_id, COUNT_DISABLED)],
                to_relay: vec![],
            },
            other if other.starts_with("NEG") => ClientOutcome {
                to_client: vec![closed(sub_id, NEG_DISABLED)],
                to_relay: vec![],
            },
            "EVENT" => self.gate_event(text, &value, directory, gifts),
            "CLOSE" => ClientOutcome {
                to_client: vec![],
                to_relay: vec![text.to_string()],
            },
            "AUTH" => self.on_auth(&value, verifier),
            _ => ClientOutcome {
                to_client: vec![],
                to_relay: vec![],
            },
        }
    }

    fn gate_event(
        &self,
        text: &str,
        value: &Value,
        directory: &ReadDirectory,
        gifts: &GiftBudget,
    ) -> ClientOutcome {
        let event_id = published_id(value);
        let Some(authed) = self.authed_pubkey.as_deref() else {
            return ClientOutcome {
                to_client: vec![ok(event_id, false, PUBLISH_AUTH_REQUIRED)],
                to_relay: vec![],
            };
        };
        if directory.reader(authed).is_none() {
            return ClientOutcome {
                to_client: vec![ok(event_id, false, PUBLISH_AUTH_REQUIRED)],
                to_relay: vec![],
            };
        }
        if published_kind(value) == Some(crate::read_policy::PLAIN_DM_KIND) {
            return ClientOutcome {
                to_client: vec![ok(
                    event_id,
                    false,
                    "blocked: kind 4 plain DMs are not accepted",
                )],
                to_relay: vec![],
            };
        }
        if published_kind(value) == Some(GIFT_WRAP_KIND) && !gifts.allow(authed, value) {
            return ClientOutcome {
                to_client: vec![ok(event_id, false, RATE_LIMITED)],
                to_relay: vec![],
            };
        }
        ClientOutcome {
            to_client: vec![],
            to_relay: vec![text.to_string()],
        }
    }

    /// `None` drops the frame. Non-EVENT frames are forwarded, except an
    /// upstream `AUTH` challenge (this gate already sent its own).
    pub fn on_relay_text(&self, text: &str, dir: &ReadDirectory) -> Option<String> {
        let value: Value = serde_json::from_str(text).ok()?;
        let kind = value.as_array()?.first()?.as_str()?;
        match kind {
            "AUTH" => None,
            "EVENT" => {
                let event_json = value.as_array()?.get(2)?;
                let event = EventView::from_json(event_json)?;
                match allow_event(self.authed_pubkey.as_deref(), &event, dir) {
                    Ok(()) => Some(text.to_string()),
                    Err(reason) => {
                        tracing::debug!(
                            reason = %reason,
                            kind = event.kind,
                            "dropped relay event"
                        );
                        None
                    }
                }
            }
            _ => Some(text.to_string()),
        }
    }

    fn gate_req(&self, text: &str, sub_id: &str) -> ClientOutcome {
        if self.authed_pubkey.is_none() {
            return ClientOutcome {
                to_client: vec![closed(sub_id, AUTH_REQUIRED)],
                to_relay: vec![],
            };
        }
        ClientOutcome {
            to_client: vec![],
            to_relay: vec![text.to_string()],
        }
    }

    fn on_auth<V: AuthVerifier + ?Sized>(&mut self, value: &Value, verifier: &V) -> ClientOutcome {
        let event_id = value
            .as_array()
            .and_then(|a| a.get(1))
            .and_then(|e| e.get("id"))
            .and_then(|id| id.as_str())
            .unwrap_or("");
        let Some(event) = value.as_array().and_then(|a| a.get(1)) else {
            return ClientOutcome {
                to_client: vec![ok(event_id, false, "auth-required: missing AUTH event")],
                to_relay: vec![],
            };
        };
        match verifier.verify_auth(event, &self.challenge) {
            Ok(pubkey) => {
                let pubkey = pubkey.trim().to_ascii_lowercase();
                tracing::info!("NIP-42 AUTH accepted");
                self.authed_pubkey = Some(pubkey);
                ClientOutcome {
                    to_client: vec![ok(event_id, true, "")],
                    to_relay: vec![],
                }
            }
            Err(reason) => {
                tracing::info!(reason = %reason, "NIP-42 AUTH rejected");
                ClientOutcome {
                    to_client: vec![ok(event_id, false, &format!("auth-required: {reason}"))],
                    to_relay: vec![],
                }
            }
        }
    }
}

fn published_kind(value: &Value) -> Option<u64> {
    value.as_array()?.get(1)?.get("kind")?.as_u64()
}

fn published_id(value: &Value) -> &str {
    value
        .as_array()
        .and_then(|items| items.get(1))
        .and_then(|event| event.get("id"))
        .and_then(|id| id.as_str())
        .unwrap_or("")
}

fn closed(sub_id: &str, reason: &str) -> String {
    json!(["CLOSED", sub_id, reason]).to_string()
}

fn ok(event_id: &str, accepted: bool, message: &str) -> String {
    json!(["OK", event_id, accepted, message]).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::read_policy::{GroupAcl, MemberAcl, ReadDirectory, GIFT_WRAP_KIND};
    use std::collections::HashSet;

    struct FixedVerifier {
        pubkey: String,
        fail: bool,
    }

    impl AuthVerifier for FixedVerifier {
        fn verify_auth(&self, _event: &Value, _challenge: &str) -> Result<String, String> {
            if self.fail {
                Err("bad signature".into())
            } else {
                Ok(self.pubkey.clone())
            }
        }
    }

    fn pk(byte: u8) -> String {
        format!("{byte:02x}").repeat(32)
    }

    fn talk(session: &mut GateSession, text: &str, verifier: &FixedVerifier) -> ClientOutcome {
        session.on_client_text(text, verifier, &dir_with_member(), &GiftBudget::new(30, 60))
    }

    fn dir_with_member() -> ReadDirectory {
        ReadDirectory::from_parts(
            [MemberAcl {
                pubkey: pk(0x11),
                member_id: "m".into(),
                officer: false,
                teams: HashSet::from(["team-a".into()]),
            }],
            [(
                "team-a".into(),
                GroupAcl {
                    team_id: "team-a".into(),
                    officer_only: false,
                },
            )],
        )
    }

    #[test]
    fn req_before_auth_is_closed_and_not_forwarded() {
        let mut session = GateSession::new("chal");
        let outcome = talk(
            &mut session,
            r#"["REQ","sub-1",{"kinds":[1,1059]}]"#,
            &FixedVerifier {
                pubkey: pk(0x11),
                fail: false,
            },
        );
        assert!(outcome.to_relay.is_empty());
        assert_eq!(outcome.to_client.len(), 1);
        let closed: Value = serde_json::from_str(&outcome.to_client[0]).unwrap();
        assert_eq!(closed[0], "CLOSED");
        assert_eq!(closed[1], "sub-1");
        assert_eq!(closed[2], AUTH_REQUIRED);
        assert!(session.authed_pubkey().is_none());
    }

    #[test]
    fn quoted_subscription_id_stays_valid_json() {
        let mut session = GateSession::new("chal");
        let outcome = talk(
            &mut session,
            r#"["REQ","a\"b",{}]"#,
            &FixedVerifier {
                pubkey: pk(0x11),
                fail: false,
            },
        );
        let closed: Value = serde_json::from_str(&outcome.to_client[0]).unwrap();
        assert_eq!(closed[1], "a\"b");
    }

    #[test]
    fn failed_auth_does_not_open_req() {
        let mut session = GateSession::new("chal");
        let auth = r#"["AUTH",{"id":"evt1","pubkey":"aa","kind":22242}]"#;
        let outcome = talk(
            &mut session,
            auth,
            &FixedVerifier {
                pubkey: pk(0x11),
                fail: true,
            },
        );
        assert!(outcome.to_relay.is_empty());
        let ok: Value = serde_json::from_str(&outcome.to_client[0]).unwrap();
        assert_eq!(ok[0], "OK");
        assert_eq!(ok[1], "evt1");
        assert_eq!(ok[2], false);
        assert!(session.authed_pubkey().is_none());

        let req = talk(
            &mut session,
            r#"["REQ","s",{}]"#,
            &FixedVerifier {
                pubkey: pk(0x11),
                fail: true,
            },
        );
        assert!(req.to_relay.is_empty());
        assert!(req.to_client[0].contains(AUTH_REQUIRED));
    }

    #[test]
    fn auth_then_req_is_forwarded_but_foreign_events_are_dropped() {
        let mut session = GateSession::new("chal");
        let verifier = FixedVerifier {
            pubkey: pk(0x11),
            fail: false,
        };
        let auth = talk(&mut session, r#"["AUTH",{"id":"evt1"}]"#, &verifier);
        assert!(auth.to_relay.is_empty());
        let ok: Value = serde_json::from_str(&auth.to_client[0]).unwrap();
        assert_eq!(ok[2], true);
        assert_eq!(session.authed_pubkey(), Some(pk(0x11).as_str()));

        let req = talk(&mut session, r#"["REQ","s",{"kinds":[1059,9]}]"#, &verifier);
        assert_eq!(
            req.to_relay,
            vec![r#"["REQ","s",{"kinds":[1059,9]}]"#.to_string()]
        );
        assert!(req.to_client.is_empty());

        let dir = dir_with_member();
        let mine = serde_json::json!([
            "EVENT",
            "s",
            {"id":"1","pubkey":"ee","kind":GIFT_WRAP_KIND,"tags":[["p", pk(0x11)]],"content":"x","sig":"s","created_at":1}
        ]);
        assert!(session.on_relay_text(&mine.to_string(), &dir).is_some());

        let theirs = serde_json::json!([
            "EVENT",
            "s",
            {"id":"2","pubkey":"ee","kind":GIFT_WRAP_KIND,"tags":[["p", pk(0x22)]],"content":"x","sig":"s","created_at":1}
        ]);
        assert!(session.on_relay_text(&theirs.to_string(), &dir).is_none());

        let other_team = serde_json::json!([
            "EVENT",
            "s",
            {"id":"3","pubkey":"ee","kind":9,"tags":[["h","team-b"]],"content":"secret","sig":"s","created_at":1}
        ]);
        assert!(session
            .on_relay_text(&other_team.to_string(), &dir)
            .is_none());

        let own_team = serde_json::json!([
            "EVENT",
            "s",
            {"id":"4","pubkey":"ee","kind":9,"tags":[["h","team-a"]],"content":"hi","sig":"s","created_at":1}
        ]);
        let forwarded = session.on_relay_text(&own_team.to_string(), &dir).unwrap();
        assert!(forwarded.contains("team-a"));
    }

    #[test]
    fn signed_out_downstream_event_is_dropped() {
        let session = GateSession::new("chal");
        let dir = dir_with_member();
        let frame = serde_json::json!([
            "EVENT",
            "s",
            {"id":"1","pubkey":"ee","kind":1,"tags":[],"content":"leak","sig":"s","created_at":1}
        ]);
        assert!(session.on_relay_text(&frame.to_string(), &dir).is_none());
    }

    #[test]
    fn count_and_negentropy_are_not_forwarded_after_auth() {
        let mut session = GateSession::new("chal");
        let verifier = FixedVerifier {
            pubkey: pk(0x11),
            fail: false,
        };
        talk(&mut session, r#"["AUTH",{"id":"evt1"}]"#, &verifier);
        for (msg, reason) in [
            (r#"["COUNT","c",{"kinds":[1059]}]"#, COUNT_DISABLED),
            (r#"["NEG-OPEN","n",{}]"#, NEG_DISABLED),
            (r#"["NEG-MSG","n","abcd"]"#, NEG_DISABLED),
        ] {
            let outcome = talk(&mut session, msg, &verifier);
            assert!(outcome.to_relay.is_empty(), "{msg}");
            let closed: Value = serde_json::from_str(&outcome.to_client[0]).unwrap();
            assert_eq!(closed[2], reason);
        }
    }

    #[test]
    fn kind4_publish_is_rejected_and_not_forwarded() {
        let mut session = GateSession::new("chal");
        let verifier = FixedVerifier {
            pubkey: pk(0x11),
            fail: false,
        };
        talk(&mut session, r#"["AUTH",{"id":"evt1"}]"#, &verifier);
        let text = r#"["EVENT",{"id":"plain","kind":4,"tags":[["p","aa"]],"content":"secret"}]"#;
        let outcome = talk(&mut session, text, &verifier);
        assert!(outcome.to_relay.is_empty());
        let reply: Value = serde_json::from_str(&outcome.to_client[0]).unwrap();
        assert_eq!(reply[0], "OK");
        assert_eq!(reply[1], "plain");
        assert_eq!(reply[2], false);
        assert!(reply[3].as_str().unwrap().contains("kind 4"));
    }

    #[test]
    fn publish_without_auth_is_refused() {
        let mut session = GateSession::new("chal");
        for text in [
            r#"["EVENT",{"id":"e9","kind":9}]"#,
            r#"["EVENT",{"id":"ew","kind":1059,"tags":[["p","aa"]]}]"#,
        ] {
            let outcome = talk(
                &mut session,
                text,
                &FixedVerifier {
                    pubkey: pk(0x11),
                    fail: true,
                },
            );
            assert!(outcome.to_relay.is_empty(), "{text}");
            let reply: Value = serde_json::from_str(&outcome.to_client[0]).unwrap();
            assert_eq!(reply[2], false);
            assert!(reply[3].as_str().unwrap().contains("auth-required:"));
        }
    }

    #[test]
    fn authed_member_publish_is_forwarded() {
        let mut session = GateSession::new("chal");
        let verifier = FixedVerifier {
            pubkey: pk(0x11),
            fail: false,
        };
        talk(&mut session, r#"["AUTH",{"id":"evt1"}]"#, &verifier);
        let text = r#"["EVENT",{"id":"e","kind":9}]"#;
        let outcome = talk(&mut session, text, &verifier);
        assert_eq!(outcome.to_relay, vec![text.to_string()]);
        assert!(outcome.to_client.is_empty());
    }

    #[test]
    fn authed_non_member_publish_is_refused() {
        let mut session = GateSession::new("chal");
        let verifier = FixedVerifier {
            pubkey: pk(0x22),
            fail: false,
        };
        talk(&mut session, r#"["AUTH",{"id":"evt1"}]"#, &verifier);
        let outcome = talk(&mut session, r#"["EVENT",{"id":"e","kind":9}]"#, &verifier);
        assert!(outcome.to_relay.is_empty());
        let reply: Value = serde_json::from_str(&outcome.to_client[0]).unwrap();
        assert_eq!(reply[2], false);
        assert!(reply[3].as_str().unwrap().contains("auth-required:"));
    }

    #[test]
    fn gift_wrap_budget_is_per_authed_sender_and_recipient() {
        let mut session = GateSession::new("chal");
        let verifier = FixedVerifier {
            pubkey: pk(0x11),
            fail: false,
        };
        talk(&mut session, r#"["AUTH",{"id":"evt1"}]"#, &verifier);
        let gifts = GiftBudget::new(1, 60);
        let frame = |id: &str, recipient: &str| {
            format!(r#"["EVENT",{{"id":"{id}","kind":1059,"tags":[["p","{recipient}"]]}}]"#)
        };
        let first = session.on_client_text(
            &frame("w1", &pk(0x22)),
            &verifier,
            &dir_with_member(),
            &gifts,
        );
        assert_eq!(first.to_relay.len(), 1);
        let second = session.on_client_text(
            &frame("w2", &pk(0x22)),
            &verifier,
            &dir_with_member(),
            &gifts,
        );
        assert!(second.to_relay.is_empty());
        let reply: Value = serde_json::from_str(&second.to_client[0]).unwrap();
        assert!(reply[3].as_str().unwrap().contains("rate-limited"));

        let other_recipient = session.on_client_text(
            &frame("w3", &pk(0x33)),
            &verifier,
            &dir_with_member(),
            &gifts,
        );
        assert_eq!(other_recipient.to_relay.len(), 1);
    }

    #[test]
    fn eose_and_ok_are_forwarded_and_upstream_auth_is_not() {
        let session = GateSession::new("chal");
        let dir = ReadDirectory::default();
        assert!(session.on_relay_text(r#"["EOSE","s"]"#, &dir).is_some());
        assert!(session
            .on_relay_text(r#"["OK","e",true,""]"#, &dir)
            .is_some());
        assert!(session
            .on_relay_text(r#"["AUTH","strfry-chal"]"#, &dir)
            .is_none());
    }
}
