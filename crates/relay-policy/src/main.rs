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
//! - `SURREALDB_URL` SurrealDB connection URL (default outside production: `ws://127.0.0.1:8000`)
//! - `SURREALDB_USER` SurrealDB username (default: `root`)
//! - `SURREALDB_PASSWORD` SurrealDB password. `SURREALDB_PASS` is a fallback alias.
//!   When `PRODUCTION` is set, a missing password, root/root, or a loopback URL
//!   refuses startup. There is no root/root fallback in production.
//! - `SURREALDB_NS` / `SURREALDB_DB` namespace and database (default `scuffed_crew` / `main`)
//! - `RELAY_POLICY_RATE_LIMIT` Max events per pubkey per window (default: 30)
//! - `RELAY_POLICY_RATE_WINDOW` Rate limit window in seconds (default: 60)
//! - `RELAY_POLICY_REFRESH_SECS` Allowlist refresh interval (default: 60)
//! - `RELAY_POLICY_ENFORCE_GROUPS` Default true. An `h` tag must name an active
//!   team channel, and the author must be on that team's roster. Set to "false"
//!   to allow any allowlisted pubkey to write any `h` group. Kind 1059 and
//!   events with no `h` tag are not group-checked.
//! - `RUST_LOG` Tracing filter (default: `relay_policy=info`)

mod policy;

use relay_policy::creds;

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
    /// NIP-42 pubkey when this connection to strfry completed AUTH.
    /// Absent for the read gate and for site-server one-shot publishes.
    #[serde(default)]
    authed: Option<String>,
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
    /// Event content. We never inspect this for encrypted events.
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

async fn refresh_writers(db: &scuffed_db::Database, engine: &Mutex<PolicyEngine>) {
    let allowlist = load_pubkey_allowlist(db).await;
    let groups = match relay_policy::directory::load_group_writers(db).await {
        Ok(groups) => Some(groups),
        Err(err) => {
            tracing::error!(
                error = %err,
                "group writer refresh failed; keeping the previous map"
            );
            None
        }
    };
    let mut eng = engine.lock().unwrap_or_else(|err| err.into_inner());
    eng.update_allowlist(allowlist);
    if let Some(groups) = groups {
        let count = groups.len();
        eng.update_group_members(groups);
        tracing::info!(
            allowlist_size = eng.allowlist_size(),
            groups = count,
            "refreshed writer list"
        );
    }
    eng.prune_rate_buckets();
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
            tracing::info!(
                count = pubkeys.len(),
                "loaded pubkey allowlist from database"
            );
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
        enforce_group_membership: !env_or("RELAY_POLICY_ENFORCE_GROUPS", "true")
            .eq_ignore_ascii_case("false"),
        ..PolicyConfig::default()
    };

    let refresh_secs: u64 = env_parse("RELAY_POLICY_REFRESH_SECS", 60);

    let engine = Arc::new(Mutex::new(PolicyEngine::new(config)));

    // Start the tokio runtime for async DB operations.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    // Refuse production boot on missing password, root/root, or a loopback URL
    // before any default credential is applied.
    let db_cfg = creds::enforce_or_exit();
    let surreal_cfg = creds::namespace_from_env();

    let db: Option<Arc<scuffed_db::Database>> = rt.block_on(async {
        match scuffed_db::Database::connect_plaintext(
            &db_cfg.url,
            &db_cfg.username,
            &db_cfg.password,
            surreal_cfg,
        )
        .await
        {
            Ok(database) => {
                let db = Arc::new(database);
                let allowlist = load_pubkey_allowlist(&db).await;
                let groups = match relay_policy::directory::load_group_writers(&db).await {
                    Ok(groups) => {
                        tracing::info!(
                            groups = groups.len(),
                            "loaded group writers from team channels"
                        );
                        groups
                    }
                    Err(err) => {
                        tracing::error!(
                            error = %err,
                            "failed to load group writers; rejecting every h-tagged event"
                        );
                        std::collections::HashMap::new()
                    }
                };
                let mut eng = engine.lock().unwrap();
                eng.update_allowlist(allowlist);
                eng.update_group_members(groups);
                drop(eng);
                tracing::info!(url = %db_cfg.url, "connected to SurrealDB");
                Some(db)
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "failed to connect to SurrealDB; running without allowlist (rejecting all)"
                );
                None
            }
        }
    });

    // Reload writers when roster edges, team channels, or member rows change.
    // LIVE is the primary signal. A timed tick and a rejected write for an
    // unknown author or group are the backup. All three share one debounced
    // reload so a burst cannot hammer SurrealDB.
    let reload_tx = if let Some(db) = db.clone() {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let engine_bg = Arc::clone(&engine);
        let db_reload = Arc::clone(&db);
        rt.spawn(async move {
            relay_policy::directory::coalesce_reloads(rx, || {
                let db = Arc::clone(&db_reload);
                let engine = Arc::clone(&engine_bg);
                async move { refresh_writers(&db, &engine).await }
            })
            .await;
        });
        let tx_live = tx.clone();
        let db_live = Arc::clone(&db);
        rt.spawn(async move {
            relay_policy::directory::watch_writer_tables(db_live, tx_live).await;
        });
        let tx_tick = tx.clone();
        rt.spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(refresh_secs));
            interval.tick().await;
            loop {
                interval.tick().await;
                let _ = tx_tick.send(());
            }
        });
        Some(tx)
    } else {
        None
    };

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

        // Skip lookback events. Only evaluate new incoming events.
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
            authed: input.authed.clone(),
        };

        let decision = engine.lock().unwrap().evaluate(&event_info);
        if let Decision::Reject(reason) = &decision {
            if policy::reload_on_write_miss(reason) {
                if let Some(tx) = &reload_tx {
                    let _ = tx.send(());
                }
            }
        }

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
        if events_processed.is_multiple_of(1000) {
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
        engine.update_allowlist(allowlist.clone());
        let mut groups = std::collections::HashMap::new();
        groups.insert("team".to_string(), allowlist);
        engine.update_group_members(groups);

        let input_json = format!(
            r#"{{"type":"new","event":{{"id":"eventid","pubkey":"{pubkey}","kind":9,"tags":[["h","team"]],"content":"hello","sig":"sig","created_at":0}}}}"#
        );
        let parsed: StrfryInput = serde_json::from_str(&input_json).unwrap();
        let event_info = EventInfo {
            id: parsed.event.id.clone(),
            pubkey: parsed.event.pubkey.to_lowercase(),
            kind: parsed.event.kind,
            tags: parsed.event.tags,
            authed: parsed.authed.clone(),
        };
        let decision = engine.evaluate(&event_info);
        assert_eq!(decision, Decision::Accept);
    }

    #[test]
    fn end_to_end_reject_unknown_pubkey() {
        let config = PolicyConfig::default();
        let mut engine = PolicyEngine::new(config);
        // Empty allowlist. All pubkeys should be rejected.
        engine.update_allowlist(HashSet::new());

        let input_json = r#"{"type":"new","event":{"id":"eventid","pubkey":"ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00","kind":9,"tags":[],"content":"","sig":"sig","created_at":0}}"#;
        let parsed: StrfryInput = serde_json::from_str(input_json).unwrap();
        let event_info = EventInfo {
            id: parsed.event.id.clone(),
            pubkey: parsed.event.pubkey.to_lowercase(),
            kind: parsed.event.kind,
            tags: parsed.event.tags,
            authed: parsed.authed.clone(),
        };
        let decision = engine.evaluate(&event_info);
        assert!(matches!(decision, Decision::Reject(_)));
    }
}
