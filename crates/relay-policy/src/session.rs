//! Per-connection Nostr session for the read gate.
//!
//! `REQ` is not forwarded until NIP-42 AUTH succeeds. After that, `EVENT`
//! frames from strfry are still filtered by [`crate::read_policy::allow_event`].
//! `COUNT` and negentropy are not forwarded: a count or a sync would leak
//! events the member is not allowed to read. Client `EVENT` publishes are
//! forwarded so the write-policy plugin can accept or reject them. The site
//! server publishes without AUTH on the compose-network strfry port, which
//! does not pass through this gate.

use serde_json::{json, Value};

use crate::read_policy::{allow_event, EventView, ReadDirectory};

pub const AUTH_REQUIRED: &str = "auth-required: NIP-42 AUTH required before REQ";
pub const COUNT_DISABLED: &str = "restricted: COUNT is disabled";
pub const NEG_DISABLED: &str = "restricted: negentropy sync is disabled";

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
            "EVENT" => ClientOutcome {
                to_client: vec![],
                to_relay: vec![text.to_string()],
            },
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
        let outcome = session.on_client_text(
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
        let outcome = session.on_client_text(
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
        let outcome = session.on_client_text(
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

        let req = session.on_client_text(
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
        let auth = session.on_client_text(r#"["AUTH",{"id":"evt1"}]"#, &verifier);
        assert!(auth.to_relay.is_empty());
        let ok: Value = serde_json::from_str(&auth.to_client[0]).unwrap();
        assert_eq!(ok[2], true);
        assert_eq!(session.authed_pubkey(), Some(pk(0x11).as_str()));

        let req = session.on_client_text(r#"["REQ","s",{"kinds":[1059,9]}]"#, &verifier);
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
        session.on_client_text(r#"["AUTH",{"id":"evt1"}]"#, &verifier);
        for (msg, reason) in [
            (r#"["COUNT","c",{"kinds":[1059]}]"#, COUNT_DISABLED),
            (r#"["NEG-OPEN","n",{}]"#, NEG_DISABLED),
            (r#"["NEG-MSG","n","abcd"]"#, NEG_DISABLED),
        ] {
            let outcome = session.on_client_text(msg, &verifier);
            assert!(outcome.to_relay.is_empty(), "{msg}");
            let closed: Value = serde_json::from_str(&outcome.to_client[0]).unwrap();
            assert_eq!(closed[2], reason);
        }
    }

    #[test]
    fn publish_is_forwarded_without_auth() {
        let mut session = GateSession::new("chal");
        let text = r#"["EVENT",{"id":"e","kind":9}]"#;
        let outcome = session.on_client_text(
            text,
            &FixedVerifier {
                pubkey: pk(0x11),
                fail: true,
            },
        );
        assert_eq!(outcome.to_relay, vec![text.to_string()]);
        assert!(outcome.to_client.is_empty());
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
