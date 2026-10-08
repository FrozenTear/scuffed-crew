//! Stored per-game role.
//!
//! History reads the `role` string uploaded with each game. Overview and the
//! member page read `GET /api/stats/me/roles` and
//! `GET /api/stats/member/{id}/roles`, which group by that same stored role.
//! An empty or unrecognized stored role is Unknown. The heroes filter is the
//! current roster role from [`scuffed_types::role_for_hero_name`], because
//! `/heroes` does not return the role captured with each game.

use scuffed_types::{HeroRole, RoleStats, role_for_hero_name};

use crate::components::member_gate::is_http_status;

pub(crate) struct RoleAgg {
    pub name: &'static str,
    pub color: &'static str,
    pub matches: u32,
    pub wins: u32,
    weighted_elims: f64,
    weighted_deaths: f64,
    weighted_damage: f64,
    weighted_healing: f64,
}

impl RoleAgg {
    fn new(name: &'static str, color: &'static str) -> Self {
        Self {
            name,
            color,
            matches: 0,
            wins: 0,
            weighted_elims: 0.0,
            weighted_deaths: 0.0,
            weighted_damage: 0.0,
            weighted_healing: 0.0,
        }
    }

    pub(crate) fn winrate(&self) -> f64 {
        super::winrate_pct(self.wins, self.matches)
    }

    pub(crate) fn avg_elims(&self) -> f64 {
        avg(self.weighted_elims, self.matches)
    }

    pub(crate) fn avg_deaths(&self) -> f64 {
        avg(self.weighted_deaths, self.matches)
    }

    pub(crate) fn avg_damage(&self) -> f64 {
        avg(self.weighted_damage, self.matches)
    }

    pub(crate) fn avg_healing(&self) -> f64 {
        avg(self.weighted_healing, self.matches)
    }

    fn add(
        &mut self,
        matches: u32,
        wins: u32,
        avg_elims: f64,
        avg_deaths: f64,
        avg_damage: f64,
        avg_healing: f64,
    ) {
        let weight = f64::from(matches);
        self.matches += matches;
        self.wins += wins;
        self.weighted_elims += avg_elims * weight;
        self.weighted_deaths += avg_deaths * weight;
        self.weighted_damage += avg_damage * weight;
        self.weighted_healing += avg_healing * weight;
    }
}

fn avg(weighted: f64, matches: u32) -> f64 {
    if matches == 0 {
        0.0
    } else {
        weighted / f64::from(matches)
    }
}

/// Signed-in member. Never used for another member's id.
pub(crate) fn my_roles_path() -> &'static str {
    "/api/stats/me/roles"
}

/// Another member. A blank id stays on this path and does not become `/me`.
pub(crate) fn member_roles_path(member_id: &str) -> String {
    format!("/api/stats/member/{member_id}/roles")
}

/// `Tank` / `Damage` / `Support`, or `None` when the stored role is missing
/// or not one of those three.
pub(super) fn known_role(stored: &str) -> Option<&'static str> {
    match stored.trim().to_ascii_lowercase().as_str() {
        "tank" => Some("Tank"),
        "damage" => Some("Damage"),
        "support" => Some("Support"),
        _ => None,
    }
}

/// Label for a stored role. Empty or unrecognized stays Unknown.
pub(super) fn stored_role_label(stored: &str) -> &'static str {
    known_role(stored).unwrap_or("Unknown")
}

/// History filter. Uses the stored role only, so an old Sombra game stays
/// Damage and a missing role stays Unknown.
pub(super) fn history_row_matches_role(stored: &str, filter: &str) -> bool {
    filter.eq_ignore_ascii_case("all") || stored_role_label(stored).eq_ignore_ascii_case(filter)
}

/// Current roster role. `None` when the name is not on the shared list, so
/// an unknown hero is not treated as Damage.
pub(super) fn current_role_label(hero: &str) -> Option<&'static str> {
    role_for_hero_name(hero).map(|role| match role {
        HeroRole::Tank => "Tank",
        HeroRole::Damage => "Damage",
        HeroRole::Support => "Support",
    })
}

/// Heroes-tab filter. `/heroes` has no stored role, so this is the current
/// roster role, not the role captured on each old game.
pub(super) fn hero_row_matches_filter(hero: &str, filter: &str) -> bool {
    filter == "All" || current_role_label(hero) == Some(filter)
}

fn fresh_buckets() -> [RoleAgg; 4] {
    [
        RoleAgg::new("Tank", "var(--chart-5)"),
        RoleAgg::new("Damage", "var(--chart-4)"),
        RoleAgg::new("Support", "var(--chart-2)"),
        RoleAgg::new("Unknown", "var(--chart-3)"),
    ]
}

fn bucket_index(role: &str) -> usize {
    match known_role(role) {
        Some("Tank") => 0,
        Some("Damage") => 1,
        Some("Support") => 2,
        _ => 3,
    }
}

/// Overview totals from `/roles`. Rows keep the stored role. An empty or
/// unrecognized `role` is Unknown.
pub(crate) fn role_aggs_from_rows(rows: &[RoleStats]) -> Vec<RoleAgg> {
    let mut buckets = fresh_buckets();
    for row in rows {
        buckets[bucket_index(&row.role)].add(
            row.matches,
            row.wins,
            row.avg_elims,
            row.avg_deaths,
            row.avg_damage,
            row.avg_healing,
        );
    }
    buckets.into_iter().collect()
}

/// What `use_api_with::<Vec<RoleStats>>` produced. An error string is a
/// failure. There is no catalog fallback.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum RolePanel<'a> {
    Pending,
    Failed,
    Ready(&'a [RoleStats]),
}

pub(crate) fn role_panel<'a>(error: Option<&str>, rows: Option<&'a [RoleStats]>) -> RolePanel<'a> {
    if error.is_some() {
        RolePanel::Failed
    } else if let Some(rows) = rows {
        RolePanel::Ready(rows)
    } else {
        RolePanel::Pending
    }
}

/// 401 and 403 are the member gate, not a role-load failure.
pub(crate) fn hide_member_role_error(error: &str) -> bool {
    is_http_status(error, 401) || is_http_status(error, 403)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(role: &str, matches: u32, wins: u32, avg_elims: f64) -> RoleStats {
        RoleStats {
            role: role.to_string(),
            matches,
            wins,
            losses: matches.saturating_sub(wins),
            draws: 0,
            avg_elims,
            avg_deaths: 3.0,
            avg_damage: 1000.0,
            avg_healing: 500.0,
        }
    }

    fn matches_of(aggs: &[RoleAgg], name: &str) -> u32 {
        aggs.iter()
            .find(|agg| agg.name == name)
            .map(|agg| agg.matches)
            .unwrap_or(0)
    }

    #[test]
    fn roles_paths_are_separate_for_me_and_member() {
        assert_eq!(my_roles_path(), "/api/stats/me/roles");
        assert_eq!(
            member_roles_path("member-1"),
            "/api/stats/member/member-1/roles"
        );
        assert_ne!(member_roles_path("  "), my_roles_path());
        assert_ne!(member_roles_path(""), my_roles_path());
    }

    #[test]
    fn roles_payload_is_a_bare_array_like_heroes() {
        let body = r#"[{"role":"Support","matches":2,"wins":1,"losses":1,"draws":0,"avg_elims":8.5,"avg_deaths":3.0,"avg_damage":1200.0,"avg_healing":4000.0}]"#;
        let rows: Vec<RoleStats> = serde_json::from_str(body).unwrap();
        assert_eq!(rows[0].role, "Support");
        assert_eq!(rows[0].matches, 2);
        assert!(serde_json::from_str::<Vec<RoleStats>>(r#"{"data":[]}"#).is_err());
    }

    #[test]
    fn old_and_new_sombra_aggregate_into_both_roles() {
        let rows = vec![row("Damage", 4, 2, 12.0), row("Support", 3, 2, 6.0)];
        let aggs = role_aggs_from_rows(&rows);
        assert_eq!(matches_of(&aggs, "Damage"), 4);
        assert_eq!(matches_of(&aggs, "Support"), 3);
    }

    #[test]
    fn empty_and_unrecognized_stored_roles_are_unknown() {
        let aggs = role_aggs_from_rows(&[row("", 2, 1, 1.0), row("flex", 1, 0, 1.0)]);
        assert_eq!(matches_of(&aggs, "Unknown"), 3);
        assert_eq!(matches_of(&aggs, "Damage"), 0);
        assert_eq!(stored_role_label(""), "Unknown");
        assert_eq!(stored_role_label("not-a-role"), "Unknown");
    }

    #[test]
    fn history_keeps_the_stored_role() {
        assert!(history_row_matches_role("Damage", "damage"));
        assert!(!history_row_matches_role("Damage", "support"));
        assert!(history_row_matches_role("Support", "support"));
        assert!(history_row_matches_role("Damage", "all"));
        assert_eq!(stored_role_label("Damage"), "Damage");
        assert_eq!(stored_role_label("Support"), "Support");
        // A missing role is not filled in from the hero, even for Sombra.
        assert_eq!(stored_role_label(""), "Unknown");
        assert!(!history_row_matches_role("", "support"));
        assert!(!history_row_matches_role("", "damage"));
        assert!(history_row_matches_role("", "all"));
        assert!(!history_row_matches_role("not-a-role", "tank"));
    }

    #[test]
    fn current_roster_roles_for_the_heroes_filter() {
        assert_eq!(current_role_label("D.Mon"), Some("Tank"));
        assert_eq!(current_role_label("dmon"), Some("Tank"));
        assert_eq!(current_role_label("Jetpack Cat"), Some("Support"));
        assert_eq!(current_role_label("Doctrine"), Some("Support"));
        assert_eq!(current_role_label("Sombra"), Some("Support"));
        assert_eq!(current_role_label("NotAHero"), None);
        assert!(hero_row_matches_filter("D.Mon", "Tank"));
        assert!(hero_row_matches_filter("Jetpack Cat", "Support"));
        assert!(!hero_row_matches_filter("Jetpack Cat", "Damage"));
        assert!(!hero_row_matches_filter("NotAHero", "Damage"));
        assert!(hero_row_matches_filter("NotAHero", "All"));
        assert!(hero_row_matches_filter("Sombra", "Support"));
        assert!(!hero_row_matches_filter("Sombra", "Damage"));
    }

    #[test]
    fn role_panel_maps_the_shared_fetch() {
        assert_eq!(role_panel(None, None), RolePanel::Pending);
        assert_eq!(
            role_panel(Some("HTTP error 500: Internal error"), None),
            RolePanel::Failed
        );
        let rows = vec![row("Tank", 1, 1, 1.0)];
        assert!(matches!(role_panel(None, Some(&rows)), RolePanel::Ready(_)));
        assert!(matches!(
            role_panel(Some("HTTP error 404: Season not found"), Some(&rows)),
            RolePanel::Failed
        ));
    }

    #[test]
    fn weighted_means_trim_and_weight_stored_roles() {
        let aggs = role_aggs_from_rows(&[
            row("", 1, 1, 4.0),
            row("flex", 3, 1, 8.0),
            row(" support ", 1, 1, 2.0),
        ]);
        let unknown = aggs.iter().find(|agg| agg.name == "Unknown").unwrap();
        assert_eq!(unknown.matches, 4);
        assert_eq!(unknown.wins, 2);
        assert_eq!(unknown.avg_elims(), 7.0);
        assert_eq!(unknown.avg_deaths(), 3.0);
        assert_eq!(unknown.avg_damage(), 1000.0);
        assert_eq!(unknown.avg_healing(), 500.0);
        assert_eq!(unknown.winrate(), 50.0);
        assert_eq!(matches_of(&aggs, "Support"), 1);
        assert_eq!(matches_of(&aggs, "Damage"), 0);
    }

    #[test]
    fn member_auth_errors_are_not_role_errors() {
        assert!(hide_member_role_error("HTTP error 401: Unauthorized"));
        assert!(hide_member_role_error("HTTP error: 403"));
        assert!(!hide_member_role_error("HTTP error 404: Season not found"));
        assert!(!hide_member_role_error("HTTP error 500: Internal error"));
    }
}
