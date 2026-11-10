use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use surrealdb::types::Datetime as SurrealDatetime;
use surrealdb_types::RecordId;
use surrealdb_types::SurrealValue;

use crate::types::{
    HeroIcon, NewHeroIcon, NewTrainingCrop, TrainingCrop, TrainingCropCountRow, TrainingCropOwner,
};
use crate::{with_timeout, Database, DbResult};
use scuffed_types::{hero_key, hero_key_to_name, CropKind, CropSpot, TrainingCropMeta};

/// One `hero_icon` row. Every column added for training crops is optional,
/// so rows from before them still read: `kind` absent means `hero_icon`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, SurrealValue)]
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
    #[surreal(default)]
    #[serde(default)]
    kind: Option<String>,
    #[surreal(default)]
    #[serde(default)]
    screen: Option<String>,
    #[surreal(default)]
    #[serde(default)]
    slot_type: Option<String>,
    #[surreal(default)]
    #[serde(default)]
    slot_index: Option<i64>,
    #[surreal(default)]
    #[serde(default)]
    game_token: Option<String>,
    #[surreal(default)]
    #[serde(default)]
    score: Option<f64>,
    #[surreal(default)]
    #[serde(default)]
    runner_up: Option<f64>,
    #[surreal(default)]
    #[serde(default)]
    margin: Option<f64>,
    #[surreal(default)]
    #[serde(default)]
    icon_set_version: Option<String>,
    #[surreal(default)]
    #[serde(default)]
    spot_version: Option<String>,
    #[surreal(default)]
    #[serde(default)]
    vote_card_filled: Option<bool>,
    #[surreal(default)]
    #[serde(default)]
    locked_in: Option<bool>,
    #[surreal(default)]
    #[serde(default)]
    screen_width: Option<i64>,
    #[surreal(default)]
    #[serde(default)]
    screen_height: Option<i64>,
    #[surreal(default)]
    #[serde(default)]
    ui_scale: Option<f64>,
    #[surreal(default)]
    #[serde(default)]
    tracker_version: Option<String>,
    #[surreal(default)]
    #[serde(default)]
    found_x: Option<i64>,
    #[surreal(default)]
    #[serde(default)]
    found_y: Option<i64>,
    #[surreal(default)]
    #[serde(default)]
    found_w: Option<i64>,
    #[surreal(default)]
    #[serde(default)]
    found_h: Option<i64>,
    #[surreal(default)]
    #[serde(default)]
    found_confidence: Option<f64>,
}

/// The stored kind. Absent (a row from before `kind`) is `hero_icon`. An
/// unknown stored value is `None` so callers can skip the row.
fn stored_kind(raw: Option<&str>) -> Option<CropKind> {
    match raw {
        None => Some(CropKind::HeroIcon),
        Some(value) => CropKind::parse(value),
    }
}

fn to_u32(value: Option<i64>) -> Option<u32> {
    value.and_then(|v| u32::try_from(v).ok())
}

/// The hero key for a stored `hero` value. Rows written before keys were
/// stored hold the display name; both map to the key. An unknown value is
/// kept as written.
fn stored_hero_key(raw: String) -> String {
    match hero_key(&raw) {
        Some(key) => key.to_string(),
        None => raw,
    }
}

fn record_key(id: Option<RecordId>) -> String {
    id.map(|r| crate::record_id_key_to_string(r.key))
        .unwrap_or_else(|| "unknown".to_string())
}

fn db_to_crop(db: DbHeroIcon) -> Option<TrainingCrop> {
    let kind = stored_kind(db.kind.as_deref())?;
    let found_spot = match (db.found_x, db.found_y, db.found_w, db.found_h) {
        (Some(x), Some(y), Some(w), Some(h)) => Some(CropSpot {
            x: u32::try_from(x).ok()?,
            y: u32::try_from(y).ok()?,
            w: u32::try_from(w).ok()?,
            h: u32::try_from(h).ok()?,
        }),
        _ => None,
    };
    let meta = TrainingCropMeta {
        screen: db.screen,
        slot_type: db.slot_type,
        slot_index: to_u32(db.slot_index),
        game_token: db.game_token,
        score: db.score,
        runner_up: db.runner_up,
        margin: db.margin,
        icon_set_version: db.icon_set_version,
        spot_version: db.spot_version,
        vote_card_filled: db.vote_card_filled,
        locked_in: db.locked_in,
        screen_width: to_u32(db.screen_width),
        screen_height: to_u32(db.screen_height),
        ui_scale: db.ui_scale,
        found_spot,
        found_confidence: db.found_confidence,
    };
    Some(TrainingCrop {
        id: record_key(db.id),
        member_id: db.member_id,
        kind,
        hero: (!db.hero.is_empty()).then(|| stored_hero_key(db.hero)),
        file_name: db.file_name,
        size_bytes: u64::try_from(db.size_bytes).unwrap_or(0),
        width: u32::try_from(db.width).unwrap_or(0),
        height: u32::try_from(db.height).unwrap_or(0),
        created_at: db.created_at.into(),
        tracker_version: db.tracker_version,
        meta,
    })
}

fn crops_from(rows: Vec<DbHeroIcon>) -> Vec<TrainingCrop> {
    rows.into_iter().filter_map(db_to_crop).collect()
}

/// The hero icon view of a crop. `None` for any other kind. The hero icon
/// API keeps answering with the display name, whatever the row stores.
fn crop_to_icon(crop: TrainingCrop) -> Option<HeroIcon> {
    if crop.kind != CropKind::HeroIcon {
        return None;
    }
    let hero = crop
        .hero
        .map(|raw| match hero_key_to_name(&raw) {
            Some(name) => name.to_string(),
            None => raw,
        })
        .unwrap_or_default();
    Some(HeroIcon {
        id: crop.id,
        member_id: crop.member_id,
        hero,
        file_name: crop.file_name,
        size_bytes: crop.size_bytes,
        width: crop.width,
        height: crop.height,
        created_at: crop.created_at,
    })
}

fn icons_from(rows: Vec<DbHeroIcon>) -> Vec<HeroIcon> {
    rows.into_iter()
        .filter_map(db_to_crop)
        .filter_map(crop_to_icon)
        .collect()
}

fn crop_to_db(crop: &NewTrainingCrop) -> Result<DbHeroIcon, crate::DbError> {
    if !crop.kind.accepts_uploads() {
        return Err(crate::DbError::Config("crop kind is never stored".into()));
    }
    let Ok(size_bytes) = i64::try_from(crop.size_bytes) else {
        return Err(crate::DbError::Config("icon size is too large".into()));
    };
    let meta = &crop.meta;
    let spot = meta.found_spot;
    Ok(DbHeroIcon {
        id: None,
        member_id: crop.member_id.clone(),
        // The column predates kinds and is a plain string: empty means no
        // hero. Stored as the hero key, whatever spelling the caller passed.
        hero: crop.hero.clone().map(stored_hero_key).unwrap_or_default(),
        file_name: crop.file_name.clone(),
        size_bytes,
        width: i64::from(crop.width),
        height: i64::from(crop.height),
        created_at: SurrealDatetime::from(crop.created_at),
        kind: Some(crop.kind.as_str().to_string()),
        screen: meta.screen.clone(),
        slot_type: meta.slot_type.clone(),
        slot_index: meta.slot_index.map(i64::from),
        game_token: meta.game_token.clone(),
        score: meta.score,
        runner_up: meta.runner_up,
        margin: meta.margin,
        icon_set_version: meta.icon_set_version.clone(),
        spot_version: meta.spot_version.clone(),
        vote_card_filled: meta.vote_card_filled,
        locked_in: meta.locked_in,
        screen_width: meta.screen_width.map(i64::from),
        screen_height: meta.screen_height.map(i64::from),
        ui_scale: meta.ui_scale,
        tracker_version: crop.tracker_version.clone(),
        found_x: spot.map(|s| i64::from(s.x)),
        found_y: spot.map(|s| i64::from(s.y)),
        found_w: spot.map(|s| i64::from(s.w)),
        found_h: spot.map(|s| i64::from(s.h)),
        found_confidence: meta.found_confidence,
    })
}

fn icon_to_crop(icon: &NewHeroIcon) -> NewTrainingCrop {
    TrainingCrop {
        id: icon.id.clone(),
        member_id: icon.member_id.clone(),
        kind: CropKind::HeroIcon,
        hero: Some(icon.hero.clone()),
        file_name: icon.file_name.clone(),
        size_bytes: icon.size_bytes,
        width: icon.width,
        height: icon.height,
        created_at: icon.created_at,
        tracker_version: None,
        meta: TrainingCropMeta::default(),
    }
}

/// Only `hero_icon` rows. Every hero icon query uses it.
const HERO_KIND: &str = "kind = 'hero_icon'";

#[derive(Deserialize, SurrealValue)]
struct CountResult {
    count: u64,
}

impl Database {
    pub async fn insert_hero_icon(&self, icon: &NewHeroIcon) -> DbResult<HeroIcon> {
        let crop = self.insert_training_crop(&icon_to_crop(icon)).await?;
        crop_to_icon(crop).ok_or_else(|| crate::DbError::NotFound("hero icon create".into()))
    }

    /// One hero icon. `None` for an unknown id and for a crop of another kind.
    pub async fn get_hero_icon(&self, id: &str) -> DbResult<Option<HeroIcon>> {
        Ok(self.get_training_crop(id).await?.and_then(crop_to_icon))
    }

    /// One member's icons, newest first: `created_at` descending, then record id
    /// descending. Each part of one upload request gets its own `created_at`
    /// one microsecond after the part before, so the last part comes first.
    pub async fn list_hero_icons_for_member(&self, member_id: &str) -> DbResult<Vec<HeroIcon>> {
        let member_id = member_id.to_string();
        with_timeout(async {
            let mut result = self
                .client
                .query(format!(
                    "SELECT * FROM hero_icon WHERE member_id = $member_id AND {HERO_KIND} \
                     ORDER BY created_at DESC, id DESC"
                ))
                .bind(("member_id", member_id))
                .await?;
            let rows: Vec<DbHeroIcon> = result.take(0)?;
            Ok(icons_from(rows))
        })
        .await
    }

    /// Icons of the given members, in the order of
    /// [`Self::list_hero_icons_for_member`]. Fetches `limit + 1` rows so the
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
                .query(format!(
                    "SELECT * FROM hero_icon WHERE member_id IN $ids AND {HERO_KIND} \
                     ORDER BY created_at DESC, id DESC LIMIT $lim START $off"
                ))
                .bind(("ids", ids))
                .bind(("lim", limit + 1))
                .bind(("off", offset))
                .await?;
            let rows: Vec<DbHeroIcon> = result.take(0)?;
            Ok(icons_from(rows))
        })
        .await
    }

    pub async fn count_hero_icons_for_member(&self, member_id: &str) -> DbResult<u64> {
        self.count_training_crops_for_member(member_id, CropKind::HeroIcon)
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
                .query(format!(
                    "SELECT member_id, count() AS count FROM hero_icon \
                     WHERE {HERO_KIND} GROUP BY member_id"
                ))
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

    /// Remove one `hero_icon` row and return it, or `None` when no hero icon
    /// has this id. A crop of another kind is never removed here; use
    /// [`Self::delete_training_crop`] for those.
    pub async fn delete_hero_icon(&self, id: &str) -> DbResult<Option<HeroIcon>> {
        let rid = RecordId::new("hero_icon", id);
        with_timeout(async {
            let mut result = self
                .client
                .query(format!(
                    "DELETE hero_icon WHERE id = $rid AND {HERO_KIND} RETURN BEFORE"
                ))
                .bind(("rid", rid))
                .await?;
            let rows: Vec<DbHeroIcon> = result.take(0)?;
            Ok(icons_from(rows).into_iter().next())
        })
        .await
    }

    /// The ids among `ids` that have a `hero_icon` row of any kind. Used by
    /// the orphan file sweep, since every kind shares one directory. An empty
    /// input does not query.
    pub async fn existing_hero_icon_ids(&self, ids: &[String]) -> DbResult<HashSet<String>> {
        if ids.is_empty() {
            return Ok(HashSet::new());
        }
        let rids: Vec<RecordId> = ids
            .iter()
            .map(|id| RecordId::new("hero_icon", id.as_str()))
            .collect();
        with_timeout(async {
            #[derive(Deserialize, SurrealValue)]
            struct Row {
                id: Option<RecordId>,
            }
            let mut result = self
                .client
                .query("SELECT id FROM hero_icon WHERE id IN $rids")
                .bind(("rids", rids))
                .await?;
            let rows: Vec<Row> = result.take(0)?;
            Ok(rows
                .into_iter()
                .filter_map(|row| row.id)
                .map(|rid| crate::record_id_key_to_string(rid.key))
                .collect())
        })
        .await
    }

    /// Store one crop. `full_screen` is refused here as well as in the route.
    pub async fn insert_training_crop(&self, crop: &NewTrainingCrop) -> DbResult<TrainingCrop> {
        let id = crop.id.clone();
        let row = crop_to_db(crop)?;
        with_timeout(async {
            let created: Option<DbHeroIcon> = self
                .client
                .create(("hero_icon", id.as_str()))
                .content(row)
                .await?;
            match created.and_then(db_to_crop) {
                Some(crop) => Ok(crop),
                None => Err(crate::DbError::NotFound("training crop create".into())),
            }
        })
        .await
    }

    pub async fn get_training_crop(&self, id: &str) -> DbResult<Option<TrainingCrop>> {
        let id = id.to_string();
        with_timeout(async {
            let row: Option<DbHeroIcon> = self.client.select(("hero_icon", id.as_str())).await?;
            Ok(row.and_then(db_to_crop))
        })
        .await
    }

    /// One member's crops, newest first like
    /// [`Self::list_hero_icons_for_member`], optionally of one kind. With
    /// `page = Some((limit, offset))` fetches `limit + 1` rows for
    /// `CursorResponse::from_oversized`; `None` returns every row.
    pub async fn list_training_crops_for_member(
        &self,
        member_id: &str,
        kind: Option<CropKind>,
        page: Option<(u32, u32)>,
    ) -> DbResult<Vec<TrainingCrop>> {
        let member_id = member_id.to_string();
        let kind_clause = if kind.is_some() {
            " AND kind = $kind"
        } else {
            ""
        };
        let page_clause = if page.is_some() {
            " LIMIT $lim START $off"
        } else {
            ""
        };
        let (limit, offset) = page.unwrap_or((0, 0));
        with_timeout(async {
            let mut result = self
                .client
                .query(format!(
                    "SELECT * FROM hero_icon WHERE member_id = $member_id{kind_clause} \
                     ORDER BY created_at DESC, id DESC{page_clause}"
                ))
                .bind(("member_id", member_id))
                .bind(("kind", kind.map(|k| k.as_str().to_string())))
                .bind(("lim", limit + 1))
                .bind(("off", offset))
                .await?;
            let rows: Vec<DbHeroIcon> = result.take(0)?;
            Ok(crops_from(rows))
        })
        .await
    }

    /// Crops staff may list: for each kind, the crops of the given members.
    /// Newest first, `limit + 1` rows from `offset`. Kinds with no members
    /// are left out; with none at all nothing is queried.
    pub async fn list_training_crops_visible(
        &self,
        visible: &[(CropKind, Vec<String>)],
        limit: u32,
        offset: u32,
    ) -> DbResult<Vec<TrainingCrop>> {
        let groups: Vec<&(CropKind, Vec<String>)> = visible
            .iter()
            .filter(|(kind, ids)| kind.accepts_uploads() && !ids.is_empty())
            .collect();
        if groups.is_empty() {
            return Ok(Vec::new());
        }
        // Fixed fragments with numbered bind names only; no value is
        // written into the query text.
        let clauses: Vec<String> = (0..groups.len())
            .map(|i| format!("(kind = $kind{i} AND member_id IN $ids{i})"))
            .collect();
        let sql = format!(
            "SELECT * FROM hero_icon WHERE {} \
             ORDER BY created_at DESC, id DESC LIMIT $lim START $off",
            clauses.join(" OR ")
        );
        with_timeout(async {
            let mut query = self.client.query(sql);
            for (i, (kind, ids)) in groups.iter().enumerate() {
                query = query
                    .bind((format!("kind{i}"), kind.as_str().to_string()))
                    .bind((format!("ids{i}"), ids.clone()));
            }
            let mut result = query.bind(("lim", limit + 1)).bind(("off", offset)).await?;
            let rows: Vec<DbHeroIcon> = result.take(0)?;
            Ok(crops_from(rows))
        })
        .await
    }

    pub async fn count_training_crops_for_member(
        &self,
        member_id: &str,
        kind: CropKind,
    ) -> DbResult<u64> {
        let member_id = member_id.to_string();
        with_timeout(async {
            let mut result = self
                .client
                .query(
                    "SELECT count() AS count FROM hero_icon \
                     WHERE member_id = $member_id AND kind = $kind GROUP ALL",
                )
                .bind(("member_id", member_id))
                .bind(("kind", kind.as_str().to_string()))
                .await?;
            let counts: Vec<CountResult> = result.take(0)?;
            Ok(counts.first().map(|c| c.count).unwrap_or(0))
        })
        .await
    }

    /// Stored crop counts per member, kind and game resolution. Rows with an
    /// unknown stored kind are skipped.
    pub async fn training_crop_counts(&self) -> DbResult<Vec<TrainingCropCountRow>> {
        with_timeout(async {
            #[derive(Deserialize, SurrealValue)]
            struct Row {
                member_id: String,
                #[serde(default)]
                #[surreal(default)]
                kind: Option<String>,
                #[serde(default)]
                #[surreal(default)]
                screen_width: Option<i64>,
                #[serde(default)]
                #[surreal(default)]
                screen_height: Option<i64>,
                count: u64,
            }
            let mut result = self
                .client
                .query(
                    "SELECT member_id, kind, screen_width, screen_height, count() AS count \
                     FROM hero_icon GROUP BY member_id, kind, screen_width, screen_height",
                )
                .await?;
            let rows: Vec<Row> = result.take(0)?;
            Ok(rows
                .into_iter()
                .filter_map(|row| {
                    Some(TrainingCropCountRow {
                        kind: stored_kind(row.kind.as_deref())?,
                        member_id: row.member_id,
                        screen_width: to_u32(row.screen_width),
                        screen_height: to_u32(row.screen_height),
                        count: row.count,
                    })
                })
                .collect())
        })
        .await
    }

    /// Ids of one member's rows, optionally of one kind. Reads ids only, so
    /// a row whose other columns fail to parse (an unknown kind, a negative
    /// `found_*`) is still listed. The delete paths use it; files are
    /// `<id>.png`, so the id is all they need.
    pub async fn training_crop_ids_for_member(
        &self,
        member_id: &str,
        kind: Option<CropKind>,
    ) -> DbResult<Vec<String>> {
        let member_id = member_id.to_string();
        let kind_clause = if kind.is_some() {
            " AND kind = $kind"
        } else {
            ""
        };
        with_timeout(async {
            #[derive(Deserialize, SurrealValue)]
            struct Row {
                id: Option<RecordId>,
            }
            let mut result = self
                .client
                .query(format!(
                    "SELECT id FROM hero_icon WHERE member_id = $member_id{kind_clause}"
                ))
                .bind(("member_id", member_id))
                .bind(("kind", kind.map(|k| k.as_str().to_string())))
                .await?;
            let rows: Vec<Row> = result.take(0)?;
            Ok(rows
                .into_iter()
                .filter_map(|row| row.id)
                .map(|rid| crate::record_id_key_to_string(rid.key))
                .collect())
        })
        .await
    }

    /// The owner and kind of one row, or `None` when no row has this id.
    /// Reads only those two columns, so a row that fails to parse in full
    /// is still found.
    pub async fn training_crop_owner(&self, id: &str) -> DbResult<Option<TrainingCropOwner>> {
        let rid = RecordId::new("hero_icon", id);
        with_timeout(async {
            #[derive(Deserialize, SurrealValue)]
            struct Row {
                member_id: String,
                #[serde(default)]
                #[surreal(default)]
                kind: Option<String>,
            }
            let mut result = self
                .client
                .query("SELECT member_id, kind FROM $rid")
                .bind(("rid", rid))
                .await?;
            let rows: Vec<Row> = result.take(0)?;
            Ok(rows.into_iter().next().map(|row| TrainingCropOwner {
                kind: stored_kind(row.kind.as_deref()),
                member_id: row.member_id,
            }))
        })
        .await
    }

    /// Remove one row of any kind by id. `true` when a row was removed. The
    /// removed row is not parsed, so a row that fails to parse is removed
    /// and counted like any other.
    pub async fn delete_training_crop(&self, id: &str) -> DbResult<bool> {
        let rid = RecordId::new("hero_icon", id);
        with_timeout(async {
            let mut result = self
                .client
                .query("DELETE $rid RETURN BEFORE")
                .bind(("rid", rid))
                .await?;
            let rows: Vec<surrealdb::types::Value> = result.take(0)?;
            Ok(!rows.is_empty())
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
        assert_eq!(db.count_hero_icons_for_member("m1").await.unwrap(), 1);
        assert_eq!(db.count_hero_icons_for_member("m2").await.unwrap(), 1);
    }

    #[tokio::test]
    async fn existing_ids_are_found_in_one_batch() {
        let db = test_db().await;
        let a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let b = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        db.insert_hero_icon(&new_icon(a, "m1", 1)).await.unwrap();

        assert!(db.existing_hero_icon_ids(&[]).await.unwrap().is_empty());
        let found = db
            .existing_hero_icon_ids(&[a.to_string(), b.to_string()])
            .await
            .unwrap();
        assert_eq!(found.len(), 1);
        assert!(found.contains(a));
    }

    #[tokio::test]
    async fn order_is_newest_first_then_id_descending() {
        let db = test_db().await;
        let at = Utc::now();
        let d = "dddddddddddddddddddddddddddddddd";
        let e = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
        let f = "ffffffffffffffffffffffffffffffff";
        // d and e share a time, so the id decides: e before d.
        for id in [e, d] {
            let mut icon = new_icon(id, "m1", 0);
            icon.created_at = at;
            db.insert_hero_icon(&icon).await.unwrap();
        }
        // The upload route gives each later part of one request one more
        // microsecond, so a later part sorts first even with a lower id.
        let mut later = new_icon(f, "m1", 0);
        later.created_at = at - Duration::microseconds(1);
        db.insert_hero_icon(&later).await.unwrap();
        let mut newest = new_icon("0000000000000000000000000000000a", "m1", 0);
        newest.created_at = at + Duration::microseconds(1);
        db.insert_hero_icon(&newest).await.unwrap();

        let mine = db.list_hero_icons_for_member("m1").await.unwrap();
        let ids: Vec<&str> = mine.iter().map(|icon| icon.id.as_str()).collect();
        assert_eq!(ids, vec!["0000000000000000000000000000000a", e, d, f]);

        let second_page = db
            .list_hero_icons_for_members(&["m1".to_string()], 1, 2)
            .await
            .unwrap();
        assert_eq!(second_page[0].id, d);
    }

    fn new_crop(id: &str, member_id: &str, kind: CropKind, minutes_ago: i64) -> NewTrainingCrop {
        TrainingCrop {
            id: id.to_string(),
            member_id: member_id.to_string(),
            kind,
            hero: None,
            file_name: format!("{id}.png"),
            size_bytes: 90,
            width: 40,
            height: 20,
            created_at: Utc::now() - Duration::minutes(minutes_ago),
            tracker_version: None,
            meta: TrainingCropMeta::default(),
        }
    }

    #[tokio::test]
    async fn crop_round_trips_every_metadata_field() {
        let db = test_db().await;
        let id = "1111111111111111111111111111111a";
        let mut crop = new_crop(id, "m1", CropKind::StatDigits, 0);
        crop.tracker_version = Some("0.6.0-alpha.3".into());
        crop.meta = TrainingCropMeta {
            screen: Some("scoreboard".into()),
            slot_type: Some("ally".into()),
            slot_index: Some(3),
            game_token: Some("tok-abcdefgh".into()),
            score: Some(0.91),
            runner_up: Some(0.4),
            margin: Some(0.51),
            icon_set_version: Some("2026.11".into()),
            spot_version: Some("v2".into()),
            vote_card_filled: Some(true),
            locked_in: Some(false),
            screen_width: Some(2560),
            screen_height: Some(1440),
            ui_scale: Some(100.0),
            found_spot: Some(CropSpot {
                x: 5,
                y: 6,
                w: 70,
                h: 30,
            }),
            found_confidence: Some(0.75),
        };
        let stored = db.insert_training_crop(&crop).await.unwrap();
        // Times come back at database precision; compare everything else.
        let mut expected = crop.clone();
        expected.created_at = stored.created_at;
        assert_eq!(stored, expected);
        let read = db.get_training_crop(id).await.unwrap().expect("row");
        assert_eq!(read, stored);
        // Not a hero icon: the hero icon view does not see it, and the hero
        // icon delete leaves it alone.
        assert!(db.get_hero_icon(id).await.unwrap().is_none());
        assert!(db.delete_hero_icon(id).await.unwrap().is_none());
        assert_eq!(db.get_training_crop(id).await.unwrap(), Some(stored));
        assert!(db.delete_training_crop(id).await.unwrap());
        assert!(db.get_training_crop(id).await.unwrap().is_none());
        assert!(!db.delete_training_crop(id).await.unwrap(), "already gone");
    }

    #[tokio::test]
    async fn full_screen_is_never_stored() {
        let db = test_db().await;
        let id = "2222222222222222222222222222222a";
        let crop = new_crop(id, "m1", CropKind::FullScreen, 0);
        assert!(db.insert_training_crop(&crop).await.is_err());
        assert!(db.get_training_crop(id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn hero_icon_queries_only_see_hero_icons() {
        let db = test_db().await;
        let icon = "3333333333333333333333333333333a";
        let digits = "3333333333333333333333333333333b";
        let map = "3333333333333333333333333333333c";
        db.insert_hero_icon(&new_icon(icon, "m1", 3)).await.unwrap();
        db.insert_training_crop(&new_crop(digits, "m1", CropKind::StatDigits, 2))
            .await
            .unwrap();
        db.insert_training_crop(&new_crop(map, "m1", CropKind::MapName, 1))
            .await
            .unwrap();

        let icons = db.list_hero_icons_for_member("m1").await.unwrap();
        assert_eq!(icons.len(), 1);
        assert_eq!(icons[0].id, icon);
        assert_eq!(db.count_hero_icons_for_member("m1").await.unwrap(), 1);
        assert_eq!(db.hero_icon_counts().await.unwrap().get("m1"), Some(&1));
        let page = db
            .list_hero_icons_for_members(&["m1".to_string()], 10, 0)
            .await
            .unwrap();
        assert_eq!(page.len(), 1);

        let all = db
            .list_training_crops_for_member("m1", None, None)
            .await
            .unwrap();
        let ids: Vec<&str> = all.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, vec![map, digits, icon]);
        assert_eq!(all[2].kind, CropKind::HeroIcon);
        // The crop view gives the hero key, the hero icon view the name.
        assert_eq!(all[2].hero.as_deref(), Some("ana"));
        assert_eq!(icons[0].hero, "Ana");
        let only_map = db
            .list_training_crops_for_member("m1", Some(CropKind::MapName), None)
            .await
            .unwrap();
        assert_eq!(only_map.len(), 1);
        let first_page = db
            .list_training_crops_for_member("m1", None, Some((1, 0)))
            .await
            .unwrap();
        assert_eq!(first_page.len(), 2, "limit + 1 rows");
        assert_eq!(
            db.count_training_crops_for_member("m1", CropKind::StatDigits)
                .await
                .unwrap(),
            1
        );
        // The sweep sees every kind, since they share one directory.
        let found = db
            .existing_hero_icon_ids(&[icon.into(), digits.into(), map.into()])
            .await
            .unwrap();
        assert_eq!(found.len(), 3);
    }

    #[tokio::test]
    async fn visible_list_filters_by_kind_and_member() {
        let db = test_db().await;
        let a = "4444444444444444444444444444444a";
        let b = "4444444444444444444444444444444b";
        let c = "4444444444444444444444444444444c";
        db.insert_training_crop(&new_crop(a, "m1", CropKind::StatDigits, 3))
            .await
            .unwrap();
        db.insert_training_crop(&new_crop(b, "m1", CropKind::MapName, 2))
            .await
            .unwrap();
        db.insert_training_crop(&new_crop(c, "m2", CropKind::StatDigits, 1))
            .await
            .unwrap();

        assert!(db
            .list_training_crops_visible(&[], 10, 0)
            .await
            .unwrap()
            .is_empty());
        let visible = vec![
            (
                CropKind::StatDigits,
                vec!["m1".to_string(), "m2".to_string()],
            ),
            (CropKind::MapName, Vec::new()),
        ];
        let rows = db
            .list_training_crops_visible(&visible, 10, 0)
            .await
            .unwrap();
        let ids: Vec<&str> = rows.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, vec![c, a]);
        let visible = vec![
            (CropKind::StatDigits, vec!["m2".to_string()]),
            (CropKind::MapName, vec!["m1".to_string()]),
        ];
        let rows = db
            .list_training_crops_visible(&visible, 10, 0)
            .await
            .unwrap();
        let ids: Vec<&str> = rows.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, vec![c, b]);
    }

    #[tokio::test]
    async fn counts_group_by_kind_and_resolution() {
        let db = test_db().await;
        let mut hd = new_crop(
            "5555555555555555555555555555555a",
            "m1",
            CropKind::StatDigits,
            3,
        );
        hd.meta.screen_width = Some(1920);
        hd.meta.screen_height = Some(1080);
        let mut hd2 = hd.clone();
        hd2.id = "5555555555555555555555555555555b".into();
        let qhd = {
            let mut c = hd.clone();
            c.id = "5555555555555555555555555555555c".into();
            c.meta.screen_width = Some(2560);
            c.meta.screen_height = Some(1440);
            c
        };
        let none = new_crop(
            "5555555555555555555555555555555d",
            "m1",
            CropKind::MapName,
            1,
        );
        for crop in [&hd, &hd2, &qhd, &none] {
            db.insert_training_crop(crop).await.unwrap();
        }
        db.insert_hero_icon(&new_icon("5555555555555555555555555555555e", "m2", 1))
            .await
            .unwrap();

        let mut rows = db.training_crop_counts().await.unwrap();
        rows.sort_by_key(|r| (r.member_id.clone(), r.kind, r.screen_width));
        type Flat<'a> = (&'a str, CropKind, Option<u32>, Option<u32>, u64);
        let flat: Vec<Flat> = rows
            .iter()
            .map(|r| {
                (
                    r.member_id.as_str(),
                    r.kind,
                    r.screen_width,
                    r.screen_height,
                    r.count,
                )
            })
            .collect();
        assert_eq!(
            flat,
            vec![
                ("m1", CropKind::StatDigits, Some(1920), Some(1080), 2),
                ("m1", CropKind::StatDigits, Some(2560), Some(1440), 1),
                ("m1", CropKind::MapName, None, None, 1),
                ("m2", CropKind::HeroIcon, None, None, 1),
            ]
        );
    }

    #[tokio::test]
    async fn legacy_rows_without_kind_become_hero_icons() {
        let db = test_db().await;
        let legacy = "6666666666666666666666666666666a";
        // A row written before `kind` existed.
        db.client
            .query(format!(
                "REMOVE FIELD IF EXISTS kind ON hero_icon; \
                 CREATE hero_icon:{legacy} SET member_id = 'm1', hero = 'Ana', \
                 file_name = '{legacy}.png', size_bytes = 10, width = 16, height = 16, \
                 created_at = time::now();"
            ))
            .await
            .unwrap()
            .check()
            .unwrap();
        // The next boot redefines the field and backfills the row.
        run_migrations(&db.client).await.unwrap();

        #[derive(Deserialize, SurrealValue)]
        struct KindRow {
            kind: Option<String>,
        }
        let mut result = db
            .client
            .query(format!("SELECT kind FROM hero_icon:{legacy}"))
            .await
            .unwrap();
        let rows: Vec<KindRow> = result.take(0).unwrap();
        assert_eq!(rows[0].kind.as_deref(), Some("hero_icon"));
        let icons = db.list_hero_icons_for_member("m1").await.unwrap();
        assert_eq!(icons.len(), 1);
        assert_eq!(icons[0].hero, "Ana");
        // The legacy display name reads as the hero key in the crop view.
        let crop = db.get_training_crop(legacy).await.unwrap().expect("row");
        assert_eq!(crop.hero.as_deref(), Some("ana"));
        assert_eq!(
            db.count_training_crops_for_member("m1", CropKind::HeroIcon)
                .await
                .unwrap(),
            1
        );
        // A second boot is a no-op.
        run_migrations(&db.client).await.unwrap();
        assert_eq!(db.list_hero_icons_for_member("m1").await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn delete_hero_icon_only_removes_hero_icons() {
        let db = test_db().await;
        let icon = "7777777777777777777777777777777a";
        let digits = "7777777777777777777777777777777b";
        let map = "7777777777777777777777777777777c";
        db.insert_hero_icon(&new_icon(icon, "m1", 3)).await.unwrap();
        db.insert_training_crop(&new_crop(digits, "m1", CropKind::StatDigits, 2))
            .await
            .unwrap();
        db.insert_training_crop(&new_crop(map, "m1", CropKind::MapName, 1))
            .await
            .unwrap();

        for other in [digits, map] {
            assert!(db.delete_hero_icon(other).await.unwrap().is_none());
            assert!(db.get_training_crop(other).await.unwrap().is_some());
        }
        assert!(db.delete_hero_icon("nope").await.unwrap().is_none());
        let removed = db.delete_hero_icon(icon).await.unwrap().expect("icon");
        assert_eq!(removed.id, icon);
        assert_eq!(removed.hero, "Ana");
        assert!(db.get_training_crop(icon).await.unwrap().is_none());
        assert_eq!(
            db.list_training_crops_for_member("m1", None, None)
                .await
                .unwrap()
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn hero_is_stored_as_the_hero_key() {
        let db = test_db().await;
        let icon = "8888888888888888888888888888888a";
        let crop = "8888888888888888888888888888888b";
        let mut legacy_spelling = new_icon(icon, "m1", 2);
        legacy_spelling.hero = "Soldier: 76".into();
        db.insert_hero_icon(&legacy_spelling).await.unwrap();
        let mut digits = new_crop(crop, "m1", CropKind::StatDigits, 1);
        digits.hero = Some("Lúcio".into());
        db.insert_training_crop(&digits).await.unwrap();

        #[derive(Deserialize, SurrealValue)]
        struct HeroRow {
            hero: String,
        }
        for (id, key) in [(icon, "soldier-76"), (crop, "lucio")] {
            let mut result = db
                .client
                .query(format!("SELECT hero FROM hero_icon:{id}"))
                .await
                .unwrap();
            let rows: Vec<HeroRow> = result.take(0).unwrap();
            assert_eq!(rows[0].hero, key, "{id}");
        }
        // The hero icon view still answers with the display name.
        let icons = db.list_hero_icons_for_member("m1").await.unwrap();
        assert_eq!(icons[0].hero, "Soldier: 76");
        let stored = db.get_training_crop(crop).await.unwrap().expect("row");
        assert_eq!(stored.hero.as_deref(), Some("lucio"));
    }

    #[tokio::test]
    async fn rows_that_fail_to_parse_are_still_found_and_deleted() {
        let db = test_db().await;
        let good = "9999999999999999999999999999999a";
        let bad_spot = "9999999999999999999999999999999b";
        let other = "9999999999999999999999999999999c";
        db.insert_training_crop(&new_crop(good, "m1", CropKind::StatDigits, 3))
            .await
            .unwrap();
        db.insert_training_crop(&new_crop(bad_spot, "m1", CropKind::MapName, 2))
            .await
            .unwrap();
        db.insert_training_crop(&new_crop(other, "m2", CropKind::MapName, 1))
            .await
            .unwrap();
        // A negative found spot passes the schema but not `db_to_crop`.
        db.client
            .query(format!(
                "UPDATE hero_icon:{bad_spot} SET found_x = -1, found_y = 0, found_w = 4, \
                 found_h = 4, found_confidence = 0.5"
            ))
            .await
            .unwrap()
            .check()
            .unwrap();
        // The parsed views hide it.
        assert!(db.get_training_crop(bad_spot).await.unwrap().is_none());
        let listed = db
            .list_training_crops_for_member("m1", None, None)
            .await
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, good);

        // The id-only reads still see it.
        let mut ids = db.training_crop_ids_for_member("m1", None).await.unwrap();
        ids.sort();
        assert_eq!(ids, vec![good.to_string(), bad_spot.to_string()]);
        assert_eq!(
            db.training_crop_ids_for_member("m1", Some(CropKind::MapName))
                .await
                .unwrap(),
            vec![bad_spot.to_string()]
        );
        assert!(db
            .training_crop_ids_for_member("m1", Some(CropKind::HeroIcon))
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            db.training_crop_owner(bad_spot).await.unwrap(),
            Some(TrainingCropOwner {
                member_id: "m1".into(),
                kind: Some(CropKind::MapName),
            })
        );
        assert_eq!(db.training_crop_owner("nope").await.unwrap(), None);

        // The id-only delete removes and counts it.
        assert!(db.delete_training_crop(bad_spot).await.unwrap());
        assert_eq!(
            db.training_crop_ids_for_member("m1", None).await.unwrap(),
            vec![good.to_string()]
        );
        assert_eq!(
            db.training_crop_ids_for_member("m2", None).await.unwrap(),
            vec![other.to_string()]
        );
    }

    #[tokio::test]
    async fn unknown_stored_kind_is_found_by_owner_and_deleted() {
        let db = test_db().await;
        let id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa1";
        // A kind this build does not know, as a later build might store:
        // drop the ASSERT for the write, the next boot puts it back.
        db.client
            .query(format!(
                "DEFINE FIELD OVERWRITE kind ON hero_icon TYPE string; \
                 CREATE hero_icon:{id} SET member_id = 'm1', hero = '', kind = 'replay', \
                 file_name = '{id}.png', size_bytes = 10, width = 16, height = 16, \
                 created_at = time::now();"
            ))
            .await
            .unwrap()
            .check()
            .unwrap();
        run_migrations(&db.client).await.unwrap();
        assert!(db.get_training_crop(id).await.unwrap().is_none());
        assert_eq!(
            db.training_crop_ids_for_member("m1", None).await.unwrap(),
            vec![id.to_string()]
        );
        assert_eq!(
            db.training_crop_owner(id).await.unwrap(),
            Some(TrainingCropOwner {
                member_id: "m1".into(),
                kind: None,
            })
        );
        assert!(db.delete_training_crop(id).await.unwrap());
        assert!(db.training_crop_owner(id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn colorblind_filter_has_no_field() {
        let db = test_db().await;
        let mut result = db.client.query("INFO FOR TABLE hero_icon").await.unwrap();
        let info: Option<serde_json::Value> = result.take(0).unwrap();
        let info = info.expect("table info");
        let names: Vec<&String> = info["fields"].as_object().expect("fields").keys().collect();
        assert!(names.iter().any(|n| n.as_str() == "ui_scale"), "{names:?}");
        assert!(
            !names.iter().any(|n| n.as_str() == "colorblind_filter"),
            "{names:?}"
        );
    }

    /// Rollback check: the build before training crops reads hero_icon rows
    /// with this exact struct (`SELECT *` and `select`). A row this build
    /// writes, with `kind` and every new metadata column set, must still
    /// read there; extra columns are ignored by the derive.
    #[tokio::test]
    async fn old_build_row_shape_reads_rows_with_kind_and_metadata() {
        #[derive(Debug, Deserialize, SurrealValue)]
        struct OldDbHeroIcon {
            #[surreal(default)]
            #[serde(default)]
            id: Option<RecordId>,
            member_id: String,
            hero: String,
            file_name: String,
            size_bytes: i64,
            width: i64,
            height: i64,
            #[allow(dead_code)]
            created_at: SurrealDatetime,
        }
        let db = test_db().await;
        let id = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb1";
        let mut icon = new_crop(id, "m1", CropKind::HeroIcon, 0);
        icon.hero = Some("Soldier: 76".into());
        icon.tracker_version = Some("0.6.0-alpha.3".into());
        icon.meta = TrainingCropMeta {
            screen: Some("scoreboard".into()),
            slot_type: Some("ally".into()),
            slot_index: Some(3),
            game_token: Some("0123456789abcdef0123456789abcdef".into()),
            score: Some(0.9),
            runner_up: Some(0.4),
            margin: Some(0.5),
            icon_set_version: Some("2026.11".into()),
            spot_version: Some("v2".into()),
            vote_card_filled: Some(true),
            locked_in: Some(false),
            screen_width: Some(2560),
            screen_height: Some(1440),
            ui_scale: Some(100.0),
            found_spot: Some(CropSpot {
                x: 1,
                y: 2,
                w: 3,
                h: 4,
            }),
            found_confidence: Some(0.5),
        };
        db.insert_training_crop(&icon).await.unwrap();

        let mut result = db
            .client
            .query("SELECT * FROM hero_icon WHERE member_id = 'm1'")
            .await
            .unwrap();
        let rows: Vec<OldDbHeroIcon> = result.take(0).unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(
            row.id
                .clone()
                .map(|r| crate::record_id_key_to_string(r.key)),
            Some(id.to_string())
        );
        assert_eq!(row.member_id, "m1");
        // The hero key, as the rollback doc says: the old build shows it as
        // written.
        assert_eq!(row.hero, "soldier-76");
        assert_eq!(row.file_name, format!("{id}.png"));
        assert_eq!((row.size_bytes, row.width, row.height), (90, 40, 20));
        let one: Option<OldDbHeroIcon> = db.client.select(("hero_icon", id)).await.unwrap();
        assert!(one.is_some());
    }
}
