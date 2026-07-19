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

    Some(PersonalMatch {
        id: None,
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
    ("Watchpoint: Gibraltar", "watchpoint"),
    ("Blizzard World", "blizzard world"),
    ("Eichenwalde", "eichenwalde"),
    ("Hollywood", "hollywood"),
    ("Midtown", "midtown"),
    ("Numbani", "numbani"),
    ("Paraiso", "paraiso"),
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
    ("New Queen Street", "new queen"),
    ("Runasapi", "runasapi"),
    ("New Junk City", "new junk"),
    ("Suravasa", "suravasa"),
    ("Aatlis", "aatlis"),
    ("Hanaoka", "hanaoka"),
    ("Throne of Anubis", "anubis"),
];

/// Fold OCR-ambiguous glyphs to one canonical letter each so a mangled map
/// name still matches. `1`, `|`, `l` all collapse to `i`; `0` collapses to `o`.
/// Operates on already-lowercased text.
///
/// Applied to BOTH the OCR candidate text and the map patterns (see `find_map`,
/// `fuzzy_match_map`, and the vote reader in `detect::match_start`). The fold is
/// deliberately symmetric — normalizing both sides means it can never corrupt a
/// legit name into a non-match (e.g. `ilios`→`iiios` on both sides still matches,
/// `hollywood`→`hoiiywood` on both sides still matches). Named for the class of
/// misread it fixes: ILIOS (three capital I's) reads as `1LIOS`/`IL10S`/`|LIOS`.
pub(crate) fn normalize_ocr_glyphs(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '1' | '|' | 'l' => 'i',
            '0' => 'o',
            other => other,
        })
        .collect()
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

fn find_map(lines: &[&str]) -> Option<String> {
    let text = normalize_ocr_glyphs(&lines.join(" ").to_lowercase());

    // Pass 1: exact substring match (patterns glyph-normalized to match).
    for &(display_name, pattern) in MAPS {
        if text.contains(&normalize_ocr_glyphs(pattern)) {
            return Some(display_name.to_string());
        }
    }

    // Pass 2: fuzzy match each word/bigram against map patterns
    fuzzy_match_map(&text)
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
            Some("Watchpoint: Gibraltar")
        );
        assert_eq!(canonical_map("ROUTE 66").as_deref(), Some("Route 66"));
        assert_eq!(
            canonical_map("NEON JUNCTION").as_deref(),
            Some("Neon Junction")
        );
        assert_eq!(canonical_map("garbage read"), None);
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
