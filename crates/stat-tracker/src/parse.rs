use crate::storage::PersonalMatch;
use chrono::Utc;
use strsim::normalized_levenshtein;
use surrealdb_types::Datetime as SurrealDatetime;

pub fn parse_scoreboard(
    raw_text: &str,
    outcome: &str,
    player_name: Option<&str>,
    player_row_index: Option<usize>,
) -> Option<PersonalMatch> {
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

    let stats = player_row_index
        .and_then(|idx| find_stat_row_by_index(&lines, idx))
        .or_else(|| {
            player_name
                .and_then(|name| find_player_row(&lines, name))
                .and_then(|r| extract_row_stats(r))
        })
        .or_else(|| find_best_stat_row(&lines));

    let stats = stats?;

    let hero = find_hero(&lines).unwrap_or_else(|| "Unknown".to_string());
    let role = guess_role(&hero);
    let map_name = find_map(&lines).unwrap_or_default();

    Some(PersonalMatch {
        hero,
        map_name,
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

fn find_stat_row_by_index(lines: &[&str], row_index: usize) -> Option<PlayerStats> {
    let stat_rows: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|line| extract_numbers(line).len() >= 6)
        .collect();

    tracing::debug!(
        stat_row_count = stat_rows.len(),
        target_index = row_index,
        "looking up stat row by portrait index"
    );

    stat_rows.get(row_index).and_then(|line| extract_row_stats(line))
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
    let text = lines.join(" ").to_lowercase();

    // --- Pass 1: exact substring matching (original logic) ---
    let mut found: Vec<(&str, usize)> = Vec::new();
    for &hero in HEROES {
        let hero_lower = hero.to_lowercase();
        let count = text.matches(&hero_lower).count();
        if count > 0 {
            found.push((hero, count));
        }
    }

    if found.len() == 1 {
        return Some(found[0].0.to_string());
    }

    if found.len() > 1 {
        let panel_keywords = ["accuracy", "critical", "weapon", "kills"];
        for &(hero, _) in &found {
            let hero_lower = hero.to_lowercase();
            for line in lines {
                let line_lower = line.to_lowercase();
                if line_lower.contains(&hero_lower) {
                    let num_count = line
                        .split(|c: char| !c.is_ascii_digit())
                        .filter(|w| !w.is_empty())
                        .count();
                    if num_count <= 1 {
                        return Some(hero.to_string());
                    }
                }
            }
            for kw in &panel_keywords {
                if text.contains(&format!("{} {}", hero_lower, kw))
                    || text.contains(&format!("{} {}", kw, hero_lower))
                {
                    return Some(hero.to_string());
                }
            }
        }
        found.sort_by_key(|&(_, count)| count);
        return Some(found[0].0.to_string());
    }

    // --- Pass 2: fuzzy matching against individual words ---
    fuzzy_match_hero(&text)
}

const FUZZY_HERO_THRESHOLD: f64 = 0.75;

fn fuzzy_match_hero(text: &str) -> Option<String> {
    let words: Vec<&str> = text.split_whitespace().collect();

    let mut best_hero: Option<&str> = None;
    let mut best_score: f64 = 0.0;

    for &hero in HEROES {
        let hero_lower = hero.to_lowercase();
        let hero_parts: Vec<&str> = hero_lower.split_whitespace().collect();

        if hero_parts.len() == 1 {
            for &word in &words {
                let score = normalized_levenshtein(word, &hero_lower);
                if score > best_score && score >= FUZZY_HERO_THRESHOLD {
                    best_score = score;
                    best_hero = Some(hero);
                }
            }
        } else {
            for window in words.windows(hero_parts.len()) {
                let candidate = window.join(" ");
                let score = normalized_levenshtein(&candidate, &hero_lower);
                if score > best_score && score >= FUZZY_HERO_THRESHOLD {
                    best_score = score;
                    best_hero = Some(hero);
                }
            }
        }
    }

    if let Some(hero) = best_hero {
        tracing::debug!(
            hero = hero,
            score = best_score,
            "fuzzy matched hero name"
        );
    }

    best_hero.map(|h| h.to_string())
}

pub fn guess_role_public(hero: &str) -> String {
    guess_role(hero)
}

fn guess_role(hero: &str) -> String {
    match hero.to_lowercase().as_str() {
        "d.va" | "dva" | "doomfist" | "domina" | "junker queen" | "junker_queen"
        | "mauga" | "orisa" | "ramattra" | "reinhardt" | "roadhog" | "sigma"
        | "winston" | "wrecking ball" | "wrecking_ball" | "zarya"
        | "hazard" => "Tank".to_string(),
        "ana" | "baptiste" | "brigitte" | "illari" | "jetpack cat" | "juno"
        | "kiriko" | "lifeweaver" | "lucio" | "mercy" | "mizuki" | "moira"
        | "wuyang" | "zenyatta" => "Support".to_string(),
        _ => "Damage".to_string(),
    }
}

const MAPS: &[(&str, &str)] = &[
    ("King's Row", "king"),
    ("Circuit Royal", "circuit royal"),
    ("Dorado", "dorado"),
    ("Havana", "havana"),
    ("Junkertown", "junkertown"),
    ("Rialto", "rialto"),
    ("Route 66", "route 66"),
    ("Shambali Monastery", "shambali"),
    ("Watchpoint: Gibraltar", "watchpoint"),
    ("Blizzard World", "blizzard world"),
    ("Eichenwalde", "eichenwalde"),
    ("Hollywood", "hollywood"),
    ("Midtown", "midtown"),
    ("Numbani", "numbani"),
    ("Paraiso", "paraiso"),
    ("Antarctic Peninsula", "antarctic"),
    ("Busan", "busan"),
    ("Ilios", "ilios"),
    ("Lijiang Tower", "lijiang"),
    ("Nepal", "nepal"),
    ("Oasis", "oasis"),
    ("Samoa", "samoa"),
    ("Colosseo", "colosseo"),
    ("Esperanca", "esperanca"),
    ("New Queen Street", "new queen"),
    ("Runasapi", "runasapi"),
    ("New Junk City", "new junk"),
    ("Suravasa", "suravasa"),
    ("Hanaoka", "hanaoka"),
    ("Throne of Anubis", "anubis"),
];

fn find_map(lines: &[&str]) -> Option<String> {
    let text = lines.join(" ").to_lowercase();

    // Pass 1: exact substring match
    for &(display_name, pattern) in MAPS {
        if text.contains(pattern) {
            return Some(display_name.to_string());
        }
    }

    // Pass 2: fuzzy match each word/bigram against map patterns
    fuzzy_match_map(&text)
}

const FUZZY_MAP_THRESHOLD: f64 = 0.75;

fn fuzzy_match_map(text: &str) -> Option<String> {
    let words: Vec<&str> = text.split_whitespace().collect();

    let mut best_map: Option<&str> = None;
    let mut best_score: f64 = 0.0;

    for &(display_name, pattern) in MAPS {
        let pattern_parts: Vec<&str> = pattern.split_whitespace().collect();

        if pattern_parts.len() == 1 {
            for &word in &words {
                let score = normalized_levenshtein(word, pattern);
                if score > best_score && score >= FUZZY_MAP_THRESHOLD {
                    best_score = score;
                    best_map = Some(display_name);
                }
            }
        } else {
            for window in words.windows(pattern_parts.len()) {
                let candidate = window.join(" ");
                let score = normalized_levenshtein(&candidate, pattern);
                if score > best_score && score >= FUZZY_MAP_THRESHOLD {
                    best_score = score;
                    best_map = Some(display_name);
                }
            }
        }
    }

    if let Some(map_name) = best_map {
        tracing::debug!(
            map = map_name,
            score = best_score,
            "fuzzy matched map name"
        );
    }

    best_map.map(|m| m.to_string())
}
