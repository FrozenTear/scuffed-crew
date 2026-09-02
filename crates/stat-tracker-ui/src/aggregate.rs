//! Season-window aggregation — pure function, UI crate only (design §6).
//!
//! A game belongs to a window when `starts_at <= played_at < ends_at` (UTC),
//! matching `GET /api/public/seasons` / the website.

use chrono::{DateTime, Utc};

use crate::model::{Game, Outcome, Role};

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

/// Filter + aggregate. `window = None` is all time. `role = None` keeps every role.
pub fn aggregate(games: &[Game], window: Option<SeasonWindow>, role: Option<Role>) -> Aggregates {
    let filtered: Vec<&Game> = games
        .iter()
        .filter(|g| window.is_none_or(|w| w.contains(g.played_at)))
        .filter(|g| role.is_none_or(|r| g.role == r))
        .collect();

    let mut record = Record {
        games: filtered.len(),
        wins: 0,
        losses: 0,
        draws: 0,
    };
    let mut hero_map: Vec<(String, Role, Record)> = Vec::new();
    let mut map_map: Vec<(String, Record)> = Vec::new();
    let mut role_map: Vec<(Role, Record)> = Vec::new();

    for g in &filtered {
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
    use chrono::TimeZone;

    fn game(hero: &str, role: Role, outcome: Outcome, at: DateTime<Utc>) -> Game {
        Game {
            session_id: "s".into(),
            hero: hero.into(),
            map_name: "King's Row".into(),
            role,
            outcome,
            elims: 12,
            deaths: 4,
            assists: 6,
            damage: 8000,
            healing: 0,
            mitigation: 0,
            played_at: at,
        }
    }

    fn ts(y: i32, m: u32, d: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, 12, 0, 0).unwrap()
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
}
