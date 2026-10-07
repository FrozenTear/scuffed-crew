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
    read_scoreboard(rows, player_row_index, raw_text, outcome, player_name)
        .ok()
        .map(|read| read.matched)
}

/// Why [`read_scoreboard`] refused a frame. The two cases used to share one
/// log line ("player row not identified"), so an early-game row whose dim
/// zeros came back empty looked the same as a frame with no row at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScoreboardMiss {
    /// No highlighted row and the configured name is not in the raw text.
    PlayerRowNotFound,
    /// The row was found, but its cells did not parse and the text fallback
    /// did not yield six in-range stats.
    CellsUnreadable,
}

/// One accepted scoreboard read.
#[derive(Debug)]
pub struct ScoreboardRead {
    pub matched: PersonalMatch,
    /// `false` when the six stats came from the raw-text fallback. The
    /// capture gate treats that latch as low-trust.
    pub trusted_cells: bool,
}

/// Same inputs as [`parse_scoreboard_cells`], plus which path produced the
/// stats and why a refusal happened.
pub fn read_scoreboard(
    rows: &[RowOcrResult],
    player_row_index: Option<usize>,
    raw_text: &str,
    outcome: &str,
    player_name: Option<&str>,
) -> Result<ScoreboardRead, ScoreboardMiss> {
    let from_cells = player_row_index
        .and_then(|idx| rows.get(idx))
        .and_then(stats_from_row);
    let (stats, trusted_cells) = if let Some(stats) = from_cells {
        (stats, true)
    } else if let Some(stats) = text_fallback_stats(raw_text, player_name) {
        (stats, false)
    } else if player_row_index.is_some() || name_in_raw_text(raw_text, player_name) {
        return Err(ScoreboardMiss::CellsUnreadable);
    } else {
        return Err(ScoreboardMiss::PlayerRowNotFound);
    };

    let lines: Vec<&str> = raw_text
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();

    let hero = find_hero(&lines).unwrap_or_else(|| "Unknown".to_string());
    let role = guess_role(&hero);
    let map_name = find_map(&lines).unwrap_or_default();
    let game_mode = map_mode(&map_name).unwrap_or("").to_string();

    Ok(ScoreboardRead {
        trusted_cells,
        matched: PersonalMatch {
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
        },
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
    // misreads: 110, 118, 311 slipping past the old 200 cap. The text fallback
    // and the capture gate use the same ceilings.
    if kill_columns_implausible(stats.elims, stats.assists, stats.deaths) {
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

/// E/A above 99 or D above 50 is a column bleed, not a real scoreboard.
/// Shared with the text fallback and the capture gate's first-capture check.
pub(crate) fn kill_columns_implausible(elims: u32, assists: u32, deaths: u32) -> bool {
    elims > 99 || assists > 99 || deaths > 50
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

fn name_in_raw_text(raw_text: &str, player_name: Option<&str>) -> bool {
    let Some(name) = player_name else {
        return false;
    };
    let name_lower = name.to_lowercase();
    raw_text.to_lowercase().contains(&name_lower)
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

/// Stats from the full-board text line that contains the player name.
///
/// The per-cell path is positional. This one is not, and the line for row 0
/// also picks up the hero panel's objective timer (`00:02`), which sits at
/// the same height. Taking the last six numbers then slid the columns:
/// `2 0 0 1,105 259 450 00:02` became elims 0, assists 1105, deaths 259.
/// Rank badges sit *before* the name, so the numbers after the name are the
/// stats. A clock token is removed first. Anything other than exactly six
/// numbers after that is contamination (a dropped digit, a timer the clock
/// strip missed) and the fallback is refused — a dropped capture is
/// recoverable, a shifted row is not. The six still have to pass the same
/// kill-column ceilings as [`stats_from_row`].
fn text_fallback_stats(raw_text: &str, player_name: Option<&str>) -> Option<PlayerStats> {
    let name = player_name?;
    let lines: Vec<&str> = raw_text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let line = find_player_row(&lines, name)?;
    let suffix = suffix_after_name(line, name)?;
    let suffix = strip_clock_tokens(suffix);
    let numbers = extract_numbers(&suffix);
    if numbers.len() != 6 {
        tracing::debug!(
            n = numbers.len(),
            "rejecting text fallback: not exactly six stats after the player name"
        );
        return None;
    }
    let stats = PlayerStats {
        elims: numbers[0],
        assists: numbers[1],
        deaths: numbers[2],
        damage: numbers[3],
        healing: numbers[4],
        mitigation: numbers[5],
    };
    if kill_columns_implausible(stats.elims, stats.assists, stats.deaths) {
        tracing::debug!(
            elims = stats.elims,
            assists = stats.assists,
            deaths = stats.deaths,
            "rejecting text fallback: kill-column value out of plausible range"
        );
        return None;
    }
    Some(stats)
}

/// Text after the first case-insensitive occurrence of `player_name`.
fn suffix_after_name<'a>(line: &'a str, player_name: &str) -> Option<&'a str> {
    let name_lower = player_name.to_lowercase();
    let line_lower = line.to_lowercase();
    let start = line_lower.find(&name_lower)?;
    let end = start + name_lower.len();
    // ASCII names (the scoreboard case) keep byte indexes aligned. A
    // lowercasing that changes width is walked so the slice stays on a
    // char boundary.
    if line_lower.len() == line.len() {
        return line.get(end..);
    }
    let mut byte = 0;
    let mut low_byte = 0;
    for ch in line.chars() {
        let low: String = ch.to_lowercase().collect();
        if low_byte >= end {
            return Some(&line[byte..]);
        }
        low_byte += low.len();
        byte += ch.len_utf8();
    }
    if low_byte >= end { Some("") } else { None }
}

/// Drop `MM:SS` / `M:SS` tokens. The hero-panel objective timer is the one
/// that lands on the player's OCR line; stat columns do not contain a colon.
fn strip_clock_tokens(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        if let Some(len) = clock_token_len(&chars, i) {
            i += len;
            out.push(' ');
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn clock_token_len(chars: &[char], i: usize) -> Option<usize> {
    if i > 0 && chars[i - 1].is_ascii_digit() {
        return None;
    }
    let mut j = i;
    let mut hour = 0;
    while j < chars.len() && chars[j].is_ascii_digit() {
        hour += 1;
        j += 1;
        if hour > 2 {
            return None;
        }
    }
    if !(1..=2).contains(&hour) || j >= chars.len() || chars[j] != ':' {
        return None;
    }
    j += 1;
    let sec_at = j;
    while j < chars.len() && chars[j].is_ascii_digit() {
        j += 1;
        if j - sec_at > 2 {
            return None;
        }
    }
    if j - sec_at != 2 || (j < chars.len() && chars[j].is_ascii_digit()) {
        return None;
    }
    Some(j - i)
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
    for &(display_name, pattern) in MAPS {
        if text.contains(&normalize_ocr_glyphs(pattern)) {
            return Some(display_name.to_string());
        }
    }
    None
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
    // Grímsvötn is before the shared "watchpoint" prefix. The first
    // substring hit wins, so a Grímsvötn key anywhere in the text beats
    // the prefix. A bare "watchpoint" still falls through to Gibraltar.
    ("Watchpoint: Grímsvötn", "grimsvotn"),
    ("Watchpoint: Gibraltar", "gibraltar"),
    ("Watchpoint: Gibraltar", "watchpoint"),
    ("Blizzard World", "blizzard world"),
    ("Eichenwalde", "eichenwalde"),
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
];

/// Fold OCR-ambiguous glyphs and Latin diacritics so a mangled map name still
/// matches. `1`, `|`, `l` collapse to `i`; `0` collapses to `o`. Precomposed
/// accents fold to ASCII (`í`→`i`, `ö`→`o`, and the other letters this table
/// already stores). Combining marks are dropped. Callers lowercase first.
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
            'í' | 'ì' | 'î' | 'ï' | 'Í' | 'Ì' | 'Î' | 'Ï' => 'i',
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
        let read = read_scoreboard(&[], None, raw, "defeat", Some("FROZEN")).unwrap();
        assert!(!read.trusted_cells);
        assert_eq!(read.matched.elims, 7);
        assert_eq!(read.matched.mitigation, 3316);
    }

    #[test]
    fn timer_on_the_player_line_does_not_shift_columns() {
        // Row 0 sits at the same height as the hero panel's OBJ CONTEST TIME.
        // The old last-6 window turned
        // "2 0 0 1,105 259 450 ... 00:02" into E 0, A 1105, D 259, DMG 450.
        let raw = "74 FROZEN Giant Troll 2 0 0 1,105 259 450 00:02";
        let read = read_scoreboard(&[], None, raw, "unknown", Some("FROZEN")).unwrap();
        assert!(!read.trusted_cells);
        let p = &read.matched;
        assert_eq!(
            (
                p.elims,
                p.assists,
                p.deaths,
                p.damage,
                p.healing,
                p.mitigation
            ),
            (2, 0, 0, 1105, 259, 450)
        );
        assert_ne!((p.assists, p.deaths), (1105, 259));
    }

    #[test]
    fn shifted_fallback_numbers_are_rejected() {
        // The six numbers the old window emitted, with the name in front.
        // Assists 1105 and deaths 259 are not a scoreboard.
        let raw = "FROZEN 0 1105 259 450 0 2";
        assert_eq!(
            read_scoreboard(&[], None, raw, "unknown", Some("FROZEN")).unwrap_err(),
            ScoreboardMiss::CellsUnreadable
        );
        // A dropped zero plus the timer is not six stats. Do not slide.
        let dropped = "FROZEN 2 1,105 259 450 00:02";
        assert_eq!(
            read_scoreboard(&[], None, dropped, "unknown", Some("FROZEN")).unwrap_err(),
            ScoreboardMiss::CellsUnreadable
        );
    }

    #[test]
    fn identified_row_with_an_empty_cell_is_unreadable_not_missing() {
        let rows = vec![row(Some("FROZEN"), ["2", "", "0", "1105", "259", "450"])];
        assert_eq!(
            read_scoreboard(&rows, Some(0), "", "unknown", Some("FROZEN")).unwrap_err(),
            ScoreboardMiss::CellsUnreadable
        );
        assert_eq!(
            read_scoreboard(
                &[],
                None,
                "no name on this frame",
                "unknown",
                Some("FROZEN")
            )
            .unwrap_err(),
            ScoreboardMiss::PlayerRowNotFound
        );
    }

    #[test]
    fn explicit_zero_cells_are_a_trusted_read() {
        let rows = vec![row(Some("FROZEN"), ["0", "0", "0", "0", "0", "0"])];
        let read = read_scoreboard(&rows, Some(0), "", "unknown", Some("FROZEN")).unwrap();
        assert!(read.trusted_cells);
        assert_eq!(read.matched.elims, 0);
        assert_eq!(read.matched.assists, 0);
        assert_eq!(read.matched.deaths, 0);
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
            "WATCHPOINT",
            "watchpoint",
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
