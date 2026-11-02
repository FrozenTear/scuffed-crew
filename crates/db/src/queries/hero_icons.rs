use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use surrealdb::types::Datetime as SurrealDatetime;
use surrealdb_types::RecordId;
use surrealdb_types::SurrealValue;

use crate::types::{HeroIcon, NewHeroIcon};
use crate::{with_timeout, Database, DbResult};

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbHeroIcon {
    #[surreal(default)]
    #[serde(default)]
    id: Option<RecordId>,
    member_id: String,
    hero: String,
    file_name: String,
    size_bytes: i64,
    width: i64,
    height: i64,
    created_at: SurrealDatetime,
}

fn db_to_icon(db: DbHeroIcon) -> HeroIcon {
    let id = db
        .id
        .map(|r| crate::record_id_key_to_string(r.key))
        .unwrap_or_else(|| "unknown".to_string());
    HeroIcon {
        id,
        member_id: db.member_id,
        hero: db.hero,
        file_name: db.file_name,
        size_bytes: u64::try_from(db.size_bytes).unwrap_or(0),
        width: u32::try_from(db.width).unwrap_or(0),
        height: u32::try_from(db.height).unwrap_or(0),
        created_at: db.created_at.into(),
    }
}

fn to_db(icon: &NewHeroIcon) -> Result<DbHeroIcon, crate::DbError> {
    let Ok(size_bytes) = i64::try_from(icon.size_bytes) else {
        return Err(crate::DbError::Config("icon size is too large".into()));
    };
    Ok(DbHeroIcon {
        id: None,
        member_id: icon.member_id.clone(),
        hero: icon.hero.clone(),
        file_name: icon.file_name.clone(),
        size_bytes,
        width: i64::from(icon.width),
        height: i64::from(icon.height),
        created_at: SurrealDatetime::from(icon.created_at),
    })
}

impl Database {
    pub async fn insert_hero_icon(&self, icon: &NewHeroIcon) -> DbResult<HeroIcon> {
        let id = icon.id.clone();
        let row = to_db(icon)?;
        with_timeout(async {
            let created: Option<DbHeroIcon> = self
                .client
                .create(("hero_icon", id.as_str()))
                .content(row)
                .await?;
            match created {
                Some(row) => Ok(db_to_icon(row)),
                None => Err(crate::DbError::NotFound("hero icon create".into())),
            }
        })
        .await
    }

    pub async fn get_hero_icon(&self, id: &str) -> DbResult<Option<HeroIcon>> {
        let id = id.to_string();
        with_timeout(async {
            let row: Option<DbHeroIcon> = self.client.select(("hero_icon", id.as_str())).await?;
            Ok(row.map(db_to_icon))
        })
        .await
    }

    /// One member's icons, newest first. Ties (one upload request) go by file name.
    pub async fn list_hero_icons_for_member(&self, member_id: &str) -> DbResult<Vec<HeroIcon>> {
        let member_id = member_id.to_string();
        with_timeout(async {
            let mut result = self
                .client
                .query(
                    "SELECT * FROM hero_icon WHERE member_id = $member_id \
                     ORDER BY created_at DESC, file_name DESC",
                )
                .bind(("member_id", member_id))
                .await?;
            let rows: Vec<DbHeroIcon> = result.take(0)?;
            Ok(rows.into_iter().map(db_to_icon).collect())
        })
        .await
    }

    /// Icons of the given members, newest first. Fetches `limit + 1` rows so the
    /// caller can build a cursor page with `CursorResponse::from_oversized`.
    pub async fn list_hero_icons_for_members(
        &self,
        member_ids: &[String],
        limit: u32,
        offset: u32,
    ) -> DbResult<Vec<HeroIcon>> {
        if member_ids.is_empty() {
            return Ok(Vec::new());
        }
        let ids = member_ids.to_vec();
        with_timeout(async {
            let mut result = self
                .client
                .query(
                    "SELECT * FROM hero_icon WHERE member_id IN $ids \
                     ORDER BY created_at DESC, file_name DESC LIMIT $lim START $off",
                )
                .bind(("ids", ids))
                .bind(("lim", limit + 1))
                .bind(("off", offset))
                .await?;
            let rows: Vec<DbHeroIcon> = result.take(0)?;
            Ok(rows.into_iter().map(db_to_icon).collect())
        })
        .await
    }

    pub async fn count_hero_icons_for_member(&self, member_id: &str) -> DbResult<u64> {
        let member_id = member_id.to_string();
        with_timeout(async {
            #[derive(Deserialize, SurrealValue)]
            struct CountResult {
                count: u64,
            }
            let mut result = self
                .client
                .query(
                    "SELECT count() AS count FROM hero_icon \
                     WHERE member_id = $member_id GROUP ALL",
                )
                .bind(("member_id", member_id))
                .await?;
            let counts: Vec<CountResult> = result.take(0)?;
            Ok(counts.first().map(|c| c.count).unwrap_or(0))
        })
        .await
    }

    /// Stored icon count per member id. Members without icons are absent.
    pub async fn hero_icon_counts(&self) -> DbResult<HashMap<String, u64>> {
        with_timeout(async {
            #[derive(Deserialize, SurrealValue)]
            struct Row {
                member_id: String,
                count: u64,
            }
            let mut result = self
                .client
                .query("SELECT member_id, count() AS count FROM hero_icon GROUP BY member_id")
                .await?;
            let rows: Vec<Row> = result.take(0)?;
            let mut map = HashMap::with_capacity(rows.len());
            for row in rows {
                map.insert(row.member_id, row.count);
            }
            Ok(map)
        })
        .await
    }

    /// Remove one row and return it as it was, or `None` when it did not exist.
    pub async fn delete_hero_icon(&self, id: &str) -> DbResult<Option<HeroIcon>> {
        let id = id.to_string();
        with_timeout(async {
            let row: Option<DbHeroIcon> = self.client.delete(("hero_icon", id.as_str())).await?;
            Ok(row.map(db_to_icon))
        })
        .await
    }

    /// Remove every row of one member and return them as they were.
    pub async fn delete_hero_icons_for_member(&self, member_id: &str) -> DbResult<Vec<HeroIcon>> {
        let member_id = member_id.to_string();
        with_timeout(async {
            let mut result = self
                .client
                .query("DELETE hero_icon WHERE member_id = $member_id RETURN BEFORE")
                .bind(("member_id", member_id))
                .await?;
            let rows: Vec<DbHeroIcon> = result.take(0)?;
            Ok(rows.into_iter().map(db_to_icon).collect())
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};

    use super::*;
    use crate::migrations::run_migrations;

    async fn test_db() -> Database {
        let db = Database::connect_memory().await.unwrap();
        run_migrations(&db.client).await.unwrap();
        db
    }

    fn new_icon(id: &str, member_id: &str, minutes_ago: i64) -> NewHeroIcon {
        NewHeroIcon {
            id: id.to_string(),
            member_id: member_id.to_string(),
            hero: "Ana".to_string(),
            file_name: format!("{id}.png"),
            size_bytes: 120,
            width: 32,
            height: 32,
            created_at: Utc::now() - Duration::minutes(minutes_ago),
        }
    }

    #[tokio::test]
    async fn insert_list_count_and_delete() {
        let db = test_db().await;
        let a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let b = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let c = "cccccccccccccccccccccccccccccccc";
        db.insert_hero_icon(&new_icon(a, "m1", 10)).await.unwrap();
        db.insert_hero_icon(&new_icon(b, "m1", 5)).await.unwrap();
        db.insert_hero_icon(&new_icon(c, "m2", 1)).await.unwrap();

        let stored = db.get_hero_icon(a).await.unwrap().expect("row");
        assert_eq!(stored.member_id, "m1");
        assert_eq!(stored.file_name, format!("{a}.png"));
        assert_eq!(stored.width, 32);
        assert_eq!(stored.height, 32);
        assert_eq!(stored.size_bytes, 120);

        let mine = db.list_hero_icons_for_member("m1").await.unwrap();
        let ids: Vec<&str> = mine.iter().map(|icon| icon.id.as_str()).collect();
        assert_eq!(ids, vec![b, a]);
        assert_eq!(db.count_hero_icons_for_member("m1").await.unwrap(), 2);
        assert_eq!(db.count_hero_icons_for_member("nobody").await.unwrap(), 0);

        let counts = db.hero_icon_counts().await.unwrap();
        assert_eq!(counts.get("m1"), Some(&2));
        assert_eq!(counts.get("m2"), Some(&1));

        let page = db
            .list_hero_icons_for_members(&["m1".to_string(), "m2".to_string()], 2, 0)
            .await
            .unwrap();
        let ids: Vec<&str> = page.iter().map(|icon| icon.id.as_str()).collect();
        assert_eq!(ids, vec![c, b, a]);

        let removed = db.delete_hero_icon(a).await.unwrap().expect("removed row");
        assert_eq!(removed.id, a);
        assert!(db.delete_hero_icon(a).await.unwrap().is_none());
        assert!(db.get_hero_icon(a).await.unwrap().is_none());

        let removed = db.delete_hero_icons_for_member("m1").await.unwrap();
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].id, b);
        assert_eq!(db.count_hero_icons_for_member("m1").await.unwrap(), 0);
        assert_eq!(db.count_hero_icons_for_member("m2").await.unwrap(), 1);
    }

    #[tokio::test]
    async fn rows_from_one_upload_have_a_stable_order() {
        let db = test_db().await;
        let at = Utc::now();
        let d = "dddddddddddddddddddddddddddddddd";
        let e = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
        for id in [d, e] {
            let mut icon = new_icon(id, "m1", 0);
            icon.created_at = at;
            db.insert_hero_icon(&icon).await.unwrap();
        }

        let mine = db.list_hero_icons_for_member("m1").await.unwrap();
        let ids: Vec<&str> = mine.iter().map(|icon| icon.id.as_str()).collect();
        assert_eq!(ids, vec![e, d]);

        let second_page = db
            .list_hero_icons_for_members(&["m1".to_string()], 1, 1)
            .await
            .unwrap();
        assert_eq!(second_page.len(), 1);
        assert_eq!(second_page[0].id, d);
    }
}
