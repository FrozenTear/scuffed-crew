use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use surrealdb_types::{RecordId, SurrealValue};

use crate::types::MemberSettings;
use crate::{with_timeout, Database, DbResult};
use scuffed_types::StatsVisibility;

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbMemberSettings {
    #[surreal(default)]
    #[allow(dead_code)]
    id: Option<RecordId>,
    member_id: String,
    player_name: Option<String>,
    /// Absent on rows written before `stats_visibility` existed.
    #[serde(default)]
    #[surreal(default)]
    stats_visibility: Option<String>,
}

fn parse_visibility(raw: Option<&str>) -> StatsVisibility {
    match raw {
        Some("members") => StatsVisibility::Members,
        Some("private") => StatsVisibility::Private,
        _ => StatsVisibility::Public,
    }
}

fn db_to_settings(db: DbMemberSettings) -> MemberSettings {
    MemberSettings {
        member_id: db.member_id,
        player_name: db.player_name,
        stats_visibility: parse_visibility(db.stats_visibility.as_deref()),
    }
}

impl Database {
    pub async fn get_member_settings(&self, member_id: &str) -> DbResult<Option<MemberSettings>> {
        with_timeout(async {
            let mut result = self
                .client
                .query("SELECT * FROM member_settings WHERE member_id = $mid LIMIT 1")
                .bind(("mid", member_id.to_string()))
                .await?;
            let rows: Vec<DbMemberSettings> = result.take(0)?;
            Ok(rows.into_iter().next().map(db_to_settings))
        })
        .await
    }

    /// Stored visibility, or [`StatsVisibility::Public`] when the member has no row.
    pub async fn get_stats_visibility(&self, member_id: &str) -> DbResult<StatsVisibility> {
        Ok(self
            .get_member_settings(member_id)
            .await?
            .map(|s| s.stats_visibility)
            .unwrap_or_default())
    }

    /// Visibility for the given member ids. Ids with no row are omitted.
    /// Callers treat a missing id as public.
    pub async fn stats_visibility_for_ids(
        &self,
        member_ids: &[String],
    ) -> DbResult<HashMap<String, StatsVisibility>> {
        if member_ids.is_empty() {
            return Ok(HashMap::new());
        }
        with_timeout(async {
            #[derive(Deserialize, SurrealValue)]
            struct Row {
                member_id: String,
                #[serde(default)]
                #[surreal(default)]
                stats_visibility: Option<String>,
            }
            let mut result = self
                .client
                .query(
                    "SELECT member_id, stats_visibility FROM member_settings WHERE member_id IN $ids",
                )
                .bind(("ids", member_ids.to_vec()))
                .await?;
            let rows: Vec<Row> = result.take(0)?;
            let mut map = HashMap::with_capacity(rows.len());
            for row in rows {
                map.insert(
                    row.member_id,
                    parse_visibility(row.stats_visibility.as_deref()),
                );
            }
            Ok(map)
        })
        .await
    }

    /// `stats_visibility == None` keeps the stored value, or public on insert.
    pub async fn upsert_member_settings(
        &self,
        member_id: &str,
        player_name: Option<&str>,
        stats_visibility: Option<StatsVisibility>,
    ) -> DbResult<MemberSettings> {
        with_timeout(async {
            let mut result = self
                .client
                .query("SELECT * FROM member_settings WHERE member_id = $mid LIMIT 1")
                .bind(("mid", member_id.to_string()))
                .await?;
            let rows: Vec<DbMemberSettings> = result.take(0)?;

            if let Some(existing) = rows.into_iter().next() {
                let visibility = stats_visibility
                    .unwrap_or_else(|| parse_visibility(existing.stats_visibility.as_deref()));
                let id = existing
                    .id
                    .as_ref()
                    .map(|r| crate::record_id_key_to_string(r.key.clone()))
                    .unwrap_or_default();
                let updated: Option<DbMemberSettings> = self
                    .client
                    .update(("member_settings", id.as_str()))
                    .content(DbMemberSettings {
                        id: existing.id,
                        member_id: member_id.to_string(),
                        player_name: player_name.map(|s| s.to_string()),
                        stats_visibility: Some(visibility.as_str().to_string()),
                    })
                    .await?;
                Ok(db_to_settings(updated.ok_or_else(|| {
                    crate::DbError::NotFound("member_settings not found after update".into())
                })?))
            } else {
                let visibility = stats_visibility.unwrap_or_default();
                let created: Option<DbMemberSettings> = self
                    .client
                    .create("member_settings")
                    .content(DbMemberSettings {
                        id: None,
                        member_id: member_id.to_string(),
                        player_name: player_name.map(|s| s.to_string()),
                        stats_visibility: Some(visibility.as_str().to_string()),
                    })
                    .await?;
                Ok(db_to_settings(created.ok_or_else(|| {
                    crate::DbError::NotFound("failed to create member_settings".into())
                })?))
            }
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations::run_migrations;
    use crate::Database;

    async fn test_db() -> Database {
        let db = Database::connect_memory().await.unwrap();
        run_migrations(&db.client).await.unwrap();
        db
    }

    #[tokio::test]
    async fn missing_row_is_public() {
        let db = test_db().await;
        assert_eq!(
            db.get_stats_visibility("nobody").await.unwrap(),
            StatsVisibility::Public
        );
    }

    #[tokio::test]
    async fn legacy_row_without_the_field_reads_public() {
        let db = test_db().await;
        db.client
            .query(
                r#"
                REMOVE FIELD IF EXISTS stats_visibility ON member_settings;
                CREATE member_settings SET member_id = 'legacy-vis', player_name = 'Legacy';
                DEFINE FIELD OVERWRITE stats_visibility ON member_settings TYPE string
                    DEFAULT 'public'
                    ASSERT $value IN ['public', 'members', 'private'];
                "#,
            )
            .await
            .unwrap()
            .check()
            .unwrap();

        let settings = db
            .get_member_settings("legacy-vis")
            .await
            .unwrap()
            .expect("legacy row");
        assert_eq!(settings.player_name.as_deref(), Some("Legacy"));
        assert_eq!(settings.stats_visibility, StatsVisibility::Public);
        assert_eq!(
            db.get_stats_visibility("legacy-vis").await.unwrap(),
            StatsVisibility::Public
        );
    }

    #[tokio::test]
    async fn upsert_without_visibility_keeps_the_stored_value() {
        let db = test_db().await;
        let created = db
            .upsert_member_settings("m1", Some("Ana"), Some(StatsVisibility::Private))
            .await
            .unwrap();
        assert_eq!(created.stats_visibility, StatsVisibility::Private);

        let kept = db
            .upsert_member_settings("m1", Some("AnaTwo"), None)
            .await
            .unwrap();
        assert_eq!(kept.player_name.as_deref(), Some("AnaTwo"));
        assert_eq!(kept.stats_visibility, StatsVisibility::Private);

        let cleared_name = db.upsert_member_settings("m1", None, None).await.unwrap();
        assert_eq!(cleared_name.player_name, None);
        assert_eq!(cleared_name.stats_visibility, StatsVisibility::Private);
    }
}
