use serde::{Deserialize, Serialize};
use surrealdb::types::Datetime as SurrealDatetime;
use surrealdb_types::{RecordId, SurrealValue};

use scuffed_auth::crypto::hash_session_token;

use crate::types::DaemonToken;
use crate::{with_timeout, Database, DbResult};

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbDaemonToken {
    #[surreal(default)]
    id: Option<RecordId>,
    member_id: String,
    token_hash: String,
    label: String,
    is_active: bool,
    created_at: SurrealDatetime,
    last_used_at: Option<SurrealDatetime>,
    /// NONE means the token does not expire.
    #[serde(default)]
    #[surreal(default)]
    expires_at: Option<SurrealDatetime>,
}

fn daemon_token_id(id: Option<RecordId>) -> String {
    id.map(|record| crate::record_id_key_to_string(record.key))
        .unwrap_or_else(|| "unknown".to_string())
}

fn expires_at_utc(value: Option<SurrealDatetime>) -> Option<chrono::DateTime<chrono::Utc>> {
    value.map(chrono::DateTime::from)
}

/// Same rule as the list endpoint: a missing expiry is not expired.
fn expiry_elapsed(expires_at: Option<chrono::DateTime<chrono::Utc>>) -> bool {
    expires_at.is_some_and(|at| at <= chrono::Utc::now())
}

fn db_to_token(db: DbDaemonToken) -> DaemonToken {
    let id = daemon_token_id(db.id);
    let expires_at = expires_at_utc(db.expires_at);
    let expired = expiry_elapsed(expires_at);
    DaemonToken {
        id,
        member_id: db.member_id,
        label: db.label,
        is_active: db.is_active,
        created_at: db.created_at.into(),
        last_used_at: db.last_used_at.map(|d| d.into()),
        expires_at,
        expired,
    }
}

/// Active, unexpired daemon token. Shared by the stamp and the read-only lookup.
const ACTIVE_DAEMON_SQL: &str =
    "SELECT * FROM daemon_token WHERE token_hash = $tok AND is_active = true \
     AND (expires_at IS NONE OR expires_at > time::now())";

/// Result of checking a presented daemon token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DaemonTokenAuth {
    /// Active token. `last_used_at` was stamped.
    Active { member_id: String, token_id: String },
    /// Active row with `expires_at` at or before now. `last_used_at` was not stamped.
    Expired,
    /// Unknown, inactive, or otherwise not a usable token.
    Missing,
}

impl Database {
    pub async fn create_daemon_token(
        &self,
        member_id: &str,
        raw_token: &str,
        label: &str,
    ) -> DbResult<DaemonToken> {
        if crate::is_tombstone_member(member_id) {
            return Err(crate::DbError::Conflict("tombstone".into()));
        }
        with_timeout(async {
            let token_hash = hash_session_token(raw_token);
            let db_tok = DbDaemonToken {
                id: None,
                member_id: member_id.to_string(),
                token_hash,
                label: label.to_string(),
                is_active: true,
                created_at: SurrealDatetime::from(chrono::Utc::now()),
                last_used_at: None,
                expires_at: Some(SurrealDatetime::from(
                    chrono::Utc::now()
                        + chrono::Duration::days(scuffed_types::DAEMON_TOKEN_TTL_DAYS),
                )),
            };
            let created: Option<DbDaemonToken> =
                self.client.create("daemon_token").content(db_tok).await?;
            Ok(db_to_token(created.ok_or_else(|| {
                crate::DbError::NotFound("Failed to create daemon token".into())
            })?))
        })
        .await
    }

    pub async fn validate_daemon_token(&self, raw_token: &str) -> DbResult<DaemonTokenAuth> {
        with_timeout(async {
            let token_hash = hash_session_token(raw_token);
            // One read. Expiry is decided in Rust from that row, so a token
            // that crosses `expires_at` cannot come back as missing.
            let mut result = self
                .client
                .query("SELECT * FROM daemon_token WHERE token_hash = $tok AND is_active = true")
                .bind(("tok", token_hash.clone()))
                .await?;
            let tokens: Vec<DbDaemonToken> = result.take(0)?;
            let Some(tok) = tokens.into_iter().next() else {
                return Ok(DaemonTokenAuth::Missing);
            };
            if crate::is_tombstone_member(&tok.member_id) {
                return Ok(DaemonTokenAuth::Missing);
            }
            if expiry_elapsed(expires_at_utc(tok.expires_at)) {
                return Ok(DaemonTokenAuth::Expired);
            }
            // The token is valid; `last_used_at` is bookkeeping. A failed
            // touch (e.g. a transaction conflict between two uploads on one
            // token) is logged, not turned into an auth failure.
            if let Err(e) = self
                .client
                .query("UPDATE daemon_token SET last_used_at = time::now() WHERE token_hash = $tok")
                .bind(("tok", token_hash))
                .await?
                .check()
            {
                tracing::warn!(member_id = %tok.member_id, "daemon token last_used_at touch failed: {e}");
            }
            Ok(DaemonTokenAuth::Active {
                member_id: tok.member_id,
                token_id: daemon_token_id(tok.id),
            })
        })
        .await
    }

    /// Deadline when this bearer was revoked while a stats delete is still pending.
    ///
    /// Active tokens are not this path: [`Database::validate_daemon_token`]
    /// accepts them, and the upload handler returns the 409. A hash that does
    /// not match, or a member whose deadline was cleared by restore, is
    /// `None` so the caller answers 401. The lookup does not stamp `last_used_at`.
    pub async fn stats_delete_deadline_for_token(
        &self,
        raw_token: &str,
    ) -> DbResult<Option<chrono::DateTime<chrono::Utc>>> {
        with_timeout(async {
            let token_hash = hash_session_token(raw_token);
            #[derive(Deserialize, SurrealValue)]
            struct Tok {
                member_id: String,
            }
            let mut result = self
                .client
                .query("SELECT member_id FROM daemon_token WHERE token_hash = $tok LIMIT 1")
                .bind(("tok", token_hash))
                .await?;
            let tokens: Vec<Tok> = result.take(0)?;
            let Some(tok) = tokens.into_iter().next() else {
                return Ok(None);
            };
            if crate::is_tombstone_member(&tok.member_id) {
                return Ok(None);
            }
            #[derive(Deserialize, SurrealValue)]
            struct Deadline {
                #[serde(default)]
                #[surreal(default)]
                stats_delete_scheduled_at: Option<SurrealDatetime>,
                #[serde(default)]
                #[surreal(default)]
                is_tombstone: bool,
            }
            let mut member = self
                .client
                .query("SELECT stats_delete_scheduled_at, is_tombstone FROM $rid")
                .bind(("rid", RecordId::new("member", tok.member_id.as_str())))
                .await?;
            let rows: Vec<Deadline> = member.take(0)?;
            let Some(row) = rows.into_iter().next() else {
                return Ok(None);
            };
            if row.is_tombstone {
                return Ok(None);
            }
            Ok(row.stats_delete_scheduled_at.map(Into::into))
        })
        .await
    }

    /// Active, unexpired token, without writing `last_used_at`.
    ///
    /// Pack downloads use this so a file fetch does not touch the database.
    /// An expired token looks the same as a missing one.
    pub async fn lookup_daemon_token(&self, raw_token: &str) -> DbResult<Option<String>> {
        with_timeout(async {
            let token_hash = hash_session_token(raw_token);
            let mut result = self
                .client
                .query(ACTIVE_DAEMON_SQL)
                .bind(("tok", token_hash))
                .await?;
            let tokens: Vec<DbDaemonToken> = result.take(0)?;
            Ok(tokens.into_iter().next().and_then(|tok| {
                if crate::is_tombstone_member(&tok.member_id) {
                    None
                } else {
                    Some(tok.member_id)
                }
            }))
        })
        .await
    }

    pub async fn list_daemon_tokens(&self, member_id: &str) -> DbResult<Vec<DaemonToken>> {
        with_timeout(async {
            let mut result = self
                .client
                .query("SELECT * FROM daemon_token WHERE member_id = $mid ORDER BY created_at DESC")
                .bind(("mid", member_id.to_string()))
                .await?;
            let tokens: Vec<DbDaemonToken> = result.take(0)?;
            Ok(tokens.into_iter().map(db_to_token).collect())
        })
        .await
    }

    pub async fn revoke_daemon_token(&self, token_id: &str, member_id: &str) -> DbResult<()> {
        with_timeout(async {
            let mut result = self
                .client
                .query("UPDATE daemon_token SET is_active = false WHERE id = $rid AND member_id = $mid")
                .bind(("rid", RecordId::new("daemon_token", token_id)))
                .bind(("mid", member_id.to_string()))
                .await?;
            let updated: Vec<DbDaemonToken> = result.take(0)?;
            if updated.is_empty() {
                return Err(crate::DbError::NotFound(format!(
                    "Daemon token {token_id} not found"
                )));
            }
            Ok(())
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Duration, Utc};
    use serde::Deserialize;
    use surrealdb::types::Datetime as SurrealDatetime;
    use surrealdb_types::SurrealValue;

    use super::*;
    use crate::migrations::run_migrations;
    use crate::Database;
    use scuffed_auth::crypto::hash_session_token;
    use scuffed_types::DAEMON_TOKEN_TTL_DAYS;

    async fn test_db() -> Database {
        let db = Database::connect_memory().await.expect("mem db");
        run_migrations(&db.client).await.expect("migrations");
        db
    }

    #[derive(Debug, Deserialize, SurrealValue)]
    struct ExpiryRow {
        expires_at: Option<SurrealDatetime>,
        last_used_at: Option<SurrealDatetime>,
    }

    async fn expiry_row(db: &Database, label: &str) -> ExpiryRow {
        let mut query = db
            .client
            .query("SELECT expires_at, last_used_at FROM daemon_token WHERE label = $label")
            .bind(("label", label.to_string()))
            .await
            .unwrap();
        let rows: Vec<ExpiryRow> = query.take(0).unwrap();
        rows.into_iter().next().expect("daemon token row")
    }

    #[tokio::test]
    async fn new_daemon_token_expires_in_about_180_days() {
        let db = test_db().await;
        let before = Utc::now();
        db.create_daemon_token("member-1", "fresh-token", "fresh")
            .await
            .unwrap();
        let after = Utc::now();
        let stored = expiry_row(&db, "fresh")
            .await
            .expires_at
            .expect("expires_at");
        let expires: DateTime<Utc> = stored.into();
        let ttl = Duration::days(DAEMON_TOKEN_TTL_DAYS);
        assert!(
            expires >= before + ttl - Duration::seconds(2),
            "expires {expires} is early"
        );
        assert!(
            expires <= after + ttl + Duration::seconds(2),
            "expires {expires} is late"
        );
        match db.validate_daemon_token("fresh-token").await.unwrap() {
            DaemonTokenAuth::Active {
                member_id,
                token_id,
            } => {
                assert_eq!(member_id, "member-1");
                assert!(!token_id.is_empty());
            }
            other => panic!("expected active, got {other:?}"),
        }
    }

    async fn insert_raw(db: &Database, raw: &str, label: &str, expires_sql: &str) {
        let token_hash = hash_session_token(raw);
        db.client
            .query(format!(
                "CREATE daemon_token SET \
                    member_id = 'member-1', \
                    token_hash = $tok, \
                    label = $label, \
                    is_active = true, \
                    created_at = time::now(), \
                    expires_at = {expires_sql}"
            ))
            .bind(("tok", token_hash))
            .bind(("label", label.to_string()))
            .await
            .unwrap()
            .check()
            .unwrap();
    }

    #[tokio::test]
    async fn none_expiry_stays_active_and_past_expiry_does_not_stamp() {
        let db = test_db().await;
        insert_raw(&db, "none-token", "none", "NONE").await;
        insert_raw(&db, "old-token", "old", "time::now() - 1d").await;

        match db.validate_daemon_token("none-token").await.unwrap() {
            DaemonTokenAuth::Active { member_id, .. } => assert_eq!(member_id, "member-1"),
            other => panic!("expected active, got {other:?}"),
        }
        assert!(expiry_row(&db, "none").await.last_used_at.is_some());
        assert_eq!(
            db.validate_daemon_token("old-token").await.unwrap(),
            DaemonTokenAuth::Expired
        );
        assert!(expiry_row(&db, "old").await.last_used_at.is_none());
    }
}
