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
//! - `SURREALDB_RELAY_USER` Relay record user (default: `scuffed_relay`)
//! - `SURREALDB_RELAY_PASSWORD` Password for that user. When it is unset, the
//!   process falls back to `SURREALDB_USER` / `SURREALDB_PASSWORD` and logs a
//!   warning. `SURREALDB_PASS` is a fallback alias for the password.
//!   When `PRODUCTION` is set, a missing password, username `root`, or a
//!   loopback URL refuses startup. There is no root sign-in in production.
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

    // Refuse production boot on a missing password, username root, or a
    // loopback URL before any default credential is applied.
    let db_cfg = creds::enforce_or_exit();

    let db: Option<Arc<scuffed_db::Database>> = rt.block_on(async {
        match creds::connect_configured(&db_cfg).await {
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
                tracing::info!(
                    url = %db_cfg.url,
                    user = %db_cfg.username,
                    auth = ?db_cfg.auth,
                    "connected to SurrealDB"
                );
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

    // Gift-wrap buckets (`gift:{sender}:{recipient}`) are trimmed inside
    // evaluate only when that same pair sends again. Sweep them on a timer
    // so a quiet pair does not keep a map entry. This runs even when
    // SurrealDB is down and the writer reload never fires.
    let engine_sweep = Arc::clone(&engine);
    rt.spawn(async move {
        let mut interval = tokio::time::interval(policy::RATE_BUCKET_SWEEP);
        interval.tick().await;
        loop {
            interval.tick().await;
            engine_sweep
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .prune_rate_buckets();
        }
    });

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

        // `new` and `lookback` both go through the write policy. strfry sends
        // `lookback` when it replays stored events (import or sync). Negentropy
        // is off in the shipped config. The same allow-list, kind check,
        // officer writer map, and gift-wrap recipient check still apply.
        let step = policy_step(&engine, &input);
        if step.reload {
            if let Some(tx) = &reload_tx {
                let _ = tx.send(());
            }
        }

        let json = serde_json::to_string(&step.output).expect("serialize output");
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

struct PolicyStep {
    output: StrfryOutput,
    reload: bool,
}

/// Write-policy decision for one strfry line. `lookback` is not a bypass.
fn policy_step(engine: &Mutex<PolicyEngine>, input: &StrfryInput) -> PolicyStep {
    let pubkey = input.event.pubkey.to_lowercase();
    let kind = input.event.kind;
    let event_info = EventInfo {
        id: input.event.id.clone(),
        pubkey: pubkey.clone(),
        kind,
        tags: input.event.tags.clone(),
        authed: input.authed.clone(),
    };
    let decision = engine
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .evaluate(&event_info);
    let reload =
        matches!(&decision, Decision::Reject(reason) if policy::reload_on_write_miss(reason));
    let output = match decision {
        Decision::Accept => StrfryOutput {
            id: input.event.id.clone(),
            action: "accept".into(),
            msg: String::new(),
        },
        Decision::Reject(reason) => {
            tracing::debug!(
                event_id = %input.event.id,
                msg_type = %input.msg_type,
                pubkey = %pubkey,
                kind,
                reason = %reason,
                "rejected event"
            );
            StrfryOutput {
                id: input.event.id.clone(),
                action: "reject".into(),
                msg: reason,
            }
        }
    };
    PolicyStep { output, reload }
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

    fn lookback(kind: u64, pubkey: &str, tags: &str) -> StrfryInput {
        let input = format!(
            r#"{{"type":"lookback","event":{{"id":"aa","pubkey":"{pubkey}","kind":{kind},"tags":{tags},"content":"","sig":"cc","created_at":0}}}}"#
        );
        serde_json::from_str(&input).unwrap()
    }

    #[test]
    fn lookback_unknown_pubkey_is_rejected() {
        let engine = Mutex::new(PolicyEngine::new(PolicyConfig::default()));
        let pubkey = "ff".repeat(32);
        let step = policy_step(&engine, &lookback(1, &pubkey, "[]"));
        assert_eq!(step.output.action, "reject");
        assert!(
            step.output.msg.contains("allowlist"),
            "lookback skipped the write policy: {}",
            step.output.msg
        );
        assert!(step.reload);
    }

    #[test]
    fn lookback_kind_4_is_rejected() {
        let mut engine = PolicyEngine::new(PolicyConfig::default());
        let pubkey = "aa".repeat(32);
        engine.update_allowlist(HashSet::from([pubkey.clone()]));
        let step = policy_step(&Mutex::new(engine), &lookback(4, &pubkey, "[]"));
        assert_eq!(step.output.action, "reject");
        assert!(
            step.output.msg.contains("kind 4"),
            "lookback accepted a plain DM: {}",
            step.output.msg
        );
    }

    #[test]
    fn lookback_allowlisted_event_is_accepted() {
        let mut engine = PolicyEngine::new(PolicyConfig::default());
        let pubkey = "aa".repeat(32);
        engine.update_allowlist(HashSet::from([pubkey.clone()]));
        let step = policy_step(&Mutex::new(engine), &lookback(1, &pubkey, "[]"));
        assert_eq!(step.output.action, "accept");
        assert!(step.output.msg.is_empty());
        assert!(!step.reload);
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
