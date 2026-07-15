use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use surrealdb::types::Datetime as SurrealDatetime;
use surrealdb_types::RecordId;
use surrealdb_types::SurrealValue;

use scuffed_auth::crypto::{aad, EncryptedBlob};

use crate::types::{conversation_key, DmConversation, DmMessage, DmReadMarker};
use crate::{record_id_key_to_string, with_timeout, Database, DbResult};

/// Prefix for AES-GCM encrypted DM content stored in `dm_message.content`.
const DM_ENC_PREFIX: &str = "enc1:";

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbDmMessage {
    #[surreal(default)]
    #[allow(dead_code)]
    id: Option<RecordId>,
    gift_wrap_id: String,
    sender_pubkey: String,
    recipient_pubkey: String,
    conversation_key: String,
    content: String,
    reply_to_event_id: Option<String>,
    created_at: SurrealDatetime,
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbDmReadMarker {
    #[surreal(default)]
    #[allow(dead_code)]
    id: Option<RecordId>,
    member_id: String,
    peer_pubkey: String,
    last_read_at: SurrealDatetime,
}

/// Encrypt DM plaintext for at-rest storage when CryptoService is configured.
///
/// AAD binds ciphertext to gift-wrap id + conversation key.
/// When `PRODUCTION` is set, plaintext storage is refused (must have ENCRYPTION_KEY).
fn seal_dm_content(
    db: &Database,
    plaintext: &str,
    gift_wrap_id: &str,
    conv_key: &str,
) -> DbResult<String> {
    match &db.crypto {
        Some(crypto) => {
            let aad_s = aad::dm_content(gift_wrap_id, conv_key);
            let blob = crypto.encrypt(plaintext, &aad_s)?;
            let json = serde_json::to_string(&blob).map_err(|e| {
                crate::DbError::Config(format!("Failed to serialize DM ciphertext: {e}"))
            })?;
            Ok(format!("{DM_ENC_PREFIX}{json}"))
        }
        None if scuffed_auth::is_production_env() => Err(crate::DbError::Config(
            "Refusing plaintext DM storage when PRODUCTION is set (require ENCRYPTION_KEY)".into(),
        )),
        None => Ok(plaintext.to_string()),
    }
}

/// Decrypt DM content if sealed.
///
/// Production: plaintext rows are rejected (must be encrypted at rest).
/// Dev: legacy plaintext rows pass through for migration convenience.
/// Fail-closed: ciphertext that cannot be opened returns an error (never empty string).
fn open_dm_content(
    db: &Database,
    stored: &str,
    gift_wrap_id: &str,
    conv_key: &str,
) -> DbResult<String> {
    let Some(rest) = stored.strip_prefix(DM_ENC_PREFIX) else {
        if scuffed_auth::is_production_env() {
            return Err(crate::DbError::Config(
                "Refusing plaintext DM content when PRODUCTION is set (data must be encrypted)"
                    .into(),
            ));
        }
        return Ok(stored.to_string());
    };
    let Some(crypto) = db.crypto.as_ref() else {
        return Err(crate::DbError::Config(
            "DM content is encrypted but CryptoService is not configured (set ENCRYPTION_KEY)"
                .into(),
        ));
    };
    let blob: EncryptedBlob = serde_json::from_str(rest)
        .map_err(|e| crate::DbError::Config(format!("DM ciphertext JSON invalid: {e}")))?;
    let aad_s = aad::dm_content(gift_wrap_id, conv_key);
    crypto
        .decrypt(&blob, &aad_s)
        .map_err(crate::DbError::Crypto)
}

fn db_to_dm(db: &Database, row: DbDmMessage) -> DbResult<DmMessage> {
    let id = row
        .id
        .map(|r| record_id_key_to_string(r.key))
        .unwrap_or_else(|| "unknown".to_string());
    let content = open_dm_content(db, &row.content, &row.gift_wrap_id, &row.conversation_key)?;
    Ok(DmMessage {
        id,
        gift_wrap_id: row.gift_wrap_id,
        sender_pubkey: row.sender_pubkey,
        recipient_pubkey: row.recipient_pubkey,
        conversation_key: row.conversation_key,
        content,
        reply_to_event_id: row.reply_to_event_id,
        created_at: row.created_at.into(),
    })
}

impl Database {
    /// Insert a delivered/sent DM. Idempotent on `gift_wrap_id` — duplicate
    /// inserts (e.g. sender stored on send, then receiver pulls the same wrap
    /// from the relay) return the existing row with `was_new = false`.
    pub async fn insert_dm_message(
        &self,
        gift_wrap_id: &str,
        sender_pubkey: &str,
        recipient_pubkey: &str,
        content: &str,
        reply_to_event_id: Option<&str>,
        created_at: DateTime<Utc>,
    ) -> DbResult<(DmMessage, bool)> {
        let gid = gift_wrap_id.to_string();
        let sender = sender_pubkey.to_string();
        let recipient = recipient_pubkey.to_string();
        let conv = conversation_key(sender_pubkey, recipient_pubkey);
        let body = seal_dm_content(self, content, gift_wrap_id, &conv)?;
        let reply = reply_to_event_id.map(|s| s.to_string());
        let ts = SurrealDatetime::from(created_at);

        with_timeout(async {
            // Fast path: row already present from a prior insert (sender or
            // a prior sync) — return it without writing.
            let mut existing = self
                .client
                .query("SELECT * FROM dm_message WHERE gift_wrap_id = $gid LIMIT 1")
                .bind(("gid", gid.clone()))
                .await?;
            let rows: Vec<DbDmMessage> = existing.take(0)?;
            if let Some(found) = rows.into_iter().next() {
                return Ok((db_to_dm(self, found)?, false));
            }

            let row = DbDmMessage {
                id: None,
                gift_wrap_id: gid,
                sender_pubkey: sender,
                recipient_pubkey: recipient,
                conversation_key: conv,
                content: body,
                reply_to_event_id: reply,
                created_at: ts,
            };
            let created: Option<DbDmMessage> =
                self.client.create("dm_message").content(row).await?;
            let inserted = created
                .ok_or_else(|| crate::DbError::NotFound("Failed to insert dm_message".into()))?;
            Ok((db_to_dm(self, inserted)?, true))
        })
        .await
    }

    /// Load a thread between `me_pubkey` and `peer_pubkey`, newest first,
    /// optionally bounded by `before_ts` for pagination.
    pub async fn list_dm_thread(
        &self,
        me_pubkey: &str,
        peer_pubkey: &str,
        limit: u32,
        before_ts: Option<DateTime<Utc>>,
    ) -> DbResult<Vec<DmMessage>> {
        let conv = conversation_key(me_pubkey, peer_pubkey);
        with_timeout(async {
            let mut result = if let Some(before) = before_ts {
                self.client
                    .query(
                        "SELECT * FROM dm_message WHERE conversation_key = $conv \
                         AND created_at < $before \
                         ORDER BY created_at DESC LIMIT $lim",
                    )
                    .bind(("conv", conv))
                    .bind(("before", SurrealDatetime::from(before)))
                    .bind(("lim", limit))
                    .await?
            } else {
                self.client
                    .query(
                        "SELECT * FROM dm_message WHERE conversation_key = $conv \
                         ORDER BY created_at DESC LIMIT $lim",
                    )
                    .bind(("conv", conv))
                    .bind(("lim", limit))
                    .await?
            };
            let rows: Vec<DbDmMessage> = result.take(0)?;
            rows.into_iter()
                .map(|r| db_to_dm(self, r))
                .collect::<DbResult<Vec<_>>>()
        })
        .await
    }

    /// Recent DMs across all conversations for `me_pubkey` (sender or recipient),
    /// newest first.
    pub async fn list_dm_inbox(
        &self,
        me_pubkey: &str,
        limit: u32,
        since_ts: Option<DateTime<Utc>>,
    ) -> DbResult<Vec<DmMessage>> {
        let me = me_pubkey.to_string();
        with_timeout(async {
            let mut result = if let Some(since) = since_ts {
                self.client
                    .query(
                        "SELECT * FROM dm_message \
                         WHERE (sender_pubkey = $me OR recipient_pubkey = $me) \
                         AND created_at > $since \
                         ORDER BY created_at DESC LIMIT $lim",
                    )
                    .bind(("me", me))
                    .bind(("since", SurrealDatetime::from(since)))
                    .bind(("lim", limit))
                    .await?
            } else {
                self.client
                    .query(
                        "SELECT * FROM dm_message \
                         WHERE sender_pubkey = $me OR recipient_pubkey = $me \
                         ORDER BY created_at DESC LIMIT $lim",
                    )
                    .bind(("me", me))
                    .bind(("lim", limit))
                    .await?
            };
            let rows: Vec<DbDmMessage> = result.take(0)?;
            rows.into_iter()
                .map(|r| db_to_dm(self, r))
                .collect::<DbResult<Vec<_>>>()
        })
        .await
    }

    /// Highest `created_at` across all DMs the member has touched. Used as
    /// the floor when subscribing to the relay so we don't refetch ancient
    /// gift wraps on every sync.
    pub async fn dm_inbox_high_water(&self, me_pubkey: &str) -> DbResult<Option<DateTime<Utc>>> {
        let me = me_pubkey.to_string();
        with_timeout(async {
            let mut result = self
                .client
                .query(
                    "SELECT * FROM dm_message \
                     WHERE sender_pubkey = $me OR recipient_pubkey = $me \
                     ORDER BY created_at DESC LIMIT 1",
                )
                .bind(("me", me))
                .await?;
            let rows: Vec<DbDmMessage> = result.take(0)?;
            Ok(rows.into_iter().next().map(|r| r.created_at.into()))
        })
        .await
    }

    /// Distinct conversations for the current member, newest activity first.
    /// Returns the latest message per peer + an unread count relative to the
    /// member's read marker.
    pub async fn list_dm_conversations(
        &self,
        member_id: &str,
        me_pubkey: &str,
    ) -> DbResult<Vec<DmConversation>> {
        let messages = self.list_dm_inbox(me_pubkey, 500, None).await?;
        let markers = self.list_dm_read_markers(member_id).await?;
        let marker_map: std::collections::HashMap<String, DateTime<Utc>> = markers
            .into_iter()
            .map(|m| (m.peer_pubkey, m.last_read_at))
            .collect();

        let mut latest: std::collections::HashMap<String, DmConversation> =
            std::collections::HashMap::new();
        for msg in messages {
            let peer = if msg.sender_pubkey == me_pubkey {
                msg.recipient_pubkey.clone()
            } else {
                msg.sender_pubkey.clone()
            };
            let entry = latest.entry(peer.clone()).or_insert(DmConversation {
                peer_pubkey: peer.clone(),
                conversation_key: msg.conversation_key.clone(),
                last_message_preview: String::new(),
                last_message_at: msg.created_at,
                last_sender_pubkey: msg.sender_pubkey.clone(),
                unread_count: 0,
            });
            // Newest message wins for the preview because list_dm_inbox is DESC.
            if msg.created_at >= entry.last_message_at {
                entry.last_message_at = msg.created_at;
                entry.last_message_preview = preview(&msg.content);
                entry.last_sender_pubkey = msg.sender_pubkey.clone();
            }
            // Unread = inbound after the read marker for this peer.
            if msg.sender_pubkey != me_pubkey {
                let read_floor = marker_map.get(&peer).copied();
                if read_floor.is_none_or(|floor| msg.created_at > floor) {
                    entry.unread_count += 1;
                }
            }
        }
        let mut convs: Vec<DmConversation> = latest.into_values().collect();
        convs.sort_by_key(|b| std::cmp::Reverse(b.last_message_at));
        Ok(convs)
    }

    /// All read markers belonging to a member.
    pub async fn list_dm_read_markers(&self, member_id: &str) -> DbResult<Vec<DmReadMarker>> {
        let mid = member_id.to_string();
        with_timeout(async {
            let mut result = self
                .client
                .query("SELECT * FROM dm_read_marker WHERE member_id = $mid")
                .bind(("mid", mid))
                .await?;
            let rows: Vec<DbDmReadMarker> = result.take(0)?;
            Ok(rows
                .into_iter()
                .map(|r| DmReadMarker {
                    member_id: r.member_id,
                    peer_pubkey: r.peer_pubkey,
                    last_read_at: r.last_read_at.into(),
                })
                .collect())
        })
        .await
    }

    /// Set the read marker for (member, peer) to `until_ts`. Only advances —
    /// never rewinds (ignores writes older than the existing marker).
    pub async fn upsert_dm_read_marker(
        &self,
        member_id: &str,
        peer_pubkey: &str,
        until_ts: DateTime<Utc>,
    ) -> DbResult<()> {
        let mid = member_id.to_string();
        let peer = peer_pubkey.to_string();
        let ts = SurrealDatetime::from(until_ts);
        with_timeout(async {
            self.client
                .query(
                    r#"UPSERT dm_read_marker
                       SET member_id = $mid, peer_pubkey = $peer,
                           last_read_at = IF $ts > last_read_at THEN $ts ELSE last_read_at END,
                           updated_at = time::now()
                       WHERE member_id = $mid AND peer_pubkey = $peer"#,
                )
                .bind(("mid", mid))
                .bind(("peer", peer))
                .bind(("ts", ts))
                .await?;
            Ok(())
        })
        .await
    }
}

fn preview(content: &str) -> String {
    const PREVIEW_LIMIT: usize = 140;
    let trimmed = content.trim();
    if trimmed.chars().count() <= PREVIEW_LIMIT {
        trimmed.to_string()
    } else {
        let cut: String = trimmed.chars().take(PREVIEW_LIMIT).collect();
        format!("{cut}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations::run_migrations;
    use scuffed_auth::crypto::CryptoService;
    use std::sync::Arc;

    async fn test_db() -> Database {
        let key = CryptoService::generate_key();
        let crypto = CryptoService::new(&key, 1).unwrap();
        let mut db = Database::connect_memory().await.unwrap();
        db.crypto = Some(Arc::new(crypto));
        run_migrations(&db.client).await.unwrap();
        db
    }

    fn ts(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, 0).unwrap()
    }

    #[tokio::test]
    async fn insert_and_list_thread() {
        let db = test_db().await;
        let alice = "a".repeat(64);
        let bob = "b".repeat(64);

        db.insert_dm_message("gw1", &alice, &bob, "hi bob", None, ts(100))
            .await
            .unwrap();
        db.insert_dm_message("gw2", &bob, &alice, "hi alice", None, ts(200))
            .await
            .unwrap();
        db.insert_dm_message("gw3", &alice, &bob, "second from alice", None, ts(300))
            .await
            .unwrap();

        let thread = db.list_dm_thread(&alice, &bob, 50, None).await.unwrap();
        assert_eq!(thread.len(), 3);
        // newest first
        assert_eq!(thread[0].content, "second from alice");
        assert_eq!(thread[2].content, "hi bob");
    }

    #[tokio::test]
    async fn idempotent_insert() {
        let db = test_db().await;
        let alice = "a".repeat(64);
        let bob = "b".repeat(64);

        let (first, was_new1) = db
            .insert_dm_message("gw-dup", &alice, &bob, "once", None, ts(100))
            .await
            .unwrap();
        let (second, was_new2) = db
            .insert_dm_message("gw-dup", &alice, &bob, "once", None, ts(100))
            .await
            .unwrap();
        assert!(was_new1);
        assert!(!was_new2);
        assert_eq!(first.id, second.id);

        // Only one row stored.
        let thread = db.list_dm_thread(&alice, &bob, 50, None).await.unwrap();
        assert_eq!(thread.len(), 1);
    }

    #[tokio::test]
    async fn seal_and_open_roundtrip_with_crypto() {
        let db = test_db().await;
        let sealed = seal_dm_content(&db, "secret message", "gw1", "ck1").unwrap();
        assert!(sealed.starts_with(DM_ENC_PREFIX));
        assert_ne!(sealed, "secret message");
        let opened = open_dm_content(&db, &sealed, "gw1", "ck1").unwrap();
        assert_eq!(opened, "secret message");
    }

    #[tokio::test]
    async fn plaintext_passthrough_without_crypto() {
        let mut db = Database::connect_memory().await.unwrap();
        db.crypto = None;
        run_migrations(&db.client).await.unwrap();
        let sealed = seal_dm_content(&db, "plain", "gw", "ck").unwrap();
        assert_eq!(sealed, "plain");
        assert_eq!(open_dm_content(&db, "plain", "gw", "ck").unwrap(), "plain");
    }

    #[tokio::test]
    async fn open_encrypted_without_crypto_fails() {
        let db_with = test_db().await;
        let sealed = seal_dm_content(&db_with, "hidden", "gw1", "ck1").unwrap();

        let mut db_none = Database::connect_memory().await.unwrap();
        db_none.crypto = None;
        let err = open_dm_content(&db_none, &sealed, "gw1", "ck1").unwrap_err();
        assert!(
            matches!(err, crate::DbError::Config(_)),
            "expected Config error, got {err:?}"
        );
    }

    #[tokio::test]
    async fn open_corrupt_ciphertext_fails() {
        let db = test_db().await;
        let err = open_dm_content(&db, "enc1:{not-json", "gw", "ck").unwrap_err();
        assert!(matches!(err, crate::DbError::Config(_)));
    }

    #[tokio::test]
    async fn conversations_with_unread() {
        let db = test_db().await;
        let alice = "a".repeat(64);
        let bob = "b".repeat(64);
        let carol = "c".repeat(64);
        let alice_member = "member:alice";

        // Two inbound from Bob, one outbound, one inbound from Carol.
        db.insert_dm_message("gw1", &bob, &alice, "from bob 1", None, ts(100))
            .await
            .unwrap();
        db.insert_dm_message("gw2", &bob, &alice, "from bob 2", None, ts(200))
            .await
            .unwrap();
        db.insert_dm_message("gw3", &alice, &bob, "to bob", None, ts(250))
            .await
            .unwrap();
        db.insert_dm_message("gw4", &carol, &alice, "from carol", None, ts(300))
            .await
            .unwrap();

        // No read markers yet → all inbound count as unread.
        let convs = db
            .list_dm_conversations(alice_member, &alice)
            .await
            .unwrap();
        assert_eq!(convs.len(), 2);
        // Carol is newest.
        assert_eq!(convs[0].peer_pubkey, carol);
        assert_eq!(convs[0].unread_count, 1);
        assert_eq!(convs[1].peer_pubkey, bob);
        // Two inbound from Bob, the outbound doesn't count.
        assert_eq!(convs[1].unread_count, 2);

        // Mark Bob read up through the latest inbound.
        db.upsert_dm_read_marker(alice_member, &bob, ts(200))
            .await
            .unwrap();
        let convs = db
            .list_dm_conversations(alice_member, &alice)
            .await
            .unwrap();
        let bob_conv = convs.iter().find(|c| c.peer_pubkey == bob).unwrap();
        assert_eq!(bob_conv.unread_count, 0);
    }

    #[tokio::test]
    async fn read_marker_does_not_rewind() {
        let db = test_db().await;
        let alice_member = "member:alice";
        let bob = "b".repeat(64);

        db.upsert_dm_read_marker(alice_member, &bob, ts(500))
            .await
            .unwrap();
        // A stale write must not rewind the marker.
        db.upsert_dm_read_marker(alice_member, &bob, ts(100))
            .await
            .unwrap();
        let markers = db.list_dm_read_markers(alice_member).await.unwrap();
        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0].last_read_at, ts(500));
    }

    #[tokio::test]
    async fn high_water_returns_latest() {
        let db = test_db().await;
        let alice = "a".repeat(64);
        let bob = "b".repeat(64);

        assert_eq!(db.dm_inbox_high_water(&alice).await.unwrap(), None);

        db.insert_dm_message("gw1", &alice, &bob, "x", None, ts(100))
            .await
            .unwrap();
        db.insert_dm_message("gw2", &bob, &alice, "y", None, ts(900))
            .await
            .unwrap();
        db.insert_dm_message("gw3", &alice, &bob, "z", None, ts(500))
            .await
            .unwrap();

        let hi = db.dm_inbox_high_water(&alice).await.unwrap();
        assert_eq!(hi, Some(ts(900)));
    }
}
