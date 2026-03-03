use chrono::Utc;
use serde::{Deserialize, Serialize};
use surrealdb_types::RecordId;
use surrealdb::types::Datetime as SurrealDatetime;
use surrealdb_types::SurrealValue;

use crate::types::{AuditAction, AuditLogEntry, AuditTargetType};
use crate::{with_timeout, Database, DbResult};

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbAuditLogEntry {
    #[surreal(default)]
    #[allow(dead_code)]
    id: Option<RecordId>,
    actor_id: String,
    action: String,
    target_type: String,
    target_id: String,
    details: Option<String>,
    created_at: SurrealDatetime,
}

fn db_to_entry(db: DbAuditLogEntry) -> AuditLogEntry {
    let id = db
        .id
        .map(|r| crate::record_id_key_to_string(r.key))
        .unwrap_or_else(|| "unknown".to_string());
    AuditLogEntry {
        id,
        actor_id: db.actor_id,
        action: db.action,
        target_type: db.target_type,
        target_id: db.target_id,
        details: db.details,
        created_at: db.created_at.into(),
    }
}

impl Database {
    /// Insert an audit log entry.
    pub async fn insert_audit_log(
        &self,
        actor_id: &str,
        action: AuditAction,
        target_type: AuditTargetType,
        target_id: &str,
        details: Option<&str>,
    ) -> DbResult<()> {
        with_timeout(async {
            let entry = DbAuditLogEntry {
                id: None,
                actor_id: actor_id.to_string(),
                action: action.to_string(),
                target_type: target_type.to_string(),
                target_id: target_id.to_string(),
                details: details.map(|s| s.to_string()),
                created_at: SurrealDatetime::from(Utc::now()),
            };
            let _: Option<DbAuditLogEntry> =
                self.client.create("audit_log").content(entry).await?;
            Ok(())
        })
        .await
    }

    /// List audit log entries with pagination.
    pub async fn list_audit_log(
        &self,
        limit: u32,
        offset: u32,
    ) -> DbResult<Vec<AuditLogEntry>> {
        with_timeout(async {
            let mut result = self
                .client
                .query("SELECT * FROM audit_log ORDER BY created_at DESC LIMIT $limit START $offset")
                .bind(("limit", limit))
                .bind(("offset", offset))
                .await?;
            let entries: Vec<DbAuditLogEntry> = result.take(0)?;
            Ok(entries.into_iter().map(db_to_entry).collect())
        })
        .await
    }

    /// Count total audit log entries.
    pub async fn count_audit_log(&self) -> DbResult<u64> {
        with_timeout(async {
            #[derive(Deserialize, SurrealValue)]
            struct CountResult {
                count: u64,
            }
            let mut result = self
                .client
                .query("SELECT count() FROM audit_log GROUP ALL")
                .await?;
            let counts: Vec<CountResult> = result.take(0)?;
            Ok(counts.first().map(|c| c.count).unwrap_or(0))
        })
        .await
    }
}
