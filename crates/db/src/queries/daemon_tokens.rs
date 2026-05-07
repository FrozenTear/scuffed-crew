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
                .query("SELECT * FROM daemon_token WHERE token_hash = $tok AND is_active = true")
                .bind(("tok", token_hash.clone()))
                .await?;
            let tokens: Vec<DbDaemonToken> = result.take(0)?;
            if let Some(tok) = tokens.into_iter().next() {
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
