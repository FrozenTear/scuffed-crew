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
                .await?
                .check()?;

            // Cap concurrent live sessions so the insert below lands at
            // MAX_SESSIONS_PER_USER. SurrealDB v3 DELETE accepts WHERE only —
            // `ORDER BY` / `LIMIT` are a parse error (`Unexpected token ORDER`),
            // which made the 11th login 500. Select the oldest ids, then delete
            // those records.
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
                .await?
                .check()?;
            let counts: Vec<CountResult> = count_q.take(0)?;
            let n = counts.first().map(|c| c.count).unwrap_or(0);
            let room = MAX_SESSIONS_PER_USER - 1;
            if n > room {
                let excess = n - room;
                #[derive(Deserialize, SurrealValue)]
                struct IdRow {
                    id: RecordId,
                    // SurrealDB v3 requires every ORDER BY field in the projection.
                    #[allow(dead_code)]
                    created_at: SurrealDatetime,
                }
                let mut oldest = self
                    .client
                    .query(
                        "SELECT id, created_at FROM session WHERE user_id = $uid \
                         AND expires_at > time::now() \
                         ORDER BY created_at ASC LIMIT $excess",
                    )
                    .bind(("uid", uid.clone()))
                    .bind(("excess", excess))
                    .await?
                    .check()?;
                let rows: Vec<IdRow> = oldest.take(0)?;
                if rows.is_empty() {
                    tracing::error!(user_id = %uid, n, "session cap eviction selected no rows");
                    return Err(crate::DbError::NotFound(
                        "session cap eviction selected no rows".into(),
                    ));
                }
                for row in rows {
                    self.client
                        .query("DELETE $rid")
                        .bind(("rid", row.id))
                        .await?
                        .check()?;
                }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations::run_migrations;

    async fn test_db() -> Database {
        let db = Database::connect_memory().await.expect("mem db");
        run_migrations(&db.client).await.expect("migrations");
        db
    }

    async fn seed_live_session(db: &Database, user_id: &str, raw_token: &str, age_secs: i64) {
        let session = DbSession {
            id: None,
            user_id: user_id.to_string(),
            token: hash_session_token(raw_token),
            expires_at: SurrealDatetime::from(Utc::now() + chrono::Duration::hours(48)),
            created_at: SurrealDatetime::from(Utc::now() - chrono::Duration::seconds(age_secs)),
        };
        let created: Option<DbSession> = db
            .client
            .create("session")
            .content(session)
            .await
            .expect("seed session");
        assert!(created.is_some());
    }

    async fn live_count(db: &Database, user_id: &str) -> i64 {
        #[derive(Deserialize, SurrealValue)]
        struct CountResult {
            count: i64,
        }
        let mut q = db
            .client
            .query(
                "SELECT count() FROM session WHERE user_id = $uid \
                 AND expires_at > time::now() GROUP ALL",
            )
            .bind(("uid", user_id.to_string()))
            .await
            .expect("count")
            .check()
            .expect("count ok");
        let counts: Vec<CountResult> = q.take(0).expect("take count");
        counts.first().map(|c| c.count).unwrap_or(0)
    }

    #[tokio::test]
    async fn create_session_under_cap_keeps_existing() {
        let db = test_db().await;
        for i in 0..9 {
            seed_live_session(&db, "user-a", &format!("old-{i}"), 30 - i).await;
        }
        db.create_session("user-a", "new-tok", 24)
            .await
            .expect("under cap");
        assert_eq!(live_count(&db, "user-a").await, 10);
        assert_eq!(
            db.get_session("old-0").await.expect("lookup").as_deref(),
            Some("user-a")
        );
        assert_eq!(
            db.get_session("new-tok").await.expect("lookup").as_deref(),
            Some("user-a")
        );
    }

    #[tokio::test]
    async fn create_session_at_cap_evicts_oldest_and_succeeds() {
        let db = test_db().await;
        // age_secs 20 is oldest; 11 is newest of the seeded set.
        for i in 0..10 {
            seed_live_session(&db, "user-a", &format!("old-{i}"), 20 - i).await;
        }
        assert_eq!(live_count(&db, "user-a").await, 10);

        db.create_session("user-a", "new-tok", 24)
            .await
            .expect("at cap must not 500");

        assert_eq!(live_count(&db, "user-a").await, 10);
        assert!(
            db.get_session("old-0").await.expect("lookup").is_none(),
            "oldest live session must be evicted"
        );
        assert_eq!(
            db.get_session("old-1").await.expect("lookup").as_deref(),
            Some("user-a")
        );
        assert_eq!(
            db.get_session("new-tok").await.expect("lookup").as_deref(),
            Some("user-a")
        );
    }

    #[tokio::test]
    async fn create_session_over_cap_evicts_down_to_limit() {
        let db = test_db().await;
        for i in 0..12 {
            seed_live_session(&db, "user-a", &format!("old-{i}"), 30 - i).await;
        }
        db.create_session("user-a", "new-tok", 24)
            .await
            .expect("over cap");
        assert_eq!(live_count(&db, "user-a").await, 10);
        for i in 0..3 {
            assert!(
                db.get_session(&format!("old-{i}"))
                    .await
                    .expect("lookup")
                    .is_none(),
                "old-{i} should have been evicted"
            );
        }
        assert_eq!(
            db.get_session("old-3").await.expect("lookup").as_deref(),
            Some("user-a")
        );
        assert_eq!(
            db.get_session("new-tok").await.expect("lookup").as_deref(),
            Some("user-a")
        );
    }
}
