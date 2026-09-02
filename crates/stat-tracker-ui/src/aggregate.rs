//! Season-window aggregation — pure function, UI crate only (design §6).
//!
//! A game belongs to a window when `starts_at <= played_at < ends_at` (UTC),
//! matching `GET /api/public/seasons` / the website.

use chrono::{DateTime, Datelike, Local, NaiveDate, Utc};

use crate::model::{Game, Outcome, Role, RoleFilter};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeasonWindow {
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
}

impl SeasonWindow {
    pub fn contains(self, played_at: DateTime<Utc>) -> bool {
        self.starts_at <= played_at && played_at < self.ends_at
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct GameFilter {
    pub window: Option<SeasonWindow>,
    pub roles: Vec<Role>,
    pub hero: Option<String>,
    pub map: Option<String>,
    pub outcome: Option<Outcome>,
}

impl GameFilter {
    pub fn from_header(window: Option<SeasonWindow>, roles: RoleFilter) -> Self {
        Self {
            window,
            roles: roles.selected_roles(),
            hero: None,
            map: None,
            outcome: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    pub games: usize,
    pub wins: usize,
    pub losses: usize,
    pub draws: usize,
}

impl Record {
    pub fn win_rate(&self) -> f32 {
        let decided = self.wins + self.losses + self.draws;
        if decided == 0 {
            0.0
        } else {
            self.wins as f32 / decided as f32
        }
    }

    pub fn win_rate_pct(&self) -> f32 {
        self.win_rate() * 100.0
    }

    /// `4–1` or `4–1–1` when draws are present (Games session header).
    pub fn wl_label(&self) -> String {
        if self.draws > 0 {
            format!("{}–{}–{}", self.wins, self.losses, self.draws)
        } else {
            format!("{}–{}", self.wins, self.losses)
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HeroAgg {
    pub hero: String,
    pub role: Role,
    pub record: Record,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MapAgg {
    pub map_name: String,
    pub record: Record,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RoleAgg {
    pub role: Role,
    pub record: Record,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Aggregates {
    pub record: Record,
    pub heroes: Vec<HeroAgg>,
    pub maps: Vec<MapAgg>,
    pub roles: Vec<RoleAgg>,
}

/// Local-calendar-day group for the Games screen (`Sat Aug 30 · 4–1`).
#[derive(Debug, Clone, PartialEq)]
pub struct DayGroup {
    pub heading: String,
    pub day: NaiveDate,
    pub record: Record,
    pub games: Vec<Game>,
}

/// Filter + aggregate. `window = None` is all time. `role = None` keeps every role.
pub fn aggregate(games: &[Game], window: Option<SeasonWindow>, role: Option<Role>) -> Aggregates {
    aggregate_filtered(
        games,
        &GameFilter {
            window,
            roles: role.into_iter().collect(),
            ..GameFilter::default()
        },
    )
}

pub fn aggregate_filtered(games: &[Game], filter: &GameFilter) -> Aggregates {
    aggregate_rows(&filter_games(games, filter))
}

pub fn filter_games<'a>(games: &'a [Game], filter: &GameFilter) -> Vec<&'a Game> {
    games
        .iter()
        .filter(|g| filter.window.is_none_or(|w| w.contains(g.played_at)))
        .filter(|g| filter.roles.is_empty() || filter.roles.contains(&g.role))
        .filter(|g| filter.hero.as_ref().is_none_or(|h| &g.hero == h))
        .filter(|g| filter.map.as_ref().is_none_or(|m| &g.map_name == m))
        .filter(|g| filter.outcome.is_none_or(|o| g.outcome == o))
        .collect()
}

pub fn distinct_heroes(games: &[&Game]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for g in games {
        if !g.hero.is_empty() && !out.iter().any(|h| h == &g.hero) {
            out.push(g.hero.clone());
        }
    }
    out.sort();
    out
}

pub fn distinct_maps(games: &[&Game]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for g in games {
        if !g.map_name.is_empty() && !out.iter().any(|m| m == &g.map_name) {
            out.push(g.map_name.clone());
        }
    }
    out.sort();
    out
}

/// Group newest-first games by the local calendar day. Header matches the
/// design example (`Sat Aug 30 · 4–1`). Daemon `session_id` is one game, so
/// the Games screen groups by sitting/day, not by session_id.
pub fn group_by_local_day(games: &[&Game]) -> Vec<DayGroup> {
    let mut groups: Vec<DayGroup> = Vec::new();
    for g in games {
        let day = g.played_at.with_timezone(&Local).date_naive();
        match groups.last_mut() {
            Some(grp) if grp.day == day => {
                bump_outcome(&mut grp.record, g.outcome);
                grp.record.games += 1;
                grp.games.push((*g).clone());
            }
            _ => {
                let mut record = Record {
                    games: 1,
                    wins: 0,
                    losses: 0,
                    draws: 0,
                };
                bump_outcome(&mut record, g.outcome);
                groups.push(DayGroup {
                    heading: String::new(),
                    day,
                    record,
                    games: vec![(*g).clone()],
                });
            }
        }
    }
    for grp in &mut groups {
        grp.heading = format!("{} · {}", format_day(grp.day), grp.record.wl_label());
    }
    groups
}

fn format_day(day: NaiveDate) -> String {
    format!(
        "{} {} {}",
        weekday_short(day.weekday().num_days_from_sunday()),
        month_short(day.month()),
        day.day()
    )
}

fn weekday_short(from_sunday: u32) -> &'static str {
    match from_sunday {
        0 => "Sun",
        1 => "Mon",
        2 => "Tue",
        3 => "Wed",
        4 => "Thu",
        5 => "Fri",
        _ => "Sat",
    }
}

fn month_short(m: u32) -> &'static str {
    match m {
        1 => "Jan",
        2 => "Feb",
        3 => "Mar",
        4 => "Apr",
        5 => "May",
        6 => "Jun",
        7 => "Jul",
        8 => "Aug",
        9 => "Sep",
        10 => "Oct",
        11 => "Nov",
        _ => "Dec",
    }
}

fn aggregate_rows(filtered: &[&Game]) -> Aggregates {
    let mut record = Record {
        games: filtered.len(),
        wins: 0,
        losses: 0,
        draws: 0,
    };
    let mut hero_map: Vec<(String, Role, Record)> = Vec::new();
    let mut map_map: Vec<(String, Record)> = Vec::new();
    let mut role_map: Vec<(Role, Record)> = Vec::new();

    for g in filtered {
        bump_outcome(&mut record, g.outcome);
        bump_hero(&mut hero_map, g);
        if !g.map_name.is_empty() {
            bump_map(&mut map_map, g);
        }
        bump_role(&mut role_map, g);
    }

    hero_map.sort_by(|a, b| b.2.games.cmp(&a.2.games).then_with(|| a.0.cmp(&b.0)));
    map_map.sort_by(|a, b| b.1.games.cmp(&a.1.games).then_with(|| a.0.cmp(&b.0)));
    let role_order = [Role::Tank, Role::Damage, Role::Support];
    role_map.sort_by_key(|(r, _)| role_order.iter().position(|o| o == r).unwrap_or(99));

    Aggregates {
        record,
        heroes: hero_map
            .into_iter()
            .map(|(hero, role, record)| HeroAgg { hero, role, record })
            .collect(),
        maps: map_map
            .into_iter()
            .map(|(map_name, record)| MapAgg { map_name, record })
            .collect(),
        roles: role_map
            .into_iter()
            .map(|(role, record)| RoleAgg { role, record })
            .collect(),
    }
}

fn bump_outcome(r: &mut Record, outcome: Outcome) {
    match outcome {
        Outcome::Win => r.wins += 1,
        Outcome::Loss => r.losses += 1,
        Outcome::Draw => r.draws += 1,
        Outcome::Unknown => {}
    }
}

fn bump_hero(rows: &mut Vec<(String, Role, Record)>, g: &Game) {
    if let Some((_, _, rec)) = rows.iter_mut().find(|(h, _, _)| h == &g.hero) {
        rec.games += 1;
        bump_outcome(rec, g.outcome);
    } else {
        let mut rec = Record {
            games: 1,
            wins: 0,
            losses: 0,
            draws: 0,
        };
        bump_outcome(&mut rec, g.outcome);
        rows.push((g.hero.clone(), g.role, rec));
    }
}

fn bump_map(rows: &mut Vec<(String, Record)>, g: &Game) {
    if let Some((_, rec)) = rows.iter_mut().find(|(n, _)| n == &g.map_name) {
        rec.games += 1;
        bump_outcome(rec, g.outcome);
    } else {
        let mut rec = Record {
            games: 1,
            wins: 0,
            losses: 0,
            draws: 0,
        };
        bump_outcome(&mut rec, g.outcome);
        rows.push((g.map_name.clone(), rec));
    }
}

fn bump_role(rows: &mut Vec<(Role, Record)>, g: &Game) {
    if let Some((_, rec)) = rows.iter_mut().find(|(r, _)| *r == g.role) {
        rec.games += 1;
        bump_outcome(rec, g.outcome);
    } else {
        let mut rec = Record {
            games: 1,
            wins: 0,
            losses: 0,
            draws: 0,
        };
        bump_outcome(&mut rec, g.outcome);
        rows.push((g.role, rec));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::GameOcr;
    use chrono::TimeZone;

    fn game(hero: &str, role: Role, outcome: Outcome, at: DateTime<Utc>) -> Game {
        game_on("King's Row", hero, role, outcome, at)
    }

    fn game_on(map: &str, hero: &str, role: Role, outcome: Outcome, at: DateTime<Utc>) -> Game {
        Game {
            session_id: format!("s-{hero}-{at}"),
            hero: hero.into(),
            map_name: map.into(),
            role,
            outcome,
            elims: 12,
            deaths: 4,
            assists: 6,
            damage: 8000,
            healing: 0,
            mitigation: 0,
            played_at: at,
            edited: false,
            edited_fields: Vec::new(),
            ocr: GameOcr::default(),
            segments: Vec::new(),
        }
    }

    fn ts(y: i32, m: u32, d: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, 12, 0, 0).unwrap()
    }

    fn ts_h(y: i32, m: u32, d: u32, hh: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, hh, 0, 0).unwrap()
    }

    #[test]
    fn half_open_window_excludes_end_and_keeps_start() {
        let window = SeasonWindow {
            starts_at: ts(2026, 9, 1),
            ends_at: ts(2026, 12, 1),
        };
        let rows = vec![
            game("Ana", Role::Support, Outcome::Win, ts(2026, 8, 31)),
            game("Ana", Role::Support, Outcome::Win, ts(2026, 9, 1)),
            game("Ashe", Role::Damage, Outcome::Loss, ts(2026, 11, 30)),
            game("Reinhardt", Role::Tank, Outcome::Loss, ts(2026, 12, 1)),
        ];
        let agg = aggregate(&rows, Some(window), None);
        assert_eq!(agg.record.games, 2);
        assert_eq!(agg.record.wins, 1);
        assert_eq!(agg.record.losses, 1);
        assert!(
            (agg.record.win_rate() - 0.5).abs() < f32::EPSILON,
            "win rate {}",
            agg.record.win_rate()
        );
    }

    #[test]
    fn all_time_and_role_filter() {
        let rows = vec![
            game("Ana", Role::Support, Outcome::Win, ts(2026, 8, 1)),
            game("Ashe", Role::Damage, Outcome::Loss, ts(2026, 9, 2)),
            game("Junker Queen", Role::Tank, Outcome::Win, ts(2026, 9, 2)),
        ];
        let all = aggregate(&rows, None, None);
        assert_eq!(all.record.games, 3);
        assert_eq!(all.heroes.len(), 3);
        let tank = aggregate(&rows, None, Some(Role::Tank));
        assert_eq!(tank.record.games, 1);
        assert_eq!(tank.heroes[0].hero, "Junker Queen");
    }

    #[test]
    fn unknown_outcomes_do_not_count_as_decided() {
        let rows = vec![
            game("Ana", Role::Support, Outcome::Win, ts(2026, 9, 2)),
            game("Ana", Role::Support, Outcome::Unknown, ts(2026, 9, 2)),
        ];
        let agg = aggregate(&rows, None, None);
        assert_eq!(agg.record.games, 2);
        assert_eq!(agg.record.wins, 1);
        assert!((agg.record.win_rate() - 1.0).abs() < f32::EPSILON);
    }

    /// Games immediately before and on a season start stay on the correct
    /// side of the half-open window; hero/map lists and day groups follow.
    #[test]
    fn season_boundary_filters_hero_map_and_day_groups() {
        let window = SeasonWindow {
            starts_at: ts_h(2026, 9, 1, 0),
            ends_at: ts_h(2026, 12, 10, 0),
        };
        let before = game_on(
            "Numbani",
            "Reinhardt",
            Role::Tank,
            Outcome::Win,
            ts_h(2026, 8, 31, 23),
        );
        let on_start = game_on(
            "King's Row",
            "Junker Queen",
            Role::Tank,
            Outcome::Loss,
            ts_h(2026, 9, 1, 0),
        );
        let later = game_on(
            "Ilios",
            "Ana",
            Role::Support,
            Outcome::Win,
            ts_h(2026, 9, 1, 21),
        );
        let rows = vec![later.clone(), on_start.clone(), before.clone()];

        let in_s17 = filter_games(
            &rows,
            &GameFilter {
                window: Some(window),
                ..GameFilter::default()
            },
        );
        assert_eq!(in_s17.len(), 2);
        assert_eq!(in_s17[0].hero, "Ana");
        assert_eq!(in_s17[1].hero, "Junker Queen");
        assert!(in_s17.iter().all(|g| g.hero != "Reinhardt"));

        let tank_s17 = aggregate_filtered(
            &rows,
            &GameFilter {
                window: Some(window),
                roles: vec![Role::Tank],
                ..GameFilter::default()
            },
        );
        assert_eq!(tank_s17.record.games, 1);
        assert_eq!(tank_s17.heroes.len(), 1);
        assert_eq!(tank_s17.heroes[0].hero, "Junker Queen");
        assert_eq!(tank_s17.maps.len(), 1);
        assert_eq!(tank_s17.maps[0].map_name, "King's Row");

        let hero_only = filter_games(
            &rows,
            &GameFilter {
                window: Some(window),
                hero: Some("Ana".into()),
                ..GameFilter::default()
            },
        );
        assert_eq!(hero_only.len(), 1);
        assert_eq!(hero_only[0].map_name, "Ilios");

        let map_only = filter_games(
            &rows,
            &GameFilter {
                window: Some(window),
                map: Some("King's Row".into()),
                ..GameFilter::default()
            },
        );
        assert_eq!(map_only.len(), 1);
        assert_eq!(map_only[0].outcome, Outcome::Loss);

        // Midday stamps so local-day grouping is stable across timezones.
        let day_a = game("Ashe", Role::Damage, Outcome::Win, ts_h(2026, 8, 18, 12));
        let day_b1 = game("Ana", Role::Support, Outcome::Win, ts_h(2026, 9, 2, 12));
        let day_b2 = game("Ashe", Role::Damage, Outcome::Loss, ts_h(2026, 9, 2, 14));
        let day_b3 = game(
            "Junker Queen",
            Role::Tank,
            Outcome::Win,
            ts_h(2026, 9, 2, 16),
        );
        let sitting = vec![&day_b3, &day_b2, &day_b1, &day_a];
        let groups = group_by_local_day(&sitting);
        assert_eq!(
            groups.len(),
            2,
            "Aug 18 and Sep 2 must stay split: {groups:?}"
        );
        assert!(
            groups[0].heading.contains("· 2–1"),
            "newest day heading {}",
            groups[0].heading
        );
        assert!(
            groups[1].heading.contains("· 1–0"),
            "older day heading {}",
            groups[1].heading
        );
        assert!(groups[0].heading.contains("Sep"), "{}", groups[0].heading);
        assert!(groups[1].heading.contains("Aug"), "{}", groups[1].heading);
    }

    #[test]
    fn role_filter_union_and_distinct_lists() {
        let rows = vec![
            game("Ana", Role::Support, Outcome::Win, ts(2026, 9, 2)),
            game("Ashe", Role::Damage, Outcome::Loss, ts(2026, 9, 2)),
            game("Junker Queen", Role::Tank, Outcome::Win, ts(2026, 9, 2)),
        ];
        let mut roles = RoleFilter::default();
        roles = roles.toggle(Role::Tank);
        roles = roles.toggle(Role::Support);
        let filtered = filter_games(&rows, &GameFilter::from_header(None, roles));
        assert_eq!(filtered.len(), 2);
        let heroes = distinct_heroes(&filtered);
        assert_eq!(heroes, vec!["Ana".to_string(), "Junker Queen".to_string()]);
        let maps = distinct_maps(&filtered);
        assert_eq!(maps, vec!["King's Row".to_string()]);
    }
}
