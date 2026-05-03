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
    "Ana", "Ashe", "Baptiste", "Bastion", "Brigitte", "Cassidy",
    "D.Va", "Doomfist", "Echo", "Genji", "Hanzo", "Hazard",
    "Illari", "Junker Queen", "Junkrat", "Juno", "Kiriko",
    "Lifeweaver", "Lucio", "Mauga", "Mei", "Mercy", "Moira",
    "Orisa", "Pharah", "Ramattra", "Reaper", "Reinhardt",
    "Roadhog", "Sigma", "Sojourn", "Soldier: 76", "Sombra",
    "Symmetra", "Torbjorn", "Tracer", "Venture", "Widowmaker",
    "Winston", "Wrecking Ball", "Zarya", "Zenyatta",
];

fn find_hero(lines: &[&str]) -> Option<String> {
    let text = lines.join(" ").to_lowercase();
    for &hero in HEROES {
        if text.contains(&hero.to_lowercase()) {
            return Some(hero.to_string());
        }
    }
    None
}

fn guess_role(hero: &str) -> String {
    match hero {
        "D.Va" | "Doomfist" | "Junker Queen" | "Mauga" | "Orisa" | "Ramattra"
        | "Reinhardt" | "Roadhog" | "Sigma" | "Winston" | "Wrecking Ball" | "Zarya"
        | "Hazard" => "Tank".to_string(),
        "Ana" | "Baptiste" | "Brigitte" | "Illari" | "Juno" | "Kiriko"
        | "Lifeweaver" | "Lucio" | "Mercy" | "Moira" | "Zenyatta" => "Support".to_string(),
        _ => "Damage".to_string(),
    }
}
