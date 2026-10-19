use serde::{Deserialize, Serialize};
use surrealdb::types::Datetime as SurrealDatetime;
use surrealdb_types::{RecordId, SurrealValue};

use scuffed_auth::crypto::hash_session_token;

use crate::types::DaemonToken;
use crate::{with_timeout, Database, DbResult};

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbDaemonToken {
    #[surreal(default)]
    #[allow(dead_code)]
    id: Option<RecordId>,
    member_id: String,
    token_hash: String,
    label: String,
    is_active: bool,
    created_at: SurrealDatetime,
    last_used_at: Option<SurrealDatetime>,
    /// NONE means the token does not expire. Read by the validate query, not Rust.
    #[serde(default)]
    #[surreal(default)]
    #[allow(dead_code)]
    expires_at: Option<SurrealDatetime>,
}

fn db_to_token(db: DbDaemonToken) -> DaemonToken {
    let id = db
        .id
        .map(|r| crate::record_id_key_to_string(r.key))
        .unwrap_or_else(|| "unknown".to_string());
    DaemonToken {
        id,
        member_id: db.member_id,
        label: db.label,
        is_active: db.is_active,
        created_at: db.created_at.into(),
        last_used_at: db.last_used_at.map(|d| d.into()),
    }
}

/// Active, unexpired daemon token. Shared by the stamp and the read-only lookup.
const ACTIVE_DAEMON_SQL: &str =
    "SELECT * FROM daemon_token WHERE token_hash = $tok AND is_active = true \
     AND (expires_at IS NONE OR expires_at > time::now())";

/// Active token whose `expires_at` is set and not in the future.
/// `NONE` is not expired. Does not include revoked rows.
const EXPIRED_DAEMON_SQL: &str =
    "SELECT * FROM daemon_token WHERE token_hash = $tok AND is_active = true \
     AND expires_at IS NOT NONE AND expires_at <= time::now()";

/// Result of checking a presented daemon token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DaemonTokenAuth {
    /// Active member id. `last_used_at` was stamped.
    Active(String),
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
            let mut result = self
                .client
                .query(ACTIVE_DAEMON_SQL)
                .bind(("tok", token_hash.clone()))
                .await?;
            let tokens: Vec<DbDaemonToken> = result.take(0)?;
            if let Some(tok) = tokens.into_iter().next() {
                self.client
                    .query("UPDATE daemon_token SET last_used_at = time::now() WHERE token_hash = $tok")
                    .bind(("tok", token_hash))
                    .await?;
                return Ok(DaemonTokenAuth::Active(tok.member_id));
            }
            let mut expired = self
                .client
                .query(EXPIRED_DAEMON_SQL)
                .bind(("tok", token_hash))
                .await?;
            let expired_rows: Vec<DbDaemonToken> = expired.take(0)?;
            if expired_rows.is_empty() {
                Ok(DaemonTokenAuth::Missing)
            } else {
                Ok(DaemonTokenAuth::Expired)
            }
        })
        .await
    }

    /// Same match as [`Database::validate_daemon_token`] without writing `last_used_at`.
    ///
    /// Pack downloads use this so a file fetch does not touch the database.
    pub async fn lookup_daemon_token(&self, raw_token: &str) -> DbResult<Option<String>> {
        with_timeout(async {
            let token_hash = hash_session_token(raw_token);
            let mut result = self
                .client
                .query(ACTIVE_DAEMON_SQL)
                .bind(("tok", token_hash))
                .await?;
            let tokens: Vec<DbDaemonToken> = result.take(0)?;
            Ok(tokens.into_iter().next().map(|tok| tok.member_id))
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
        assert_eq!(
            db.validate_daemon_token("fresh-token").await.unwrap(),
            DaemonTokenAuth::Active("member-1".into())
        );
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

        assert_eq!(
            db.validate_daemon_token("none-token").await.unwrap(),
            DaemonTokenAuth::Active("member-1".into())
        );
        assert!(expiry_row(&db, "none").await.last_used_at.is_some());
        assert_eq!(
            db.validate_daemon_token("old-token").await.unwrap(),
            DaemonTokenAuth::Expired
        );
        assert!(expiry_row(&db, "old").await.last_used_at.is_none());
    }
}
