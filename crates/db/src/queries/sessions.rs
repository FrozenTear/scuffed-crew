use chrono::Utc;
use serde::{Deserialize, Serialize};
use surrealdb_types::RecordId;
use surrealdb::types::Datetime as SurrealDatetime;
use surrealdb_types::SurrealValue;

use scuffed_auth::crypto::hash_session_token;

use crate::{with_timeout, Database, DbResult};

/// Internal DB representation of a session.
#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbSession {
    #[surreal(default)]
    #[allow(dead_code)]
    id: Option<RecordId>,
    user_id: String,
    token: String,
    expires_at: SurrealDatetime,
    created_at: SurrealDatetime,
}

impl Database {
    /// Create a new session for a user. The raw token is hashed before storage.
    pub async fn create_session(
        &self,
        user_id: &str,
        raw_token: &str,
        duration_hours: i64,
    ) -> DbResult<()> {
        with_timeout(async {
            let token_hash = hash_session_token(raw_token);
            let session = DbSession {
                id: None,
                user_id: user_id.to_string(),
                token: token_hash,
                expires_at: SurrealDatetime::from(Utc::now() + chrono::Duration::hours(duration_hours)),
                created_at: SurrealDatetime::from(Utc::now()),
            };
            let _: Option<DbSession> = self.client.create("session").content(session).await?;
            Ok(())
        })
        .await
    }

    /// Look up a valid (non-expired) session by raw token. Returns the user_id.
    pub async fn get_session(&self, raw_token: &str) -> DbResult<Option<String>> {
        with_timeout(async {
            let token_hash = hash_session_token(raw_token);
            let mut result = self
                .client
                .query(
                    "SELECT * FROM session WHERE token = $tok AND expires_at > time::now()",
                )
                .bind(("tok", token_hash))
                .await?;
            let sessions: Vec<DbSession> = result.take(0)?;
            Ok(sessions.into_iter().next().map(|s| s.user_id))
        })
        .await
    }

    /// Delete a session by raw token (logout).
    pub async fn delete_session(&self, raw_token: &str) -> DbResult<()> {
        with_timeout(async {
            let token_hash = hash_session_token(raw_token);
            self.client
                .query("DELETE FROM session WHERE token = $tok")
                .bind(("tok", token_hash))
                .await?;
            Ok(())
        })
        .await
    }

    /// Delete all expired sessions. Returns count removed.
    pub async fn cleanup_expired_sessions(&self) -> DbResult<u64> {
        with_timeout(async {
            #[derive(Deserialize, SurrealValue)]
            struct CountResult {
                count: u64,
            }

            let mut result = self
                .client
                .query(
                    "SELECT count() FROM session WHERE expires_at <= time::now() GROUP ALL",
                )
                .await?;
            let counts: Vec<CountResult> = result.take(0)?;
            let count = counts.first().map(|c| c.count).unwrap_or(0);

            if count > 0 {
                self.client
                    .query("DELETE FROM session WHERE expires_at <= time::now()")
                    .await?;
                tracing::info!("Cleaned up {count} expired sessions");
            }

            Ok(count)
        })
        .await
    }
}
