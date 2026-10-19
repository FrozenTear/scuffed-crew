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
                expires_at: None,
            };
            let created: Option<DbDaemonToken> =
                self.client.create("daemon_token").content(db_tok).await?;
            Ok(db_to_token(created.ok_or_else(|| {
                crate::DbError::NotFound("Failed to create daemon token".into())
            })?))
        })
        .await
    }

    pub async fn validate_daemon_token(&self, raw_token: &str) -> DbResult<Option<String>> {
        with_timeout(async {
            let token_hash = hash_session_token(raw_token);
            let mut result = self
                .client
                .query(ACTIVE_DAEMON_SQL)
                .bind(("tok", token_hash.clone()))
                .await?;
            let tokens: Vec<DbDaemonToken> = result.take(0)?;
            if let Some(tok) = tokens.into_iter().next() {
                if crate::is_tombstone_member(&tok.member_id) {
                    return Ok(None);
                }
                self.client
                    .query("UPDATE daemon_token SET last_used_at = time::now() WHERE token_hash = $tok")
                    .bind(("tok", token_hash))
                    .await?;
                Ok(Some(tok.member_id))
            } else {
                Ok(None)
            }
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
