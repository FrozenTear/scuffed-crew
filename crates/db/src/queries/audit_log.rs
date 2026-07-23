use chrono::Utc;
use serde::{Deserialize, Serialize};
use surrealdb::types::Datetime as SurrealDatetime;
use surrealdb_types::RecordId;
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
    let actor_id = db.actor_id;
    // Fallback until list path enriches via member join.
    let actor_name = actor_id.clone();
    AuditLogEntry {
        id,
        actor_id,
        actor_name,
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
            let _: Option<DbAuditLogEntry> = self.client.create("audit_log").content(entry).await?;
            Ok(())
        })
        .await
    }

    /// List audit log entries with pagination.
    ///
    /// Enriches each entry with `actor_name` from `member.display_name` (read-time
    /// join only — audit_log rows stay append-only without a name column).
    pub async fn list_audit_log(&self, limit: u32, offset: u32) -> DbResult<Vec<AuditLogEntry>> {
        with_timeout(async {
            let mut result = self
                .client
                .query(
                    "SELECT * FROM audit_log ORDER BY created_at DESC LIMIT $limit START $offset",
                )
                .bind(("limit", limit))
                .bind(("offset", offset))
                .await?;
            let entries: Vec<DbAuditLogEntry> = result.take(0)?;
            let mut out: Vec<AuditLogEntry> = entries.into_iter().map(db_to_entry).collect();
            self.enrich_audit_actor_names(&mut out).await?;
            Ok(out)
        })
        .await
    }

    /// Fill `actor_name` for a page of audit entries from member display names.
    ///
    /// Loads the (small) member name map in one query and joins in process —
    /// avoids Surreal record-id `IN` quirks and keeps audit_log append-only.
    async fn enrich_audit_actor_names(&self, entries: &mut [AuditLogEntry]) -> DbResult<()> {
        if entries.is_empty() {
            return Ok(());
        }

        #[derive(Debug, Deserialize, SurrealValue)]
        struct NameRow {
            id: Option<RecordId>,
            display_name: String,
        }

        let mut result = self
            .client
            .query("SELECT id, display_name FROM member")
            .await?;
        let rows: Vec<NameRow> = result.take(0).unwrap_or_default();
        let mut map = std::collections::HashMap::with_capacity(rows.len());
        for row in rows {
            if let Some(rid) = row.id {
                let key = crate::record_id_key_to_string(rid.key);
                map.insert(key, row.display_name);
            }
        }
        for entry in entries.iter_mut() {
            if let Some(name) = map.get(&entry.actor_id) {
                entry.actor_name = name.clone();
            }
            // else leave actor_name = actor_id (set in db_to_entry)
        }
        Ok(())
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

#[cfg(test)]
mod tests {
    use crate::migrations::run_migrations;
    use crate::types::{AuditAction, AuditTargetType, OrgRole};
    use crate::Database;

    async fn test_db() -> Database {
        let db = Database::connect_memory().await.unwrap();
        run_migrations(&db.client).await.unwrap();
        db
    }

    async fn seed_one(db: &Database) {
        db.insert_audit_log(
            "actor-1",
            AuditAction::CreatedWikiPage,
            AuditTargetType::WikiPage,
            "target-1",
            Some("original"),
        )
        .await
        .expect("insert_audit_log (CREATE) must succeed");
    }

    /// Append (CREATE) and read (SELECT) paths keep working after the
    /// append-only migration (DR1-DB-007).
    #[tokio::test]
    async fn create_and_list_still_work() {
        let db = test_db().await;
        seed_one(&db).await;

        assert_eq!(db.count_audit_log().await.unwrap(), 1);
        let entries = db.list_audit_log(10, 0).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].actor_id, "actor-1");
        // No matching member → actor_name falls back to actor_id
        assert_eq!(entries[0].actor_name, "actor-1");
        assert_eq!(entries[0].details.as_deref(), Some("original"));
    }

    #[tokio::test]
    async fn list_enriches_actor_name_from_member() {
        let db = test_db().await;
        // Seed a member with a known id key, then audit as that actor.
        db.create_member("user-audit", "Audit Actor", OrgRole::Member)
            .await
            .expect("create_member");
        let member = db
            .get_member_by_user("user-audit")
            .await
            .unwrap()
            .expect("member");
        db.insert_audit_log(
            &member.id,
            AuditAction::UpdatedSettings,
            AuditTargetType::Settings,
            "org",
            Some("changed"),
        )
        .await
        .unwrap();

        let entries = db.list_audit_log(10, 0).await.unwrap();
        let hit = entries
            .iter()
            .find(|e| e.actor_id == member.id)
            .expect("audit row");
        assert_eq!(hit.actor_name, "Audit Actor");
    }

    /// UPDATE on audit_log is rejected by the append-only event — even for the
    /// owner-level in-mem connection (the event fires for every writer, which is
    /// how it binds the EDITOR app user in prod that table permissions cannot).
    /// The row content is left intact.
    #[tokio::test]
    async fn update_is_rejected_and_row_unchanged() {
        let db = test_db().await;
        seed_one(&db).await;

        let res = db
            .client
            .query("UPDATE audit_log SET details = 'tampered'")
            .await
            .expect("query dispatches")
            .check();
        assert!(res.is_err(), "UPDATE on audit_log must be rejected");

        // Transaction rolled back: original content survives.
        let entries = db.list_audit_log(10, 0).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].details.as_deref(), Some("original"));
    }

    /// DELETE on audit_log is rejected by the append-only event; the row remains.
    #[tokio::test]
    async fn delete_is_rejected_and_row_survives() {
        let db = test_db().await;
        seed_one(&db).await;

        let res = db
            .client
            .query("DELETE audit_log")
            .await
            .expect("query dispatches")
            .check();
        assert!(res.is_err(), "DELETE on audit_log must be rejected");

        assert_eq!(
            db.count_audit_log().await.unwrap(),
            1,
            "row must survive a blocked DELETE"
        );
    }

    /// Re-running migrations (prod boots them every start via OVERWRITE) must not
    /// drop existing audit rows — confirms DEFINE TABLE OVERWRITE preserves data.
    #[tokio::test]
    async fn rerun_migrations_preserves_rows() {
        let db = test_db().await;
        seed_one(&db).await;
        run_migrations(&db.client).await.unwrap();
        assert_eq!(
            db.count_audit_log().await.unwrap(),
            1,
            "OVERWRITE migration must preserve existing rows"
        );
    }
}
