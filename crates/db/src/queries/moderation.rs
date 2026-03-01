use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use surrealdb::sql::Thing;
use surrealdb::sql::Datetime as SurrealDatetime;

use crate::types::{ModerationAction, ModerationActionType};
use crate::{with_timeout, Database, DbResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DbModerationAction {
    #[serde(skip_serializing)]
    #[allow(dead_code)]
    id: Option<Thing>,
    member_id: String,
    action_type: String,
    reason: String,
    issued_by: String,
    expires_at: Option<SurrealDatetime>,
    is_active: bool,
    created_at: SurrealDatetime,
}

fn parse_action_type(s: &str) -> ModerationActionType {
    match s {
        "note" => ModerationActionType::Note,
        "warning" => ModerationActionType::Warning,
        "suspension" => ModerationActionType::Suspension,
        "ban" => ModerationActionType::Ban,
        _ => ModerationActionType::Note,
    }
}

fn db_to_action(db: DbModerationAction) -> ModerationAction {
    let id = db
        .id
        .map(|t| t.id.to_raw())
        .unwrap_or_else(|| "unknown".to_string());
    ModerationAction {
        id,
        member_id: db.member_id,
        action_type: parse_action_type(&db.action_type),
        reason: db.reason,
        issued_by: db.issued_by,
        expires_at: db.expires_at.map(|d| d.into()),
        is_active: db.is_active,
        created_at: db.created_at.into(),
    }
}

impl Database {
    /// Create a new moderation action.
    pub async fn create_moderation_action(
        &self,
        member_id: &str,
        action_type: ModerationActionType,
        reason: &str,
        issued_by: &str,
        expires_at: Option<DateTime<Utc>>,
    ) -> DbResult<ModerationAction> {
        with_timeout(async {
            let entry = DbModerationAction {
                id: None,
                member_id: member_id.to_string(),
                action_type: action_type.to_string(),
                reason: reason.to_string(),
                issued_by: issued_by.to_string(),
                expires_at: expires_at.map(SurrealDatetime::from),
                is_active: true,
                created_at: SurrealDatetime::from(Utc::now()),
            };
            let created: Option<DbModerationAction> = self
                .client
                .create("moderation_action")
                .content(entry)
                .await?;
            Ok(db_to_action(created.ok_or_else(|| {
                crate::DbError::NotFound("Failed to create moderation action".into())
            })?))
        })
        .await
    }

    /// List moderation actions for a specific member.
    pub async fn list_member_moderation(
        &self,
        member_id: &str,
    ) -> DbResult<Vec<ModerationAction>> {
        with_timeout(async {
            let mut result = self
                .client
                .query(
                    "SELECT * FROM moderation_action WHERE member_id = $mid ORDER BY created_at DESC",
                )
                .bind(("mid", member_id.to_string()))
                .await?;
            let entries: Vec<DbModerationAction> = result.take(0)?;
            Ok(entries.into_iter().map(db_to_action).collect())
        })
        .await
    }

    /// List all moderation actions with pagination.
    pub async fn list_all_moderation(
        &self,
        limit: u32,
        offset: u32,
    ) -> DbResult<Vec<ModerationAction>> {
        with_timeout(async {
            let mut result = self
                .client
                .query("SELECT * FROM moderation_action ORDER BY created_at DESC LIMIT $limit START $offset")
                .bind(("limit", limit))
                .bind(("offset", offset))
                .await?;
            let entries: Vec<DbModerationAction> = result.take(0)?;
            Ok(entries.into_iter().map(db_to_action).collect())
        })
        .await
    }

    /// Count total moderation actions.
    pub async fn count_moderation(&self) -> DbResult<u64> {
        with_timeout(async {
            #[derive(Deserialize)]
            struct CountResult {
                count: u64,
            }
            let mut result = self
                .client
                .query("SELECT count() FROM moderation_action GROUP ALL")
                .await?;
            let counts: Vec<CountResult> = result.take(0)?;
            Ok(counts.first().map(|c| c.count).unwrap_or(0))
        })
        .await
    }

    /// Lift (deactivate) a moderation action.
    pub async fn lift_moderation_action(&self, id: &str) -> DbResult<ModerationAction> {
        with_timeout(async {
            let existing: Option<DbModerationAction> =
                self.client.select(("moderation_action", id)).await?;
            let mut db = existing.ok_or_else(|| {
                crate::DbError::NotFound(format!("Moderation action {id} not found"))
            })?;

            db.is_active = false;

            let updated: Option<DbModerationAction> = self
                .client
                .update(("moderation_action", id))
                .content(db)
                .await?;
            Ok(db_to_action(updated.ok_or_else(|| {
                crate::DbError::NotFound(format!(
                    "Moderation action {id} not found after update"
                ))
            })?))
        })
        .await
    }

    /// Check if a member is currently suspended or banned.
    pub async fn is_member_suspended_or_banned(&self, member_id: &str) -> DbResult<bool> {
        with_timeout(async {
            let mut result = self
                .client
                .query(
                    "SELECT count() FROM moderation_action WHERE member_id = $mid AND is_active = true AND action_type IN ['suspension', 'ban'] AND (expires_at IS NONE OR expires_at > time::now()) GROUP ALL",
                )
                .bind(("mid", member_id.to_string()))
                .await?;

            #[derive(Deserialize)]
            struct CountResult {
                count: u64,
            }
            let counts: Vec<CountResult> = result.take(0)?;
            Ok(counts.first().map(|c| c.count > 0).unwrap_or(false))
        })
        .await
    }
}
