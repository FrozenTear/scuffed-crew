use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use surrealdb_types::{RecordId, SurrealValue};

use crate::types::MemberSettings;
use crate::{with_timeout, Database, DbResult};
use scuffed_types::{StatsVisibility, TrainingCropConsent, TrainingCropConsentUpdate};

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
    /// Absent on rows written before `hero_icon_opt_in` existed. Reads as false.
    #[serde(default)]
    #[surreal(default)]
    hero_icon_opt_in: Option<bool>,
    /// Training crop consent for the other kinds. Absent on older rows.
    /// Reads as false.
    #[serde(default)]
    #[surreal(default)]
    stat_digits_opt_in: Option<bool>,
    #[serde(default)]
    #[surreal(default)]
    map_name_opt_in: Option<bool>,
    #[serde(default)]
    #[surreal(default)]
    full_screen_opt_in: Option<bool>,
}

fn db_to_consent(db: &DbMemberSettings) -> TrainingCropConsent {
    TrainingCropConsent {
        hero_icon: db.hero_icon_opt_in.unwrap_or(false),
        stat_digits: db.stat_digits_opt_in.unwrap_or(false),
        map_name: db.map_name_opt_in.unwrap_or(false),
        full_screen: db.full_screen_opt_in.unwrap_or(false),
    }
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
        hero_icon_opt_in: db.hero_icon_opt_in.unwrap_or(false),
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
                let Some(rid) = existing.id else {
                    return Err(crate::DbError::NotFound("member_settings id".into()));
                };
                // Only the two fields this call owns are written, so a concurrent
                // hero icon opt-in change is never reverted. `??` keeps the stored
                // visibility when none is given, and fills fields a legacy row lacks.
                let mut result = self
                    .client
                    .query(
                        "UPDATE $rid SET player_name = $pn, \
                         stats_visibility = $vis ?? stats_visibility ?? 'public', \
                         hero_icon_opt_in = hero_icon_opt_in ?? false",
                    )
                    .bind(("rid", rid))
                    .bind(("pn", player_name.map(|s| s.to_string())))
                    .bind(("vis", stats_visibility.map(|v| v.as_str().to_string())))
                    .await?;
                let updated: Vec<DbMemberSettings> = result.take(0)?;
                match updated.into_iter().next() {
                    Some(row) => Ok(db_to_settings(row)),
                    None => Err(crate::DbError::NotFound("member_settings update".into())),
                }
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
                        hero_icon_opt_in: Some(false),
                        stat_digits_opt_in: Some(false),
                        map_name_opt_in: Some(false),
                        full_screen_opt_in: Some(false),
                    })
                    .await?;
                Ok(db_to_settings(created.ok_or_else(|| {
                    crate::DbError::NotFound("failed to create member_settings".into())
                })?))
            }
        })
        .await
    }

    /// Hero icon contribution opt-in. False when the member has no row.
    pub async fn get_hero_icon_opt_in(&self, member_id: &str) -> DbResult<bool> {
        Ok(self
            .get_member_settings(member_id)
            .await?
            .is_some_and(|s| s.hero_icon_opt_in))
    }

    /// Set only `hero_icon_opt_in` and return the stored value.
    ///
    /// One `UPSERT` keyed on `member_id`, so there is no read before the write.
    /// Player name and stats visibility are kept. A member without a row gets
    /// one with no player name and public stats, like a first
    /// [`Self::upsert_member_settings`] call. Two first calls for the same
    /// member can both try to create the row. The unique `ms_member_idx` lets
    /// only one through, so a failed attempt is tried once more, and by then it
    /// finds the row and updates it.
    pub async fn set_hero_icon_opt_in(&self, member_id: &str, opt_in: bool) -> DbResult<bool> {
        with_timeout(async {
            match self.upsert_hero_icon_opt_in(member_id, opt_in).await {
                Ok(value) => Ok(value),
                Err(error) => {
                    tracing::debug!(%error, "hero icon opt-in upsert retried");
                    self.upsert_hero_icon_opt_in(member_id, opt_in).await
                }
            }
        })
        .await
    }

    async fn upsert_hero_icon_opt_in(&self, member_id: &str, opt_in: bool) -> DbResult<bool> {
        // Only the opt-in is written. `??` fills a visibility that a new row or a
        // row from before `stats_visibility` lacks and keeps every other stored
        // value, so a concurrent stats settings change is never reverted.
        let mut result = self
            .client
            .query(
                "UPSERT member_settings SET member_id = $mid, hero_icon_opt_in = $v, \
                 stats_visibility = stats_visibility ?? 'public' \
                 WHERE member_id = $mid",
            )
            .bind(("mid", member_id.to_string()))
            .bind(("v", opt_in))
            .await?;
        let rows: Vec<DbMemberSettings> = result.take(0)?;
        match rows.into_iter().next() {
            Some(row) => Ok(row.hero_icon_opt_in.unwrap_or(false)),
            None => Err(crate::DbError::NotFound("member_settings upsert".into())),
        }
    }

    /// Training crop consent per kind. All false when the member has no row.
    /// `hero_icon` is the same flag as [`Self::get_hero_icon_opt_in`].
    pub async fn get_training_crop_consent(
        &self,
        member_id: &str,
    ) -> DbResult<TrainingCropConsent> {
        with_timeout(async {
            let mut result = self
                .client
                .query("SELECT * FROM member_settings WHERE member_id = $mid LIMIT 1")
                .bind(("mid", member_id.to_string()))
                .await?;
            let rows: Vec<DbMemberSettings> = result.take(0)?;
            Ok(rows.first().map(db_to_consent).unwrap_or_default())
        })
        .await
    }

    /// Change only the kinds `update` names and return every kind as stored.
    ///
    /// One `UPSERT` keyed on `member_id`, like [`Self::set_hero_icon_opt_in`]
    /// (same one retry for two first writes racing on `ms_member_idx`). A
    /// kind left out keeps its stored value, or false on a new or legacy row.
    /// Player name and stats visibility are kept.
    pub async fn set_training_crop_consent(
        &self,
        member_id: &str,
        update: TrainingCropConsentUpdate,
    ) -> DbResult<TrainingCropConsent> {
        with_timeout(async {
            match self.upsert_training_crop_consent(member_id, update).await {
                Ok(value) => Ok(value),
                Err(error) => {
                    tracing::debug!(%error, "training crop consent upsert retried");
                    self.upsert_training_crop_consent(member_id, update).await
                }
            }
        })
        .await
    }

    async fn upsert_training_crop_consent(
        &self,
        member_id: &str,
        update: TrainingCropConsentUpdate,
    ) -> DbResult<TrainingCropConsent> {
        let mut result = self
            .client
            .query(
                "UPSERT member_settings SET member_id = $mid, \
                 hero_icon_opt_in = $hero ?? hero_icon_opt_in ?? false, \
                 stat_digits_opt_in = $digits ?? stat_digits_opt_in ?? false, \
                 map_name_opt_in = $map ?? map_name_opt_in ?? false, \
                 full_screen_opt_in = $full ?? full_screen_opt_in ?? false, \
                 stats_visibility = stats_visibility ?? 'public' \
                 WHERE member_id = $mid",
            )
            .bind(("mid", member_id.to_string()))
            .bind(("hero", update.hero_icon))
            .bind(("digits", update.stat_digits))
            .bind(("map", update.map_name))
            .bind(("full", update.full_screen))
            .await?;
        let rows: Vec<DbMemberSettings> = result.take(0)?;
        match rows.first() {
            Some(row) => Ok(db_to_consent(row)),
            None => Err(crate::DbError::NotFound("member_settings upsert".into())),
        }
    }

    /// Consent of every member who consents to at least one kind.
    pub async fn training_crop_consents(&self) -> DbResult<HashMap<String, TrainingCropConsent>> {
        with_timeout(async {
            let mut result = self
                .client
                .query(
                    "SELECT * FROM member_settings WHERE hero_icon_opt_in = true \
                     OR stat_digits_opt_in = true OR map_name_opt_in = true \
                     OR full_screen_opt_in = true",
                )
                .await?;
            let rows: Vec<DbMemberSettings> = result.take(0)?;
            Ok(rows
                .iter()
                .map(|row| (row.member_id.clone(), db_to_consent(row)))
                .filter(|(_, consent)| consent.any())
                .collect())
        })
        .await
    }

    /// Member ids that are opted in to contributing hero icons.
    pub async fn hero_icon_opted_in_member_ids(&self) -> DbResult<Vec<String>> {
        with_timeout(async {
            #[derive(Deserialize, SurrealValue)]
            struct Row {
                member_id: String,
            }
            let mut result = self
                .client
                .query("SELECT member_id FROM member_settings WHERE hero_icon_opt_in = true")
                .await?;
            let rows: Vec<Row> = result.take(0)?;
            Ok(rows.into_iter().map(|row| row.member_id).collect())
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

    #[tokio::test]
    async fn legacy_rows_take_settings_and_opt_in_writes() {
        let db = test_db().await;
        db.client
            .query(
                r#"
                REMOVE FIELD IF EXISTS stats_visibility ON member_settings;
                REMOVE FIELD IF EXISTS hero_icon_opt_in ON member_settings;
                CREATE member_settings SET member_id = 'legacy-a', player_name = 'Old';
                CREATE member_settings SET member_id = 'legacy-b', player_name = 'Old';
                DEFINE FIELD OVERWRITE stats_visibility ON member_settings TYPE string
                    DEFAULT 'public'
                    ASSERT $value IN ['public', 'members', 'private'];
                DEFINE FIELD OVERWRITE hero_icon_opt_in ON member_settings TYPE bool
                    DEFAULT false;
                "#,
            )
            .await
            .unwrap()
            .check()
            .unwrap();

        let kept = db
            .upsert_member_settings("legacy-a", Some("New"), None)
            .await
            .unwrap();
        assert_eq!(kept.player_name.as_deref(), Some("New"));
        assert_eq!(kept.stats_visibility, StatsVisibility::Public);
        assert!(!kept.hero_icon_opt_in);

        assert!(db.set_hero_icon_opt_in("legacy-b", true).await.unwrap());
        let after = db
            .get_member_settings("legacy-b")
            .await
            .unwrap()
            .expect("row");
        assert!(after.hero_icon_opt_in);
        assert_eq!(after.player_name.as_deref(), Some("Old"));
        assert_eq!(after.stats_visibility, StatsVisibility::Public);
    }

    #[tokio::test]
    async fn new_settings_row_starts_opted_out_of_hero_icons() {
        let db = test_db().await;
        let created = db
            .upsert_member_settings("m3", Some("Ana"), None)
            .await
            .unwrap();
        assert!(!created.hero_icon_opt_in);
        assert!(!db.get_hero_icon_opt_in("m3").await.unwrap());
        assert!(!db.get_hero_icon_opt_in("nobody").await.unwrap());
    }

    #[tokio::test]
    async fn hero_icon_opt_in_survives_settings_upsert() {
        let db = test_db().await;
        assert!(db.set_hero_icon_opt_in("m2", true).await.unwrap());
        assert!(db.get_hero_icon_opt_in("m2").await.unwrap());
        let created = db.get_member_settings("m2").await.unwrap().expect("row");
        assert_eq!(created.stats_visibility, StatsVisibility::Public);
        assert_eq!(created.player_name, None);

        let kept = db
            .upsert_member_settings("m2", Some("Mei"), Some(StatsVisibility::Members))
            .await
            .unwrap();
        assert!(kept.hero_icon_opt_in);
        assert!(db.get_hero_icon_opt_in("m2").await.unwrap());
        assert_eq!(
            db.hero_icon_opted_in_member_ids().await.unwrap(),
            vec!["m2".to_string()]
        );

        assert!(!db.set_hero_icon_opt_in("m2", false).await.unwrap());
        let after = db.get_member_settings("m2").await.unwrap().expect("row");
        assert!(!after.hero_icon_opt_in);
        assert_eq!(after.player_name.as_deref(), Some("Mei"));
        assert_eq!(after.stats_visibility, StatsVisibility::Members);
        assert!(db.hero_icon_opted_in_member_ids().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn concurrent_first_opt_in_writes_leave_one_row() {
        let db = test_db().await;
        let (first, second) = tokio::join!(
            db.set_hero_icon_opt_in("race", true),
            db.set_hero_icon_opt_in("race", true)
        );
        assert!(first.unwrap());
        assert!(second.unwrap());

        assert!(!db.set_hero_icon_opt_in("race", false).await.unwrap());

        #[derive(Deserialize, SurrealValue)]
        struct CountResult {
            count: u64,
        }
        let mut result = db
            .client
            .query("SELECT count() AS count FROM member_settings WHERE member_id = $mid GROUP ALL")
            .bind(("mid", "race".to_string()))
            .await
            .unwrap();
        let counts: Vec<CountResult> = result.take(0).unwrap();
        assert_eq!(counts.first().map(|c| c.count), Some(1));

        let settings = db.get_member_settings("race").await.unwrap().expect("row");
        assert!(!settings.hero_icon_opt_in);
        assert_eq!(settings.player_name, None);
        assert_eq!(settings.stats_visibility, StatsVisibility::Public);
    }

    #[tokio::test]
    async fn crop_consent_is_off_by_default_and_per_kind() {
        let db = test_db().await;
        assert_eq!(
            db.get_training_crop_consent("nobody").await.unwrap(),
            TrainingCropConsent::default()
        );
        let set = db
            .set_training_crop_consent(
                "c1",
                TrainingCropConsentUpdate {
                    map_name: Some(true),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(
            set,
            TrainingCropConsent {
                map_name: true,
                ..Default::default()
            }
        );
        // A later update of another kind keeps map_name.
        let set = db
            .set_training_crop_consent(
                "c1",
                TrainingCropConsentUpdate {
                    hero_icon: Some(true),
                    full_screen: Some(true),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(set.map_name && set.hero_icon && set.full_screen && !set.stat_digits);
        // hero_icon consent is the hero icon opt-in.
        assert!(db.get_hero_icon_opt_in("c1").await.unwrap());
        assert!(!db.set_hero_icon_opt_in("c1", false).await.unwrap());
        assert!(!db.get_training_crop_consent("c1").await.unwrap().hero_icon);
        assert!(db.get_training_crop_consent("c1").await.unwrap().map_name);

        let all = db.training_crop_consents().await.unwrap();
        assert_eq!(all.len(), 1);
        assert!(all["c1"].map_name);
        // Settings writes keep consent.
        db.upsert_member_settings("c1", Some("Kiriko"), Some(StatsVisibility::Members))
            .await
            .unwrap();
        assert!(
            db.get_training_crop_consent("c1")
                .await
                .unwrap()
                .full_screen
        );
        let row = db.get_member_settings("c1").await.unwrap().expect("row");
        assert_eq!(row.stats_visibility, StatsVisibility::Members);
    }

    #[tokio::test]
    async fn old_hero_icon_opt_in_rows_map_to_hero_icon_consent_only() {
        let db = test_db().await;
        db.client
            .query(
                r#"
                REMOVE FIELD IF EXISTS stat_digits_opt_in ON member_settings;
                REMOVE FIELD IF EXISTS map_name_opt_in ON member_settings;
                REMOVE FIELD IF EXISTS full_screen_opt_in ON member_settings;
                CREATE member_settings SET member_id = 'old-in', player_name = 'Old',
                    stats_visibility = 'public', hero_icon_opt_in = true;
                CREATE member_settings SET member_id = 'old-out', player_name = 'Old',
                    stats_visibility = 'public', hero_icon_opt_in = false;
                "#,
            )
            .await
            .unwrap()
            .check()
            .unwrap();
        run_migrations(&db.client).await.unwrap();

        assert_eq!(
            db.get_training_crop_consent("old-in").await.unwrap(),
            TrainingCropConsent {
                hero_icon: true,
                ..Default::default()
            }
        );
        assert_eq!(
            db.get_training_crop_consent("old-out").await.unwrap(),
            TrainingCropConsent::default()
        );
        let all = db.training_crop_consents().await.unwrap();
        assert_eq!(all.keys().collect::<Vec<_>>(), vec!["old-in"]);
        // The old row takes a new kind without losing the old opt-in.
        let set = db
            .set_training_crop_consent(
                "old-in",
                TrainingCropConsentUpdate {
                    stat_digits: Some(true),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(set.hero_icon && set.stat_digits && !set.map_name);
    }

    #[tokio::test]
    async fn concurrent_first_consent_writes_leave_one_row() {
        let db = test_db().await;
        let update = TrainingCropConsentUpdate {
            stat_digits: Some(true),
            ..Default::default()
        };
        let (first, second) = tokio::join!(
            db.set_training_crop_consent("race2", update),
            db.set_training_crop_consent("race2", update)
        );
        assert!(first.unwrap().stat_digits);
        assert!(second.unwrap().stat_digits);
        let settings = db.get_member_settings("race2").await.unwrap().expect("row");
        assert_eq!(settings.stats_visibility, StatsVisibility::Public);
        assert!(!settings.hero_icon_opt_in);
    }
}
