use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use surrealdb::types::Datetime as SurrealDatetime;
use surrealdb_types::{RecordId, SurrealValue};

use crate::types::{HeroStats, MapStats, PersonalMatch, PersonalStats};
use crate::{with_timeout, Database, DbResult};

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbPersonalMatch {
    #[surreal(default)]
    #[allow(dead_code)]
    id: Option<RecordId>,
    member_id: String,
    hero: String,
    map_name: String,
    game_mode: String,
    role: String,
    outcome: String,
    elims: u32,
    deaths: u32,
    assists: u32,
    damage: u32,
    healing: u32,
    mitigation: u32,
    played_at: SurrealDatetime,
    uploaded_at: SurrealDatetime,
}

fn db_to_personal_match(db: DbPersonalMatch) -> PersonalMatch {
    let id = db
        .id
        .map(|r| crate::record_id_key_to_string(r.key))
        .unwrap_or_else(|| "unknown".to_string());
    PersonalMatch {
        id,
        member_id: db.member_id,
        hero: db.hero,
        map_name: db.map_name,
        game_mode: db.game_mode,
        role: db.role,
        outcome: db.outcome,
        elims: db.elims,
        deaths: db.deaths,
        assists: db.assists,
        damage: db.damage,
        healing: db.healing,
        mitigation: db.mitigation,
        played_at: db.played_at.into(),
        uploaded_at: db.uploaded_at.into(),
    }
}

impl Database {
    pub async fn insert_personal_match(
        &self,
        member_id: &str,
        hero: &str,
        map_name: &str,
        game_mode: &str,
        role: &str,
        outcome: &str,
        elims: u32,
        deaths: u32,
        assists: u32,
        damage: u32,
        healing: u32,
        mitigation: u32,
        played_at: DateTime<Utc>,
    ) -> DbResult<PersonalMatch> {
        with_timeout(async {
            let db_match = DbPersonalMatch {
                id: None,
                member_id: member_id.to_string(),
                hero: hero.to_string(),
                map_name: map_name.to_string(),
                game_mode: game_mode.to_string(),
                role: role.to_string(),
                outcome: outcome.to_string(),
                elims,
                deaths,
                assists,
                damage,
                healing,
                mitigation,
                played_at: SurrealDatetime::from(played_at),
                uploaded_at: SurrealDatetime::from(Utc::now()),
            };
            let created: Option<DbPersonalMatch> = self
                .client
                .create("personal_match")
                .content(db_match)
                .await?;
            Ok(db_to_personal_match(created.ok_or_else(|| {
                crate::DbError::NotFound("Failed to insert personal match".into())
            })?))
        })
        .await
    }

    pub async fn bulk_insert_personal_matches(
        &self,
        member_id: &str,
        matches: &[crate::types::PersonalMatch],
    ) -> DbResult<u32> {
        let mut inserted = 0u32;
        for m in matches {
            match self
                .insert_personal_match(
                    member_id,
                    &m.hero,
                    &m.map_name,
                    &m.game_mode,
                    &m.role,
                    &m.outcome,
                    m.elims,
                    m.deaths,
                    m.assists,
                    m.damage,
                    m.healing,
                    m.mitigation,
                    m.played_at,
                )
                .await
            {
                Ok(_) => inserted += 1,
                Err(crate::DbError::Surreal(e)) if e.to_string().contains("unique") => {
                    tracing::debug!("Skipping duplicate personal match: {e}");
                }
                Err(e) => return Err(e),
            }
        }
        Ok(inserted)
    }

    pub async fn list_personal_matches(
        &self,
        member_id: &str,
        limit: u32,
        offset: u32,
    ) -> DbResult<Vec<PersonalMatch>> {
        with_timeout(async {
            let fetch = limit + 1;
            let mut result = self
                .client
                .query("SELECT * FROM personal_match WHERE member_id = $mid ORDER BY played_at DESC LIMIT $lim START $off")
                .bind(("mid", member_id.to_string()))
                .bind(("lim", fetch))
                .bind(("off", offset))
                .await?;
            let matches: Vec<DbPersonalMatch> = result.take(0)?;
            Ok(matches.into_iter().map(db_to_personal_match).collect())
        })
        .await
    }

    pub async fn get_personal_stats(&self, member_id: &str) -> DbResult<PersonalStats> {
        with_timeout(async {
            #[derive(Deserialize, SurrealValue)]
            struct CountResult {
                count: u32,
            }

            let mut total_result = self
                .client
                .query("SELECT count() FROM personal_match WHERE member_id = $mid GROUP ALL")
                .bind(("mid", member_id.to_string()))
                .await?;
            let total: Vec<CountResult> = total_result.take(0)?;

            let mut wins_result = self
                .client
                .query("SELECT count() FROM personal_match WHERE member_id = $mid AND outcome = 'victory' GROUP ALL")
                .bind(("mid", member_id.to_string()))
                .await?;
            let wins: Vec<CountResult> = wins_result.take(0)?;

            let mut losses_result = self
                .client
                .query("SELECT count() FROM personal_match WHERE member_id = $mid AND outcome = 'defeat' GROUP ALL")
                .bind(("mid", member_id.to_string()))
                .await?;
            let losses: Vec<CountResult> = losses_result.take(0)?;

            let mut draws_result = self
                .client
                .query("SELECT count() FROM personal_match WHERE member_id = $mid AND outcome = 'draw' GROUP ALL")
                .bind(("mid", member_id.to_string()))
                .await?;
            let draws: Vec<CountResult> = draws_result.take(0)?;

            Ok(PersonalStats {
                member_id: member_id.to_string(),
                total_matches: total.first().map(|c| c.count).unwrap_or(0),
                wins: wins.first().map(|c| c.count).unwrap_or(0),
                losses: losses.first().map(|c| c.count).unwrap_or(0),
                draws: draws.first().map(|c| c.count).unwrap_or(0),
            })
        })
        .await
    }

    pub async fn get_hero_stats(&self, member_id: &str) -> DbResult<Vec<HeroStats>> {
        with_timeout(async {
            #[derive(Deserialize, SurrealValue)]
            struct HeroRow {
                hero: String,
                matches: u32,
                wins: u32,
                avg_elims: f64,
                avg_deaths: f64,
                avg_damage: f64,
                avg_healing: f64,
            }

            let mut result = self
                .client
                .query(r#"
                    SELECT
                        hero,
                        count() AS matches,
                        math::sum(IF outcome = 'victory' THEN 1 ELSE 0 END) AS wins,
                        math::mean(elims) AS avg_elims,
                        math::mean(deaths) AS avg_deaths,
                        math::mean(damage) AS avg_damage,
                        math::mean(healing) AS avg_healing
                    FROM personal_match
                    WHERE member_id = $mid
                    GROUP BY hero
                    ORDER BY matches DESC
                "#)
                .bind(("mid", member_id.to_string()))
                .await?;
            let rows: Vec<HeroRow> = result.take(0)?;

            let mut hero_stats = Vec::with_capacity(rows.len());
            for row in rows {
                let mut losses_result = self
                    .client
                    .query("SELECT count() FROM personal_match WHERE member_id = $mid AND hero = $hero AND outcome = 'defeat' GROUP ALL")
                    .bind(("mid", member_id.to_string()))
                    .bind(("hero", row.hero.clone()))
                    .await?;

                #[derive(Deserialize, SurrealValue)]
                struct Cnt { count: u32 }

                let losses_vec: Vec<Cnt> = losses_result.take(0)?;
                let losses = losses_vec.first().map(|c| c.count).unwrap_or(0);
                let draws = row.matches.saturating_sub(row.wins).saturating_sub(losses);

                hero_stats.push(HeroStats {
                    hero: row.hero,
                    matches: row.matches,
                    wins: row.wins,
                    losses,
                    draws,
                    avg_elims: row.avg_elims,
                    avg_deaths: row.avg_deaths,
                    avg_damage: row.avg_damage,
                    avg_healing: row.avg_healing,
                });
            }

            Ok(hero_stats)
        })
        .await
    }

    pub async fn get_map_stats(&self, member_id: &str) -> DbResult<Vec<MapStats>> {
        with_timeout(async {
            #[derive(Deserialize, SurrealValue)]
            struct MapRow {
                map_name: String,
                matches: u32,
                wins: u32,
            }

            let mut result = self
                .client
                .query(r#"
                    SELECT
                        map_name,
                        count() AS matches,
                        math::sum(IF outcome = 'victory' THEN 1 ELSE 0 END) AS wins
                    FROM personal_match
                    WHERE member_id = $mid
                    GROUP BY map_name
                    ORDER BY matches DESC
                "#)
                .bind(("mid", member_id.to_string()))
                .await?;
            let rows: Vec<MapRow> = result.take(0)?;

            let mut map_stats = Vec::with_capacity(rows.len());
            for row in rows {
                let mut losses_result = self
                    .client
                    .query("SELECT count() FROM personal_match WHERE member_id = $mid AND map_name = $map AND outcome = 'defeat' GROUP ALL")
                    .bind(("mid", member_id.to_string()))
                    .bind(("map", row.map_name.clone()))
                    .await?;

                #[derive(Deserialize, SurrealValue)]
                struct Cnt { count: u32 }

                let losses_vec: Vec<Cnt> = losses_result.take(0)?;
                let losses = losses_vec.first().map(|c| c.count).unwrap_or(0);
                let draws = row.matches.saturating_sub(row.wins).saturating_sub(losses);

                map_stats.push(MapStats {
                    map_name: row.map_name,
                    matches: row.matches,
                    wins: row.wins,
                    losses,
                    draws,
                });
            }

            Ok(map_stats)
        })
        .await
    }
}
