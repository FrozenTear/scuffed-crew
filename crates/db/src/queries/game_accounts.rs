use chrono::Utc;
use serde::{Deserialize, Serialize};
use surrealdb::types::Datetime as SurrealDatetime;
use surrealdb_types::RecordId;
use surrealdb_types::SurrealValue;

use crate::types::GameAccount;
use crate::{with_timeout, Database, DbResult};

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbGameAccount {
    #[surreal(default)]
    #[allow(dead_code)]
    id: Option<RecordId>,
    member_id: String,
    game_id: String,
    account_name: String,
    account_id: Option<String>,
    created_at: SurrealDatetime,
    #[serde(default)]
    #[surreal(default)]
    rank: Option<String>,
    #[serde(default)]
    #[surreal(default)]
    sr: Option<u32>,
    #[serde(default)]
    #[surreal(default)]
    role: Option<String>,
}

fn db_to_game_account(db: DbGameAccount) -> GameAccount {
    let id = db
        .id
        .map(|r| crate::record_id_key_to_string(r.key))
        .unwrap_or_else(|| "unknown".to_string());
    GameAccount {
        id,
        member_id: db.member_id,
        game_id: db.game_id,
        account_name: db.account_name,
        account_id: db.account_id,
        created_at: db.created_at.into(),
        rank: db.rank,
        sr: db.sr,
        role: db.role,
    }
}

impl Database {
    /// Create or update a game account for a member (upsert by member_id + game_id).
    ///
    /// `rank` / `sr` / `role` use `Option<Option<_>>`: `None` leaves existing values,
    /// `Some(None)` clears, `Some(Some(v))` sets.
    pub async fn upsert_game_account(
        &self,
        member_id: &str,
        game_id: &str,
        account_name: &str,
        account_id: Option<&str>,
        rank: Option<Option<&str>>,
        sr: Option<Option<u32>>,
        role: Option<Option<&str>>,
    ) -> DbResult<GameAccount> {
        with_timeout(async {
            let now = SurrealDatetime::from(Utc::now());
            // Load existing so we can preserve rank/sr/role when not provided.
            let mut existing_q = self
                .client
                .query(
                    "SELECT * FROM game_account WHERE member_id = $mid AND game_id = $gid LIMIT 1",
                )
                .bind(("mid", member_id.to_string()))
                .bind(("gid", game_id.to_string()))
                .await?;
            let existing: Vec<DbGameAccount> = existing_q.take(0)?;
            let existing = existing.into_iter().next();

            let next_rank = match rank {
                Some(v) => v.map(|s| s.to_string()),
                None => existing.as_ref().and_then(|e| e.rank.clone()),
            };
            let next_sr = match sr {
                Some(v) => v,
                None => existing.as_ref().and_then(|e| e.sr),
            };
            let next_role = match role {
                Some(v) => v.map(|s| s.to_string()),
                None => existing.as_ref().and_then(|e| e.role.clone()),
            };

            let mut result = self
                .client
                .query(
                    r#"
                    LET $existing = (SELECT * FROM game_account WHERE member_id = $mid AND game_id = $gid LIMIT 1);
                    IF array::len($existing) > 0 {
                        UPDATE $existing[0].id SET
                            account_name = $aname,
                            account_id = $aid,
                            rank = $rank,
                            sr = $sr,
                            role = $role
                        ;
                    } ELSE {
                        CREATE game_account SET
                            member_id = $mid,
                            game_id = $gid,
                            account_name = $aname,
                            account_id = $aid,
                            created_at = $cat,
                            rank = $rank,
                            sr = $sr,
                            role = $role
                        ;
                    };
                    "#,
                )
                .bind(("mid", member_id.to_string()))
                .bind(("gid", game_id.to_string()))
                .bind(("aname", account_name.to_string()))
                .bind(("aid", account_id.map(|s| s.to_string())))
                .bind(("cat", now))
                .bind(("rank", next_rank))
                .bind(("sr", next_sr))
                .bind(("role", next_role))
                .await?;

            // The result of the IF/ELSE is at index 1
            let accounts: Vec<DbGameAccount> = result.take(1)?;
            accounts
                .into_iter()
                .next()
                .map(db_to_game_account)
                .ok_or_else(|| crate::DbError::NotFound("Failed to upsert game account".into()))
        })
        .await
    }

    /// List all game accounts for a member.
    pub async fn list_member_game_accounts(&self, member_id: &str) -> DbResult<Vec<GameAccount>> {
        with_timeout(async {
            let mut result = self
                .client
                .query("SELECT * FROM game_account WHERE member_id = $mid ORDER BY game_id ASC")
                .bind(("mid", member_id.to_string()))
                .await?;
            let accounts: Vec<DbGameAccount> = result.take(0)?;
            Ok(accounts.into_iter().map(db_to_game_account).collect())
        })
        .await
    }

    /// Delete a game account if it belongs to `member_id`.
    ///
    /// Returns [`crate::DbError::NotFound`] when the id is missing **or** owned
    /// by a different member (no existence leak across members).
    pub async fn delete_game_account(&self, member_id: &str, id: &str) -> DbResult<()> {
        with_timeout(async {
            let existing: Option<DbGameAccount> = self.client.select(("game_account", id)).await?;
            let existing = existing
                .ok_or_else(|| crate::DbError::NotFound(format!("Game account {id} not found")))?;
            if existing.member_id != member_id {
                return Err(crate::DbError::NotFound(format!(
                    "Game account {id} not found"
                )));
            }
            let _: Option<DbGameAccount> = self.client.delete(("game_account", id)).await?;
            Ok(())
        })
        .await
    }
}
