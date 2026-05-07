//! strfry write-policy plugin for Scuffed Crew relay.
//!
//! Long-running binary that strfry spawns as a write-policy plugin.
//! Reads Nostr events from stdin (one JSON object per line), evaluates policy,
//! and writes accept/reject decisions to stdout.
//!
//! Connects to SurrealDB to load the member pubkey allowlist and refreshes
//! it periodically in the background.
//!
//! ## Environment Variables
//!
//! - `SURREALDB_URL` — SurrealDB connection URL (default: `ws://127.0.0.1:8000`)
//! - `SURREALDB_USER` — SurrealDB username (default: `root`)
//! - `SURREALDB_PASS` — SurrealDB password (default: `root`)
//! - `RELAY_POLICY_RATE_LIMIT` — Max events per pubkey per window (default: 30)
//! - `RELAY_POLICY_RATE_WINDOW` — Rate limit window in seconds (default: 60)
//! - `RELAY_POLICY_REFRESH_SECS` — Allowlist refresh interval (default: 60)
//! - `RELAY_POLICY_ENFORCE_GROUPS` — Set to "true" to enforce NIP-29 group membership
//! - `RUST_LOG` — Tracing filter (default: `relay_policy=info`)

mod policy;

use policy::{Decision, EventInfo, PolicyConfig, PolicyEngine};

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::{self, BufRead, Write};
use std::sync::{Arc, Mutex};

/// Incoming message from strfry's write-policy pipeline.
///
/// strfry sends the full event object with additional metadata.
/// We only deserialize the fields we need for policy decisions.
#[derive(Deserialize)]
struct StrfryInput {
    /// The type of policy check: "new" for incoming events, "lookback" for backfilled.
    #[serde(rename = "type")]
    msg_type: String,
    /// The Nostr event to evaluate.
    event: StrfryEvent,
    /// Unix timestamp when strfry received the event.
    #[serde(rename = "receivedAt")]
    #[allow(dead_code)]
    received_at: Option<u64>,
    /// Source type: "IP4", "IP6", "Import", "Stream", "Sync".
    #[serde(rename = "sourceType")]
    #[allow(dead_code)]
    source_type: Option<String>,
    /// Source info (e.g., IP address).
    #[serde(rename = "sourceInfo")]
    #[allow(dead_code)]
    source_info: Option<String>,
}

/// A Nostr event as sent by strfry.
#[derive(Deserialize)]
struct StrfryEvent {
    /// Event ID (64-char hex).
    id: String,
    /// Author pubkey (64-char hex).
    pubkey: String,
    /// Event kind number.
    kind: u64,
    /// Event tags (array of string arrays).
    #[serde(default)]
    tags: Vec<Vec<String>>,
    /// Event content — we never inspect this for encrypted events.
    #[allow(dead_code)]
    content: Option<String>,
}

/// Policy decision sent back to strfry.
#[derive(Serialize)]
struct StrfryOutput {
    /// Must echo the input event ID.
    id: String,
    /// "accept", "reject", or "shadowReject".
    action: String,
    /// Human-readable reason (strfry uses this in NIP-20 OK messages for rejections).
    msg: String,
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn env_parse<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Load the set of active member hex pubkeys from SurrealDB.
async fn load_pubkey_allowlist(db: &scuffed_db::Database) -> HashSet<String> {
    match db.list_nostr_identities().await {
        Ok(members) => {
            let pubkeys: HashSet<String> = members
                .into_iter()
                .filter_map(|m| m.nostr_pubkey)
                .filter(|pk| pk.len() == 64 && pk.chars().all(|c| c.is_ascii_hexdigit()))
                .map(|pk| pk.to_lowercase())
                .collect();
            tracing::info!(count = pubkeys.len(), "loaded pubkey allowlist from database");
            pubkeys
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to load pubkey allowlist from database");
            HashSet::new()
        }
    }
}

fn main() {
    // Initialize tracing (logs go to stderr so they don't interfere with stdout protocol).
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "relay_policy=info".parse().unwrap()),
        )
        .with_writer(io::stderr)
        .init();

    tracing::info!("relay-policy starting");

    // Build policy configuration from environment.
    let config = PolicyConfig {
        rate_limit_events: env_parse("RELAY_POLICY_RATE_LIMIT", 30),
        rate_limit_window_secs: env_parse("RELAY_POLICY_RATE_WINDOW", 60),
        enforce_group_membership: env_or("RELAY_POLICY_ENFORCE_GROUPS", "false") == "true",
        ..PolicyConfig::default()
    };

    let refresh_secs: u64 = env_parse("RELAY_POLICY_REFRESH_SECS", 60);

    let engine = Arc::new(Mutex::new(PolicyEngine::new(config)));

    // Start the tokio runtime for async DB operations.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    // Attempt initial DB connection and allowlist load.
    let db_url = env_or("SURREALDB_URL", "ws://127.0.0.1:8000");
    let db_user = env_or("SURREALDB_USER", "root");
    let db_pass = env_or("SURREALDB_PASS", "root");

    let db: Option<Arc<scuffed_db::Database>> = rt.block_on(async {
        match scuffed_db::Database::connect(&db_url, &db_user, &db_pass, Default::default()).await {
            Ok(database) => {
                let db = Arc::new(database);
                let allowlist = load_pubkey_allowlist(&db).await;
                engine.lock().unwrap().update_allowlist(allowlist);
                tracing::info!("connected to SurrealDB at {db_url}");
                Some(db)
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to connect to SurrealDB — running without allowlist (rejecting all)");
                None
            }
        }
    });

    // Spawn background allowlist refresh task.
    if let Some(db) = db.clone() {
        let engine_bg = Arc::clone(&engine);
        rt.spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(refresh_secs));
            // Skip the first tick (we already loaded on startup).
            interval.tick().await;
            loop {
                interval.tick().await;
                let allowlist = load_pubkey_allowlist(&db).await;
                let mut eng = engine_bg.lock().unwrap();
                eng.update_allowlist(allowlist);
                eng.prune_rate_buckets();
                tracing::debug!(
                    allowlist_size = eng.allowlist_size(),
                    "refreshed allowlist and pruned rate buckets"
                );
            }
        });
    }

    // Main stdin/stdout policy loop.
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());
    let mut events_processed: u64 = 0;

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                tracing::error!(error = %e, "stdin read error");
                break;
            }
        };

        if line.is_empty() {
            continue;
        }

        let input: StrfryInput = match serde_json::from_str(&line) {
            Ok(i) => i,
            Err(e) => {
                tracing::warn!(error = %e, "failed to parse strfry input");
                continue;
            }
        };

        // Skip lookback events — only evaluate new incoming events.
        if input.msg_type == "lookback" {
            let output = StrfryOutput {
                id: input.event.id,
                action: "accept".into(),
                msg: String::new(),
            };
            let json = serde_json::to_string(&output).expect("serialize output");
            let _ = writeln!(out, "{json}");
            let _ = out.flush();
            continue;
        }

        let event_info = EventInfo {
            id: input.event.id.clone(),
            pubkey: input.event.pubkey.to_lowercase(),
            kind: input.event.kind,
            tags: input.event.tags,
        };

        let decision = engine.lock().unwrap().evaluate(&event_info);

        let output = match decision {
            Decision::Accept => StrfryOutput {
                id: input.event.id,
                action: "accept".into(),
                msg: String::new(),
            },
            Decision::Reject(reason) => {
                tracing::debug!(
                    event_id = %input.event.id,
                    pubkey = %event_info.pubkey,
                    kind = event_info.kind,
                    reason = %reason,
                    "rejected event"
                );
                StrfryOutput {
                    id: input.event.id,
                    action: "reject".into(),
                    msg: reason,
                }
            }
        };

        let json = serde_json::to_string(&output).expect("serialize output");
        if writeln!(out, "{json}").is_err() {
            break;
        }
        if out.flush().is_err() {
            break;
        }

        events_processed += 1;
        if events_processed % 1000 == 0 {
            tracing::info!(events_processed, "policy checkpoint");
        }
    }

    tracing::info!(events_processed, "relay-policy shutting down");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_strfry_new_event() {
        let input = r#"{
            "type": "new",
            "event": {
                "id": "aabb",
                "pubkey": "ccdd",
                "kind": 9,
                "tags": [["h", "team-alpha"], ["p", "ffee"]],
                "content": "hello",
                "sig": "dead",
                "created_at": 1234567890
            },
            "receivedAt": 1234567890,
            "sourceType": "IP4",
            "sourceInfo": "1.2.3.4"
        }"#;
        let parsed: StrfryInput = serde_json::from_str(input).unwrap();
        assert_eq!(parsed.msg_type, "new");
        assert_eq!(parsed.event.id, "aabb");
        assert_eq!(parsed.event.pubkey, "ccdd");
        assert_eq!(parsed.event.kind, 9);
        assert_eq!(parsed.event.tags.len(), 2);
        assert_eq!(parsed.event.tags[0], vec!["h", "team-alpha"]);
    }

    #[test]
    fn parse_strfry_lookback_event() {
        let input = r#"{"type":"lookback","event":{"id":"aa","pubkey":"bb","kind":1,"tags":[],"content":"","sig":"cc","created_at":0}}"#;
        let parsed: StrfryInput = serde_json::from_str(input).unwrap();
        assert_eq!(parsed.msg_type, "lookback");
    }

    #[test]
    fn serialize_accept_output() {
        let output = StrfryOutput {
            id: "aabb".to_string(),
            action: "accept".to_string(),
            msg: String::new(),
        };
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("\"action\":\"accept\""));
        assert!(json.contains("\"id\":\"aabb\""));
    }

    #[test]
    fn serialize_reject_output() {
        let output = StrfryOutput {
            id: "aabb".to_string(),
            action: "reject".to_string(),
            msg: "blocked: pubkey not in member allowlist".to_string(),
        };
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("\"action\":\"reject\""));
        assert!(json.contains("allowlist"));
    }

    #[test]
    fn end_to_end_accept_flow() {
        // Simulate: known pubkey, allowed kind -> accept
        let config = PolicyConfig::default();
        let mut engine = PolicyEngine::new(config);
        let pubkey = "aa".repeat(32);
        let mut allowlist = HashSet::new();
        allowlist.insert(pubkey.clone());
        engine.update_allowlist(allowlist);

        let input_json = format!(
            r#"{{"type":"new","event":{{"id":"eventid","pubkey":"{pubkey}","kind":9,"tags":[["h","team"]],"content":"hello","sig":"sig","created_at":0}}}}"#
        );
        let parsed: StrfryInput = serde_json::from_str(&input_json).unwrap();
        let event_info = EventInfo {
            id: parsed.event.id.clone(),
            pubkey: parsed.event.pubkey.to_lowercase(),
            kind: parsed.event.kind,
            tags: parsed.event.tags,
        };
        let decision = engine.evaluate(&event_info);
        assert_eq!(decision, Decision::Accept);
    }

    #[test]
    fn end_to_end_reject_unknown_pubkey() {
        let config = PolicyConfig::default();
        let mut engine = PolicyEngine::new(config);
        // Empty allowlist — all pubkeys should be rejected
        engine.update_allowlist(HashSet::new());

        let input_json = r#"{"type":"new","event":{"id":"eventid","pubkey":"ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00","kind":9,"tags":[],"content":"","sig":"sig","created_at":0}}"#;
        let parsed: StrfryInput = serde_json::from_str(input_json).unwrap();
        let event_info = EventInfo {
            id: parsed.event.id.clone(),
            pubkey: parsed.event.pubkey.to_lowercase(),
            kind: parsed.event.kind,
            tags: parsed.event.tags,
        };
        let decision = engine.evaluate(&event_info);
        assert!(matches!(decision, Decision::Reject(_)));
    }
}
