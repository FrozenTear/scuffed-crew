use crate::ocr::RowOcrResult;
use crate::storage::PersonalMatch;
use chrono::Utc;
use strsim::normalized_levenshtein;
use surrealdb_types::Datetime as SurrealDatetime;

// Hero list + matching live in scuffed-types (hero-stats W1). Re-export for
// existing parse:: call sites and keep scoreboard find_hero available here.
pub use scuffed_types::{HEROES, canonical_hero, find_hero, match_hero_in_text};

/// Build a match from the column-calibrated per-cell OCR rows.
///
/// This is the preferred path: stats come from individually-cropped, per-column
/// OCR cells (positionally stable, numeric whitelists) rather than scraping
/// numbers out of a full-image text dump. `raw_text` is still the full-image OCR
/// and is used only for hero/map name lookup, which the per-cell pipeline does
/// not read.
///
/// The player's row must be POSITIVELY identified — by `player_row_index`
/// (name match across row cells or the brightness-highlighted row) or by the
/// configured player name appearing in the raw text. There is deliberately no
/// "first plausible row" fallback: it silently recorded a teammate's stats as
/// the player's, which corrupts every aggregate downstream. A dropped capture
/// is recoverable (press Tab again); a wrong row is not. Returns `None` when
/// the player row can't be identified or its cells don't parse.
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
        .or_else(|| {
            let lines: Vec<&str> = raw_text
                .lines()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty())
                .collect();
            player_name
                .and_then(|name| find_player_row(&lines, name))
                .and_then(extract_row_stats)
        })?;

    let lines: Vec<&str> = raw_text
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();

    let hero = find_hero(&lines).unwrap_or_else(|| "Unknown".to_string());
    let role = guess_role(&hero);
    let map_name = find_map(&lines).unwrap_or_default();
    let game_mode = map_mode(&map_name).unwrap_or("").to_string();

    Some(PersonalMatch {
        id: None,
        hero,
        map_name,
        game_mode,
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
        sync_rev: 0,
        session_id: String::new(),
        corrected_hero: None,
        corrected_role: None,
        corrected_map_name: None,
        corrected_outcome: None,
        corrected_elims: None,
        corrected_deaths: None,
        corrected_assists: None,
        corrected_damage: None,
        corrected_healing: None,
        corrected_mitigation: None,
        edited_fields: Vec::new(),
        edited_at: None,
        heroes_played: Vec::new(),
        segment_resolutions: Vec::new(),
    })
}

/// Whether this capture's stats came from the identified player row.
/// A raw-text fallback and an unidentified row do not count toward a
/// stat-reset split and must not replace the reset baseline.
pub fn row_counts(rows: &[RowOcrResult], player_row_index: Option<usize>) -> bool {
    player_row_index
        .and_then(|idx| rows.get(idx))
        .and_then(stats_from_row)
        .is_some()
}

/// Whether the OCR'd rows plausibly come from an actual scoreboard, as opposed
/// to a menu, a replay browser, or an arbitrary desktop frame that happened to
/// be captured (Tab is a global hook). A real scoreboard renders a full team of
/// stat rows (early-game zeros are still clean cells); non-scoreboard frames
/// measured 0-1 rows with ≥4 clean cells (rank screen: 1, menus/in-game: 0).
/// Requiring 3 keeps 3x margin over the worst observed negative while staying
/// far below what any readable scoreboard produces — a false rejection drops a
/// real capture, so this is deliberately a weak gate; the strict per-row
/// validation in `stats_from_row` remains the primary defense.
pub fn looks_like_scoreboard(rows: &[RowOcrResult]) -> bool {
    let plausible_rows = rows
        .iter()
        .filter(|r| {
            r.stats
                .iter()
                .filter(|c| crate::ocr::is_clean_stat(c.value.trim()))
                .count()
                >= 4
        })
        .count();
    plausible_rows >= 3
}

/// True when each team half has at least one row with four clean stat cells.
///
/// `looks_like_scoreboard` only counts rows. A 4-player co-op board can pass
/// it. Row order is team 1, then team 2, `team_size` rows each: the same
/// layout the scoreboard crop uses. An Adlersbrunn read is stored as
/// Eichenwalde only when both halves clear this check.
pub fn both_teams_have_stats(rows: &[RowOcrResult], team_size: usize) -> bool {
    if team_size == 0 {
        return false;
    }
    let plausible = |row: &RowOcrResult| {
        row.stats
            .iter()
            .filter(|cell| crate::ocr::is_clean_stat(cell.value.trim()))
            .count()
            >= 4
    };
    let team1 = rows.iter().take(team_size).any(plausible);
    let team2 = rows.iter().skip(team_size).take(team_size).any(plausible);
    team1 && team2
}

/// PvE Adlersbrunn (Junkenstein) has no enemy team. Do not store it as
/// Eichenwalde. A real Eichenwalde board, or an alias board with both
/// teams, is kept.
pub fn reject_pve_adlersbrunn(alias: bool, map_name: &str, both_teams: bool) -> bool {
    alias && map_name == "Eichenwalde" && !both_teams
}

/// Adlersbrunn is the Junkenstein event map. The both-teams gate excludes
/// the PvE board, which has no enemy team. Literal "Eichenwalde" is not
/// an alias. A one-letter misread counts when it is closer to Adlersbrunn
/// than to Eichenwalde and the same matcher would store Eichenwalde.
pub fn is_adlersbrunn_alias(text: &str) -> bool {
    let folded = normalize_ocr_glyphs(&text.to_lowercase());
    let eichenwalde = normalize_ocr_glyphs("eichenwalde");
    if folded.contains(&eichenwalde) {
        return false;
    }
    let adlersbrunn = normalize_ocr_glyphs("adlersbrunn");
    if folded.contains(&adlersbrunn) {
        return true;
    }
    if map_from_normalized(&folded, true).as_deref() != Some("Eichenwalde") {
        return false;
    }
    best_word_score(&folded, &adlersbrunn) > best_word_score(&folded, &eichenwalde)
}

fn best_word_score(text: &str, pattern: &str) -> f64 {
    text.split_whitespace()
        .map(|word| normalized_levenshtein(word, pattern))
        .fold(0.0_f64, f64::max)
}

/// Deathmatch maps are kept in the local store and never uploaded.
/// Only a trusted map read (top bar, accolade, or an already trusted
/// session) may divert a capture. A fuzzy board read must not.
pub fn map_is_untracked(name: &str) -> bool {
    map_mode(name) == Some("Deathmatch")
}

/// Mode sent for a row. A known map wins, including a manual map correction.
/// An unknown map keeps the mode stored on the row.
pub fn uploaded_game_mode(map_name: &str, game_mode: &str) -> String {
    let from_map = stored_game_mode(map_name);
    if from_map.is_empty() {
        game_mode.to_string()
    } else {
        from_map
    }
}

/// A stats row is uploaded only when neither the map nor the mode is Deathmatch.
pub fn stats_row_is_tracked(map_name: &str, game_mode: &str) -> bool {
    !map_is_untracked(map_name) && !game_mode.eq_ignore_ascii_case("Deathmatch")
}

/// Mode stored on a row, from the canonical map that was actually kept.
pub fn stored_game_mode(canonical: &str) -> String {
    map_mode(canonical).unwrap_or("").to_string()
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
    // OW2 (extreme games top out around 70 elims / 30 deaths). A larger value
    // means a neighboring column or badge digit bled into the cell — observed
    // misreads: 110, 118, 311 slipping past the old 200 cap.
    if stats.elims > 99 || stats.assists > 99 || stats.deaths > 50 {
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

/// The per-column edge-ink suspect mask for the player's row, in the same
/// positional order as the gate counters `[E, A, D, DMG, HLG, MIT]`.
///
/// This is the CG-3 signal the capture gate needs alongside the parsed counters:
/// a `true` entry means that cell's glyph ink touched a crop edge (clip/bleed),
/// so the gate must strip its corroboration/un-latch influence. The mask is read
/// from the SAME per-cell OCR row that `parse_scoreboard_cells` reads its stats
/// from (`stats_from_row`). When the player row was not positively identified —
/// or the stats came from the raw-text fallback, which carries no per-cell
/// pixels — every column is `false` (no edge-ink evidence either way).
pub fn player_row_suspect_mask(
    rows: &[RowOcrResult],
    player_row_index: Option<usize>,
) -> [bool; 6] {
    let mut mask = [false; 6];
    if let Some(row) = player_row_index.and_then(|i| rows.get(i))
        && row.stats.len() >= 6
    {
        for (col, flag) in mask.iter_mut().enumerate() {
            *flag = row.stats[col].suspect;
        }
    }
    mask
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

/// Canonicalize a map identifier to its display name in the MAPS table —
/// e.g. the map-vote screen's "SHAMBALI" becomes "Shambali Monastery".
/// `None` when nothing matches: an uncanonicalizable name must not be stored,
/// or the same map fractures into several aggregate rows.
pub fn canonical_map(name: &str) -> Option<String> {
    match_map_in_text(name)
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

/// Exact substring match only. The accolade crop uses this so a fuzzy
/// near-miss in the gameplay HUD cannot become the session map.
pub fn exact_map_in_text(text: &str) -> Option<String> {
    // Join wrapped lines first. "New Junk\nCity" is one map name.
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let text = normalize_ocr_glyphs(&joined.to_lowercase());
    map_from_normalized(&text, false)
}

/// Result word printed as the header of the scoreboard region itself, read
/// from the full-board OCR text a Tab capture already paid for. Only the
/// first two non-empty lines count — that is where a header lives; player
/// names and chat sit deeper and must never supply an outcome. Whole-word,
/// case-insensitive (the 2026-08-16 22:19:24Z frame OCR'd as "~ Defeat").
pub fn outcome_from_board_header(raw_text: &str) -> crate::detect::MatchOutcome {
    use crate::detect::MatchOutcome;
    for line in raw_text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .take(2)
    {
        for word in line.split(|c: char| !c.is_ascii_alphabetic()) {
            match word.to_ascii_uppercase().as_str() {
                "VICTORY" => return MatchOutcome::Victory,
                "DEFEAT" => return MatchOutcome::Defeat,
                "DRAW" => return MatchOutcome::Draw,
                _ => {}
            }
        }
    }
    MatchOutcome::Unknown
}

fn guess_role(hero: &str) -> String {
    match hero.to_lowercase().as_str() {
        "d.mon" | "dmon" | "d.va" | "dva" | "doomfist" | "domina" | "junker queen"
        | "junker_queen" | "mauga" | "orisa" | "ramattra" | "reinhardt" | "roadhog" | "sigma"
        | "winston" | "wrecking ball" | "wrecking_ball" | "zarya" | "hazard" => "Tank".to_string(),
        "ana" | "baptiste" | "brigitte" | "doctrine" | "illari" | "jetpack cat" | "juno"
        | "kiriko" | "lifeweaver" | "lucio" | "mercy" | "mizuki" | "moira" | "sombra"
        | "wuyang" | "zenyatta" => "Support".to_string(),
        _ => "Damage".to_string(),
    }
}

const MAPS: &[(&str, &str)] = &[
    // NOT plain "king": that substring matches "wrecKING ball" in scoreboard
    // text and fabricated King's Row reads on every Wrecking Ball game. The
    // fuzzy pass still catches OCR variants like "kings row".
    ("King's Row", "king's row"),
    ("Circuit Royal", "circuit royal"),
    ("Dorado", "dorado"),
    ("Havana", "havana"),
    ("Junkertown", "junkertown"),
    ("Rialto", "rialto"),
    ("Route 66", "route 66"),
    ("Shambali Monastery", "shambali"),
    // Grímsvötn is before Gibraltar. A bare "watchpoint" is not a key:
    // `resolve_watchpoint` decides the family, and an undecided prefix
    // matches neither map.
    ("Watchpoint: Grímsvötn", "grimsvotn"),
    ("Watchpoint: Gibraltar", "gibraltar"),
    ("Blizzard World", "blizzard world"),
    ("Eichenwalde", "eichenwalde"),
    // Halloween label for the same Hybrid map. Junkenstein's Revenge uses
    // the name too; the capture path refuses it unless both teams have stats.
    ("Eichenwalde", "adlersbrunn"),
    ("Hollywood", "hollywood"),
    ("Midtown", "midtown"),
    ("Numbani", "numbani"),
    ("Paraiso", "paraiso"),
    ("Paraiso", "paraíso"),
    ("Neon Junction", "neon junction"),
    ("Antarctic Peninsula", "antarctic"),
    ("Busan", "busan"),
    ("Ilios", "ilios"),
    ("Lijiang Tower", "lijiang"),
    ("Nepal", "nepal"),
    ("Oasis", "oasis"),
    ("Samoa", "samoa"),
    ("Colosseo", "colosseo"),
    ("Esperanca", "esperanca"),
    ("Esperanca", "esperança"),
    ("New Queen Street", "new queen"),
    ("Runasapi", "runasapi"),
    ("New Junk City", "new junk"),
    ("Suravasa", "suravasa"),
    ("Aatlis", "aatlis"),
    ("Hanaoka", "hanaoka"),
    ("Throne of Anubis", "anubis"),
    // Deathmatch only. Kept in the local store. Never uploaded.
    // A trusted read is required before a capture is diverted.
    ("Château Guillard", "guillard"),
];

/// Fold OCR-ambiguous glyphs and Latin diacritics so a mangled map name still
/// matches. `1`, `|`, `l` collapse to `i`; `0` collapses to `o`. Precomposed
/// accents fold to ASCII (`í` to `i`, `ö` to `o`, and the other letters this
/// table already stores). Turkish `İ` (U+0130) and dotless `ı` (U+0131) fold
/// to `i`. Combining marks are dropped. Callers lowercase first; `İ` then
/// arrives as `i` plus a combining dot, which this fold deletes, leaving `i`.
/// The explicit `İ` arm still covers a caller that has not lowercased.
///
/// Applied to BOTH the OCR candidate text and the map patterns (see `find_map`,
/// `fuzzy_match_map`, and the vote reader in `detect::match_start`). The fold is
/// deliberately symmetric — normalizing both sides means it can never corrupt a
/// legit name into a non-match (e.g. `ilios`→`iiios` on both sides still matches,
/// `hollywood`→`hoiiywood` on both sides still matches). Named for the class of
/// misread it fixes: ILIOS (three capital I's) reads as `1LIOS`/`IL10S`/`|LIOS`.
pub(crate) fn normalize_ocr_glyphs(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if ('\u{0300}'..='\u{036f}').contains(&c) {
            continue;
        }
        let c = match c {
            'á' | 'à' | 'ã' | 'â' | 'ä' | 'Á' | 'À' | 'Ã' | 'Â' | 'Ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' | 'É' | 'È' | 'Ê' | 'Ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' | 'Í' | 'Ì' | 'Î' | 'Ï' | 'İ' | 'ı' => 'i',
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' | 'Ó' | 'Ò' | 'Ô' | 'Õ' | 'Ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' | 'Ú' | 'Ù' | 'Û' | 'Ü' => 'u',
            'ç' | 'Ç' => 'c',
            'ñ' | 'Ñ' => 'n',
            'ý' | 'ÿ' | 'Ý' => 'y',
            other => other,
        };
        out.push(match c {
            '1' | '|' | 'l' => 'i',
            '0' => 'o',
            other => other,
        });
    }
    out
}

/// Mode bucket for a canonical map display name from [`MAPS`].
///
/// The tracker stores the display string and fills `game_mode` from this
/// table. It does not consult `scuffed_types::MapName`. A name that is not
/// in the table has no bucket.
pub(crate) fn map_mode(canonical_name: &str) -> Option<&'static str> {
    match canonical_name {
        "Circuit Royal"
        | "Dorado"
        | "Havana"
        | "Junkertown"
        | "Rialto"
        | "Route 66"
        | "Shambali Monastery"
        | "Watchpoint: Gibraltar"
        | "Watchpoint: Grímsvötn" => Some("Escort"),
        "Blizzard World" | "Eichenwalde" | "Hollywood" | "King's Row" | "Midtown"
        | "Neon Junction" | "Numbani" | "Paraiso" => Some("Hybrid"),
        "Antarctic Peninsula"
        | "Busan"
        | "Ilios"
        | "Lijiang Tower"
        | "Nepal"
        | "Oasis"
        | "Samoa" => Some("Control"),
        "Colosseo" | "Esperanca" | "New Queen Street" | "Runasapi" => Some("Push"),
        "Aatlis" | "New Junk City" | "Suravasa" => Some("Flashpoint"),
        "Hanaoka" | "Throne of Anubis" => Some("Clash"),
        "Château Guillard" => Some("Deathmatch"),
        _ => None,
    }
}

/// Fuzzy threshold for a map name of `len` non-space chars. Short names get a
/// touch more slack: a single wrong glyph in a 5-char name (BUSAN→BUSVN) is a
/// 0.80 score, which the strict long-name bar would reject. Kept close to the
/// long-name bar so player names / stat fragments still don't cross it — the
/// King's-Row false-positive that justified the strict bar is a substring trap
/// (guarded by dropping bare "king" from MAPS), not a fuzzy near-miss.
pub(crate) fn map_fuzzy_threshold(len: usize) -> f64 {
    if len <= 6 { 0.80 } else { 0.85 }
}

const GRIMSVOTN_NAME: &str = "Watchpoint: Grímsvötn";
const GIBRALTAR_NAME: &str = "Watchpoint: Gibraltar";

/// What a "watchpoint" token decided. Absent means the text never said it,
/// so the ordinary table still runs. Named is one of the two maps. Undecided
/// means the prefix was there and neither name won: the bare key must not
/// become Gibraltar, and the rest of the table may still match.
enum WatchpointFamily {
    Absent,
    Named(&'static str),
    Undecided,
}

fn map_from_normalized(text: &str, allow_fuzzy: bool) -> Option<String> {
    if let WatchpointFamily::Named(name) = resolve_watchpoint(text) {
        return Some(name.to_string());
    }
    for &(display_name, pattern) in MAPS {
        if text.contains(&normalize_ocr_glyphs(pattern)) {
            return Some(display_name.to_string());
        }
    }
    if allow_fuzzy {
        fuzzy_match_map(text)
    } else {
        None
    }
}

/// The word or two words after "watchpoint", apostrophes removed.
fn watchpoint_tails(words: &[String]) -> Vec<String> {
    let mut tails = Vec::new();
    for (i, word) in words.iter().enumerate() {
        if word != "watchpoint" {
            continue;
        }
        let Some(next) = words.get(i + 1) else {
            continue;
        };
        tails.push(next.clone());
        if let Some(after) = words.get(i + 2) {
            tails.push(format!("{next}{after}"));
        }
    }
    tails
}

fn score_watchpoint_tail(tail: &str) -> (f64, f64) {
    let grim_pattern = "grimsvotn";
    // `l` folds to `i`, so the Gibraltar pattern is "gibraitar", not the
    // spelling "gibraltar". Comparing the unfolded spelling made one-letter
    // misreads miss on the accolade path.
    let gib_pattern = normalize_ocr_glyphs("gibraltar");
    let tail = normalize_ocr_glyphs(tail);
    let folded_six = tail.replace('6', "o");
    let mut grim = normalized_levenshtein(&tail, grim_pattern);
    if folded_six != tail {
        grim = grim.max(normalized_levenshtein(&folded_six, grim_pattern));
    }
    if tail.contains(grim_pattern) || folded_six.contains(grim_pattern) {
        grim = 1.0;
    }
    let gib = if tail.contains(&gib_pattern) || folded_six.contains(&gib_pattern) {
        1.0
    } else {
        normalized_levenshtein(&tail, &gib_pattern)
            .max(normalized_levenshtein(&folded_six, &gib_pattern))
    };
    (grim, gib)
}

/// Bare or ambiguous "watchpoint" is not Gibraltar. A following word is
/// fuzzy-matched against Grímsvötn and Gibraltar; one side has to clear the
/// long-name threshold and beat the other. `6` folds to `o` only here, so
/// Route 66 is left alone. The next two tokens are also joined, so
/// "GRIMS VOTN" can still hit Grímsvötn.
fn resolve_watchpoint(text: &str) -> WatchpointFamily {
    // Fold first so an accented letter stays inside its word. Splitting on
    // every non-ASCII byte used to break "Grímsvötn" into two tokens.
    let text = normalize_ocr_glyphs(&text.to_lowercase());
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter(|word| !word.is_empty())
        .map(|word| word.replace('\'', ""))
        .filter(|word| !word.is_empty())
        .collect();
    let saw_prefix = words.iter().any(|word| word == "watchpoint");
    if !saw_prefix {
        return WatchpointFamily::Absent;
    }
    let stripped = text.replace('\'', "");
    if stripped.contains("grimsvotn") {
        return WatchpointFamily::Named(GRIMSVOTN_NAME);
    }
    let tails = watchpoint_tails(&words);
    if tails.is_empty() {
        return WatchpointFamily::Undecided;
    }
    let mut best_grim: f64 = 0.0;
    let mut best_gib: f64 = 0.0;
    for tail in &tails {
        let (grim, gib) = score_watchpoint_tail(tail);
        best_grim = best_grim.max(grim);
        best_gib = best_gib.max(gib);
    }
    let threshold = map_fuzzy_threshold("grimsvotn".chars().count());
    if best_grim >= threshold && best_grim > best_gib {
        WatchpointFamily::Named(GRIMSVOTN_NAME)
    } else if best_gib >= threshold && best_gib > best_grim {
        WatchpointFamily::Named(GIBRALTAR_NAME)
    } else {
        WatchpointFamily::Undecided
    }
}

fn find_map(lines: &[&str]) -> Option<String> {
    let text = normalize_ocr_glyphs(&lines.join(" ").to_lowercase());
    map_from_normalized(&text, true)
}

fn fuzzy_match_map(text: &str) -> Option<String> {
    // `text` is expected to already be glyph-normalized by the caller.
    let text = normalize_ocr_glyphs(text);
    let words: Vec<&str> = text.split_whitespace().collect();

    let mut best_map: Option<&str> = None;
    let mut best_score: f64 = 0.0;

    for &(display_name, pattern) in MAPS {
        let pattern = normalize_ocr_glyphs(pattern);
        let pattern_parts: Vec<&str> = pattern.split_whitespace().collect();
        let threshold = map_fuzzy_threshold(pattern.chars().filter(|c| !c.is_whitespace()).count());

        if pattern_parts.len() == 1 {
            for &word in &words {
                let score = normalized_levenshtein(word, &pattern);
                if score > best_score && score >= threshold {
                    best_score = score;
                    best_map = Some(display_name);
                }
            }
        } else {
            for window in words.windows(pattern_parts.len()) {
                let candidate = window.join(" ");
                let score = normalized_levenshtein(&candidate, &pattern);
                if score > best_score && score >= threshold {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ocr::{CellOcrResult, RowOcrResult};

    fn cell(value: &str) -> CellOcrResult {
        CellOcrResult {
            value: value.to_string(),
            confidence: 80,
            suspect: false,
        }
    }

    fn row(name: Option<&str>, stats: [&str; 6]) -> RowOcrResult {
        RowOcrResult {
            name: name.map(cell),
            stats: stats.iter().map(|s| cell(s)).collect(),
            mean_confidence: 80,
        }
    }

    fn valid_row(name: &str) -> RowOcrResult {
        row(Some(name), ["5", "3", "2", "4,316", "1,200", "899"])
    }

    fn garbage_row() -> RowOcrResult {
        row(None, ["", "x", "", "", "9o", ""])
    }

    #[test]
    fn exact_map_joins_wrapped_lines() {
        assert_eq!(
            exact_map_in_text("New Junk\nCity").as_deref(),
            Some("New Junk City")
        );
        assert_eq!(exact_map_in_text("BUSAN").as_deref(), Some("Busan"));
        assert!(exact_map_in_text("not a map").is_none());
    }

    #[test]
    fn row_counts_only_for_a_validated_identified_row() {
        let rows = vec![valid_row("TEAMMATE"), valid_row("FROZEN")];
        assert!(row_counts(&rows, Some(1)));
        assert!(
            !row_counts(&rows, None),
            "an unidentified row does not count"
        );
        assert!(
            !row_counts(&rows, Some(9)),
            "a missing row index does not count"
        );
        let garbage = vec![garbage_row()];
        assert!(!row_counts(&garbage, Some(0)));
        let implausible = vec![row(
            Some("FROZEN"),
            ["118", "3", "2", "4,316", "1,200", "899"],
        )];
        assert!(
            !row_counts(&implausible, Some(0)),
            "a row stats_from_row rejects does not count"
        );
    }

    #[test]
    fn identified_player_row_parses() {
        let rows = vec![valid_row("OTHER"), valid_row("FROZEN")];
        let parsed = parse_scoreboard_cells(&rows, Some(1), "", "victory", Some("FROZEN")).unwrap();
        assert_eq!(parsed.elims, 5);
        assert_eq!(parsed.damage, 4316);
        assert_eq!(parsed.outcome, "victory");
    }

    #[test]
    fn unidentified_player_row_records_nothing() {
        // Valid rows exist, but none was positively identified as the player's.
        // The old "first plausible row" fallback recorded a teammate here.
        let rows = vec![valid_row("TEAMMATE"), valid_row("ANOTHER")];
        assert!(parse_scoreboard_cells(&rows, None, "", "victory", None).is_none());
        // A configured name that matches nothing must not change that.
        assert!(
            parse_scoreboard_cells(&rows, None, "no match here", "victory", Some("FROZEN"))
                .is_none()
        );
    }

    #[test]
    fn implausible_kill_columns_reject_the_row() {
        // A digit bleeding into the elims cell ("118" for a real ~18) must not
        // be recorded; the capture is dropped rather than poisoned.
        let rows = vec![row(
            Some("FROZEN"),
            ["118", "3", "2", "4,316", "1,200", "899"],
        )];
        assert!(parse_scoreboard_cells(&rows, Some(0), "", "defeat", Some("FROZEN")).is_none());
    }

    #[test]
    fn raw_text_fallback_is_name_anchored() {
        // No per-cell row index, but the player's line is present in the
        // full-image OCR text → stats come from that line, not an arbitrary one.
        let raw = "SOMEONE 9 9 9 9999 9999 9999\nFROZEN 7 1 3 5,155 1,326 3,316";
        let parsed = parse_scoreboard_cells(&[], None, raw, "defeat", Some("FROZEN")).unwrap();
        assert_eq!(parsed.elims, 7);
        assert_eq!(parsed.mitigation, 3316);
    }

    #[test]
    fn wrecking_ball_text_is_not_kings_row() {
        // "wrecKING ball" used to substring-match the King's Row pattern and
        // fabricate map reads on every Wrecking Ball game.
        assert_eq!(
            match_map_in_text("WRECKING BALL\n31% WEAPON ACCURACY"),
            None
        );
        assert_eq!(
            match_map_in_text("KING'S ROW").as_deref(),
            Some("King's Row")
        );
        // OCR commonly drops the apostrophe — fuzzy pass must still match.
        assert_eq!(
            match_map_in_text("KINGS ROW").as_deref(),
            Some("King's Row")
        );
    }

    #[test]
    fn shion_is_detected() {
        // New S3 Damage hero. Text detection (career panel / scoreboard OCR) is
        // the primary path and needs no portrait reference.
        assert_eq!(match_hero_in_text("SHION").as_deref(), Some("Shion"));
        assert_eq!(guess_role("Shion"), "Damage");
    }

    #[test]
    fn dmon_is_a_tank() {
        // Added 2026-08-18 (WL-5, USER-confirmed role). Career-panel text is
        // the primary path; the portrait reference seeds itself on the first
        // game (see handle_capture's missing-reference auto-collect).
        assert_eq!(match_hero_in_text("D.MON").as_deref(), Some("D.Mon"));
        assert_eq!(guess_role("D.Mon"), "Tank");
        assert_eq!(guess_role("dmon"), "Tank");
        assert_eq!(guess_role("D.Va"), "Tank");
    }

    /// Season 5 (2026-10-06): Doctrine is Support, Sombra moved to Support.
    /// An unknown name still falls through to Damage. Roadhog stays Tank.
    #[test]
    fn season5_roles_and_damage_fallback() {
        assert_eq!(guess_role("Doctrine"), "Support");
        assert_eq!(guess_role("doctrine"), "Support");
        assert_eq!(guess_role("Sombra"), "Support");
        assert_eq!(guess_role("sombra"), "Support");
        assert_eq!(guess_role("Roadhog"), "Tank");
        assert_eq!(guess_role("NotAHero"), "Damage");
        assert_eq!(guess_role(""), "Damage");
    }

    /// Site role lookup and the role stamped on a captured game stay aligned.
    /// A stored game keeps whatever was captured, including pre-Season 5 Sombra
    /// games that were Damage.
    #[test]
    fn role_for_hero_name_matches_guess_role() {
        let pins = [
            ("D.Mon", "Tank"),
            ("d.mon", "Tank"),
            ("dmon", "Tank"),
            ("Jetpack Cat", "Support"),
            ("Doctrine", "Support"),
            ("Sombra", "Support"),
        ];
        for (name, role) in pins {
            assert_eq!(guess_role(name), role, "tracker {name}");
            assert_eq!(
                scuffed_types::role_for_hero_name(name)
                    .map(|r| r.to_string())
                    .as_deref(),
                Some(role),
                "site {name}"
            );
        }
        for name in scuffed_types::HEROES {
            let site = scuffed_types::role_for_hero_name(name)
                .map(|role| role.to_string())
                .unwrap_or_else(|| panic!("{name} has no site role"));
            assert_eq!(site, guess_role(name), "{name}");
        }
    }

    #[test]
    fn recent_maps_are_detected() {
        // Neon Junction (Hybrid, S3) and Aatlis (Flashpoint, S17) — both were
        // missing from the canonical MAPS list and never got stored.
        assert_eq!(
            match_map_in_text("NEON JUNCTION").as_deref(),
            Some("Neon Junction")
        );
        assert_eq!(match_map_in_text("AATLIS").as_deref(), Some("Aatlis"));
    }

    #[test]
    fn accented_and_unaccented_map_names_canonicalize() {
        // Live career/OCR names arrive both with and without Portuguese
        // accents. Canonical store form stays the unaccented MAPS display name.
        assert_eq!(match_map_in_text("Paraíso").as_deref(), Some("Paraiso"));
        assert_eq!(match_map_in_text("Paraiso").as_deref(), Some("Paraiso"));
        assert_eq!(match_map_in_text("Esperança").as_deref(), Some("Esperanca"));
        assert_eq!(match_map_in_text("Esperanca").as_deref(), Some("Esperanca"));
        assert_eq!(
            match_map_in_text("Neon Junction").as_deref(),
            Some("Neon Junction")
        );
        assert_eq!(
            match_map_in_text("NEON JUNCTION").as_deref(),
            Some("Neon Junction")
        );
    }

    #[test]
    fn scoreboard_check_accepts_real_rows() {
        let rows: Vec<RowOcrResult> = (0..10).map(|_| valid_row("X")).collect();
        assert!(looks_like_scoreboard(&rows));
        // Early-game all-zero rows are still clean cells.
        let zeros: Vec<RowOcrResult> = (0..10)
            .map(|_| row(Some("X"), ["0", "0", "0", "0", "0", "0"]))
            .collect();
        assert!(looks_like_scoreboard(&zeros));
        // A poorly-OCR'd but real scoreboard: only 3 of 10 rows readable.
        let mut sparse: Vec<RowOcrResult> = (0..3).map(|_| valid_row("X")).collect();
        sparse.extend((0..7).map(|_| garbage_row()));
        assert!(looks_like_scoreboard(&sparse));
    }

    #[test]
    fn scoreboard_check_rejects_garbage_frames() {
        assert!(!looks_like_scoreboard(&[]));
        let rows: Vec<RowOcrResult> = (0..10).map(|_| garbage_row()).collect();
        assert!(!looks_like_scoreboard(&rows));
        // 1-2 valid-looking rows among garbage (e.g. the rank screen flukes
        // digit cells, a desktop frame with a number column) is not enough.
        let mut mixed: Vec<RowOcrResult> = (0..2).map(|_| valid_row("X")).collect();
        mixed.extend((0..8).map(|_| garbage_row()));
        assert!(!looks_like_scoreboard(&mixed));
    }

    #[test]
    fn text_map_sets_game_mode_on_the_parsed_row() {
        let rows = vec![valid_row("FROZEN")];
        let parsed = parse_scoreboard_cells(
            &rows,
            Some(0),
            "DORADO\nFROZEN 5 3 2 4,316 1,200 899",
            "victory",
            Some("FROZEN"),
        )
        .unwrap();
        assert_eq!(parsed.map_name, "Dorado");
        assert_eq!(parsed.game_mode, "Escort");
        assert_eq!(stored_game_mode("Busan"), "Control");
        assert_eq!(stored_game_mode("Château Guillard"), "Deathmatch");
    }

    #[test]
    fn both_teams_need_a_stat_row_before_an_alias_is_trusted() {
        let team: Vec<_> = (0..10).map(|_| valid_row("X")).collect();
        assert!(both_teams_have_stats(&team, 5));
        let one_team: Vec<_> = (0..4).map(|_| valid_row("X")).collect();
        assert!(
            !both_teams_have_stats(&one_team, 5),
            "a 4-player co-op board has no enemy half"
        );
        let mut enemy_blank: Vec<_> = team.into_iter().take(5).collect();
        enemy_blank.extend((0..5).map(|_| garbage_row()));
        assert!(!both_teams_have_stats(&enemy_blank, 5));
    }
}

#[cfg(test)]
mod hero_map_name_tests {
    use super::*;

    #[test]
    fn short_hero_names_need_word_boundaries() {
        // "ana" ⊂ "havana": a bare map label must not read as the hero Ana.
        assert_eq!(match_hero_in_text("HAVANA"), None);
        assert_eq!(match_hero_in_text("Hanaoka"), None);
        // Standalone the name still matches, with or without punctuation.
        assert_eq!(match_hero_in_text("Ana").as_deref(), Some("Ana"));
        assert_eq!(match_hero_in_text("ana: 14 elims").as_deref(), Some("Ana"));
    }

    #[test]
    fn hero_ties_break_to_most_mentioned() {
        // A support duo on the scoreboard: the player's hero recurs across
        // stat lines — most-mentioned must win (this sorted ascending for a
        // while, so the LEAST-mentioned hero won every multi-match).
        let text = "HAVANA\nana 14 8 2 3400\nmercy 30 2 11 8000\nmercy 1 2 3 4\nmercy 5 6 7 8";
        assert_eq!(match_hero_in_text(text).as_deref(), Some("Mercy"));
    }

    #[test]
    fn map_vote_names_canonicalize_to_display_names() {
        assert_eq!(
            canonical_map("SHAMBALI").as_deref(),
            Some("Shambali Monastery")
        );
        assert_eq!(
            canonical_map("WATCHPOINT").as_deref(),
            None,
            "a bare watchpoint is not Gibraltar"
        );
        assert_eq!(
            canonical_map("GIBRALTAR").as_deref(),
            Some("Watchpoint: Gibraltar")
        );
        assert_eq!(canonical_map("ROUTE 66").as_deref(), Some("Route 66"));
        assert_eq!(
            canonical_map("NEON JUNCTION").as_deref(),
            Some("Neon Junction")
        );
        assert_eq!(canonical_map("garbage read"), None);
    }

    const GRIMSVOTN: &str = "Watchpoint: Grímsvötn";

    #[test]
    fn grimsvotn_canonical_name_is_byte_exact() {
        // Same precomposed display string and Escort bucket as
        // `scuffed_types::MapName::WatchpointGrimsvotn`.
        let shared = scuffed_types::MapName::WatchpointGrimsvotn.display_name();
        assert_eq!(shared, GRIMSVOTN);
        assert!(
            !shared
                .chars()
                .any(|c| ('\u{0300}'..='\u{036F}').contains(&c)),
            "display name must be precomposed"
        );
        let name = canonical_map(GRIMSVOTN).expect("canonical name");
        assert_eq!(name.as_bytes(), shared.as_bytes());
        assert_eq!(
            map_mode(&name),
            Some(scuffed_types::MapName::game_mode_label(shared))
        );
        assert_eq!(map_mode(&name), Some("Escort"));
        assert_eq!(map_mode("Watchpoint: Gibraltar"), Some("Escort"));
        for &(display, _) in MAPS {
            assert!(map_mode(display).is_some(), "{display} has no mode bucket");
        }
    }

    #[test]
    fn bare_grimsvotn_resolves_with_or_without_accents() {
        for raw in [
            "grimsvotn",
            "GRIMSVOTN",
            "Grimsvotn",
            "GRIMSVÖTN",
            "GRÍMSVÖTN",
        ] {
            let name = canonical_map(raw).unwrap_or_else(|| panic!("{raw} did not match"));
            assert_eq!(name.as_bytes(), GRIMSVOTN.as_bytes(), "{raw}");
        }
    }

    #[test]
    fn prefixed_grimsvotn_wins_over_watchpoint() {
        // The prefix is also the Gibraltar key. Grímsvötn's own key, or an
        // OCR variant of it, has to win wherever it appears in the text.
        for raw in [
            "WATCHPOINT: GRIMSVOTN",
            "WATCHPOINT GRIMSV0TN",
            "WATCHPOINT: GRÍMSVÖTN",
            "watchpoint grimsvotn",
            "Watchpoint: Grímsvötn",
            "GRLMSVOTN",
            "GR1MSVOTN",
        ] {
            let name = match_map_in_text(raw).unwrap_or_else(|| panic!("{raw} did not match"));
            assert_eq!(name.as_bytes(), GRIMSVOTN.as_bytes(), "{raw}");
        }
    }

    #[test]
    fn watchpoint_maps_do_not_collide() {
        let gibraltar = [
            "Watchpoint: Gibraltar",
            "WATCHPOINT: GIBRALTAR",
            "GIBRALTAR",
            "gibraltar",
        ];
        let grimsvotn = [
            "grimsvotn",
            "GRIMSVÖTN",
            "Watchpoint: Grímsvötn",
            "WATCHPOINT: GRIMSVOTN",
            "WATCHPOINT GRIMSV0TN",
            "GRLMSVOTN",
            "GR1MSVOTN",
            "GRÍMSVÖTN",
        ];
        for raw in gibraltar {
            assert_eq!(
                match_map_in_text(raw).as_deref(),
                Some("Watchpoint: Gibraltar"),
                "{raw} must not become Grímsvötn"
            );
        }
        for raw in grimsvotn {
            assert_eq!(
                match_map_in_text(raw).as_deref(),
                Some(GRIMSVOTN),
                "{raw} must not become Gibraltar"
            );
        }
        for raw in ["WATCHPOINT", "watchpoint", "WATCHPOINT:"] {
            assert_eq!(
                match_map_in_text(raw),
                None,
                "{raw} must not become Gibraltar"
            );
            assert_eq!(exact_map_in_text(raw), None, "{raw}");
        }
    }

    #[test]
    fn garbled_grimsvotn_after_watchpoint_is_not_gibraltar() {
        // eng LSTM has no í/ö. A misread second word used to hit the bare
        // watchpoint key and become a trusted Gibraltar.
        for raw in [
            "WATCHPOINT: GRIMSV6TN",
            "WATCHPOINT: GRIMSVOT",
            "WATCHPOINT: GRIMSVQTN",
            "WATCHPOINT: GRMSVOTN",
            "WATCHPOINT: GR'IMSVOTN",
            "WATCHPOINT: GRIMS VOTN",
            "WATCHPOINT: GRIMSV6T",
        ] {
            assert_eq!(
                exact_map_in_text(raw).as_deref(),
                Some(GRIMSVOTN),
                "accolade {raw}"
            );
            assert_eq!(
                match_map_in_text(raw).as_deref(),
                Some(GRIMSVOTN),
                "top bar {raw}"
            );
        }
    }

    #[test]
    fn dotted_turkish_i_in_grimsvotn_canonicalizes() {
        // U+0130 lowercases to i + a combining dot. U+0131 is dotless i.
        assert_eq!(canonical_map("GRİMSVÖTN").as_deref(), Some(GRIMSVOTN));
        assert_eq!(
            canonical_map("GR\u{0131}MSVÖTN").as_deref(),
            Some(GRIMSVOTN)
        );
        assert_eq!(
            canonical_map("Gri\u{301}msvo\u{308}tn").as_deref(),
            Some(GRIMSVOTN)
        );
        // Dotless ı survives lowercasing, so the accolade path pins that arm.
        assert_eq!(
            exact_map_in_text("GR\u{0131}MSVÖTN").as_deref(),
            Some(GRIMSVOTN)
        );
        // İ is already i plus a combining dot after lowercasing. Call the
        // fold directly so dropping the İ arm fails this test.
        assert_eq!(normalize_ocr_glyphs("İ"), "i");
        assert_eq!(normalize_ocr_glyphs("ı"), "i");
    }

    #[test]
    fn one_letter_gibraltar_misread_stays_gibraltar() {
        for raw in [
            "WATCHPOINT: GIBRALTAP",
            "WATCHPOINT: GIBRALTA",
            "WATCHPOINT: CIBRALTAR",
        ] {
            assert_eq!(
                exact_map_in_text(raw).as_deref(),
                Some(GIBRALTAR_NAME),
                "accolade {raw}"
            );
            assert_eq!(
                match_map_in_text(raw).as_deref(),
                Some(GIBRALTAR_NAME),
                "top bar {raw}"
            );
        }
    }

    #[test]
    fn adlersbrunn_and_halloween_prefixes_canonicalize() {
        for raw in ["ADLERSBRUNN", "Adlersbrunn", "adiersbrunn", "ADLERSBRUNN "] {
            assert_eq!(
                canonical_map(raw.trim()).as_deref(),
                Some("Eichenwalde"),
                "{raw}"
            );
        }
        assert_eq!(
            match_map_in_text("HALLOWEEN HOLLYWOOD").as_deref(),
            Some("Hollywood")
        );
        assert_eq!(
            match_map_in_text("HALLOWEEN LIJIANG TOWER").as_deref(),
            Some("Lijiang Tower")
        );
        assert_eq!(canonical_map("EICHENWALDE").as_deref(), Some("Eichenwalde"));
        assert_eq!(canonical_map("KING'S ROW").as_deref(), Some("King's Row"));
        assert!(!is_adlersbrunn_alias("Eichenwalde"));
        assert!(is_adlersbrunn_alias("Adlersbrunn"));
        assert!(is_adlersbrunn_alias("adiersbrunn"));
        assert!(is_adlersbrunn_alias("ADLERSBRUNM"));
        assert!(is_adlersbrunn_alias("ADLER5BRUNN"));
        assert!(!is_adlersbrunn_alias("EICHENWALD"));
        assert!(reject_pve_adlersbrunn(true, "Eichenwalde", false));
        assert!(!reject_pve_adlersbrunn(true, "Eichenwalde", true));
        assert!(!reject_pve_adlersbrunn(false, "Eichenwalde", false));
        assert!(!reject_pve_adlersbrunn(true, "Busan", false));
    }

    #[test]
    fn guillard_ocr_near_misses_still_canonicalize() {
        // koverwatch on a rendered "Chateau Guillard": a line crop is exact.
        // A tall canvas reads Guiltard, and a 43px-tall crop reads Guitlard.
        assert_eq!(
            match_map_in_text("Chateau Guillard").as_deref(),
            Some("Château Guillard")
        );
        assert_eq!(
            match_map_in_text("Chateau Guiltard").as_deref(),
            Some("Château Guillard")
        );
        assert_eq!(
            match_map_in_text("Chateau Guitlard").as_deref(),
            Some("Château Guillard")
        );
        assert_eq!(match_map_in_text("Chateau Guiltarod").as_deref(), None);
        assert_eq!(
            match_map_in_text("AdLErSBRuNN").as_deref(),
            Some("Eichenwalde")
        );
        assert_eq!(
            match_map_in_text("Grimsvotn").as_deref(),
            Some("Watchpoint: Grímsvötn")
        );
    }

    #[test]
    fn chateau_guillard_is_deathmatch_and_untracked() {
        for raw in ["GUILLARD", "Château Guillard", "Chateau Guillard"] {
            let name = canonical_map(raw).unwrap_or_else(|| panic!("{raw}"));
            assert_eq!(name, "Château Guillard");
            assert_eq!(map_mode(&name), Some("Deathmatch"));
            assert!(map_is_untracked(&name));
            assert!(!stats_row_is_tracked(&name, "Deathmatch"));
        }
        assert!(stats_row_is_tracked("Busan", "Control"));
        assert!(!stats_row_is_tracked("Busan", "Deathmatch"));
    }

    #[test]
    fn glyph_mangled_map_names_resolve() {
        // ILIOS is three capital I's — the most OCR-mangled name in the pool.
        // Field 07-17: an Ilios game registered no map on any reader. Each of
        // these fed the old exact `contains` path an empty result.
        // (Mutation check: with FUZZY_MAP_THRESHOLD reverted to 0.85 AND the
        // glyph fold removed, every assertion in this test fails.)
        assert_eq!(match_map_in_text("1LIOS").as_deref(), Some("Ilios"));
        assert_eq!(match_map_in_text("IL10S").as_deref(), Some("Ilios"));
        assert_eq!(match_map_in_text("|LIOS").as_deref(), Some("Ilios"));
        // Oasis with a zero-for-O; Route 66 with a zero-for-O.
        assert_eq!(match_map_in_text("0ASIS").as_deref(), Some("Oasis"));
        assert_eq!(match_map_in_text("R0UTE 66").as_deref(), Some("Route 66"));
        // Busan with a V-for-A (not a glyph fold — needs the looser short-name
        // fuzzy threshold: BUSVN↔BUSAN scores 0.80).
        assert_eq!(match_map_in_text("BUSVN").as_deref(), Some("Busan"));
    }

    #[test]
    fn player_names_and_hero_names_do_not_false_positive_as_maps() {
        // The map matcher runs over full-board OCR text too, so hero names,
        // gamer tags and stat fragments must not resolve to a map even with the
        // looser short-name threshold.
        assert_eq!(match_map_in_text("REAPER"), None);
        assert_eq!(match_map_in_text("SOMBRA"), None);
        assert_eq!(match_map_in_text("DOCTRINE"), None);
        assert_eq!(match_map_in_text("WIDOWMAKER"), None);
        assert_eq!(match_map_in_text("xXGamerTagXx"), None);
        assert_eq!(match_map_in_text("FR0ZEN"), None);
        assert_eq!(match_map_in_text("31% WEAPON ACCURACY"), None);
    }

    #[test]
    fn wrecking_ball_guard_survives_glyph_fold() {
        // The King's-Row false positive that justified the strict threshold:
        // "wrecKING ball" must still NOT read as King's Row after the glyph
        // fold (which turns "ball"→"baii" but leaves the guard intact).
        assert_eq!(
            match_map_in_text("WRECKING BALL\n31% WEAPON ACCURACY"),
            None
        );
        // And the genuine map still resolves, apostrophe dropped or not.
        assert_eq!(
            match_map_in_text("KING'S ROW").as_deref(),
            Some("King's Row")
        );
        assert_eq!(
            match_map_in_text("KINGS ROW").as_deref(),
            Some("King's Row")
        );
    }
}

#[cfg(test)]
mod board_header_tests {
    use super::outcome_from_board_header;
    use crate::detect::MatchOutcome;

    #[test]
    fn reads_result_word_from_header_lines() {
        assert_eq!(
            outcome_from_board_header("~ Defeat\n\u{2014} J | J |"),
            MatchOutcome::Defeat
        );
        assert_eq!(
            outcome_from_board_header("\n  VICTORY   RIALTO\nE A D DMG"),
            MatchOutcome::Victory
        );
        assert_eq!(outcome_from_board_header("Draw"), MatchOutcome::Draw);
    }

    #[test]
    fn ignores_words_deeper_than_the_header_and_substrings() {
        // Chat / names below the header never decide a game.
        assert_eq!(
            outcome_from_board_header("E A D DMG\nFROZEN 12 3 4\nSOOT: ez victory"),
            MatchOutcome::Unknown
        );
        // Whole word only: "VICTORYLANE" is a name, not a result.
        assert_eq!(
            outcome_from_board_header("VICTORYLANE 3 4"),
            MatchOutcome::Unknown
        );
        assert_eq!(outcome_from_board_header(""), MatchOutcome::Unknown);
    }
}
