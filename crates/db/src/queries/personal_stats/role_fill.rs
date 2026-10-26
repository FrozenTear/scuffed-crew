//! Per-role aggregates with the read-time role fill.
//!
//! `personal_match.role` is grouped together with the hero, then each
//! (stored role, hero) group is folded into the role that
//! [`scuffed_types::filled_role_label`] gives it. A recognized stored role is
//! kept, an empty or unrecognized one takes the hero's current role, and an
//! unknown hero stays in the `""` row. This only reads: stored roles are
//! never rewritten.

use std::collections::BTreeMap;

use serde::Deserialize;
use surrealdb::types::Datetime as SurrealDatetime;
use surrealdb_types::SurrealValue;

use super::{
    bind_stat_caps, season_filter, SeasonWindow, DEATHS_PLAUSIBLE_SQL, ELIMS_PLAUSIBLE_SQL,
};
use crate::types::RoleStats;
use crate::{with_timeout, Database, DbResult};

const DAMAGE_INCLUDED_SQL: &str = "suspect_fields IS NONE OR NOT (suspect_fields CONTAINS 'dmg')";
const HEALING_INCLUDED_SQL: &str = "suspect_fields IS NONE OR NOT (suspect_fields CONTAINS 'h')";

/// One (stored role, hero) group. Sums and counts only cover the values that
/// the plain role averages include, so folding groups gives the same means.
#[derive(Debug, Clone, Default, Deserialize, SurrealValue)]
pub(crate) struct RoleHeroRow {
    pub role: String,
    pub hero: String,
    pub matches: u32,
    pub wins: u32,
    pub losses: u32,
    pub draws: u32,
    pub elims_sum: f64,
    pub elims_n: u32,
    pub deaths_sum: f64,
    pub deaths_n: u32,
    pub damage_sum: f64,
    pub damage_n: u32,
    pub healing_sum: f64,
    pub healing_n: u32,
}

fn sum_and_count(column: &str, predicate: &str) -> String {
    format!(
        "<float>math::sum(IF {predicate} THEN {column} ELSE 0 END) AS {column}_sum, \
         math::sum(IF {predicate} THEN 1 ELSE 0 END) AS {column}_n"
    )
}

fn role_hero_sql(season: Option<SeasonWindow>) -> String {
    let sums = [
        sum_and_count("elims", ELIMS_PLAUSIBLE_SQL),
        sum_and_count("deaths", DEATHS_PLAUSIBLE_SQL),
        sum_and_count("damage", DAMAGE_INCLUDED_SQL),
        sum_and_count("healing", HEALING_INCLUDED_SQL),
    ]
    .join(", ");
    format!(
        r#"
        SELECT
            role,
            hero,
            count() AS matches,
            math::sum(IF outcome = 'victory' THEN 1 ELSE 0 END) AS wins,
            math::sum(IF outcome = 'defeat' THEN 1 ELSE 0 END) AS losses,
            math::sum(IF outcome = 'draw' THEN 1 ELSE 0 END) AS draws,
            {sums}
        FROM personal_match
        WHERE member_id = $mid{}
        GROUP BY role, hero
    "#,
        season_filter(season)
    )
}

fn mean(sum: f64, n: u32) -> f64 {
    if n == 0 {
        0.0
    } else {
        sum / f64::from(n)
    }
}

/// Fold (stored role, hero) groups into filled roles. Order is matches
/// descending, then role ascending, the same contract as the plain query.
pub(crate) fn fold_filled_roles(rows: Vec<RoleHeroRow>) -> Vec<RoleStats> {
    let mut by_role: BTreeMap<String, RoleHeroRow> = BTreeMap::new();
    for row in rows {
        let role = scuffed_types::filled_role_label(&row.role, &row.hero);
        let acc = by_role.entry(role).or_default();
        acc.matches += row.matches;
        acc.wins += row.wins;
        acc.losses += row.losses;
        acc.draws += row.draws;
        acc.elims_sum += row.elims_sum;
        acc.elims_n += row.elims_n;
        acc.deaths_sum += row.deaths_sum;
        acc.deaths_n += row.deaths_n;
        acc.damage_sum += row.damage_sum;
        acc.damage_n += row.damage_n;
        acc.healing_sum += row.healing_sum;
        acc.healing_n += row.healing_n;
    }
    let mut out: Vec<RoleStats> = by_role
        .into_iter()
        .map(|(role, acc)| RoleStats {
            role,
            matches: acc.matches,
            wins: acc.wins,
            losses: acc.losses,
            draws: acc.draws,
            avg_elims: mean(acc.elims_sum, acc.elims_n),
            avg_deaths: mean(acc.deaths_sum, acc.deaths_n),
            avg_damage: mean(acc.damage_sum, acc.damage_n),
            avg_healing: mean(acc.healing_sum, acc.healing_n),
        })
        .collect();
    out.sort_by(|a, b| b.matches.cmp(&a.matches).then_with(|| a.role.cmp(&b.role)));
    out
}

impl Database {
    /// All-time [`Database::get_filled_role_stats_in`].
    pub async fn get_filled_role_stats(&self, member_id: &str) -> DbResult<Vec<RoleStats>> {
        self.get_filled_role_stats_in(member_id, None).await
    }

    /// Per-role aggregates for the Overview role cards and the member page,
    /// with an empty or unrecognized stored role filled from the hero at read
    /// time. Same counters, averages, season window, and order as
    /// [`Database::get_role_stats_in`]. Never writes.
    pub async fn get_filled_role_stats_in(
        &self,
        member_id: &str,
        season: Option<SeasonWindow>,
    ) -> DbResult<Vec<RoleStats>> {
        with_timeout(async {
            let mut q = self
                .client
                .query(role_hero_sql(season))
                .bind(("mid", member_id.to_string()));
            q = bind_stat_caps(q);
            if let Some((start, end)) = season {
                q = q
                    .bind(("season_start", SurrealDatetime::from(start)))
                    .bind(("season_end", SurrealDatetime::from(end)));
            }
            let mut result = q.await?;
            let rows: Vec<RoleHeroRow> = result.take(0)?;
            Ok(fold_filled_roles(rows))
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations::run_migrations;
    use crate::types::PersonalMatch;
    use chrono::{TimeZone, Utc};

    async fn test_db() -> Database {
        let db = Database::connect_memory().await.unwrap();
        run_migrations(&db.client).await.unwrap();
        db
    }

    fn close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1e-6, "{actual} != {expected}");
    }

    #[allow(clippy::too_many_arguments)]
    fn game(
        sid: &str,
        hero: &str,
        role: &str,
        outcome: &str,
        elims: u32,
        deaths: u32,
        damage: u32,
        healing: u32,
    ) -> PersonalMatch {
        PersonalMatch {
            id: String::new(),
            member_id: "rf".into(),
            session_id: sid.into(),
            hero: hero.into(),
            map_name: "Oasis".into(),
            game_mode: "control".into(),
            role: role.into(),
            outcome: outcome.into(),
            elims,
            deaths,
            assists: 0,
            damage,
            healing,
            mitigation: 0,
            played_at: Utc.with_ymd_and_hms(2026, 3, 1, 20, 0, 0).unwrap(),
            uploaded_at: Utc::now(),
            edited: false,
            recognizer: scuffed_types::RECOGNIZER_OCR_V1.into(),
            name_recognizer: None,
            suspect_fields: Vec::new(),
        }
    }

    fn row(role: &str, hero: &str, matches: u32, elims_sum: f64) -> RoleHeroRow {
        RoleHeroRow {
            role: role.into(),
            hero: hero.into(),
            matches,
            wins: matches,
            elims_sum,
            elims_n: matches,
            ..RoleHeroRow::default()
        }
    }

    fn role_of<'a>(rows: &'a [RoleStats], role: &str) -> &'a RoleStats {
        rows.iter()
            .find(|r| r.role == role)
            .unwrap_or_else(|| panic!("no {role:?} row in {rows:?}"))
    }

    #[test]
    fn fold_keeps_recognized_stored_role() {
        let out = fold_filled_roles(vec![row("Damage", "Ana", 2, 10.0)]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].role, "Damage");
        assert_eq!(out[0].matches, 2);
        close(out[0].avg_elims, 5.0);
    }

    #[test]
    fn fold_fills_empty_and_garbage_roles_from_hero() {
        let out = fold_filled_roles(vec![
            row("", "Reaper", 1, 4.0),
            row("dps", "Reaper", 1, 8.0),
            row("Damage", "Cassidy", 2, 0.0),
            row("Flex", "Reinhardt", 1, 3.0),
        ]);
        assert_eq!(
            out.iter().map(|r| r.role.as_str()).collect::<Vec<_>>(),
            vec!["Damage", "Tank"]
        );
        let damage = role_of(&out, "Damage");
        assert_eq!(damage.matches, 4);
        close(damage.avg_elims, 3.0);
        assert_eq!(role_of(&out, "Tank").matches, 1);
    }

    #[test]
    fn fold_unknown_hero_stays_unknown() {
        let out = fold_filled_roles(vec![
            row("", "Unknown", 2, 0.0),
            row("garbage", "NotAHero", 1, 0.0),
            row("Support", "Unknown", 1, 0.0),
        ]);
        assert_eq!(role_of(&out, "").matches, 3);
        assert_eq!(role_of(&out, "Support").matches, 1);
    }

    #[test]
    fn fold_normalizes_recognized_case_into_one_row() {
        let out = fold_filled_roles(vec![
            row("support", "Ana", 1, 0.0),
            row(" Support ", "Ana", 1, 0.0),
            row("Support", "Ana", 1, 0.0),
        ]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].role, "Support");
        assert_eq!(out[0].matches, 3);
    }

    /// Reads fill, rows stay as stored. Sombra stored as Support stays
    /// Support and stored as Damage stays Damage.
    #[tokio::test]
    async fn filled_role_stats_read_without_writing() {
        let db = test_db().await;
        db.upsert_personal_matches(
            "rf",
            &[
                game("sd", "Sombra", "Damage", "victory", 10, 4, 8000, 0),
                game("ss", "Sombra", "Support", "victory", 4, 2, 2000, 9000),
                game("e1", "Reaper", "", "defeat", 6, 6, 600, 0),
                game("g1", "Reinhardt", "dps", "draw", 3, 1, 1000, 0),
                game("u1", "Unknown", "", "defeat", 1, 1, 100, 0),
            ],
        )
        .await
        .unwrap();

        let roles = db.get_filled_role_stats("rf").await.unwrap();
        assert_eq!(
            roles.iter().map(|r| r.role.as_str()).collect::<Vec<_>>(),
            vec!["Damage", "", "Support", "Tank"],
            "matches desc, then role asc"
        );
        let damage = role_of(&roles, "Damage");
        assert_eq!(
            (damage.matches, damage.wins, damage.losses, damage.draws),
            (2, 1, 1, 0),
            "Sombra Damage plus the filled Reaper game"
        );
        close(damage.avg_elims, 8.0);
        close(damage.avg_damage, 4300.0);
        let support = role_of(&roles, "Support");
        assert_eq!(support.matches, 1, "Sombra stored Support stays Support");
        close(support.avg_healing, 9000.0);
        assert_eq!(role_of(&roles, "Tank").matches, 1, "garbage role filled");
        assert_eq!(role_of(&roles, "").matches, 1, "Unknown hero stays unknown");

        // The plain stored-role query still sees the stored strings.
        let stored = db.get_role_stats("rf").await.unwrap();
        let mut stored_roles: Vec<&str> = stored.iter().map(|r| r.role.as_str()).collect();
        stored_roles.sort_unstable();
        assert_eq!(stored_roles, vec!["", "Damage", "Support", "dps"]);
        assert_eq!(role_of(&stored, "").matches, 2);

        let rows = db.list_personal_matches("rf", 10, 0).await.unwrap();
        let by_sid = |sid: &str| rows.iter().find(|m| m.session_id == sid).unwrap();
        assert_eq!(by_sid("e1").role, "");
        assert_eq!(by_sid("g1").role, "dps");
        assert_eq!(by_sid("ss").role, "Support");
    }

    /// Averages over filled roles match the plain query when every stored
    /// role is already recognized, including the season window and caps.
    #[tokio::test]
    async fn filled_role_stats_match_plain_query_for_recognized_roles() {
        let db = test_db().await;
        let mut capped = game("cap", "Ana", "Support", "victory", 5000, 2, 100, 100);
        capped.suspect_fields = vec!["h".into()];
        db.upsert_personal_matches(
            "rf",
            &[
                game("a1", "Ana", "Support", "victory", 10, 2, 1000, 8000),
                game("a2", "Kiriko", "Support", "defeat", 6, 4, 3000, 6000),
                game("t1", "Winston", "Tank", "draw", 9, 5, 7000, 0),
                capped,
            ],
        )
        .await
        .unwrap();

        let window = Some((
            Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap(),
        ));
        for season in [None, window] {
            let filled = db.get_filled_role_stats_in("rf", season).await.unwrap();
            let plain = db.get_role_stats_in("rf", season).await.unwrap();
            assert_eq!(filled.len(), plain.len());
            for (f, p) in filled.iter().zip(plain.iter()) {
                assert_eq!(f.role, p.role);
                assert_eq!(
                    (f.matches, f.wins, f.losses, f.draws),
                    (p.matches, p.wins, p.losses, p.draws)
                );
                close(f.avg_elims, p.avg_elims);
                close(f.avg_deaths, p.avg_deaths);
                close(f.avg_damage, p.avg_damage);
                close(f.avg_healing, p.avg_healing);
            }
        }
        let empty_window = Some((
            Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2025, 6, 1, 0, 0, 0).unwrap(),
        ));
        assert!(db
            .get_filled_role_stats_in("rf", empty_window)
            .await
            .unwrap()
            .is_empty());
    }
}
