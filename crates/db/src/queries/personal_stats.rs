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
    #[surreal(default)]
    session_id: String,
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
        session_id: db.session_id,
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

/// Stable session id for uploads from pre-session daemons. Derived from the
/// row's content (FNV-1a — stable across releases, unlike `DefaultHasher`),
/// so a retried legacy upload updates the same row instead of duplicating —
/// the same dedup the old `(member, hero, map, played_at)` unique index
/// provided, without its wedge-the-queue failure mode.
fn legacy_session_id(member_id: &str, m: &crate::types::PersonalMatch) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for part in [
        member_id,
        &m.hero,
        &m.map_name,
        &m.played_at.to_rfc3339(),
    ] {
        for b in part.as_bytes() {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        h ^= u64::from(b'\x1f');
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("legacy-{h:016x}")
}

impl Database {
    /// Upsert uploaded matches, one server row per (member, session).
    ///
    /// Capture snapshots of one game share a session id, so re-uploads and
    /// outcome/map corrections update the existing row in place — a game can
    /// never count more than once no matter how many Tab snapshots it produced
    /// or how often the client retries. Entries without a session id get a
    /// stable content-derived legacy id (see [`legacy_session_id`]).
    ///
    /// Returns the number of rows upserted.
    pub async fn upsert_personal_matches(
        &self,
        member_id: &str,
        matches: &[crate::types::PersonalMatch],
    ) -> DbResult<u32> {
        let mut upserted = 0u32;
        for m in matches {
            let session_id = if m.session_id.is_empty() {
                legacy_session_id(member_id, m)
            } else {
                m.session_id.clone()
            };
            with_timeout(async {
                self.client
                    .query(
                        r#"UPSERT personal_match SET
                               member_id = $mid, session_id = $sid,
                               hero = $hero, map_name = $map, game_mode = $mode,
                               role = $role, outcome = $outcome,
                               elims = $elims, deaths = $deaths, assists = $assists,
                               damage = $damage, healing = $healing, mitigation = $mit,
                               played_at = $played, uploaded_at = time::now()
                           WHERE member_id = $mid AND session_id = $sid"#,
                    )
                    .bind(("mid", member_id.to_string()))
                    .bind(("sid", session_id))
                    .bind(("hero", m.hero.clone()))
                    .bind(("map", m.map_name.clone()))
                    .bind(("mode", m.game_mode.clone()))
                    .bind(("role", m.role.clone()))
                    .bind(("outcome", m.outcome.clone()))
                    .bind(("elims", m.elims))
                    .bind(("deaths", m.deaths))
                    .bind(("assists", m.assists))
                    .bind(("damage", m.damage))
                    .bind(("healing", m.healing))
                    .bind(("mit", m.mitigation))
                    .bind(("played", SurrealDatetime::from(m.played_at)))
                    .await?
                    .check()?;
                Ok(())
            })
            .await?;
            upserted += 1;
        }
        Ok(upserted)
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

#[cfg(test)]
mod tests {
    use crate::migrations::run_migrations;
    use crate::Database;
    use chrono::{TimeZone, Utc};

    async fn test_db() -> Database {
        let db = Database::connect_memory().await.unwrap();
        run_migrations(&db.client).await.unwrap();
        db
    }

    fn entry(session_id: &str, outcome: &str, elims: u32) -> crate::types::PersonalMatch {
        crate::types::PersonalMatch {
            id: String::new(),
            member_id: "m1".into(),
            session_id: session_id.into(),
            hero: "Ana".into(),
            map_name: "Oasis".into(),
            game_mode: "control".into(),
            role: "Support".into(),
            outcome: outcome.into(),
            elims,
            deaths: 2,
            assists: 8,
            damage: 3000,
            healing: 9000,
            mitigation: 0,
            played_at: Utc.with_ymd_and_hms(2026, 7, 1, 20, 0, 0).unwrap(),
            uploaded_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn snapshots_of_one_session_collapse_to_one_row() {
        let db = test_db().await;
        // Three capture snapshots of the same game, uploaded over two batches
        // (simulates mid-game sync + retry after the game).
        db.upsert_personal_matches("m1", &[entry("s1", "victory", 10)])
            .await
            .unwrap();
        db.upsert_personal_matches(
            "m1",
            &[entry("s1", "victory", 15), entry("s1", "victory", 22)],
        )
        .await
        .unwrap();

        let rows = db.list_personal_matches("m1", 10, 0).await.unwrap();
        assert_eq!(rows.len(), 1, "one game must be one server row");
        assert_eq!(rows[0].elims, 22, "last snapshot wins");
        let stats = db.get_personal_stats("m1").await.unwrap();
        assert_eq!((stats.total_matches, stats.wins), (1, 1));
    }

    #[tokio::test]
    async fn corrections_update_in_place() {
        let db = test_db().await;
        db.upsert_personal_matches("m1", &[entry("s1", "victory", 10)])
            .await
            .unwrap();
        // Outcome + map corrected after the fact (manual edit / accolade read).
        let mut fixed = entry("s1", "defeat", 10);
        fixed.map_name = "Circuit Royal".into();
        db.upsert_personal_matches("m1", &[fixed]).await.unwrap();

        let rows = db.list_personal_matches("m1", 10, 0).await.unwrap();
        assert_eq!(rows.len(), 1, "correction must not create a second row");
        assert_eq!(rows[0].outcome, "defeat");
        assert_eq!(rows[0].map_name, "Circuit Royal");
        let stats = db.get_personal_stats("m1").await.unwrap();
        assert_eq!((stats.wins, stats.losses), (0, 1));
    }

    #[tokio::test]
    async fn distinct_sessions_stay_distinct() {
        let db = test_db().await;
        db.upsert_personal_matches(
            "m1",
            &[entry("s1", "victory", 10), entry("s2", "defeat", 5)],
        )
        .await
        .unwrap();
        let rows = db.list_personal_matches("m1", 10, 0).await.unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[tokio::test]
    async fn legacy_entries_dedup_by_content() {
        let db = test_db().await;
        // Pre-session daemon retries the same row → still one row.
        db.upsert_personal_matches("m1", &[entry("", "victory", 10)])
            .await
            .unwrap();
        db.upsert_personal_matches("m1", &[entry("", "victory", 10)])
            .await
            .unwrap();
        // A different game (different played_at) is a new row.
        let mut other = entry("", "defeat", 3);
        other.played_at = Utc.with_ymd_and_hms(2026, 7, 2, 21, 0, 0).unwrap();
        db.upsert_personal_matches("m1", &[other]).await.unwrap();

        let rows = db.list_personal_matches("m1", 10, 0).await.unwrap();
        assert_eq!(rows.len(), 2, "legacy dedup by content, not by batch");
    }

    #[tokio::test]
    async fn sessions_are_scoped_per_member() {
        let db = test_db().await;
        db.upsert_personal_matches("m1", &[entry("s1", "victory", 10)])
            .await
            .unwrap();
        let mut m2 = entry("s1", "defeat", 4);
        m2.member_id = "m2".into();
        db.upsert_personal_matches("m2", &[m2]).await.unwrap();

        assert_eq!(db.list_personal_matches("m1", 10, 0).await.unwrap().len(), 1);
        assert_eq!(db.list_personal_matches("m2", 10, 0).await.unwrap().len(), 1);
        assert_eq!(
            db.list_personal_matches("m1", 10, 0).await.unwrap()[0].outcome,
            "victory",
            "another member's upload must not touch m1's row"
        );
    }
}
