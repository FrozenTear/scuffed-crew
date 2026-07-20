use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use surrealdb::types::Datetime as SurrealDatetime;
use surrealdb_types::{RecordId, SurrealValue};

use crate::types::{
    HeroStats, MapStats, MemberHeroScopedAgg, MemberLeaderboardRow, PersonalMatch, PersonalStats,
};
use crate::{with_timeout, Database, DbResult};

/// Minimum games required for rate metrics (winrate / kd) on public boards.
const LEADERBOARD_MIN_GAMES: u32 = 5;

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
    #[surreal(default)]
    edited: bool,
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
        edited: db.edited,
    }
}

/// Stable session id for uploads from pre-session daemons. Derived from the
/// row's content (FNV-1a — stable across releases, unlike `DefaultHasher`),
/// so a retried legacy upload updates the same row instead of duplicating —
/// the same dedup the old `(member, hero, map, played_at)` unique index
/// provided, without its wedge-the-queue failure mode.
fn legacy_session_id(member_id: &str, m: &crate::types::PersonalMatch) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for part in [member_id, &m.hero, &m.map_name, &m.played_at.to_rfc3339()] {
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
                               edited = $edited,
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
                    .bind(("edited", m.edited))
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

    /// Remove a member's rows for locally-deleted sessions (client tombstones).
    /// Scoped to the member — a daemon token can only delete its own games.
    /// Returns the number of rows removed.
    pub async fn delete_personal_matches_by_sessions(
        &self,
        member_id: &str,
        session_ids: &[String],
    ) -> DbResult<u32> {
        if session_ids.is_empty() {
            return Ok(0);
        }
        let mut result = with_timeout(async {
            Ok(self
                .client
                .query(
                    "DELETE personal_match WHERE member_id = $mid AND session_id IN $sids RETURN BEFORE",
                )
                .bind(("mid", member_id.to_string()))
                .bind(("sids", session_ids.to_vec()))
                .await?
                .check()?)
        })
        .await?;
        let deleted: Vec<DbPersonalMatch> = result.take(0)?;
        Ok(deleted.len() as u32)
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

    /// Top heroes for a member, ranked by games then winrate.
    ///
    /// `limit == 0` returns **all** heroes (no truncation). Non-zero limits
    /// take the first `limit` rows after ranking (hero-stats W2 B4).
    pub async fn top_heroes(&self, member_id: &str, limit: u32) -> DbResult<Vec<HeroStats>> {
        let mut all = self.get_hero_stats(member_id).await?;
        if limit > 0 {
            all.truncate(limit as usize);
        }
        Ok(all)
    }

    /// Public member leaderboard from personal_match aggregates.
    ///
    /// - `metric`: `"winrate"` | `"kd"` | `"games"`
    /// - optional `season_window` `(starts_at, ends_at)` filters via
    ///   `played_at >= starts AND played_at < ends` (bound params only)
    /// - optional `hero`: when `Some`, adds `AND hero = $hero` (bound param;
    ///   canonical `scuffed_types::HEROES` string — validate at the HTTP layer)
    /// - inactive / missing members are omitted (banned stay off public boards)
    /// - winrate/kd require ≥ [`LEADERBOARD_MIN_GAMES`] matches (per-hero games
    ///   when `hero` is set)
    pub async fn member_leaderboard(
        &self,
        metric: &str,
        limit: u32,
        season_window: Option<(DateTime<Utc>, DateTime<Utc>)>,
        hero: Option<&str>,
    ) -> DbResult<Vec<MemberLeaderboardRow>> {
        with_timeout(async {
            #[derive(Deserialize, SurrealValue)]
            struct AggRow {
                member_id: String,
                games: u32,
                wins: u32,
                elims: u32,
                deaths: u32,
            }

            let season_filter = if season_window.is_some() {
                " AND played_at >= $season_start AND played_at < $season_end"
            } else {
                ""
            };
            let hero_filter = if hero.is_some() {
                " AND hero = $hero"
            } else {
                ""
            };
            let sql = format!(
                r#"
                    SELECT
                        member_id,
                        count() AS games,
                        math::sum(IF outcome = 'victory' THEN 1 ELSE 0 END) AS wins,
                        math::sum(elims) AS elims,
                        math::sum(deaths) AS deaths
                    FROM personal_match
                    WHERE outcome IN ['victory', 'defeat', 'draw']{season_filter}{hero_filter}
                    GROUP BY member_id
                    "#
            );

            let mut q = self.client.query(sql);
            if let Some((start, end)) = season_window {
                q = q
                    .bind(("season_start", SurrealDatetime::from(start)))
                    .bind(("season_end", SurrealDatetime::from(end)));
            }
            if let Some(hero_name) = hero {
                q = q.bind(("hero", hero_name.to_owned()));
            }
            let mut result = q.await?;
            let rows: Vec<AggRow> = result.take(0)?;

            // One active-member map instead of get_member_safe N+1 (HS-DR P1).
            #[derive(Deserialize, SurrealValue)]
            struct ActiveName {
                id: Option<RecordId>,
                display_name: String,
            }
            let mut active_q = self
                .client
                .query("SELECT id, display_name FROM member WHERE is_active = true")
                .await?;
            let active_rows: Vec<ActiveName> = active_q.take(0)?;
            let active: std::collections::HashMap<String, String> = active_rows
                .into_iter()
                .filter_map(|r| {
                    let id = r.id.map(|rid| crate::record_id_key_to_string(rid.key))?;
                    Some((id, r.display_name))
                })
                .collect();

            let rate_metric = matches!(metric, "winrate" | "kd");
            let mut out = Vec::with_capacity(rows.len());
            for row in rows {
                if row.games == 0 {
                    continue;
                }
                if rate_metric && row.games < LEADERBOARD_MIN_GAMES {
                    continue;
                }
                // Normalize member_id in case the aggregate returns table-prefixed keys.
                let mid = row
                    .member_id
                    .strip_prefix("member:")
                    .unwrap_or(&row.member_id)
                    .to_string();
                let Some(display_name) = active.get(&mid).cloned() else {
                    continue;
                };
                let winrate = row.wins as f32 / row.games as f32;
                let kd = row.elims as f64 / (row.deaths.max(1) as f64);
                out.push(MemberLeaderboardRow {
                    member_id: mid,
                    display_name,
                    games: row.games,
                    winrate,
                    kd,
                });
            }

            match metric {
                "kd" => out.sort_by(|a, b| {
                    b.kd.partial_cmp(&a.kd)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then_with(|| b.games.cmp(&a.games))
                }),
                "games" => out.sort_by(|a, b| {
                    b.games.cmp(&a.games).then_with(|| {
                        b.winrate
                            .partial_cmp(&a.winrate)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                }),
                _ => out.sort_by(|a, b| {
                    b.winrate
                        .partial_cmp(&a.winrate)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then_with(|| b.games.cmp(&a.games))
                }),
            }
            out.truncate(limit as usize);
            Ok(out)
        })
        .await
    }

    /// Page-scoped per-hero aggregates for a known set of member ids (HS-DR P1).
    ///
    /// Single `GROUP BY` over `personal_match` filtered to `member_id IN $ids`
    /// and `hero = $hero` — no full-table leaderboard and no per-row member fetch.
    pub async fn hero_scoped_for_members(
        &self,
        member_ids: &[String],
        hero: &str,
    ) -> DbResult<Vec<MemberHeroScopedAgg>> {
        if member_ids.is_empty() {
            return Ok(Vec::new());
        }
        with_timeout(async {
            #[derive(Deserialize, SurrealValue)]
            struct AggRow {
                member_id: String,
                games: u32,
                wins: u32,
            }

            let mut result = self
                .client
                .query(
                    r#"
                    SELECT
                        member_id,
                        count() AS games,
                        math::sum(IF outcome = 'victory' THEN 1 ELSE 0 END) AS wins
                    FROM personal_match
                    WHERE member_id IN $ids
                      AND hero = $hero
                      AND outcome IN ['victory', 'defeat', 'draw']
                    GROUP BY member_id
                    "#,
                )
                .bind(("ids", member_ids.to_vec()))
                .bind(("hero", hero.to_owned()))
                .await?;
            let rows: Vec<AggRow> = result.take(0)?;
            Ok(rows
                .into_iter()
                .filter(|r| r.games > 0)
                .map(|r| {
                    let mid = r
                        .member_id
                        .strip_prefix("member:")
                        .unwrap_or(&r.member_id)
                        .to_string();
                    MemberHeroScopedAgg {
                        member_id: mid,
                        games: r.games,
                        winrate: r.wins as f32 / r.games as f32,
                    }
                })
                .collect())
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
            edited: false,
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
    async fn edited_flag_persists_and_corrected_values_count() {
        let db = test_db().await;
        // An OCR'd row, then a manual correction: the effective (corrected)
        // values are uploaded with edited=true and must survive the round-trip
        // and feed aggregates (leaderboard policy v1).
        let mut m = entry("s1", "victory", 12);
        m.edited = true;
        m.elims = 30; // effective value the daemon uploaded
        db.upsert_personal_matches("m1", &[m]).await.unwrap();

        let rows = db.list_personal_matches("m1", 10, 0).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].edited, "edited flag must persist for the badge");
        assert_eq!(rows[0].elims, 30, "corrected value stored");

        // A plain upload defaults edited=false.
        db.upsert_personal_matches("m1", &[entry("s2", "defeat", 4)])
            .await
            .unwrap();
        let s2 = db.list_personal_matches("m1", 10, 0).await.unwrap();
        let s2 = s2.iter().find(|r| r.session_id == "s2").unwrap();
        assert!(!s2.edited);
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
    async fn tombstones_delete_only_that_members_session() {
        let db = test_db().await;
        db.upsert_personal_matches(
            "m1",
            &[entry("s1", "victory", 10), entry("s2", "defeat", 5)],
        )
        .await
        .unwrap();
        db.upsert_personal_matches("m2", &[entry("s1", "victory", 9)])
            .await
            .unwrap();

        let deleted = db
            .delete_personal_matches_by_sessions("m1", &["s1".to_string()])
            .await
            .unwrap();
        assert_eq!(deleted, 1);
        let m1_rows = db.list_personal_matches("m1", 10, 0).await.unwrap();
        assert_eq!(m1_rows.len(), 1);
        assert_eq!(m1_rows[0].session_id, "s2");
        // Another member's identically-named session is untouched.
        assert_eq!(
            db.list_personal_matches("m2", 10, 0).await.unwrap().len(),
            1
        );
        // Empty tombstone list is a no-op.
        assert_eq!(
            db.delete_personal_matches_by_sessions("m1", &[])
                .await
                .unwrap(),
            0
        );
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

        assert_eq!(
            db.list_personal_matches("m1", 10, 0).await.unwrap().len(),
            1
        );
        assert_eq!(
            db.list_personal_matches("m2", 10, 0).await.unwrap().len(),
            1
        );
        assert_eq!(
            db.list_personal_matches("m1", 10, 0).await.unwrap()[0].outcome,
            "victory",
            "another member's upload must not touch m1's row"
        );
    }

    /// W2 B1 owed: `member_leaderboard(..., hero: Some)` must narrow rows to
    /// matches on that hero only (bound `AND hero = $hero`). Uses metric
    /// `"games"` so LEADERBOARD_MIN_GAMES does not mask the filter.
    #[tokio::test]
    async fn member_leaderboard_hero_filter_narrows_rows() {
        use crate::types::OrgRole;

        let db = test_db().await;
        let ana_main = db
            .create_member("u-ana", "AnaMain", OrgRole::Member)
            .await
            .unwrap();
        let tracer_only = db
            .create_member("u-tr", "TracerOnly", OrgRole::Member)
            .await
            .unwrap();

        // ana_main: 2 Ana + 1 Tracer. tracer_only: 3 Tracer (no Ana).
        let mut ana_g1 = entry("a1", "victory", 5);
        ana_g1.member_id = ana_main.id.clone();
        ana_g1.hero = "Ana".into();
        let mut ana_g2 = entry("a2", "defeat", 3);
        ana_g2.member_id = ana_main.id.clone();
        ana_g2.hero = "Ana".into();
        ana_g2.played_at = Utc.with_ymd_and_hms(2026, 7, 2, 20, 0, 0).unwrap();
        let mut ana_tr = entry("a3", "victory", 8);
        ana_tr.member_id = ana_main.id.clone();
        ana_tr.hero = "Tracer".into();
        ana_tr.played_at = Utc.with_ymd_and_hms(2026, 7, 3, 20, 0, 0).unwrap();
        db.upsert_personal_matches(&ana_main.id, &[ana_g1, ana_g2, ana_tr])
            .await
            .unwrap();

        for (i, sid) in ["t1", "t2", "t3"].iter().enumerate() {
            let mut m = entry(sid, "victory", 4 + i as u32);
            m.member_id = tracer_only.id.clone();
            m.hero = "Tracer".into();
            m.played_at = Utc
                .with_ymd_and_hms(2026, 7, 1 + i as u32, 21, 0, 0)
                .unwrap();
            db.upsert_personal_matches(&tracer_only.id, &[m])
                .await
                .unwrap();
        }

        let all = db
            .member_leaderboard("games", 50, None, None)
            .await
            .unwrap();
        assert_eq!(all.len(), 2, "unfiltered LB includes both members");
        let all_ids: Vec<_> = all.iter().map(|r| r.member_id.as_str()).collect();
        assert!(all_ids.contains(&ana_main.id.as_str()));
        assert!(all_ids.contains(&tracer_only.id.as_str()));
        let ana_all = all.iter().find(|r| r.member_id == ana_main.id).unwrap();
        assert_eq!(ana_all.games, 3, "unfiltered counts all heroes");

        let ana_lb = db
            .member_leaderboard("games", 50, None, Some("Ana"))
            .await
            .unwrap();
        assert_eq!(ana_lb.len(), 1, "hero=Ana must drop Tracer-only members");
        assert_eq!(ana_lb[0].member_id, ana_main.id);
        assert_eq!(
            ana_lb[0].games, 2,
            "hero filter must count only Ana matches"
        );

        let tr_lb = db
            .member_leaderboard("games", 50, None, Some("Tracer"))
            .await
            .unwrap();
        assert_eq!(tr_lb.len(), 2, "both members have Tracer games");
        let tr_ana = tr_lb.iter().find(|r| r.member_id == ana_main.id).unwrap();
        let tr_only = tr_lb
            .iter()
            .find(|r| r.member_id == tracer_only.id)
            .unwrap();
        assert_eq!(tr_ana.games, 1);
        assert_eq!(tr_only.games, 3);
    }
}
