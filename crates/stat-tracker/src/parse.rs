use crate::storage::PersonalMatch;
use chrono::Utc;
use surrealdb_types::Datetime as SurrealDatetime;

pub fn parse_scoreboard(raw_text: &str, outcome: &str, player_name: Option<&str>) -> Option<PersonalMatch> {
    let lines: Vec<&str> = raw_text
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();

    if lines.is_empty() {
        tracing::warn!("OCR produced no text");
        return None;
    }

    tracing::debug!(line_count = lines.len(), "parsing scoreboard text");

    let row = if let Some(name) = player_name {
        find_player_row(&lines, name)
    } else {
        None
    };

    let stats = row
        .and_then(|r| extract_row_stats(r))
        .or_else(|| find_best_stat_row(&lines));

    let stats = stats?;

    let hero = find_hero(&lines).unwrap_or_else(|| "Unknown".to_string());
    let role = guess_role(&hero);

    Some(PersonalMatch {
        hero,
        map_name: String::new(),
        game_mode: String::new(),
        role,
        outcome: outcome.to_string(),
        elims: stats.elims,
        deaths: stats.deaths,
        assists: stats.assists,
        damage: stats.damage,
        healing: stats.healing,
        mitigation: stats.mitigation,
        played_at: SurrealDatetime::from(Utc::now()),
        synced: false,
        session_id: String::new(),
    })
}

struct PlayerStats {
    elims: u32,
    deaths: u32,
    assists: u32,
    damage: u32,
    healing: u32,
    mitigation: u32,
}

fn find_player_row<'a>(lines: &[&'a str], player_name: &str) -> Option<&'a str> {
    let name_lower = player_name.to_lowercase();
    lines.iter().find(|line| {
        let lower = line.to_lowercase();
        lower.contains(&name_lower)
    }).copied()
}

fn extract_row_stats(line: &str) -> Option<PlayerStats> {
    let numbers = extract_numbers(line);
    stats_from_numbers(&numbers)
}

fn find_best_stat_row(lines: &[&str]) -> Option<PlayerStats> {
    for line in lines {
        let numbers = extract_numbers(line);
        if let Some(stats) = stats_from_numbers(&numbers) {
            return Some(stats);
        }
    }
    None
}

// OW2 scoreboard stat columns: E, A, D, DMG, HLG, MIT
fn stats_from_numbers(numbers: &[u32]) -> Option<PlayerStats> {
    if numbers.len() < 6 {
        return None;
    }

    // Take the last 6 numbers — earlier tokens may be from player name/rank OCR artifacts
    let offset = numbers.len() - 6;
    Some(PlayerStats {
        elims: numbers[offset],
        assists: numbers[offset + 1],
        deaths: numbers[offset + 2],
        damage: numbers[offset + 3],
        healing: numbers[offset + 4],
        mitigation: numbers[offset + 5],
    })
}

fn extract_numbers(s: &str) -> Vec<u32> {
    let cleaned: String = s.chars().filter(|c| *c != ',').collect();
    cleaned
        .split(|c: char| !c.is_ascii_digit())
        .filter(|w| !w.is_empty())
        .filter_map(|w| w.parse::<u32>().ok())
        .collect()
}

const HEROES: &[&str] = &[
    "Ana", "Anran", "Ashe", "Baptiste", "Bastion", "Brigitte", "Cassidy",
    "D.Va", "Domina", "Doomfist", "Echo", "Emre", "Freja",
    "Genji", "Hanzo", "Hazard",
    "Illari", "Jetpack Cat", "Junker Queen", "Junkrat", "Juno", "Kiriko",
    "Lifeweaver", "Lucio", "Mauga", "Mei", "Mercy", "Mizuki", "Moira",
    "Orisa", "Pharah", "Ramattra", "Reaper", "Reinhardt",
    "Roadhog", "Sierra", "Sigma", "Sojourn", "Soldier: 76", "Sombra",
    "Symmetra", "Torbjorn", "Tracer", "Vendetta", "Venture",
    "Widowmaker", "Winston", "Wrecking Ball", "Wuyang", "Zarya", "Zenyatta",
];

fn find_hero(lines: &[&str]) -> Option<String> {
    // Count occurrences of each hero name and also check if it appears
    // on a line by itself or in the right-side panel (capitalized hero name).
    // OW2 scoreboard shows YOUR hero in large text on the right panel,
    // while enemy heroes appear in player rows. Prefer the hero that
    // appears with a role keyword nearby or in a standalone mention.
    let text = lines.join(" ").to_lowercase();

    let mut found: Vec<(&str, usize)> = Vec::new();
    for &hero in HEROES {
        let hero_lower = hero.to_lowercase();
        let count = text.matches(&hero_lower).count();
        if count > 0 {
            found.push((hero, count));
        }
    }

    if found.is_empty() {
        return None;
    }

    // If only one hero found, use it
    if found.len() == 1 {
        return Some(found[0].0.to_string());
    }

    // When multiple heroes detected, prefer the one that appears with
    // role-indicator keywords nearby (the right panel shows hero + accuracy stats).
    // The right panel text has "WEAPON ACCURACY", "CRITICAL HIT" etc.
    let panel_keywords = ["accuracy", "critical", "weapon", "kills"];
    for &(hero, _) in &found {
        let hero_lower = hero.to_lowercase();
        for line in lines {
            let line_lower = line.to_lowercase();
            if line_lower.contains(&hero_lower) {
                // Skip if this is on a stat row (has many numbers)
                let num_count = line.split(|c: char| !c.is_ascii_digit())
                    .filter(|w| !w.is_empty())
                    .count();
                if num_count <= 1 {
                    return Some(hero.to_string());
                }
            }
        }
        // Check if hero name appears near panel keywords
        for kw in &panel_keywords {
            if text.contains(&format!("{} {}", hero_lower, kw))
                || text.contains(&format!("{} {}", kw, hero_lower))
            {
                return Some(hero.to_string());
            }
        }
    }

    // Fallback: pick the hero with fewest occurrences (likely the player's hero
    // appearing once in the panel, vs bots appearing many times in team rows)
    found.sort_by_key(|&(_, count)| count);
    Some(found[0].0.to_string())
}

pub fn guess_role_public(hero: &str) -> String {
    guess_role(hero)
}

fn guess_role(hero: &str) -> String {
    match hero.to_lowercase().as_str() {
        "d.va" | "dva" | "doomfist" | "junker queen" | "junker_queen" | "mauga"
        | "orisa" | "ramattra" | "reinhardt" | "roadhog" | "sigma" | "winston"
        | "wrecking ball" | "wrecking_ball" | "zarya" | "hazard" => "Tank".to_string(),
        "ana" | "baptiste" | "brigitte" | "illari" | "juno" | "kiriko"
        | "lifeweaver" | "lucio" | "mercy" | "moira" | "zenyatta"
        | "anran" | "freja" | "mizuki" | "sierra" => "Support".to_string(),
        _ => "Damage".to_string(),
    }
}
