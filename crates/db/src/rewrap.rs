//! Re-encrypt all at-rest `EncryptedBlob` fields under the current key version.
//!
//! Used after key rotation: keep previous keys in the keyring
//! (`ENCRYPTION_KEY_PREVIOUS`) and run `scuffed-rewrap` to upgrade stored
//! ciphertext to `ENCRYPTION_KEY` / `ENCRYPTION_KEY_VERSION`.

use serde::{Deserialize, Serialize};
use surrealdb_types::RecordId;
use surrealdb_types::SurrealValue;

use scuffed_auth::crypto::{aad, EncryptedBlob};

use crate::{record_id_key_to_string, Database, DbError, DbResult};

/// Prefix for AES-GCM encrypted DM content stored in `dm_message.content`.
const DM_ENC_PREFIX: &str = "enc1:";

/// Aggregate counters from a full rewrap pass.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RewrapStats {
    /// Rows examined that had encrypted material (any of the three targets).
    pub scanned: u64,
    /// Blobs re-encrypted to the current key version.
    pub rewrapped: u64,
    /// Blobs already on the current key version (left unchanged).
    pub skipped_current: u64,
    /// Failures encountered (0 when the call returns `Ok` — fail-fast).
    pub errors: u64,
}

impl std::fmt::Display for RewrapStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "scanned={} rewrapped={} skipped_current={} errors={}",
            self.scanned, self.rewrapped, self.skipped_current, self.errors
        )
    }
}

/// Re-encrypt all encrypted fields under the database's current `CryptoService` key.
///
/// Targets:
/// 1. `user.provider_id_encrypted` — AAD `oauth_provider_id(user_id, provider)`
/// 2. `member.nostr_secret_key_encrypted` — server_managed only; AAD `nostr_secret_key(pubkey)`
/// 3. `dm_message.content` with `enc1:` prefix — AAD `dm_content(gift_wrap_id, conversation_key)`
///
/// Fails on the first error (misconfigured keyring, corrupt blob, bad AAD) so a
/// broken rotation is not partially applied without notice.
pub async fn rewrap_all_encrypted_fields(db: &Database) -> DbResult<RewrapStats> {
    let crypto = db.crypto.as_ref().ok_or_else(|| {
        DbError::Config(
            "CryptoService not configured — set ENCRYPTION_KEY (and ENCRYPTION_KEY_PREVIOUS for old versions)"
                .into(),
        )
    })?;
    let current = crypto.key_version();
    let mut stats = RewrapStats::default();

    rewrap_users(db, current, &mut stats).await?;
    rewrap_members(db, current, &mut stats).await?;
    rewrap_dm_messages(db, current, &mut stats).await?;

    Ok(stats)
}

#[derive(Debug, Deserialize, SurrealValue)]
struct UserEncRow {
    #[surreal(default)]
    id: Option<RecordId>,
    provider: String,
    provider_id_encrypted: Option<serde_json::Value>,
}

async fn rewrap_users(db: &Database, current: u32, stats: &mut RewrapStats) -> DbResult<()> {
    let crypto = db.crypto.as_ref().expect("crypto checked by caller");
    let mut result = db
        .client
        .query(
            "SELECT id, provider, provider_id_encrypted FROM user \
             WHERE provider_id_encrypted != NONE",
        )
        .await?;
    let rows: Vec<UserEncRow> = result.take(0)?;

    for row in rows {
        let Some(enc_val) = row.provider_id_encrypted else {
            continue;
        };
        if enc_val.is_null() {
            continue;
        }
        stats.scanned += 1;

        let user_id = match row.id {
            Some(r) => record_id_key_to_string(r.key),
            None => {
                return Err(DbError::Config(
                    "user row missing id during rewrap".into(),
                ))
            }
        };

        let blob: EncryptedBlob = serde_json::from_value(enc_val).map_err(|e| {
            DbError::Config(format!(
                "user {user_id}: corrupt provider_id_encrypted: {e}"
            ))
        })?;

        if blob.key_version == current {
            stats.skipped_current += 1;
            continue;
        }

        let aad_s = aad::oauth_provider_id(&user_id, &row.provider);
        let new_blob = crypto.rewrap(&blob, &aad_s).map_err(|e| {
            DbError::Config(format!(
                "user {user_id}: rewrap provider_id_encrypted failed: {e}"
            ))
        })?;

        let enc = serde_json::to_value(&new_blob).map_err(|e| {
            DbError::Config(format!(
                "user {user_id}: serialize rewrapped blob failed: {e}"
            ))
        })?;
        let rid = RecordId::new("user", user_id.as_str());
        db.client
            .query("UPDATE $rid SET provider_id_encrypted = $enc")
            .bind(("rid", rid))
            .bind(("enc", enc))
            .await
            .map_err(|e| {
                DbError::Config(format!(
                    "user {user_id}: UPDATE provider_id_encrypted failed: {e}"
                ))
            })?;

        stats.rewrapped += 1;
        tracing::info!(user_id = %user_id, from = blob.key_version, to = current, "rewrapped provider_id_encrypted");
    }

    Ok(())
}

#[derive(Debug, Deserialize, SurrealValue)]
struct MemberEncRow {
    #[surreal(default)]
    id: Option<RecordId>,
    nostr_pubkey: Option<String>,
    #[serde(default)]
    #[surreal(default)]
    nostr_secret_key_encrypted: Option<serde_json::Value>,
}

async fn rewrap_members(db: &Database, current: u32, stats: &mut RewrapStats) -> DbResult<()> {
    let crypto = db.crypto.as_ref().expect("crypto checked by caller");
    let mut result = db
        .client
        .query(
            "SELECT id, nostr_pubkey, nostr_secret_key_encrypted FROM member \
             WHERE nostr_key_mode = 'server_managed' \
             AND nostr_secret_key_encrypted != NONE \
             AND nostr_pubkey != NONE",
        )
        .await?;
    let rows: Vec<MemberEncRow> = result.take(0)?;

    for row in rows {
        let (Some(pubkey), Some(enc_val)) = (row.nostr_pubkey, row.nostr_secret_key_encrypted)
        else {
            continue;
        };
        if enc_val.is_null() {
            continue;
        }
        stats.scanned += 1;

        let member_id = match row.id {
            Some(r) => record_id_key_to_string(r.key),
            None => {
                return Err(DbError::Config(
                    "member row missing id during rewrap".into(),
                ))
            }
        };

        let blob: EncryptedBlob = serde_json::from_value(enc_val).map_err(|e| {
            DbError::Config(format!(
                "member {member_id}: corrupt nostr_secret_key_encrypted: {e}"
            ))
        })?;

        if blob.key_version == current {
            stats.skipped_current += 1;
            continue;
        }

        let aad_s = aad::nostr_secret_key(&pubkey);
        let new_blob = crypto.rewrap(&blob, &aad_s).map_err(|e| {
            DbError::Config(format!(
                "member {member_id}: rewrap nostr_secret_key_encrypted failed: {e}"
            ))
        })?;

        let enc = serde_json::to_value(&new_blob).map_err(|e| {
            DbError::Config(format!(
                "member {member_id}: serialize rewrapped blob failed: {e}"
            ))
        })?;
        let rid = RecordId::new("member", member_id.as_str());
        db.client
            .query("UPDATE $rid SET nostr_secret_key_encrypted = $enc")
            .bind(("rid", rid))
            .bind(("enc", enc))
            .await
            .map_err(|e| {
                DbError::Config(format!(
                    "member {member_id}: UPDATE nostr_secret_key_encrypted failed: {e}"
                ))
            })?;

        stats.rewrapped += 1;
        tracing::info!(
            member_id = %member_id,
            from = blob.key_version,
            to = current,
            "rewrapped nostr_secret_key_encrypted"
        );
    }

    Ok(())
}

#[derive(Debug, Deserialize, SurrealValue)]
struct DmEncRow {
    #[surreal(default)]
    id: Option<RecordId>,
    gift_wrap_id: String,
    conversation_key: String,
    content: String,
}

async fn rewrap_dm_messages(db: &Database, current: u32, stats: &mut RewrapStats) -> DbResult<()> {
    let crypto = db.crypto.as_ref().expect("crypto checked by caller");
    let mut result = db
        .client
        .query("SELECT id, gift_wrap_id, conversation_key, content FROM dm_message")
        .await?;
    let rows: Vec<DmEncRow> = result.take(0)?;

    for row in rows {
        let Some(rest) = row.content.strip_prefix(DM_ENC_PREFIX) else {
            continue;
        };
        stats.scanned += 1;

        let rid = match row.id {
            Some(r) => r,
            None => {
                return Err(DbError::Config(format!(
                    "dm gift_wrap_id={}: missing record id during rewrap",
                    row.gift_wrap_id
                )))
            }
        };
        let msg_id = record_id_key_to_string(rid.key.clone());

        let blob: EncryptedBlob = serde_json::from_str(rest).map_err(|e| {
            DbError::Config(format!(
                "dm {msg_id} (gw={}): corrupt enc1: blob: {e}",
                row.gift_wrap_id
            ))
        })?;

        if blob.key_version == current {
            stats.skipped_current += 1;
            continue;
        }

        let aad_s = aad::dm_content(&row.gift_wrap_id, &row.conversation_key);
        let new_blob = crypto.rewrap(&blob, &aad_s).map_err(|e| {
            DbError::Config(format!(
                "dm {msg_id} (gw={}): rewrap content failed: {e}",
                row.gift_wrap_id
            ))
        })?;

        let json = serde_json::to_string(&new_blob).map_err(|e| {
            DbError::Config(format!(
                "dm {msg_id}: serialize rewrapped content failed: {e}"
            ))
        })?;
        let stored = format!("{DM_ENC_PREFIX}{json}");

        db.client
            .query("UPDATE $rid SET content = $content")
            .bind(("rid", rid))
            .bind(("content", stored))
            .await
            .map_err(|e| {
                DbError::Config(format!(
                    "dm {msg_id} (gw={}): UPDATE content failed: {e}",
                    row.gift_wrap_id
                ))
            })?;

        stats.rewrapped += 1;
        tracing::info!(
            gift_wrap_id = %row.gift_wrap_id,
            from = blob.key_version,
            to = current,
            "rewrapped dm_message content"
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations::run_migrations;
    use crate::types::OrgRole;
    use chrono::{DateTime, Utc};
    use scuffed_auth::crypto::CryptoService;
    use scuffed_auth::{AuthProvider, User};
    use std::sync::Arc;

    async fn memory_db_with(crypto: CryptoService) -> Database {
        let mut db = Database::connect_memory().await.unwrap();
        db.crypto = Some(Arc::new(crypto));
        run_migrations(&db.client).await.unwrap();
        db
    }

    fn ts(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, 0).unwrap()
    }

    #[tokio::test]
    async fn rewrap_upgrades_user_member_and_dm_with_two_key_keyring() {
        let key_v1 = CryptoService::generate_key();
        let key_v2 = CryptoService::generate_key();

        // Encrypt under v1.
        let v1 = CryptoService::new(&key_v1, 1).unwrap();
        let mut db = memory_db_with(v1).await;

        let user = User::new(
            AuthProvider::Discord,
            "discord-subject-99".into(),
            "rewruser".into(),
            None,
        );
        let created = db
            .upsert_user_from_oauth(
                AuthProvider::Discord,
                user.provider_id.clone(),
                user.username.clone(),
                None,
            )
            .await
            .unwrap();
        let user_id = created.id.clone();

        let member = db
            .create_member(&user_id, "Rewrap Member", OrgRole::Member)
            .await
            .unwrap();
        assert!(member.nostr_secret_key_encrypted.is_some());
        assert_eq!(
            member
                .nostr_secret_key_encrypted
                .as_ref()
                .unwrap()
                .key_version,
            1
        );
        let member_id = member.id.clone();
        let pubkey = member.nostr_pubkey.clone().unwrap();

        let alice = pubkey.clone();
        let bob = "b".repeat(64);
        db.insert_dm_message("gw-rewrap-1", &alice, &bob, "rotate me", None, ts(1000))
            .await
            .unwrap();

        // Swap to dual-key keyring: current v2, previous v1.
        let dual =
            CryptoService::from_keyring(&key_v2, 2, &[(1, key_v1.as_str())], false).unwrap();
        db.crypto = Some(Arc::new(dual));

        let stats = rewrap_all_encrypted_fields(&db).await.unwrap();
        assert_eq!(stats.scanned, 3, "user + member + dm: {stats}");
        assert_eq!(stats.rewrapped, 3, "{stats}");
        assert_eq!(stats.skipped_current, 0, "{stats}");
        assert_eq!(stats.errors, 0);

        // Second pass: everything already current.
        let stats2 = rewrap_all_encrypted_fields(&db).await.unwrap();
        assert_eq!(stats2.scanned, 3);
        assert_eq!(stats2.rewrapped, 0);
        assert_eq!(stats2.skipped_current, 3);

        // Decrypt path still works under v2.
        let loaded = db.get_user(&user_id).await.unwrap().unwrap();
        assert_eq!(loaded.provider_id, "discord-subject-99");

        let full = db.get_member(&member_id).await.unwrap().unwrap();
        let sk_blob = full.nostr_secret_key_encrypted.as_ref().unwrap();
        assert_eq!(sk_blob.key_version, 2);
        let sk_hex =
            crate::queries::nostr_keys::decrypt_nostr_secret_key(&db, sk_blob, &pubkey).unwrap();
        assert_eq!(sk_hex.len(), 64);

        let thread = db.list_dm_thread(&alice, &bob, 10, None).await.unwrap();
        assert_eq!(thread.len(), 1);
        assert_eq!(thread[0].content, "rotate me");
    }

    #[tokio::test]
    async fn rewrap_requires_crypto() {
        let mut db = Database::connect_memory().await.unwrap();
        db.crypto = None;
        run_migrations(&db.client).await.unwrap();
        let err = rewrap_all_encrypted_fields(&db).await.unwrap_err();
        assert!(matches!(err, DbError::Config(_)));
    }

    #[tokio::test]
    async fn rewrap_empty_db_is_noop() {
        let key = CryptoService::generate_key();
        let crypto = CryptoService::new(&key, 1).unwrap();
        let db = memory_db_with(crypto).await;
        let stats = rewrap_all_encrypted_fields(&db).await.unwrap();
        assert_eq!(stats, RewrapStats::default());
    }
}
