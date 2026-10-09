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

fn is_expired(expires_at: &SurrealDatetime, now: DateTime<Utc>) -> bool {
    let expires: DateTime<Utc> = (*expires_at).into();
    expires <= now
}

fn polled_too_fast(last_poll_at: &Option<SurrealDatetime>, now: DateTime<Utc>) -> bool {
    let Some(last) = last_poll_at else {
        return false;
    };
    let last: DateTime<Utc> = (*last).into();
    now.signed_duration_since(last) < chrono::Duration::seconds(DEVICE_LINK_INTERVAL_SECS as i64)
}

fn surreal_at(now: DateTime<Utc>) -> SurrealDatetime {
    SurrealDatetime::from(now)
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

    /// Kill a live code.
    ///
    /// A pending code can be denied by any member. An approved code that has
    /// not been collected yet can be denied only by `member_id`, the member
    /// who approved it. The caller revokes `daemon_token_id` when it is set.
    /// The secret on the previous row is wiped in memory and in the update.
    /// Returns `None` when there was nothing this member can deny.
    pub async fn deny_device_link(
        &self,
        user_code: &str,
        member_id: &str,
    ) -> DbResult<Option<DeviceLinkDenial>> {
        let user_code_hash = hash_session_token(user_code);
        with_timeout(async {
            let mut result = self
                .client
                .query(
                    "UPDATE device_link SET status = $denied, handover_token = NONE
                     WHERE user_code_hash = $h AND expires_at > time::now()
                     AND (status = $pending OR (status = $approved AND member_id = $mid))
                     RETURN BEFORE",
                )
                .bind(("denied", DENIED.to_string()))
                .bind(("h", user_code_hash))
                .bind(("pending", PENDING.to_string()))
                .bind(("approved", APPROVED.to_string()))
                .bind(("mid", member_id.to_string()))
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
    ///
    /// `now` is the caller's clock. Production passes wall time. Tests pass a
    /// manual clock so a device can poll on the 5 second interval for the
    /// whole 10 minute lifetime without sleeping.
    pub async fn poll_device_link(
        &self,
        device_code: &str,
        now: DateTime<Utc>,
    ) -> DbResult<DeviceLinkPoll> {
        let device_code_hash = hash_session_token(device_code);
        with_timeout(async {
            for _ in 0..2 {
                let Some(mut row) = self.load_by_device_hash(&device_code_hash).await? else {
                    return Ok(DeviceLinkPoll::Expired);
                };
                // The secret is returned only from the consume write, not this read.
                wipe_handover(&mut row);
                if is_expired(&row.expires_at, now) || row.status == CONSUMED {
                    return Ok(DeviceLinkPoll::Expired);
                }
                if polled_too_fast(&row.last_poll_at, now) {
                    return Ok(DeviceLinkPoll::SlowDown);
                }
                match row.status.as_str() {
                    PENDING => {
                        if self.touch_poll(&device_code_hash, PENDING, now).await? {
                            return Ok(DeviceLinkPoll::Pending);
                        }
                    }
                    DENIED => {
                        let _ = self.touch_poll(&device_code_hash, DENIED, now).await?;
                        return Ok(DeviceLinkPoll::Denied);
                    }
                    APPROVED => match self.consume_approved(&device_code_hash, now).await? {
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

    /// Delete codes whose `expires_at` is at or before now.
    ///
    /// An approved code that expired before the device collected the token is
    /// revoked first, and its handover secret is cleared. The delete runs only
    /// after those revokes succeed, so a failed revoke leaves the row for the
    /// next pass. A token that was already handed over stays active.
    pub async fn cleanup_expired_device_links(&self) -> DbResult<u64> {
        self.cleanup_expired_device_links_before(Utc::now()).await
    }

    /// Revoke and delete codes with `expires_at <= cutoff`.
    ///
    /// `cutoff` is taken once and bound into the revoke select, the handover
    /// clear, and the delete. A code that expires while this pass is running
    /// stays until the next pass, instead of being deleted with its token
    /// still active.
    async fn cleanup_expired_device_links_before(&self, cutoff: DateTime<Utc>) -> DbResult<u64> {
        with_timeout(async {
            #[derive(Deserialize, SurrealValue)]
            struct CountResult {
                count: u64,
            }
            #[derive(Deserialize, SurrealValue)]
            struct Uncollected {
                member_id: Option<String>,
                daemon_token_id: Option<String>,
            }

            let cutoff_at = surreal_at(cutoff);
            let mut listed = self
                .client
                .query(
                    "SELECT member_id, daemon_token_id FROM device_link
                     WHERE expires_at <= $cutoff AND status = $approved
                     AND member_id IS NOT NONE AND daemon_token_id IS NOT NONE",
                )
                .bind(("cutoff", cutoff_at))
                .bind(("approved", APPROVED.to_string()))
                .await?
                .check()?;
            let rows: Vec<Uncollected> = listed.take(0)?;
            for row in rows {
                let (Some(member_id), Some(token_id)) = (row.member_id, row.daemon_token_id) else {
                    return Err(crate::DbError::Conflict(
                        "device link cleanup missing token owner".into(),
                    ));
                };
                match self.revoke_daemon_token(&token_id, &member_id).await {
                    Ok(()) => {}
                    Err(crate::DbError::NotFound(_)) => {}
                    Err(error) => return Err(error),
                }
            }

            self.client
                .query("UPDATE device_link SET handover_token = NONE WHERE expires_at <= $cutoff")
                .bind(("cutoff", cutoff_at))
                .await?
                .check()?;

            let mut result = self
                .client
                .query("SELECT count() FROM device_link WHERE expires_at <= $cutoff GROUP ALL")
                .bind(("cutoff", cutoff_at))
                .await?;
            let counts: Vec<CountResult> = result.take(0)?;
            let count = counts.first().map(|c| c.count).unwrap_or(0);
            if count > 0 {
                self.client
                    .query("DELETE FROM device_link WHERE expires_at <= $cutoff")
                    .bind(("cutoff", cutoff_at))
                    .await?
                    .check()?;
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

    async fn touch_poll(
        &self,
        device_code_hash: &str,
        status: &str,
        now: DateTime<Utc>,
    ) -> DbResult<bool> {
        let cutoff = now - chrono::Duration::seconds(DEVICE_LINK_INTERVAL_SECS as i64);
        let mut result = self
            .client
            .query(
                "UPDATE device_link SET last_poll_at = $now
                 WHERE device_code_hash = $h AND status = $status AND expires_at > $now
                 AND (last_poll_at = NONE OR last_poll_at <= $cutoff)
                 RETURN AFTER",
            )
            .bind(("h", device_code_hash.to_string()))
            .bind(("status", status.to_string()))
            .bind(("now", surreal_at(now)))
            .bind(("cutoff", surreal_at(cutoff)))
            .await?
            .check()?;
        let mut rows: Vec<DbDeviceLink> = result.take(0)?;
        let found = !rows.is_empty();
        for row in &mut rows {
            wipe_handover(row);
        }
        Ok(found)
    }

    /// One statement claims the secret. The `status = approved` predicate is the
    /// compare-and-swap: a second poll matches no row and gets [`Consume::Lost`].
    async fn consume_approved(
        &self,
        device_code_hash: &str,
        now: DateTime<Utc>,
    ) -> DbResult<Consume> {
        let cutoff = now - chrono::Duration::seconds(DEVICE_LINK_INTERVAL_SECS as i64);
        let mut result = self
            .client
            .query(
                "UPDATE device_link SET
                    status = $consumed,
                    handover_token = NONE,
                    last_poll_at = $now
                 WHERE device_code_hash = $h AND status = $approved AND expires_at > $now
                 AND (last_poll_at = NONE OR last_poll_at <= $cutoff)
                 RETURN BEFORE",
            )
            .bind(("consumed", CONSUMED.to_string()))
            .bind(("approved", APPROVED.to_string()))
            .bind(("h", device_code_hash.to_string()))
            .bind(("now", surreal_at(now)))
            .bind(("cutoff", surreal_at(cutoff)))
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

        match db.poll_device_link(&device_code, Utc::now()).await.unwrap() {
            DeviceLinkPoll::Approved(got) => assert_eq!(got, secret),
            other => panic!("expected the secret once, got {other:?}"),
        }
        assert!(
            matches!(
                db.poll_device_link(&device_code, Utc::now()).await.unwrap(),
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
            db.poll_device_link(&"11".repeat(32), Utc::now())
                .await
                .unwrap(),
            DeviceLinkPoll::Pending
        ));
        assert!(matches!(
            db.poll_device_link(&"11".repeat(32), Utc::now())
                .await
                .unwrap(),
            DeviceLinkPoll::SlowDown
        ));
        db.client
            .query("UPDATE device_link SET last_poll_at = time::now() - 1h")
            .await
            .unwrap();
        assert!(matches!(
            db.poll_device_link(&"11".repeat(32), Utc::now())
                .await
                .unwrap(),
            DeviceLinkPoll::Pending
        ));

        assert!(db
            .deny_device_link("ZZZZ2345", "linkmember")
            .await
            .unwrap()
            .is_some());
        db.client
            .query("UPDATE device_link SET last_poll_at = time::now() - 1h")
            .await
            .unwrap();
        assert!(matches!(
            db.poll_device_link(&"11".repeat(32), Utc::now())
                .await
                .unwrap(),
            DeviceLinkPoll::Denied
        ));
        assert!(db.lookup_device_link("ZZZZ2345").await.unwrap().is_none());
        assert!(db
            .deny_device_link("ZZZZ2345", "linkmember")
            .await
            .unwrap()
            .is_none());
        assert!(db.lookup_device_link("NOPE2345").await.unwrap().is_none());
        assert!(matches!(
            db.poll_device_link(&"22".repeat(32), Utc::now())
                .await
                .unwrap(),
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
            db.poll_device_link(&"33".repeat(32), Utc::now())
                .await
                .unwrap(),
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

    #[tokio::test]
    async fn concurrent_polls_hand_the_secret_to_exactly_one() {
        let db = test_db().await;
        seed_member(&db).await;
        let user_code = "HJKM2345";
        let device_code = "ab".repeat(32);
        let secret = "cd".repeat(32);
        db.insert_device_link(user_code, &device_code, "Desk", "1.0.0")
            .await
            .unwrap();
        db.create_daemon_token("linkmember", &secret, "Desk")
            .await
            .unwrap();
        let tokens = db.list_daemon_tokens("linkmember").await.unwrap();
        assert!(db
            .approve_device_link(user_code, "linkmember", &tokens[0].id, &secret)
            .await
            .unwrap());

        let (left, right) = tokio::join!(
            db.poll_device_link(&device_code, Utc::now()),
            db.poll_device_link(&device_code, Utc::now()),
        );
        let outcomes = [left.unwrap(), right.unwrap()];
        let tokens: Vec<&str> = outcomes
            .iter()
            .filter_map(|outcome| match outcome {
                DeviceLinkPoll::Approved(token) => Some(token.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(tokens, vec![secret.as_str()], "{outcomes:?}");
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| matches!(outcome, DeviceLinkPoll::Expired))
                .count(),
            1,
            "{outcomes:?}"
        );
    }

    #[tokio::test]
    async fn cleanup_revokes_uncollected_tokens_and_keeps_collected_ones() {
        let db = test_db().await;
        seed_member(&db).await;
        let uncollected = "aa".repeat(32);
        let collected = "bb".repeat(32);
        db.insert_device_link("AAAA2345", &"11".repeat(32), "Uncollected", "1.0.0")
            .await
            .unwrap();
        db.insert_device_link("BBBB2345", &"22".repeat(32), "Collected", "1.0.0")
            .await
            .unwrap();
        db.create_daemon_token("linkmember", &uncollected, "Uncollected")
            .await
            .unwrap();
        db.create_daemon_token("linkmember", &collected, "Collected")
            .await
            .unwrap();
        let tokens = db.list_daemon_tokens("linkmember").await.unwrap();
        let uncollected_id = tokens
            .iter()
            .find(|token| token.label == "Uncollected")
            .unwrap()
            .id
            .clone();
        let collected_id = tokens
            .iter()
            .find(|token| token.label == "Collected")
            .unwrap()
            .id
            .clone();
        assert!(db
            .approve_device_link("AAAA2345", "linkmember", &uncollected_id, &uncollected)
            .await
            .unwrap());
        assert!(db
            .approve_device_link("BBBB2345", "linkmember", &collected_id, &collected)
            .await
            .unwrap());
        match db
            .poll_device_link(&"22".repeat(32), Utc::now())
            .await
            .unwrap()
        {
            DeviceLinkPoll::Approved(got) => assert_eq!(got, collected),
            other => panic!("expected the collected secret, got {other:?}"),
        }
        db.client
            .query("UPDATE device_link SET expires_at = time::now() - 1s")
            .await
            .unwrap()
            .check()
            .unwrap();
        let removed = db.cleanup_expired_device_links().await.unwrap();
        assert!(removed >= 2, "removed {removed}");
        let left = db.list_daemon_tokens("linkmember").await.unwrap();
        let uncollected_row = left
            .iter()
            .find(|token| token.id == uncollected_id)
            .unwrap();
        let collected_row = left.iter().find(|token| token.id == collected_id).unwrap();
        assert!(
            !uncollected_row.is_active,
            "an expired code that was never collected revokes its token"
        );
        assert!(
            collected_row.is_active,
            "a token the device already collected stays active"
        );
    }

    #[tokio::test]
    async fn cleanup_revoke_and_delete_share_one_cutoff() {
        let db = test_db().await;
        seed_member(&db).await;
        let early_secret = "aa".repeat(32);
        let late_secret = "bb".repeat(32);
        db.insert_device_link("EARLY234", &"11".repeat(32), "Early", "1.0.0")
            .await
            .unwrap();
        db.insert_device_link("LATE2345", &"22".repeat(32), "Late", "1.0.0")
            .await
            .unwrap();
        db.create_daemon_token("linkmember", &early_secret, "Early")
            .await
            .unwrap();
        db.create_daemon_token("linkmember", &late_secret, "Late")
            .await
            .unwrap();
        let tokens = db.list_daemon_tokens("linkmember").await.unwrap();
        let early_id = tokens
            .iter()
            .find(|token| token.label == "Early")
            .unwrap()
            .id
            .clone();
        let late_id = tokens
            .iter()
            .find(|token| token.label == "Late")
            .unwrap()
            .id
            .clone();
        assert!(db
            .approve_device_link("EARLY234", "linkmember", &early_id, &early_secret)
            .await
            .unwrap());
        assert!(db
            .approve_device_link("LATE2345", "linkmember", &late_id, &late_secret)
            .await
            .unwrap());

        // Both codes are still inside their wall-clock TTL. The cutoff is an
        // hour ahead, so only Early (15 minutes out) is in this pass. A delete
        // that called time::now() would remove nothing. A delete that used the
        // cutoff while the revoke select used time::now() would drop Early
        // and leave its token active.
        let now = Utc::now();
        db.client
            .query("UPDATE device_link SET expires_at = $at WHERE device_label = 'Early'")
            .bind(("at", surreal_at(now + chrono::Duration::minutes(15))))
            .await
            .unwrap()
            .check()
            .unwrap();
        db.client
            .query("UPDATE device_link SET expires_at = $at WHERE device_label = 'Late'")
            .bind(("at", surreal_at(now + chrono::Duration::hours(3))))
            .await
            .unwrap()
            .check()
            .unwrap();

        let removed = db
            .cleanup_expired_device_links_before(now + chrono::Duration::hours(1))
            .await
            .unwrap();
        assert_eq!(removed, 1, "only the code inside the cutoff is deleted");

        let left = db.list_daemon_tokens("linkmember").await.unwrap();
        let early_row = left.iter().find(|token| token.id == early_id).unwrap();
        let late_row = left.iter().find(|token| token.id == late_id).unwrap();
        assert!(
            !early_row.is_active,
            "the code inside the cutoff is revoked before it is deleted"
        );
        assert!(late_row.is_active, "a code past the cutoff keeps its token");

        let mut stored = db.client.query("SELECT * FROM device_link").await.unwrap();
        let rows: Vec<DbDeviceLink> = stored.take(0).unwrap();
        assert!(
            rows.iter().all(|row| row.device_label != "Early"),
            "Early is deleted, so its handover secret is gone"
        );
        let late = rows.iter().find(|row| row.device_label == "Late").unwrap();
        assert!(
            late.handover_token.is_some(),
            "Late is outside the cutoff and still holds its handover secret"
        );

        let removed_now = db.cleanup_expired_device_links().await.unwrap();
        assert_eq!(
            removed_now, 0,
            "wall-clock cleanup does not touch a code that expires later"
        );
    }
}
