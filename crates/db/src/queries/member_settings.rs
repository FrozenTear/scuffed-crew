use serde::{Deserialize, Serialize};
use surrealdb_types::{RecordId, SurrealValue};

use crate::types::MemberSettings;
use crate::{with_timeout, Database, DbResult};

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbMemberSettings {
    #[surreal(default)]
    #[allow(dead_code)]
    id: Option<RecordId>,
    member_id: String,
    player_name: Option<String>,
}

fn db_to_settings(db: DbMemberSettings) -> MemberSettings {
    MemberSettings {
        member_id: db.member_id,
        player_name: db.player_name,
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

    pub async fn upsert_member_settings(
        &self,
        member_id: &str,
        player_name: Option<&str>,
    ) -> DbResult<MemberSettings> {
        with_timeout(async {
            let mut result = self
                .client
                .query("SELECT * FROM member_settings WHERE member_id = $mid LIMIT 1")
                .bind(("mid", member_id.to_string()))
                .await?;
            let rows: Vec<DbMemberSettings> = result.take(0)?;

            if let Some(existing) = rows.into_iter().next() {
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
                    })
                    .await?;
                Ok(db_to_settings(updated.ok_or_else(|| {
                    crate::DbError::NotFound("member_settings not found after update".into())
                })?))
            } else {
                let created: Option<DbMemberSettings> = self
                    .client
                    .create("member_settings")
                    .content(DbMemberSettings {
                        id: None,
                        member_id: member_id.to_string(),
                        player_name: player_name.map(|s| s.to_string()),
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
