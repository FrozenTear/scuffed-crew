use chrono::Utc;
use serde::{Deserialize, Serialize};
use surrealdb::types::Datetime as SurrealDatetime;
use surrealdb_types::RecordId;
use surrealdb_types::SurrealValue;

use scuffed_auth::crypto::hash_session_token;
use scuffed_auth::User;

use crate::{with_timeout, Database, DbResult};

/// Internal DB representation of a session.
/// Field `token` stores a BLAKE3 hash of the raw session secret (never the secret itself).
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

/// Soft cap on concurrent sessions per user (oldest deleted when exceeded).
const MAX_SESSIONS_PER_USER: i64 = 10;

impl Database {
    /// Create a new session for a user. The raw token is hashed before storage.
    ///
    /// Enforces [`MAX_SESSIONS_PER_USER`]: if the user already has that many
    /// non-expired sessions, the oldest are deleted first (session inventory limit).
    pub async fn create_session(
        &self,
        user_id: &str,
        raw_token: &str,
        duration_hours: i64,
    ) -> DbResult<()> {
        with_timeout(async {
            let uid = user_id.to_string();

            // Drop expired sessions for this user.
            self.client
                .query("DELETE FROM session WHERE user_id = $uid AND expires_at <= time::now()")
                .bind(("uid", uid.clone()))
                .await?;

            // Cap concurrent live sessions: delete oldest until under limit.
            loop {
                #[derive(Deserialize, SurrealValue)]
                struct CountResult {
                    count: i64,
                }
                let mut count_q = self
                    .client
                    .query(
                        "SELECT count() FROM session WHERE user_id = $uid \
                         AND expires_at > time::now() GROUP ALL",
                    )
                    .bind(("uid", uid.clone()))
                    .await?;
                let counts: Vec<CountResult> = count_q.take(0)?;
                let n = counts.first().map(|c| c.count).unwrap_or(0);
                if n < MAX_SESSIONS_PER_USER {
                    break;
                }
                // Delete a single oldest live session.
                self.client
                    .query(
                        "DELETE FROM session WHERE user_id = $uid AND expires_at > time::now() \
                         ORDER BY created_at ASC LIMIT 1",
                    )
                    .bind(("uid", uid.clone()))
                    .await?;
            }

            let token_hash = hash_session_token(raw_token);
            let session = DbSession {
                id: None,
                user_id: uid,
                token: token_hash,
                expires_at: SurrealDatetime::from(
                    Utc::now() + chrono::Duration::hours(duration_hours),
                ),
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
                    "SELECT user_id FROM session WHERE token = $tok AND expires_at > time::now() LIMIT 1",
                )
                .bind(("tok", token_hash))
                .await?;
            #[derive(Deserialize, SurrealValue)]
            struct Row {
                user_id: String,
            }
            let sessions: Vec<Row> = result.take(0)?;
            Ok(sessions.into_iter().next().map(|s| s.user_id))
        })
        .await
    }

    /// Resolve a session token to a [`User`] under a **single** outer timeout.
    ///
    /// Session + user selects run inside one `with_timeout` so auth cannot stack
    /// two full 10s budgets.
    pub async fn get_session_user(&self, raw_token: &str) -> DbResult<Option<User>> {
        with_timeout(async {
            let token_hash = hash_session_token(raw_token);
            let mut result = self
                .client
                .query(
                    "SELECT user_id FROM session WHERE token = $tok AND expires_at > time::now() LIMIT 1",
                )
                .bind(("tok", token_hash))
                .await?;
            #[derive(Deserialize, SurrealValue)]
            struct Row {
                user_id: String,
            }
            let sessions: Vec<Row> = result.take(0)?;
            let Some(uid) = sessions.into_iter().next().map(|s| s.user_id) else {
                return Ok(None);
            };
            // Inline user select (do not call get_user — that wraps another timeout).
            self.get_user_without_timeout(&uid).await
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

    /// Delete all sessions for a user (ban / deactivate / force logout).
    /// Returns an approximate count of deleted sessions when available.
    pub async fn delete_sessions_for_user(&self, user_id: &str) -> DbResult<u64> {
        with_timeout(async {
            #[derive(Deserialize, SurrealValue)]
            struct CountResult {
                count: u64,
            }

            let mut result = self
                .client
                .query("SELECT count() FROM session WHERE user_id = $uid GROUP ALL")
                .bind(("uid", user_id.to_string()))
                .await?;
            let counts: Vec<CountResult> = result.take(0)?;
            let count = counts.first().map(|c| c.count).unwrap_or(0);

            if count > 0 {
                self.client
                    .query("DELETE FROM session WHERE user_id = $uid")
                    .bind(("uid", user_id.to_string()))
                    .await?;
                tracing::info!(user_id, count, "Revoked all sessions for user");
            }

            Ok(count)
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
                .query("SELECT count() FROM session WHERE expires_at <= time::now() GROUP ALL")
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
