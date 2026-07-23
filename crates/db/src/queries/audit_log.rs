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
    // Fallback until list path enriches via per-type target join.
    let target_label = db.target_id.clone();
    AuditLogEntry {
        id,
        actor_id,
        actor_name,
        action: db.action,
        target_type: db.target_type,
        target_id: db.target_id,
        target_label,
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
    /// Enriches each entry with `actor_name` from `member.display_name` and
    /// `target_label` from the target's display field (read-time joins only —
    /// audit_log rows stay append-only without name/label columns).
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
            self.enrich_audit_target_labels(&mut out).await?;
            Ok(out)
        })
        .await
    }

    /// Fill `actor_name` for a page of audit entries from member display names.
    ///
    /// **Page-scoped map:** only resolves actors present on *this* page (unique
    /// `actor_id`s), not the full member table. Org rosters are still small, so
    /// a full `SELECT id, display_name FROM member` would also be fine — we
    /// prefer the page set so cost stays proportional to the audit page size.
    /// Name is joined in process; audit_log stays append-only (no name column).
    async fn enrich_audit_actor_names(&self, entries: &mut [AuditLogEntry]) -> DbResult<()> {
        if entries.is_empty() {
            return Ok(());
        }

        let mut actor_ids: Vec<String> = entries.iter().map(|e| e.actor_id.clone()).collect();
        actor_ids.sort();
        actor_ids.dedup();

        #[derive(Debug, Deserialize, SurrealValue)]
        struct NameRow {
            id: Option<RecordId>,
            display_name: String,
        }

        // Bind RecordIds so Surreal matches the member table's id type (v3).
        let rids: Vec<RecordId> = actor_ids
            .iter()
            .map(|id| RecordId::new("member", id.as_str()))
            .collect();

        let mut result = self
            .client
            .query("SELECT id, display_name FROM member WHERE id IN $rids")
            .bind(("rids", rids))
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

    /// Fill `target_label` for a page of audit entries from the target's display field.
    ///
    /// **Page-scoped map:** like `enrich_audit_actor_names`, only resolves targets
    /// present on *this* page — one `SELECT id, <field>` per target type present,
    /// with the page's unique ids bound as `RecordId`s (v3). Types without a clear
    /// single display field (roster, match, settings, moderation, application,
    /// tournament participant/match, …) keep the `target_id` fallback set in
    /// `db_to_entry`. Label is joined in process; audit_log stays append-only.
    async fn enrich_audit_target_labels(&self, entries: &mut [AuditLogEntry]) -> DbResult<()> {
        if entries.is_empty() {
            return Ok(());
        }

        // (target_type as stored on audit rows, table, display field). Compile-time
        // constants only — never user input — so the fixed query fragments below
        // stay safe; the ids themselves are bound as RecordId params.
        const LABEL_SOURCES: &[(&str, &str, &str)] = &[
            ("member", "member", "display_name"),
            ("game", "game", "name"),
            ("team", "team", "name"),
            ("tournament", "tournament", "name"),
            ("event", "event", "title"),
            ("announcement", "announcement", "title"),
        ];

        #[derive(Debug, Deserialize, SurrealValue)]
        struct LabelRow {
            id: Option<RecordId>,
            label: String,
        }

        for (target_type, table, field) in LABEL_SOURCES {
            let mut target_ids: Vec<&str> = entries
                .iter()
                .filter(|e| e.target_type == *target_type)
                .map(|e| e.target_id.as_str())
                .collect();
            target_ids.sort_unstable();
            target_ids.dedup();
            if target_ids.is_empty() {
                continue;
            }

            // Bind RecordIds so Surreal matches the table's id type (v3).
            let rids: Vec<RecordId> = target_ids
                .iter()
                .map(|id| RecordId::new(*table, *id))
                .collect();

            // Projection is intentionally minimal (id + display field only) —
            // in particular member must never leak nostr_secret_key_encrypted.
            let mut result = self
                .client
                .query(format!(
                    "SELECT id, {field} AS label FROM {table} WHERE id IN $rids"
                ))
                .bind(("rids", rids))
                .await?;
            let rows: Vec<LabelRow> = result.take(0).unwrap_or_default();
            let mut map = std::collections::HashMap::with_capacity(rows.len());
            for row in rows {
                if let Some(rid) = row.id {
                    let key = crate::record_id_key_to_string(rid.key);
                    map.insert(key, row.label);
                }
            }
            for entry in entries.iter_mut().filter(|e| e.target_type == *target_type) {
                if let Some(label) = map.get(&entry.target_id) {
                    entry.target_label = label.clone();
                }
                // else leave target_label = target_id (set in db_to_entry)
            }
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
        // wiki_page has no label source → target_label falls back to target_id
        assert_eq!(entries[0].target_label, "target-1");
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

    #[tokio::test]
    async fn list_enriches_target_label_for_member_target() {
        let db = test_db().await;
        // Seed a member, then audit an action *targeting* that member.
        db.create_member("user-target", "Target Member", OrgRole::Member)
            .await
            .expect("create_member");
        let member = db
            .get_member_by_user("user-target")
            .await
            .unwrap()
            .expect("member");
        db.insert_audit_log(
            "actor-1",
            AuditAction::UpdatedMember,
            AuditTargetType::Member,
            &member.id,
            None,
        )
        .await
        .unwrap();

        let entries = db.list_audit_log(10, 0).await.unwrap();
        let hit = entries
            .iter()
            .find(|e| e.target_id == member.id)
            .expect("audit row");
        assert_eq!(hit.target_label, "Target Member");
        // The raw id stays available alongside the label.
        assert_eq!(hit.target_id, member.id);
    }

    #[tokio::test]
    async fn list_enriches_target_label_for_announcement_target() {
        let db = test_db().await;
        let ann = db
            .create_announcement("Patch Notes", "content", "author-1", false)
            .await
            .expect("create_announcement");
        db.insert_audit_log(
            "actor-1",
            AuditAction::CreatedAnnouncement,
            AuditTargetType::Announcement,
            &ann.id,
            None,
        )
        .await
        .unwrap();

        let entries = db.list_audit_log(10, 0).await.unwrap();
        let hit = entries
            .iter()
            .find(|e| e.target_id == ann.id)
            .expect("audit row");
        assert_eq!(hit.target_label, "Patch Notes");
    }

    /// Types without a display field (and unmatched ids) keep the id fallback.
    #[tokio::test]
    async fn target_label_falls_back_to_target_id() {
        let db = test_db().await;
        // Opaque type: settings has no display field → never enriched.
        db.insert_audit_log(
            "actor-1",
            AuditAction::UpdatedSettings,
            AuditTargetType::Settings,
            "org",
            None,
        )
        .await
        .unwrap();
        // Enrichable type, but the target row doesn't exist → fallback too.
        db.insert_audit_log(
            "actor-1",
            AuditAction::UpdatedTeam,
            AuditTargetType::Team,
            "no-such-team",
            None,
        )
        .await
        .unwrap();

        let entries = db.list_audit_log(10, 0).await.unwrap();
        let settings = entries
            .iter()
            .find(|e| e.target_type == "settings")
            .expect("settings row");
        assert_eq!(settings.target_label, "org");
        let team = entries
            .iter()
            .find(|e| e.target_type == "team")
            .expect("team row");
        assert_eq!(team.target_label, "no-such-team");
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
