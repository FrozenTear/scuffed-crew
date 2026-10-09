//! Device-link codes for stat-tracker sign-in.
//!
//! The raw user code and device code are hashed before insert (same BLAKE3
//! hex as session tokens). The daemon secret sits in `handover_token` only
//! until the device polls once, then that column is cleared. Callers must not
//! log arguments or driver errors from the approve write: the bound secret can
//! show up in a Surreal error string.

use chrono::{DateTime, Utc};
use serde::Deserialize;
use surrealdb::types::Datetime as SurrealDatetime;
use surrealdb_types::{RecordId, SurrealValue};
use zeroize::Zeroize;

use scuffed_auth::crypto::hash_session_token;

use crate::{with_timeout, Database, DbResult};

/// Seconds the device must wait between accepted polls. Also `interval` on start.
pub const DEVICE_LINK_INTERVAL_SECS: u64 = 5;

/// Lifetime of a fresh code. Also `expires_in` on start.
pub const DEVICE_LINK_TTL_SECS: u64 = 600;

const PENDING: &str = "pending";
const APPROVED: &str = "approved";
const DENIED: &str = "denied";
const CONSUMED: &str = "consumed";

/// What `POST /api/link/poll` should tell the device.
///
/// `Approved` owns the daemon secret. Debug redacts it.
pub enum DeviceLinkPoll {
    Pending,
    SlowDown,
    Denied,
    Expired,
    Approved(String),
}

impl std::fmt::Debug for DeviceLinkPoll {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => f.write_str("Pending"),
            Self::SlowDown => f.write_str("SlowDown"),
            Self::Denied => f.write_str("Denied"),
            Self::Expired => f.write_str("Expired"),
            Self::Approved(_) => f.write_str("Approved([redacted])"),
        }
    }
}

/// Public fields for a live pending code. No secrets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceLinkInfo {
    pub device_label: String,
    pub app_version: String,
    pub created_at: DateTime<Utc>,
}

/// A deny that won the compare-and-swap. Token ids are record keys, not secrets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceLinkDenial {
    pub member_id: Option<String>,
    pub daemon_token_id: Option<String>,
}

#[derive(Clone, Deserialize, SurrealValue)]
struct DbDeviceLink {
    #[surreal(default)]
    #[allow(dead_code)]
    id: Option<RecordId>,
    #[allow(dead_code)]
    user_code_hash: String,
    #[allow(dead_code)]
    device_code_hash: String,
    device_label: String,
    app_version: String,
    status: String,
    member_id: Option<String>,
    daemon_token_id: Option<String>,
    handover_token: Option<String>,
    created_at: SurrealDatetime,
    expires_at: SurrealDatetime,
    last_poll_at: Option<SurrealDatetime>,
}

fn wipe_handover(row: &mut DbDeviceLink) {
    if let Some(secret) = row.handover_token.as_mut() {
        secret.zeroize();
    }
}

fn ttl_literal() -> String {
    // u64 formatting is only digits. This is the compile-time TTL, not user input.
    format!("{DEVICE_LINK_TTL_SECS}s")
}

fn poll_interval_ok_sql() -> String {
    // Same rule: the duration is the compile-time interval, never a request field.
    format!("(last_poll_at = NONE OR last_poll_at <= time::now() - {DEVICE_LINK_INTERVAL_SECS}s)")
}

fn is_expired(expires_at: &SurrealDatetime) -> bool {
    let expires: DateTime<Utc> = (*expires_at).into();
    expires <= Utc::now()
}

fn polled_too_fast(last_poll_at: &Option<SurrealDatetime>) -> bool {
    let Some(last) = last_poll_at else {
        return false;
    };
    let last: DateTime<Utc> = (*last).into();
    Utc::now().signed_duration_since(last)
        < chrono::Duration::seconds(DEVICE_LINK_INTERVAL_SECS as i64)
}

impl Database {
    /// Insert a pending link. `user_code` and `device_code` must already be canonical.
    /// Only their hashes are written.
    pub async fn insert_device_link(
        &self,
        user_code: &str,
        device_code: &str,
        device_label: &str,
        app_version: &str,
    ) -> DbResult<()> {
        let user_code_hash = hash_session_token(user_code);
        let device_code_hash = hash_session_token(device_code);
        let ttl = ttl_literal();
        let sql = format!(
            "CREATE device_link SET
                user_code_hash = $uh,
                device_code_hash = $dh,
                device_label = $label,
                app_version = $version,
                status = '{PENDING}',
                member_id = NONE,
                daemon_token_id = NONE,
                handover_token = NONE,
                created_at = time::now(),
                expires_at = time::now() + {ttl},
                last_poll_at = NONE"
        );
        with_timeout(async {
            let mut result = self
                .client
                .query(&sql)
                .bind(("uh", user_code_hash))
                .bind(("dh", device_code_hash))
                .bind(("label", device_label.to_string()))
                .bind(("version", app_version.to_string()))
                .await?
                .check()?;
            let created: Vec<DbDeviceLink> = result.take(0)?;
            if created.is_empty() {
                return Err(crate::DbError::NotFound(
                    "device link was not created".into(),
                ));
            }
            Ok(())
        })
        .await
    }

    /// Live pending link for this canonical user code, if one exists.
    pub async fn lookup_device_link(&self, user_code: &str) -> DbResult<Option<DeviceLinkInfo>> {
        let user_code_hash = hash_session_token(user_code);
        with_timeout(async {
            #[derive(Deserialize, SurrealValue)]
            struct LookupRow {
                device_label: String,
                app_version: String,
                created_at: SurrealDatetime,
            }

            let mut result = self
                .client
                .query(
                    "SELECT device_label, app_version, created_at FROM device_link
                     WHERE user_code_hash = $h AND status = $pending AND expires_at > time::now()
                     LIMIT 1",
                )
                .bind(("h", user_code_hash))
                .bind(("pending", PENDING.to_string()))
                .await?
                .check()?;
            let rows: Vec<LookupRow> = result.take(0)?;
            Ok(rows.into_iter().next().map(|row| DeviceLinkInfo {
                device_label: row.device_label,
                app_version: row.app_version,
                created_at: row.created_at.into(),
            }))
        })
        .await
    }

    /// Compare-and-swap a live pending code to approved and store the one-time secret.
    ///
    /// `handover_token` is the raw daemon token. It is bound, never interpolated.
    /// Returns false when the code is missing, expired, or no longer pending.
    /// Driver errors are returned as [`crate::DbError::Conflict`] with a static
    /// message so the Surreal string (which can echo the bound secret) is dropped.
    pub async fn approve_device_link(
        &self,
        user_code: &str,
        member_id: &str,
        daemon_token_id: &str,
        handover_token: &str,
    ) -> DbResult<bool> {
        let user_code_hash = hash_session_token(user_code);
        // Not `with_timeout`'s raw driver error: that Display can echo `$secret`.
        let queried = self
            .client
            .query(
                "UPDATE device_link SET
                    status = $approved,
                    member_id = $mid,
                    daemon_token_id = $tid,
                    handover_token = $secret
                 WHERE user_code_hash = $h AND status = $pending AND expires_at > time::now()
                 RETURN AFTER",
            )
            .bind(("approved", APPROVED.to_string()))
            .bind(("mid", member_id.to_string()))
            .bind(("tid", daemon_token_id.to_string()))
            .bind(("secret", handover_token.to_string()))
            .bind(("h", user_code_hash))
            .bind(("pending", PENDING.to_string()))
            .await;
        let response = match queried {
            Ok(response) => response,
            Err(_error) => {
                return Err(crate::DbError::Conflict(
                    "device link approve failed".into(),
                ));
            }
        };
        let mut checked = match response.check() {
            Ok(checked) => checked,
            Err(_error) => {
                return Err(crate::DbError::Conflict(
                    "device link approve failed".into(),
                ));
            }
        };
        let mut rows: Vec<DbDeviceLink> = match checked.take(0) {
            Ok(rows) => rows,
            Err(_error) => {
                return Err(crate::DbError::Conflict(
                    "device link approve failed".into(),
                ));
            }
        };
        let won = !rows.is_empty();
        for row in &mut rows {
            wipe_handover(row);
        }
        Ok(won)
    }

    /// Kill a live pending or approved code. The secret on the previous row is wiped
    /// in memory and in the update. Returns `None` when there was nothing to deny.
    pub async fn deny_device_link(&self, user_code: &str) -> DbResult<Option<DeviceLinkDenial>> {
        let user_code_hash = hash_session_token(user_code);
        with_timeout(async {
            let mut result = self
                .client
                .query(
                    "UPDATE device_link SET status = $denied, handover_token = NONE
                     WHERE user_code_hash = $h AND status IN $open AND expires_at > time::now()
                     RETURN BEFORE",
                )
                .bind(("denied", DENIED.to_string()))
                .bind(("h", user_code_hash))
                .bind(("open", vec![PENDING.to_string(), APPROVED.to_string()]))
                .await?
                .check()?;
            let rows: Vec<DbDeviceLink> = result.take(0)?;
            let Some(mut row) = rows.into_iter().next() else {
                return Ok(None);
            };
            let denial = DeviceLinkDenial {
                member_id: row.member_id.clone(),
                daemon_token_id: row.daemon_token_id.clone(),
            };
            wipe_handover(&mut row);
            Ok(Some(denial))
        })
        .await
    }

    /// Advance a device poll. The secret is returned at most once.
    pub async fn poll_device_link(&self, device_code: &str) -> DbResult<DeviceLinkPoll> {
        let device_code_hash = hash_session_token(device_code);
        with_timeout(async {
            for _ in 0..2 {
                let Some(mut row) = self.load_by_device_hash(&device_code_hash).await? else {
                    return Ok(DeviceLinkPoll::Expired);
                };
                // The secret is returned only from the consume write, not this read.
                wipe_handover(&mut row);
                if is_expired(&row.expires_at) || row.status == CONSUMED {
                    return Ok(DeviceLinkPoll::Expired);
                }
                if polled_too_fast(&row.last_poll_at) {
                    return Ok(DeviceLinkPoll::SlowDown);
                }
                match row.status.as_str() {
                    PENDING => {
                        if self.touch_poll(&device_code_hash, PENDING).await? {
                            return Ok(DeviceLinkPoll::Pending);
                        }
                    }
                    DENIED => {
                        let _ = self.touch_poll(&device_code_hash, DENIED).await?;
                        return Ok(DeviceLinkPoll::Denied);
                    }
                    APPROVED => match self.consume_approved(&device_code_hash).await? {
                        Consume::Token(token) => return Ok(DeviceLinkPoll::Approved(token)),
                        Consume::Empty => return Ok(DeviceLinkPoll::Expired),
                        Consume::Lost => {}
                    },
                    _ => return Ok(DeviceLinkPoll::Expired),
                }
            }
            Ok(DeviceLinkPoll::Expired)
        })
        .await
    }

    /// Delete codes whose `expires_at` has passed, including any unclaimed secret.
    pub async fn cleanup_expired_device_links(&self) -> DbResult<u64> {
        with_timeout(async {
            #[derive(Deserialize, SurrealValue)]
            struct CountResult {
                count: u64,
            }

            let mut result = self
                .client
                .query("SELECT count() FROM device_link WHERE expires_at <= time::now() GROUP ALL")
                .await?;
            let counts: Vec<CountResult> = result.take(0)?;
            let count = counts.first().map(|c| c.count).unwrap_or(0);
            if count > 0 {
                self.client
                    .query("DELETE FROM device_link WHERE expires_at <= time::now()")
                    .await?;
                tracing::info!(removed = count, "cleaned up expired device links");
            }
            Ok(count)
        })
        .await
    }

    async fn load_by_device_hash(&self, device_code_hash: &str) -> DbResult<Option<DbDeviceLink>> {
        let mut result = self
            .client
            .query("SELECT * FROM device_link WHERE device_code_hash = $h LIMIT 1")
            .bind(("h", device_code_hash.to_string()))
            .await?
            .check()?;
        let rows: Vec<DbDeviceLink> = result.take(0)?;
        Ok(rows.into_iter().next())
    }

    async fn touch_poll(&self, device_code_hash: &str, status: &str) -> DbResult<bool> {
        let interval = poll_interval_ok_sql();
        let sql = format!(
            "UPDATE device_link SET last_poll_at = time::now()
             WHERE device_code_hash = $h AND status = $status AND expires_at > time::now()
             AND {interval}
             RETURN AFTER"
        );
        let mut result = self
            .client
            .query(&sql)
            .bind(("h", device_code_hash.to_string()))
            .bind(("status", status.to_string()))
            .await?
            .check()?;
        let mut rows: Vec<DbDeviceLink> = result.take(0)?;
        let found = !rows.is_empty();
        for row in &mut rows {
            wipe_handover(row);
        }
        Ok(found)
    }

    async fn consume_approved(&self, device_code_hash: &str) -> DbResult<Consume> {
        let interval = poll_interval_ok_sql();
        let sql = format!(
            "UPDATE device_link SET
                status = '{CONSUMED}',
                handover_token = NONE,
                last_poll_at = time::now()
             WHERE device_code_hash = $h AND status = '{APPROVED}' AND expires_at > time::now()
             AND {interval}
             RETURN BEFORE"
        );
        let mut result = self
            .client
            .query(&sql)
            .bind(("h", device_code_hash.to_string()))
            .await?
            .check()?;
        let rows: Vec<DbDeviceLink> = result.take(0)?;
        let Some(mut row) = rows.into_iter().next() else {
            return Ok(Consume::Lost);
        };
        match row.handover_token.take() {
            Some(token) if !token.is_empty() => Ok(Consume::Token(token)),
            Some(mut token) => {
                token.zeroize();
                Ok(Consume::Empty)
            }
            None => Ok(Consume::Empty),
        }
    }
}

enum Consume {
    Token(String),
    Empty,
    Lost,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations::run_migrations;

    async fn test_db() -> Database {
        let db = Database::connect_memory().await.expect("mem db");
        run_migrations(&db.client).await.expect("migrations");
        db
    }

    async fn seed_member(db: &Database) {
        db.client
            .query(
                "CREATE user:linkuser SET provider = 'discord', username = 'linkuser',
                 provider_id = 'linkuser-pid', provider_id_hash = 'linkuser-pidh',
                 created_at = time::now()",
            )
            .await
            .expect("user");
        db.client
            .query(
                "CREATE member:linkmember SET user_id = 'linkuser', org_role = 'member',
                 display_name = 'Link User', is_active = true, joined_at = time::now()",
            )
            .await
            .expect("member");
    }

    #[tokio::test]
    async fn stores_hashes_and_hands_the_secret_over_once() {
        let db = test_db().await;
        seed_member(&db).await;
        let user_code = "ABCD2345";
        let device_code = "ab".repeat(32);
        db.insert_device_link(user_code, &device_code, "Living Room PC", "0.4.2")
            .await
            .expect("insert");

        let mut dumped = db
            .client
            .query("SELECT * FROM device_link")
            .await
            .expect("dump");
        let rows: Vec<DbDeviceLink> = dumped.take(0).expect("rows");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].user_code_hash, hash_session_token(user_code));
        assert_eq!(rows[0].device_code_hash, hash_session_token(&device_code));
        assert_ne!(rows[0].user_code_hash, user_code);
        assert_ne!(rows[0].device_code_hash, device_code);
        assert!(rows[0].handover_token.is_none());

        let info = db
            .lookup_device_link(user_code)
            .await
            .unwrap()
            .expect("live");
        assert_eq!(info.device_label, "Living Room PC");
        assert_eq!(info.app_version, "0.4.2");

        let secret = "cd".repeat(32);
        db.create_daemon_token("linkmember", &secret, "Living Room PC")
            .await
            .unwrap();
        let tokens = db.list_daemon_tokens("linkmember").await.unwrap();
        assert!(db
            .approve_device_link(user_code, "linkmember", &tokens[0].id, &secret)
            .await
            .unwrap());
        assert!(
            !db.approve_device_link(user_code, "linkmember", &tokens[0].id, &secret)
                .await
                .unwrap(),
            "a second approve loses the compare-and-swap"
        );

        match db.poll_device_link(&device_code).await.unwrap() {
            DeviceLinkPoll::Approved(got) => assert_eq!(got, secret),
            other => panic!("expected the secret once, got {other:?}"),
        }
        assert!(
            matches!(
                db.poll_device_link(&device_code).await.unwrap(),
                DeviceLinkPoll::Expired
            ),
            "the code is dead after the secret is handed over"
        );

        let mut after = db.client.query("SELECT * FROM device_link").await.unwrap();
        let rows: Vec<DbDeviceLink> = after.take(0).unwrap();
        assert!(rows[0].handover_token.is_none());
        assert_ne!(rows[0].handover_token.as_deref(), Some(secret.as_str()));
    }

    #[tokio::test]
    async fn poll_slow_down_deny_expiry_and_cleanup() {
        let db = test_db().await;
        db.insert_device_link("ZZZZ2345", &"11".repeat(32), "Desk", "1.0.0")
            .await
            .unwrap();
        assert!(matches!(
            db.poll_device_link(&"11".repeat(32)).await.unwrap(),
            DeviceLinkPoll::Pending
        ));
        assert!(matches!(
            db.poll_device_link(&"11".repeat(32)).await.unwrap(),
            DeviceLinkPoll::SlowDown
        ));
        db.client
            .query("UPDATE device_link SET last_poll_at = time::now() - 1h")
            .await
            .unwrap();
        assert!(matches!(
            db.poll_device_link(&"11".repeat(32)).await.unwrap(),
            DeviceLinkPoll::Pending
        ));

        assert!(db.deny_device_link("ZZZZ2345").await.unwrap().is_some());
        db.client
            .query("UPDATE device_link SET last_poll_at = time::now() - 1h")
            .await
            .unwrap();
        assert!(matches!(
            db.poll_device_link(&"11".repeat(32)).await.unwrap(),
            DeviceLinkPoll::Denied
        ));
        assert!(db.lookup_device_link("ZZZZ2345").await.unwrap().is_none());
        assert!(db.deny_device_link("ZZZZ2345").await.unwrap().is_none());
        assert!(db.lookup_device_link("NOPE2345").await.unwrap().is_none());
        assert!(matches!(
            db.poll_device_link(&"22".repeat(32)).await.unwrap(),
            DeviceLinkPoll::Expired
        ));

        db.insert_device_link("ABCD6789", &"33".repeat(32), "Other", "1.2.3")
            .await
            .unwrap();
        db.client
            .query(
                "UPDATE device_link SET expires_at = time::now() - 1s WHERE device_label = 'Other'",
            )
            .await
            .unwrap();
        assert!(db.lookup_device_link("ABCD6789").await.unwrap().is_none());
        assert!(matches!(
            db.poll_device_link(&"33".repeat(32)).await.unwrap(),
            DeviceLinkPoll::Expired
        ));
        let removed = db.cleanup_expired_device_links().await.unwrap();
        assert!(removed >= 1);
        let mut left = db.client.query("SELECT * FROM device_link").await.unwrap();
        let rows: Vec<DbDeviceLink> = left.take(0).unwrap();
        assert!(
            rows.iter().all(|row| row.device_label != "Other"),
            "expired rows are removed"
        );
        assert!(
            rows.iter().any(|row| row.device_label == "Desk"),
            "a code that is only denied, not expired, stays until its TTL"
        );
    }
}
