//! Aggregate match statistics from local `PersonalMatch` rows.
//!
//! Lives in the lib (not the GUI feature) so win-rate / per-hero math can be
//! unit-tested without compiling the Dioxus desktop binary.

use std::collections::HashMap;

use crate::storage::PersonalMatch;

#[derive(Debug, Clone, PartialEq)]
pub struct OverallStats {
    pub total: usize,
    pub wins: usize,
    pub losses: usize,
    pub draws: usize,
}

impl OverallStats {
    /// Win rate over decided games only — unknown outcomes must not dilute it.
    pub fn win_rate(&self) -> f64 {
        let decided = self.wins + self.losses + self.draws;
        if decided == 0 {
            return 0.0;
        }
        (self.wins as f64 / decided as f64) * 100.0
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HeroStats {
    pub hero: String,
    pub role: String,
    pub games: usize,
    pub wins: usize,
    pub decided: usize,
    pub avg_elims: f64,
    pub avg_deaths: f64,
    pub avg_assists: f64,
    pub avg_damage: f64,
    pub avg_healing: f64,
    pub avg_mitigation: f64,
}

impl HeroStats {
    pub fn win_rate(&self) -> f64 {
        if self.decided == 0 {
            return 0.0;
        }
        (self.wins as f64 / self.decided as f64) * 100.0
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RoleStats {
    pub role: String,
    pub games: usize,
    pub wins: usize,
    pub decided: usize,
}

impl RoleStats {
    pub fn win_rate(&self) -> f64 {
        if self.decided == 0 {
            return 0.0;
        }
        (self.wins as f64 / self.decided as f64) * 100.0
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MapStats {
    pub map_name: String,
    pub games: usize,
    pub wins: usize,
    pub losses: usize,
}

impl MapStats {
    pub fn win_rate(&self) -> f64 {
        let decided = self.wins + self.losses;
        if decided == 0 {
            return 0.0;
        }
        (self.wins as f64 / decided as f64) * 100.0
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HeroMapBreakdown {
    pub map_name: String,
    pub games: usize,
    pub wins: usize,
    pub decided: usize,
}

impl HeroMapBreakdown {
    pub fn win_rate(&self) -> f64 {
        if self.decided == 0 {
            return 0.0;
        }
        (self.wins as f64 / self.decided as f64) * 100.0
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComputedStats {
    pub overall: OverallStats,
    pub heroes: Vec<HeroStats>,
    pub roles: Vec<RoleStats>,
    pub maps: Vec<MapStats>,
    pub hero_maps: HashMap<String, Vec<HeroMapBreakdown>>,
    pub rolling_wr: Vec<f64>,
}

// The daemon records outcomes as "victory"/"defeat"/"draw"/"unknown". Accept
// the legacy "win"/"loss" spellings too so older local data still classifies.
// (GUI will switch to typed MatchOutcome in Phase 2 — keep strings for now.)
fn outcome_is_win(outcome: &str) -> bool {
    outcome.eq_ignore_ascii_case("victory") || outcome.eq_ignore_ascii_case("win")
}
fn outcome_is_loss(outcome: &str) -> bool {
    outcome.eq_ignore_ascii_case("defeat") || outcome.eq_ignore_ascii_case("loss")
}
fn outcome_is_decided(outcome: &str) -> bool {
    outcome_is_win(outcome) || outcome_is_loss(outcome) || outcome.eq_ignore_ascii_case("draw")
}

/// Aggregate per-game rows (already collapsed with `latest_per_game` if needed).
pub fn compute_stats(matches: &[PersonalMatch]) -> ComputedStats {
    let mut wins = 0usize;
    let mut losses = 0usize;
    let mut draws = 0usize;

    struct Acc {
        role: String,
        games: usize,
        wins: usize,
        decided: usize,
        elims: u64,
        deaths: u64,
        assists: u64,
        damage: u64,
        healing: u64,
        mitigation: u64,
    }

    let mut hero_acc: HashMap<String, Acc> = HashMap::new();
    let mut role_map: HashMap<String, (usize, usize, usize)> = HashMap::new();
    let mut map_acc: HashMap<String, (usize, usize, usize)> = HashMap::new();
    let mut hero_map_acc: HashMap<String, HashMap<String, (usize, usize, usize)>> = HashMap::new();

    for m in matches {
        let is_win = outcome_is_win(&m.outcome);
        let is_loss = outcome_is_loss(&m.outcome);
        let is_decided = outcome_is_decided(&m.outcome);
        if is_win {
            wins += 1;
        } else if is_loss {
            losses += 1;
        } else if m.outcome.eq_ignore_ascii_case("draw") {
            draws += 1;
        }
        // "unknown"/unparsed outcomes are excluded from the W/L/D totals.

        let entry = hero_acc.entry(m.hero.clone()).or_insert_with(|| Acc {
            role: m.role.clone(),
            games: 0,
            wins: 0,
            decided: 0,
            elims: 0,
            deaths: 0,
            assists: 0,
            damage: 0,
            healing: 0,
            mitigation: 0,
        });
        entry.games += 1;
        if is_win {
            entry.wins += 1;
        }
        if is_decided {
            entry.decided += 1;
        }
        entry.elims += m.elims as u64;
        entry.deaths += m.deaths as u64;
        entry.assists += m.assists as u64;
        entry.damage += m.damage as u64;
        entry.healing += m.healing as u64;
        entry.mitigation += m.mitigation as u64;

        let role_entry = role_map.entry(m.role.clone()).or_insert((0, 0, 0));
        role_entry.0 += 1;
        if is_win {
            role_entry.1 += 1;
        }
        if is_decided {
            role_entry.2 += 1;
        }

        if !m.map_name.is_empty() {
            let me = map_acc.entry(m.map_name.clone()).or_insert((0, 0, 0));
            me.0 += 1;
            if is_win {
                me.1 += 1;
            }
            if is_loss {
                me.2 += 1;
            }

            let hm = hero_map_acc
                .entry(m.hero.clone())
                .or_default()
                .entry(m.map_name.clone())
                .or_insert((0, 0, 0));
            hm.0 += 1;
            if is_win {
                hm.1 += 1;
            }
            if is_decided {
                hm.2 += 1;
            }
        }
    }

    let overall = OverallStats {
        total: matches.len(),
        wins,
        losses,
        draws,
    };

    let mut heroes: Vec<HeroStats> = hero_acc
        .into_iter()
        .map(|(hero, a)| {
            let g = a.games as f64;
            HeroStats {
                hero,
                role: a.role,
                games: a.games,
                wins: a.wins,
                decided: a.decided,
                avg_elims: a.elims as f64 / g,
                avg_deaths: a.deaths as f64 / g,
                avg_assists: a.assists as f64 / g,
                avg_damage: a.damage as f64 / g,
                avg_healing: a.healing as f64 / g,
                avg_mitigation: a.mitigation as f64 / g,
            }
        })
        .collect();
    heroes.sort_by_key(|h| std::cmp::Reverse(h.games));

    let role_order = ["Tank", "Damage", "Support"];
    let mut roles: Vec<RoleStats> = role_map
        .into_iter()
        .map(|(role, (games, wins, decided))| RoleStats {
            role,
            games,
            wins,
            decided,
        })
        .collect();
    roles.sort_by_key(|r| {
        role_order
            .iter()
            .position(|o| o.eq_ignore_ascii_case(&r.role))
            .unwrap_or(99)
    });

    let mut maps: Vec<MapStats> = map_acc
        .into_iter()
        .map(|(map_name, (games, wins, losses))| MapStats {
            map_name,
            games,
            wins,
            losses,
        })
        .collect();
    maps.sort_by(|a, b| {
        b.games
            .cmp(&a.games)
            .then_with(|| a.map_name.cmp(&b.map_name))
    });

    let mut hero_maps: HashMap<String, Vec<HeroMapBreakdown>> = HashMap::new();
    for (hero, map_data) in hero_map_acc {
        let mut breakdowns: Vec<HeroMapBreakdown> = map_data
            .into_iter()
            .map(|(map_name, (games, wins, decided))| HeroMapBreakdown {
                map_name,
                games,
                wins,
                decided,
            })
            .collect();
        breakdowns.sort_by_key(|b| std::cmp::Reverse(b.games));
        hero_maps.insert(hero, breakdowns);
    }

    // Rolling 10-game winrate over decided games only (matches come in DESC
    // order, reverse for chronological).
    let mut rolling_wr = Vec::new();
    let window = 10usize;
    let chronological: Vec<&PersonalMatch> = matches
        .iter()
        .rev()
        .filter(|m| outcome_is_decided(&m.outcome))
        .collect();
    let mut win_count = 0usize;
    for (i, m) in chronological.iter().enumerate() {
        if outcome_is_win(&m.outcome) {
            win_count += 1;
        }
        if i >= window && outcome_is_win(&chronological[i - window].outcome) {
            win_count -= 1;
        }
        let denom = (i + 1).min(window);
        rolling_wr.push((win_count as f64 / denom as f64) * 100.0);
    }

    ComputedStats {
        overall,
        heroes,
        roles,
        maps,
        hero_maps,
        rolling_wr,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::PersonalMatch;
    use chrono::Utc;
    use surrealdb_types::Datetime as SurrealDatetime;

    fn pm(hero: &str, role: &str, outcome: &str, map: &str) -> PersonalMatch {
        PersonalMatch {
            id: None,
            session_id: String::new(),
            hero: hero.into(),
            role: role.into(),
            map_name: map.into(),
            game_mode: String::new(),
            elims: 10,
            deaths: 5,
            assists: 3,
            damage: 1000,
            healing: 0,
            mitigation: 0,
            outcome: outcome.into(),
            played_at: SurrealDatetime::from(Utc::now()),
            synced: false,
        }
    }

    #[test]
    fn win_rate_ignores_unknown() {
        let rows = vec![
            pm("Ana", "Support", "victory", "King's Row"),
            pm("Ana", "Support", "defeat", "King's Row"),
            pm("Ana", "Support", "unknown", "King's Row"),
        ];
        let s = compute_stats(&rows);
        assert_eq!(s.overall.total, 3);
        assert_eq!(s.overall.wins, 1);
        assert_eq!(s.overall.losses, 1);
        assert!((s.overall.win_rate() - 50.0).abs() < 0.01);
    }

    #[test]
    fn legacy_win_loss_spellings() {
        let rows = vec![
            pm("Reinhardt", "Tank", "win", "Busan"),
            pm("Reinhardt", "Tank", "loss", "Busan"),
        ];
        let s = compute_stats(&rows);
        assert_eq!(s.overall.wins, 1);
        assert_eq!(s.overall.losses, 1);
        assert!((s.heroes[0].win_rate() - 50.0).abs() < 0.01);
    }

    #[test]
    fn per_map_and_role_totals() {
        let rows = vec![
            pm("Ana", "Support", "victory", "Havana"),
            pm("Ana", "Support", "victory", "Havana"),
            pm("Ashe", "Damage", "defeat", "King's Row"),
        ];
        let s = compute_stats(&rows);
        assert_eq!(s.roles.len(), 2);
        let havana = s.maps.iter().find(|m| m.map_name == "Havana").unwrap();
        assert_eq!(havana.games, 2);
        assert_eq!(havana.wins, 2);
    }
}
