use crate::ocr::RowOcrResult;
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
                .and_then(extract_row_stats)
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

/// Build a match from the column-calibrated per-cell OCR rows.
///
/// This is the preferred path: stats come from individually-cropped, per-column
/// OCR cells (positionally stable, numeric whitelists) rather than scraping
/// numbers out of a full-image text dump. `raw_text` is still the full-image OCR
/// and is used only for hero/map name lookup, which the per-cell pipeline does
/// not read. Returns `None` if no row yields plausible stats.
pub fn parse_scoreboard_cells(
    rows: &[RowOcrResult],
    player_row_index: Option<usize>,
    raw_text: &str,
    outcome: &str,
    player_name: Option<&str>,
) -> Option<PersonalMatch> {
    let stats = player_row_index
        .and_then(|idx| rows.get(idx))
        .and_then(stats_from_row)
        // Portrait row detection missed — fall back to the first row whose cells
        // pass validation, then to scraping the full-image text (by player name
        // if we have one, else the first plausible stat line).
        .or_else(|| rows.iter().find_map(stats_from_row))
        .or_else(|| {
            let lines: Vec<&str> = raw_text
                .lines()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty())
                .collect();
            player_name
                .and_then(|name| find_player_row(&lines, name))
                .and_then(extract_row_stats)
                .or_else(|| find_best_stat_row(&lines))
        })?;

    let lines: Vec<&str> = raw_text
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();

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

/// Extract the six stats from one OCR'd row. Columns are positional:
/// 0=Elims, 1=Assists, 2=Deaths, 3=Damage, 4=Healing, 5=Mitigation.
/// Returns `None` if any cell is unreadable or the narrow E/A/D columns hold
/// implausibly large values (a sign columns are misaligned and a damage figure
/// has leaked into a kill column).
fn stats_from_row(row: &RowOcrResult) -> Option<PlayerStats> {
    if row.stats.len() < 6 {
        return None;
    }
    let n: Vec<u32> = row
        .stats
        .iter()
        .take(6)
        .map(|c| parse_cell_number(&c.value))
        .collect::<Option<Vec<_>>>()?;

    let stats = PlayerStats {
        elims: n[0],
        assists: n[1],
        deaths: n[2],
        damage: n[3],
        healing: n[4],
        mitigation: n[5],
    };

    // Sanity gate: eliminations/assists/deaths are small two-digit figures in
    // OW2. A large value here means the wider DMG/HLG/MIT columns bled left.
    if stats.elims > 200 || stats.assists > 200 || stats.deaths > 200 {
        tracing::debug!(
            elims = stats.elims,
            assists = stats.assists,
            deaths = stats.deaths,
            "rejecting row: kill-column value out of plausible range"
        );
        return None;
    }

    Some(stats)
}

fn parse_cell_number(s: &str) -> Option<u32> {
    let cleaned: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
    if cleaned.is_empty() {
        None
    } else {
        cleaned.parse::<u32>().ok()
    }
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
    lines
        .iter()
        .find(|line| {
            let lower = line.to_lowercase();
            lower.contains(&name_lower)
        })
        .copied()
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

    stat_rows
        .get(row_index)
        .and_then(|line| extract_row_stats(line))
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
    "Ana",
    "Anran",
    "Ashe",
    "Baptiste",
    "Bastion",
    "Brigitte",
    "Cassidy",
    "D.Va",
    "Domina",
    "Doomfist",
    "Echo",
    "Emre",
    "Freja",
    "Genji",
    "Hanzo",
    "Hazard",
    "Illari",
    "Jetpack Cat",
    "Junker Queen",
    "Junkrat",
    "Juno",
    "Kiriko",
    "Lifeweaver",
    "Lucio",
    "Mauga",
    "Mei",
    "Mercy",
    "Mizuki",
    "Moira",
    "Orisa",
    "Pharah",
    "Ramattra",
    "Reaper",
    "Reinhardt",
    "Roadhog",
    "Sierra",
    "Sigma",
    "Sojourn",
    "Soldier: 76",
    "Sombra",
    "Symmetra",
    "Torbjorn",
    "Tracer",
    "Vendetta",
    "Venture",
    "Widowmaker",
    "Winston",
    "Wrecking Ball",
    "Wuyang",
    "Zarya",
    "Zenyatta",
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
        tracing::debug!(hero = hero, score = best_score, "fuzzy matched hero name");
    }

    best_hero.map(|h| h.to_string())
}

pub fn guess_role_public(hero: &str) -> String {
    guess_role(hero)
}

/// Find which row (across all rows, both teams) best matches the configured
/// player name. Returns the row index and match score.
///
/// Used for replay and post-match screens where the player may be on team 2,
/// so brightness-based team-1 scanning can't find them. The name cells from
/// `recognize_row` are noisy OCR, so we use a generous fuzzy threshold (0.55)
/// and pick the best score across all rows.
pub fn find_player_row_by_name(rows: &[RowOcrResult], player_name: &str) -> Option<usize> {
    let name_lower = player_name.to_lowercase();

    let mut best_row: Option<usize> = None;
    let mut best_score = 0.0f64;

    for (i, row) in rows.iter().enumerate() {
        let cell_text = match &row.name {
            Some(c) if !c.value.is_empty() => c.value.to_lowercase(),
            _ => continue,
        };

        // Try substring match first (handles "FROZEN" inside "L7 mRoE FROZEN")
        if cell_text.contains(&name_lower) {
            tracing::debug!(row = i, text = %cell_text, "player name found via substring in row");
            return Some(i);
        }

        // Fuzzy: slide a window the length of the player name over the cell text
        let name_chars: Vec<char> = name_lower.chars().collect();
        let cell_chars: Vec<char> = cell_text.chars().collect();
        let window = name_chars.len();
        if window == 0 || window > cell_chars.len() + 4 {
            continue;
        }
        // Also compare whole cell text against the name
        let score_whole = normalized_levenshtein(&cell_text, &name_lower);
        let score_window = if cell_chars.len() >= window {
            (0..=(cell_chars.len().saturating_sub(window)))
                .map(|s| {
                    let slice: String = cell_chars[s..s + window].iter().collect();
                    normalized_levenshtein(&slice, &name_lower)
                })
                .fold(0.0f64, f64::max)
        } else {
            0.0
        };
        let score = score_whole.max(score_window);

        if score > best_score {
            best_score = score;
            best_row = Some(i);
        }
    }

    if best_score >= 0.55 {
        tracing::debug!(row = ?best_row, score = best_score, "player name fuzzy-matched in row");
        best_row
    } else {
        None
    }
}

/// Match a hero name from arbitrary OCR text (e.g. the career-panel title).
pub fn match_hero_in_text(text: &str) -> Option<String> {
    let lines: Vec<&str> = text
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();
    find_hero(&lines)
}

/// Match a map name from arbitrary OCR text (e.g. the top-bar map label).
pub fn match_map_in_text(text: &str) -> Option<String> {
    let lines: Vec<&str> = text
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();
    find_map(&lines)
}

fn guess_role(hero: &str) -> String {
    match hero.to_lowercase().as_str() {
        "d.va" | "dva" | "doomfist" | "domina" | "junker queen" | "junker_queen" | "mauga"
        | "orisa" | "ramattra" | "reinhardt" | "roadhog" | "sigma" | "winston"
        | "wrecking ball" | "wrecking_ball" | "zarya" | "hazard" => "Tank".to_string(),
        "ana" | "baptiste" | "brigitte" | "illari" | "jetpack cat" | "juno" | "kiriko"
        | "lifeweaver" | "lucio" | "mercy" | "mizuki" | "moira" | "wuyang" | "zenyatta" => {
            "Support".to_string()
        }
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

// Higher than the hero threshold: the map label is only OCR'd from a dedicated
// region, and a loose threshold here was false-matching player names / stat
// fragments to maps (e.g. "King's Row").
const FUZZY_MAP_THRESHOLD: f64 = 0.85;

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
        tracing::debug!(map = map_name, score = best_score, "fuzzy matched map name");
    }

    best_map.map(|m| m.to_string())
}
