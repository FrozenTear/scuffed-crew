//! Persistent server-side subscription that delivers NIP-44 DMs in real time.
//!
//! The subscriber opens a single shared WebSocket subscription to the configured
//! Nostr relay with a filter on `kind = 1059` gift-wrap events p-tagged at every
//! server-managed member. As gift wraps arrive, it routes each one to the right
//! recipient by pubkey, unwraps via [`scuffed_chat::EncryptionService`], dedups
//! on the gift-wrap event id, persists into `dm_message`, and broadcasts a
//! lightweight [`DmEvent`] so SSE/WebSocket handlers can push notifications to
//! the active session.
//!
//! Replaces the polling-based `POST /api/nostr/dm/sync` (Phase 5 v1) for
//! real-time delivery on [THE-878].

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use scuffed_auth::crypto::{CryptoService, EncryptedBlob};
use scuffed_chat::EncryptionService;
use scuffed_chat::nostr::relay::RelayClient;
use scuffed_db::Database;
use scuffed_types::nostr::{NostrFilter, RelayMessage};

/// Broadcast payload for newly stored DMs. SSE/WebSocket handlers fan this out
/// to the recipient's open client sessions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DmEvent {
    pub recipient_pubkey: String,
    pub sender_pubkey: String,
    pub dm_message_id: String,
    pub gift_wrap_id: String,
    pub created_at_unix: i64,
}

/// Sender side of the in-process DM event bus.
///
/// Cloned into [`AppState`](crate::state::AppState) so SSE handlers can
/// `subscribe()` and stream events to authenticated clients.
pub type DmEventBus = broadcast::Sender<DmEvent>;

/// How often to reload the server-managed member set so newly-onboarded members
/// start receiving real-time DMs without a server restart.
const MEMBER_REFRESH_INTERVAL: Duration = Duration::from_secs(300);

/// Reconnect backoff cap.
const RECONNECT_BACKOFF_MAX: Duration = Duration::from_secs(30);

/// Broadcast channel capacity. Lagged subscribers drop oldest events; the
/// frontend treats every event as a hint to refetch, so drops are recoverable.
const DM_EVENT_BUFFER: usize = 256;

/// Spawn the persistent DM relay subscriber task.
///
/// Returns the broadcast sender for the in-process event bus, or `None` when
/// preconditions are missing (no `NOSTR_RELAY_URL` or no `ENCRYPTION_KEY`).
/// In that case real-time delivery is disabled and clients must continue to
/// poll `/api/nostr/dm/sync`.
pub fn start(
    db: Arc<Database>,
    crypto: Option<Arc<CryptoService>>,
    relay_url: Option<String>,
) -> Option<DmEventBus> {
    let relay_url = relay_url?;
    let crypto = crypto?;
    let (tx, _) = broadcast::channel(DM_EVENT_BUFFER);
    let task_tx = tx.clone();

    tokio::spawn(async move {
        run_subscriber_loop(db, crypto, relay_url, task_tx).await;
    });

    tracing::info!("DM relay subscriber started");
    Some(tx)
}

async fn run_subscriber_loop(
    db: Arc<Database>,
    crypto: Arc<CryptoService>,
    relay_url: String,
    tx: broadcast::Sender<DmEvent>,
) {
    let encryption = EncryptionService::new((*crypto).clone());
    let mut backoff = Duration::from_secs(1);

    loop {
        match run_subscription_session(&db, &encryption, &relay_url, &tx).await {
            Ok(reason) => {
                tracing::info!("DM subscription session ended: {reason}; reconnecting");
                backoff = Duration::from_secs(1);
            }
            Err(e) => {
                tracing::warn!(
                    "DM subscription session failed: {e}; reconnecting after {:?}",
                    backoff
                );
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(RECONNECT_BACKOFF_MAX);
            }
        }
    }
}

/// Run one connection lifetime. Returns `Ok(reason)` for clean exits that
/// should reconnect immediately (e.g. member set changed) and `Err(_)` for
/// errors that warrant backoff.
async fn run_subscription_session(
    db: &Arc<Database>,
    encryption: &EncryptionService,
    relay_url: &str,
    tx: &broadcast::Sender<DmEvent>,
) -> Result<String, String> {
    let mut current_index = build_recipient_index(db)
        .await
        .map_err(|e| format!("failed to load member set: {e}"))?;

    if current_index.is_empty() {
        tokio::time::sleep(MEMBER_REFRESH_INTERVAL).await;
        return Ok("no server-managed members".into());
    }

    let mut tags: HashMap<String, Vec<String>> = HashMap::new();
    tags.insert("#p".to_string(), current_index.keys().cloned().collect());
    let filter = NostrFilter {
        kinds: Some(vec![scuffed_types::nostr::event_kinds::GIFT_WRAP]),
        tags,
        ..Default::default()
    };

    let client = RelayClient::new(relay_url);
    let mut rx = client
        .connect()
        .await
        .map_err(|e| format!("relay connect: {e}"))?;
    client
        .subscribe("dm-feed", vec![filter])
        .await
        .map_err(|e| format!("relay subscribe: {e}"))?;

    tracing::info!(
        "DM subscriber listening for {} server-managed pubkey(s)",
        current_index.len()
    );

    let mut refresh = tokio::time::interval(MEMBER_REFRESH_INTERVAL);
    refresh.tick().await; // skip immediate tick

    loop {
        tokio::select! {
            msg = rx.recv() => {
                let Some(msg) = msg else {
                    return Err("relay receiver closed".into());
                };
                if let RelayMessage::Event { event, .. } = msg {
                    handle_gift_wrap(db, encryption, &current_index, &event, tx).await;
                }
            }
            _ = refresh.tick() => {
                match build_recipient_index(db).await {
                    Ok(updated) if member_set_changed(&current_index, &updated) => {
                        client.disconnect().await;
                        return Ok(format!(
                            "member set changed ({} -> {})",
                            current_index.len(),
                            updated.len()
                        ));
                    }
                    Ok(updated) => {
                        current_index = updated;
                    }
                    Err(e) => {
                        tracing::warn!("Failed to refresh members: {e}");
                    }
                }
            }
        }
    }
}

async fn build_recipient_index(
    db: &Arc<Database>,
) -> scuffed_db::DbResult<HashMap<String, EncryptedBlob>> {
    // Dedicated query loads only the secret material needed for unwrap —
    // not via list_members (which intentionally omits secrets).
    let pairs = db.list_server_managed_nostr_secrets().await?;
    Ok(pairs.into_iter().collect())
}

fn member_set_changed(
    current: &HashMap<String, EncryptedBlob>,
    updated: &HashMap<String, EncryptedBlob>,
) -> bool {
    if current.len() != updated.len() {
        return true;
    }
    !current.keys().all(|k| updated.contains_key(k))
}

async fn handle_gift_wrap(
    db: &Arc<Database>,
    encryption: &EncryptionService,
    recipient_index: &HashMap<String, EncryptedBlob>,
    event: &scuffed_types::nostr::NostrEvent,
    tx: &broadcast::Sender<DmEvent>,
) {
    let recipient_pubkey = event
        .tags
        .iter()
        .filter(|t| t.first().map(String::as_str) == Some("p"))
        .filter_map(|t| t.get(1))
        .find(|pk| recipient_index.contains_key(*pk))
        .cloned();

    let Some(recipient_pubkey) = recipient_pubkey else {
        tracing::debug!(
            "Gift wrap {} not p-tagged at any server-managed member; skipping",
            event.id
        );
        return;
    };
    let blob = match recipient_index.get(&recipient_pubkey) {
        Some(b) => b,
        None => return,
    };

    let event_json = match serde_json::to_string(event) {
        Ok(j) => j,
        Err(e) => {
            tracing::warn!("Skipping unserializable relay event: {e}");
            return;
        }
    };

    let unwrapped = match encryption
        .unwrap_gift_wrap_json(blob, &recipient_pubkey, &event_json)
        .await
    {
        Ok(u) => u,
        Err(e) => {
            tracing::debug!("Skipping unwrap failure for {}: {e}", event.id);
            return;
        }
    };

    if unwrapped.kind != scuffed_types::nostr::event_kinds::PRIVATE_DIRECT_MESSAGE {
        return;
    }

    let reply_to = unwrapped.tags.iter().find_map(|tag| {
        if tag.first().map(String::as_str) == Some("e") {
            tag.get(1).cloned()
        } else {
            None
        }
    });
    let created_at =
        chrono::DateTime::<chrono::Utc>::from_timestamp(unwrapped.created_at as i64, 0)
            .unwrap_or_else(chrono::Utc::now);

    match db
        .insert_dm_message(
            &event.id,
            &unwrapped.sender_pubkey,
            &recipient_pubkey,
            &unwrapped.content,
            reply_to.as_deref(),
            created_at,
        )
        .await
    {
        Ok((stored, true)) => {
            let _ = tx.send(DmEvent {
                recipient_pubkey,
                sender_pubkey: unwrapped.sender_pubkey,
                dm_message_id: stored.id,
                gift_wrap_id: event.id.clone(),
                created_at_unix: created_at.timestamp(),
            });
        }
        Ok((_, false)) => {
            // Already stored (sender's own send path or a prior sync).
        }
        Err(e) => {
            tracing::warn!("Failed to store DM {}: {e}", event.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn member_set_changed_detects_added_pubkey() {
        let mut a = HashMap::new();
        a.insert("pk1".to_string(), dummy_blob());
        let mut b = a.clone();
        b.insert("pk2".to_string(), dummy_blob());
        assert!(member_set_changed(&a, &b));
    }

    #[test]
    fn member_set_changed_detects_removed_pubkey() {
        let mut a = HashMap::new();
        a.insert("pk1".to_string(), dummy_blob());
        a.insert("pk2".to_string(), dummy_blob());
        let mut b = HashMap::new();
        b.insert("pk1".to_string(), dummy_blob());
        assert!(member_set_changed(&a, &b));
    }

    #[test]
    fn member_set_changed_ignores_blob_changes() {
        let mut a = HashMap::new();
        a.insert("pk1".to_string(), dummy_blob());
        let b = a.clone();
        assert!(!member_set_changed(&a, &b));
    }

    fn dummy_blob() -> EncryptedBlob {
        EncryptedBlob {
            ciphertext: String::new(),
            nonce: String::new(),
            key_version: 1,
        }
    }
}
