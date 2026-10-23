use crate::model::{PersonalMatch, RowOcrResult};
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
    /// No identified row, by name or highlight, and the configured name is
    /// not a whole word in the raw text.
    PlayerRowNotFound,
    /// A row index was set, or the configured name appears as a whole word,
    /// but the cells did not parse and the text fallback did not yield
    /// exactly six in-range stats. The name can be in chat or the kill feed
    /// rather than on a scoreboard row.
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
    let lines: Vec<&str> = raw_text
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();
    let from_cells = player_row_index
        .and_then(|idx| rows.get(idx))
        .and_then(stats_from_row);
    let (stats, trusted_cells) = if let Some(stats) = from_cells {
        (stats, true)
    } else if let Some(stats) = text_fallback_stats(&lines, player_name) {
        (stats, false)
    } else if player_row_index.is_some() || name_in_lines(&lines, player_name) {
        return Err(ScoreboardMiss::CellsUnreadable);
    } else {
        return Err(ScoreboardMiss::PlayerRowNotFound);
    };

    let hero = find_hero(&lines).unwrap_or_else(|| "Unknown".to_string());
    let map_name = find_map(&lines).unwrap_or_default();

    Ok(ScoreboardRead {
        trusted_cells,
        matched: new_match(stats, hero, map_name, outcome),
    })
}

/// A fresh, unsaved row for one board read with no text off the frame: the
/// six stats in board order (E, A, D, DMG, H, MIT), hero `Unknown`, no map.
/// `reader = "new"` starts from this; the hero and the map come from the
/// icon and banner readers, never from OCR text.
pub fn match_from_stats(stats: [u32; 6], outcome: &str) -> PersonalMatch {
    let [elims, assists, deaths, damage, healing, mitigation] = stats;
    new_match(
        PlayerStats {
            elims,
            deaths,
            assists,
            damage,
            healing,
            mitigation,
        },
        "Unknown".to_string(),
        String::new(),
        outcome,
    )
}

fn new_match(stats: PlayerStats, hero: String, map_name: String, outcome: &str) -> PersonalMatch {
    let role = guess_role(&hero);
    let game_mode = map_mode(&map_name).unwrap_or("").to_string();
    PersonalMatch {
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
        upload_reject: None,
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
        recognizer: scuffed_types::RECOGNIZER_OCR_V1.to_string(),
        suspect_fields: Vec::new(),
        name_recognizer: None,
    }
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
                .filter(|c| crate::model::is_clean_stat(c.value.trim()))
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
            .filter(|cell| crate::model::is_clean_stat(cell.value.trim()))
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

/// Modes kept in the local store and never uploaded.
///
/// Deathmatch and Practice were already local. Workshop, Elimination,
/// Capture the Flag, Payload Race, Assault, and Stadium stay local too,
/// until there is a decision on whether those games count.
const UNTRACKED_MODES: &[&str] = &[
    "Deathmatch",
    "Practice",
    "Workshop",
    "Elimination",
    "Capture the Flag",
    "Payload Race",
    "Assault",
    "Stadium",
];

/// True when `mode` is one of [`UNTRACKED_MODES`], ignoring ASCII case.
pub fn mode_is_untracked(mode: &str) -> bool {
    let mode = mode.trim();
    UNTRACKED_MODES
        .iter()
        .any(|name| mode.eq_ignore_ascii_case(name))
}

/// Deathmatch, practice, and the other [`UNTRACKED_MODES`] maps are kept
/// in the local store and never uploaded. Only a trusted map read (top
/// bar, accolade, or an already trusted session) may divert a capture. A
/// fuzzy board read must not.
///
/// Practice Range is not a game. With no table entry the name used to
/// canonicalize to nothing, the row kept an empty map, and that empty map
/// was uploaded. The Practice bucket keeps it out of uploads.
pub fn map_is_untracked(name: &str) -> bool {
    map_mode(name).is_some_and(mode_is_untracked)
}

/// Reason recorded when an untracked map is kept out of uploads.
pub fn untracked_close_reason(name: &str) -> &'static str {
    match map_mode(name) {
        Some("Practice") => "practice: not tracked",
        Some("Workshop") => "workshop: not tracked",
        Some("Elimination") => "elimination: not tracked",
        Some("Capture the Flag") => "capture the flag: not tracked",
        Some("Payload Race") => "payload race: not tracked",
        Some("Assault") => "assault: not tracked",
        Some("Stadium") => "stadium: not tracked",
        _ => "deathmatch: not tracked",
    }
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

/// Display names from [`MAPS`], in table order, duplicates removed.
///
/// The strings are the table's own spellings. `Paraiso` and `Esperanca`
/// stay unaccented. `Watchpoint: Grímsvötn` and `Château Guillard` keep
/// the accents already stored on those rows.
pub fn known_map_names() -> Vec<&'static str> {
    let mut names = Vec::new();
    for &(display, _) in MAPS {
        if !names.contains(&display) {
            names.push(display);
        }
    }
    names
}

/// True when `name` is a [`known_map_names`] display string.
pub fn map_is_known(name: &str) -> bool {
    map_mode(name.trim()).is_some()
}

/// Empty, or the literal `Unknown` in any ASCII case.
///
/// Other hero strings are left as read. This is the OCR miss that must
/// not be uploaded.
pub fn hero_is_unknown_label(name: &str) -> bool {
    let trimmed = name.trim();
    trimmed.is_empty() || trimmed.eq_ignore_ascii_case("unknown")
}

/// Server `suspect_fields` names for one row that is not ready to upload.
///
/// `map` when the map is empty or not in [`known_map_names`]. `mode` when
/// the mode that would be sent is empty. `hero` when
/// [`hero_is_unknown_label`] is true.
///
/// A non-empty list holds the whole match on the machine. The upload
/// leaves that match out. `POST /api/stats/upload` requires `hero` and
/// `map_name` as strings: a null hero and an omitted hero both fail JSON
/// decode, and an empty `map_name` is what got stored on the server.
pub fn review_suspect_fields(map_name: &str, game_mode: &str, hero: &str) -> Vec<&'static str> {
    let map = map_name.trim();
    let mut fields = Vec::new();
    if !map_is_known(map) {
        fields.push("map");
    }
    if uploaded_game_mode(map, game_mode).trim().is_empty() {
        fields.push("mode");
    }
    if hero_is_unknown_label(hero) {
        fields.push("hero");
    }
    fields
}

/// A stats row is uploaded only when neither the map nor the mode is in
/// [`UNTRACKED_MODES`].
pub fn stats_row_is_tracked(map_name: &str, game_mode: &str) -> bool {
    !map_is_untracked(map_name) && !mode_is_untracked(game_mode)
}

/// Mode stored on a row, from the canonical map that was actually kept.
pub fn stored_game_mode(canonical: &str) -> String {
    map_mode(canonical).unwrap_or("").to_string()
}

/// A row with a blank map, mode, or hero must not be uploaded.
///
/// Whitespace-only counts as blank. Hero `Unknown` is a real stored value
/// and is not blank.
pub fn upload_identity_blank(map_name: &str, game_mode: &str, hero: &str) -> bool {
    map_name.trim().is_empty() || game_mode.trim().is_empty() || hero.trim().is_empty()
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

/// Elims past this are a damage or timer digit in the elims column.
///
/// The number is `scuffed_types::PLAUSIBLE_ELIMS_MAX`. Server hero and role
/// averages use that same ceiling.
pub const MAX_ELIMS: u32 = scuffed_types::PLAUSIBLE_ELIMS_MAX;
/// Assists past this are a damage or timer digit in the assists column.
///
/// The number is `scuffed_types::PLAUSIBLE_ASSISTS_MAX`.
pub const MAX_ASSISTS: u32 = scuffed_types::PLAUSIBLE_ASSISTS_MAX;
/// Deaths past this are a damage or timer digit in the deaths column.
///
/// The number is `scuffed_types::PLAUSIBLE_DEATHS_MAX`. Server hero and role
/// averages use that same ceiling.
pub const MAX_DEATHS: u32 = scuffed_types::PLAUSIBLE_DEATHS_MAX;

/// E/A above [`MAX_ELIMS`] / [`MAX_ASSISTS`] or D above [`MAX_DEATHS`] is a
/// column bleed, not a real scoreboard. Shared with the text fallback and
/// the capture gate's first-capture check.
pub(crate) fn kill_columns_implausible(elims: u32, assists: u32, deaths: u32) -> bool {
    elims > MAX_ELIMS || assists > MAX_ASSISTS || deaths > MAX_DEATHS
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

fn configured_name(player_name: Option<&str>) -> Option<&str> {
    player_name.map(str::trim).filter(|name| !name.is_empty())
}

fn name_in_lines(lines: &[&str], player_name: Option<&str>) -> bool {
    let Some(name) = configured_name(player_name) else {
        return false;
    };
    lines
        .iter()
        .any(|line| name_word_index(line, name).is_some())
}

/// Byte index of `player_name` in the lowercased line, when it is a whole
/// word. `Ana` does not match inside `BANANA`. An empty name matches nothing.
fn name_word_index(line: &str, player_name: &str) -> Option<usize> {
    let name_lower = player_name.trim().to_lowercase();
    if name_lower.is_empty() {
        return None;
    }
    let line_lower = line.to_lowercase();
    let mut search_from = 0;
    while let Some(rel) = line_lower[search_from..].find(&name_lower) {
        let start = search_from + rel;
        let end = start + name_lower.len();
        let before_ok = line_lower[..start]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric());
        let after_ok = line_lower[end..]
            .chars()
            .next()
            .is_none_or(|c| !c.is_alphanumeric());
        if before_ok && after_ok {
            return Some(start);
        }
        let step = name_lower.chars().next().map_or(1, char::len_utf8);
        search_from = start + step;
        if search_from >= line_lower.len() {
            break;
        }
    }
    None
}

/// Stats from a full-board text line that contains the player name as a
/// whole word.
///
/// The per-cell path is positional. This one is not, and the line for row 0
/// can also pick up the hero panel's objective timer (`00:02`), which sits
/// at the same height. Taking the last six numbers then slid the columns:
/// `2 0 0 1,105 259 450 00:02` became elims 0, assists 1105, deaths 259.
/// Rank badges sit *before* the name, so the numbers after the name are the
/// stats. A clock token is removed first. The six have to be one unbroken
/// run. A word or `%` after that run refuses the line. `ACCURACY 35%` is
/// five numbers, then a word; the run length refuses it, not the percent.
/// `8 1 2 989 1,583 35%` is six numbers and a percent, and the percent
/// refuses it. `8 1 2 989 1,583 35` has lost both the label and the
/// percent, so a count of six still accepts that shifted line. A
/// hero-panel label after a correct row (`450 OBJ CONTEST TIME 00:02`)
/// refuses the line too. That is intentional: the per-cell path still has
/// the row, and a label is not a seventh stat. A number that does not fit
/// in `u32` refuses the line instead of being dropped. A chat line
/// (`name: 1 2 3 4 5 6`) is skipped so a later stat line can match. The
/// first line that merely mentions the name (a join message) is skipped
/// the same way. The six still have to pass the same kill-column ceilings
/// as [`stats_from_row`].
fn text_fallback_stats(lines: &[&str], player_name: Option<&str>) -> Option<PlayerStats> {
    let name = configured_name(player_name)?;
    for line in lines {
        if name_word_index(line, name).is_none() {
            continue;
        }
        let Some(suffix) = suffix_after_name(line, name) else {
            continue;
        };
        // `[Team] <name>: 1 2 3 4 5 6` is chat. The row's own line does not
        // put a colon right after the name. Skip it so a later stat line
        // can still match.
        if suffix.trim_start().starts_with(':') {
            continue;
        }
        if let Some(stats) = stats_from_player_suffix(suffix) {
            return Some(stats);
        }
    }
    None
}

fn stats_from_player_suffix(suffix: &str) -> Option<PlayerStats> {
    let suffix = strip_clock_tokens(suffix);
    let numbers = stat_run_after_name(&suffix)?;
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

/// Text after the first whole-word, case-insensitive match of `player_name`.
///
/// An empty or whitespace name does not match. `Ana` does not match inside
/// `BANANA`. ASCII lines keep byte indexes (lowercasing does not move them).
/// Any other line is walked, because a character that grows and one that
/// shrinks can leave the lowercased string the same length while the indexes
/// no longer line up.
fn suffix_after_name<'a>(line: &'a str, player_name: &str) -> Option<&'a str> {
    let name_lower = player_name.trim().to_lowercase();
    if name_lower.is_empty() {
        return None;
    }
    let end = name_word_index(line, player_name)? + name_lower.len();
    if line.is_ascii() {
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
    let mut minute_digits = 0;
    while j < chars.len() && chars[j].is_ascii_digit() {
        minute_digits += 1;
        j += 1;
        if minute_digits > 2 {
            return None;
        }
    }
    if !(1..=2).contains(&minute_digits) || j >= chars.len() || chars[j] != ':' {
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

enum SuffixToken {
    Number(u32),
    /// Digits that do not fit in `u32`. Dropping one used to leave a
    /// shifted line looking like six stats.
    Overflow,
    Word,
    Percent,
}

/// The six stats after the name, or nothing.
///
/// They have to be one consecutive run of numbers. A word or `%` after
/// that run refuses the line, including a correct row whose hero-panel
/// label follows the stats. A word before the run is a title and is
/// allowed. A second run cannot appear: it would have to follow a word
/// or a percent, which already refused the line. A number that overflows
/// `u32` refuses the line.
///
/// Six numbers with the accuracy label and the percent both missing
/// (`8 1 2 989 1,583 35`) are still accepted. Nothing in the line says
/// the last figure is not mitigation.
fn stat_run_after_name(suffix: &str) -> Option<[u32; 6]> {
    let tokens = suffix_tokens(suffix);
    if tokens.iter().any(|t| matches!(t, SuffixToken::Overflow)) {
        tracing::debug!("rejecting text fallback: a number does not fit in u32");
        return None;
    }
    let mut i = 0;
    while i < tokens.len() {
        if !matches!(tokens[i], SuffixToken::Number(_)) {
            i += 1;
            continue;
        }
        let mut nums = Vec::new();
        while let Some(SuffixToken::Number(n)) = tokens.get(i) {
            nums.push(*n);
            i += 1;
        }
        if nums.len() != 6 {
            tracing::debug!(
                n = nums.len(),
                "rejecting text fallback: not exactly six stats after the player name"
            );
            return None;
        }
        if tokens[i..]
            .iter()
            .any(|t| matches!(t, SuffixToken::Word | SuffixToken::Percent))
        {
            tracing::debug!("rejecting text fallback: a word or percent follows the stat run");
            return None;
        }
        return Some([nums[0], nums[1], nums[2], nums[3], nums[4], nums[5]]);
    }
    None
}

fn suffix_tokens(s: &str) -> Vec<SuffixToken> {
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    while i < chars.len() {
        let c = chars[i];
        if c.is_ascii_digit() {
            let mut digits = String::new();
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == ',') {
                if chars[i].is_ascii_digit() {
                    digits.push(chars[i]);
                }
                i += 1;
            }
            match digits.parse::<u32>() {
                Ok(n) => out.push(SuffixToken::Number(n)),
                Err(_) => out.push(SuffixToken::Overflow),
            }
            continue;
        }
        if c == '%' {
            out.push(SuffixToken::Percent);
            i += 1;
            continue;
        }
        if c.is_alphabetic() {
            while i < chars.len() && chars[i].is_alphabetic() {
                i += 1;
            }
            out.push(SuffixToken::Word);
            continue;
        }
        i += 1;
    }
    out
}

pub fn guess_role_public(hero: &str) -> String {
    guess_role(hero)
}

/// Fuzzy score a name read needs to count as the player's name.
const NAME_MATCH_MIN: f64 = 0.55;

/// The row that matched the configured player name, and which name window
/// matched it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NameMatch {
    /// Row index across both teams.
    pub row: usize,
    /// Index into `ocr::preprocess::name_windows` of the read that
    /// matched: a substring hit if there is one, else the best fuzzy read
    /// (the lower index on a tie).
    pub window: usize,
    /// No other window's read on that row matched. On 5v5 the windows belong
    /// to different table layouts, so a sole match also names the layout.
    pub sole_window: bool,
}

/// Find which row (across all rows, both teams) best matches the configured
/// player name. Returns the row index.
///
/// Used for replay and post-match screens where the player may be on team 2,
/// so brightness-based team-1 scanning can't find them. The name cells from
/// `recognize_row` are noisy OCR, so we use a generous fuzzy threshold (0.55)
/// and pick the best score across all rows. See [`match_player_row_by_name`]
/// for the matching window.
pub fn find_player_row_by_name(rows: &[RowOcrResult], player_name: &str) -> Option<usize> {
    match_player_row_by_name(rows, player_name).map(|m| m.row)
}

/// Like [`find_player_row_by_name`], and also reports which name window
/// matched.
///
/// Each row scores as its best read over every name window (5v5 reads one
/// per table layout). The first row, in board order, with a substring hit in
/// any window wins outright; otherwise the best fuzzy row at or above 0.55.
pub fn match_player_row_by_name(rows: &[RowOcrResult], player_name: &str) -> Option<NameMatch> {
    let name_lower = player_name.to_lowercase();

    let mut best: Option<(usize, f64)> = None;

    for (i, row) in rows.iter().enumerate() {
        let scores = row_window_scores(row, &name_lower);
        if let Some(&(window, _)) = scores.iter().find(|(_, s)| s.is_substring()) {
            // `name` so the report log scrub drops the field (report-bundle §6).
            tracing::debug!(row = i, window, "player name found via substring in row");
            return Some(name_match(i, window, &scores));
        }
        let row_best = scores
            .iter()
            .filter_map(|(_, s)| s.fuzzy())
            .fold(None, |acc: Option<f64>, s| {
                Some(acc.map_or(s, |a| a.max(s)))
            });
        if let Some(score) = row_best
            && best.is_none_or(|(_, b)| score > b)
        {
            best = Some((i, score));
        }
    }

    let (row, score) = best?;
    if score < NAME_MATCH_MIN {
        return None;
    }
    let scores = row_window_scores(&rows[row], &name_lower);
    let window = scores
        .iter()
        .filter(|(_, s)| s.fuzzy() == Some(score))
        .map(|(w, _)| *w)
        .min()
        .unwrap_or(0);
    tracing::debug!(row, window, score, "player name fuzzy-matched in row");
    Some(name_match(row, window, &scores))
}

fn name_match(row: usize, window: usize, scores: &[(usize, NameRead)]) -> NameMatch {
    let sole_window = !scores
        .iter()
        .any(|(w, s)| *w != window && s.passes(NAME_MATCH_MIN));
    NameMatch {
        row,
        window,
        sole_window,
    }
}

/// `(window index, score)` for every non-empty name read of `row`. `name` is
/// window 0 and `name_alternates[k]` is window `k + 1`.
fn row_window_scores(row: &RowOcrResult, name_lower: &str) -> Vec<(usize, NameRead)> {
    std::iter::once(row.name.as_ref())
        .chain(row.name_alternates.iter().map(Option::as_ref))
        .enumerate()
        .filter_map(|(window, read)| {
            let read = read.filter(|c| !c.value.is_empty())?;
            Some((
                window,
                name_read_score(&read.value.to_lowercase(), name_lower),
            ))
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum NameRead {
    /// The read contains the name (handles "FROZEN" inside "L7 mRoE FROZEN").
    Substring,
    /// Best fuzzy score of the read against the name.
    Fuzzy(f64),
    /// The read is too short to hold the name; not scored.
    TooShort,
}

impl NameRead {
    fn is_substring(self) -> bool {
        self == NameRead::Substring
    }

    fn fuzzy(self) -> Option<f64> {
        match self {
            NameRead::Fuzzy(s) => Some(s),
            _ => None,
        }
    }

    fn passes(self, min: f64) -> bool {
        match self {
            NameRead::Substring => true,
            NameRead::Fuzzy(s) => s >= min,
            NameRead::TooShort => false,
        }
    }
}

/// Score one lowercased name-cell read against the lowercased player name.
fn name_read_score(cell_text: &str, name_lower: &str) -> NameRead {
    if cell_text.contains(name_lower) {
        return NameRead::Substring;
    }

    // Fuzzy: slide a window the length of the player name over the cell text
    let name_chars: Vec<char> = name_lower.chars().collect();
    let cell_chars: Vec<char> = cell_text.chars().collect();
    let window = name_chars.len();
    if window == 0 || window > cell_chars.len() + 4 {
        return NameRead::TooShort;
    }
    // Also compare whole cell text against the name
    let score_whole = normalized_levenshtein(cell_text, name_lower);
    let score_window = if cell_chars.len() >= window {
        (0..=(cell_chars.len().saturating_sub(window)))
            .map(|s| {
                let slice: String = cell_chars[s..s + window].iter().collect();
                normalized_levenshtein(&slice, name_lower)
            })
            .fold(0.0f64, f64::max)
    } else {
        0.0
    };
    NameRead::Fuzzy(score_whole.max(score_window))
}

/// Canonicalize a map identifier to its display name in the MAPS table —
/// e.g. the map-vote screen's "SHAMBALI" becomes "Shambali Monastery".
/// `None` when nothing matches: an uncanonicalizable name must not be stored,
/// or the same map fractures into several aggregate rows.
pub fn canonical_map(name: &str) -> Option<String> {
    match_map_in_text(name)
}

/// Mode word printed on the Tab banner, before the `|` and the map name.
/// None of these is a map key. A longer banner phrase still starts with one
/// of them (`PAYLOAD RACE`, `CAPTURE THE FLAG`).
const BANNER_MODE_WORDS: &[&str] = &[
    "CONTROL",
    "ESCORT",
    "HYBRID",
    "PUSH",
    "FLASHPOINT",
    "CLASH",
    "ASSAULT",
    "ELIMINATION",
    "DEATHMATCH",
    "PAYLOAD",
    "CAPTURE",
];

/// Map text from a Tab banner read.
///
/// The banner is `icon MODE | MAP TIME`. The map is the text after the first
/// real `|`. A leading `|` is icon junk (`| CONTROL | ILIOS`), and the mode
/// word after that split is dropped too. This split happens before glyph
/// folding, which would turn `|` into `i` and glue it onto the map
/// (`|ILIOS` becomes a word the matcher misses). A trailing clock
/// (`05:12`, or glued on as `ILIOS05:12`) is not part of the name. With no
/// bar, a mode word in the first few tokens is skipped the same way, and so
/// is the junk in front of it.
fn tab_banner_map_text(text: &str) -> &str {
    let text = text.trim().trim_start_matches('|').trim();
    let map_side = if let Some((_, right)) = text.split_once('|') {
        strip_leading_mode(right.trim().trim_start_matches('|').trim())
    } else {
        strip_icon_and_mode(text)
    };
    strip_trailing_clock(map_side)
}

/// Drop a mode word that survived the bar split, and a bar glued to it.
fn strip_leading_mode(text: &str) -> &str {
    let Some(tok) = text.split_whitespace().next() else {
        return text;
    };
    if !mode_token(tok) {
        return text;
    }
    let Some(rel) = text.find(tok) else {
        return text;
    };
    let after = text[rel + tok.len()..]
        .trim_start()
        .trim_start_matches('|')
        .trim();
    if after.is_empty() { text } else { after }
}

/// `05:12` at the end of the map side, with or without a space before it.
fn strip_trailing_clock(text: &str) -> &str {
    let text = text.trim_end();
    let b = text.as_bytes();
    let n = b.len();
    if n < 4 || !b[n - 1].is_ascii_digit() || !b[n - 2].is_ascii_digit() || b[n - 3] != b':' {
        return text;
    }
    let mut start = n - 3;
    let mut hour_digits = 0u8;
    while start > 0 && b[start - 1].is_ascii_digit() && hour_digits < 2 {
        start -= 1;
        hour_digits += 1;
    }
    if hour_digits == 0 {
        return text;
    }
    text[..start].trim_end()
}

fn mode_token(tok: &str) -> bool {
    let bare = tok.trim_matches(|c: char| !c.is_ascii_alphabetic());
    BANNER_MODE_WORDS
        .iter()
        .any(|word| bare.eq_ignore_ascii_case(word))
}

/// Drop icon junk and a following mode word. The map name is what remains.
/// A string with no mode word is returned unchanged, so `King's Row` is kept.
fn strip_icon_and_mode(text: &str) -> &str {
    let mut offset = 0usize;
    for (seen, tok) in text.split_whitespace().enumerate() {
        if seen >= 4 {
            break;
        }
        let Some(rel) = text[offset..].find(tok) else {
            break;
        };
        let start = offset + rel;
        if mode_token(tok) {
            let after = text[start + tok.len()..].trim_start();
            if !after.is_empty() {
                return after;
            }
            break;
        }
        offset = start + tok.len();
    }
    text
}

/// Match a map name from arbitrary OCR text (e.g. the top-bar map label).
///
/// See [`tab_banner_map_text`]: only the map side of a Tab banner is matched,
/// case-insensitively, against the known map list.
pub fn match_map_in_text(text: &str) -> Option<String> {
    let label = tab_banner_map_text(text);
    let lines: Vec<&str> = label
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
pub fn outcome_from_board_header(raw_text: &str) -> crate::model::MatchOutcome {
    use crate::model::MatchOutcome;
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
    // The role follows the canonical hero, including a pack key such as
    // `wrecking-ball`. An unknown key is not Damage: that used to file every
    // missed name under the Damage column.
    scuffed_types::role_for_hero_name(hero)
        .map(|role| role.to_string())
        .unwrap_or_else(|| "Unknown".to_string())
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
    // "antarctica" starts with "antarctic", so the short control-map key
    // used to store Ecopoint: Antarctica as Antarctic Peninsula. The
    // peninsula's own word is checked first. The full "antarctica" token,
    // and "ecopoint", are the arena map. A bare "antarctic" stays the
    // control map. Both names stay in the table.
    ("Antarctic Peninsula", "peninsula"),
    ("Ecopoint: Antarctica", "ecopoint"),
    ("Ecopoint: Antarctica", "antarctica"),
    ("Antarctic Peninsula", "antarctic"),
    ("Busan", "busan"),
    ("Ilios", "ilios"),
    ("Lijiang Tower", "lijiang"),
    // A koverwatch read drops the J ("LIANG TOWER") or turns IJ into UL
    // ("LULANG TOWER"). Both are this map. Neither string is a substring
    // of another canonical name.
    ("Lijiang Tower", "liang tower"),
    ("Lijiang Tower", "lulang tower"),
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
    // "anubis" alone is Throne of Anubis. "temple" has to win first, or
    // Temple of Anubis is stored as the clash map. Both names stay.
    ("Temple of Anubis", "temple"),
    ("Throne of Anubis", "throne"),
    ("Throne of Anubis", "anubis"),
    // Assault left Quick Play, and these five are still in custom games
    // and arcade. Temple of Anubis is one of them; the others had no entry.
    ("Hanamura", "hanamura"),
    ("Horizon Lunar Colony", "horizon"),
    ("Horizon Lunar Colony", "lunar colony"),
    ("Paris", "paris"),
    ("Volskaya Industries", "volskaya"),
    // Arena maps. Ecopoint: Antarctica is one of them; the short
    // "antarctic" key above must not claim it.
    ("Black Forest", "black forest"),
    ("Castillo", "castillo"),
    ("Necropolis", "necropolis"),
    // Stadium maps that are not a sub-area of a map already stored here.
    // A name that contains an existing key (Oasis University, Busan
    // Sanctuary) still stores as that map, so those rows do not split.
    ("Arena Victoriae", "victoriae"),
    ("Gogadoro", "gogadoro"),
    ("Wuxing University - Water College", "wuxing"),
    ("Wuxing University - Water College", "water college"),
    ("Place Lacroix", "lacroix"),
    ("Redwood Dam", "redwood"),
    ("Serenza", "serenza"),
    ("Powder Keg Mine", "powder keg"),
    ("Thames District", "thames"),
    ("Ayutthaya", "ayutthaya"),
    // Workshop layouts. The full phrase is the key so "workshop" alone
    // cannot pick one of them.
    ("Workshop Chamber", "workshop chamber"),
    ("Workshop Expanse", "workshop expanse"),
    ("Workshop Green Screen", "workshop green"),
    ("Workshop Island", "workshop island"),
    // Training. Practice Range is not a game: see `map_is_untracked`.
    ("Practice Range", "practice range"),
    ("Practice Range", "practice"),
    ("Mastery Course", "mastery"),
    ("Tutorial", "tutorial"),
    // Deathmatch only. Kept in the local store. Never uploaded.
    // A trusted read is required before a capture is diverted.
    ("Château Guillard", "guillard"),
    ("Kanezaka", "kanezaka"),
    ("Malevento", "malevento"),
    ("Petra", "petra"),
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
pub fn normalize_ocr_glyphs(s: &str) -> String {
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
pub fn map_mode(canonical_name: &str) -> Option<&'static str> {
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
        "Hanamura"
        | "Horizon Lunar Colony"
        | "Paris"
        | "Temple of Anubis"
        | "Volskaya Industries" => Some("Assault"),
        "Black Forest" | "Castillo" | "Ecopoint: Antarctica" | "Necropolis" => Some("Elimination"),
        "Ayutthaya" => Some("Capture the Flag"),
        "Powder Keg Mine" | "Thames District" => Some("Payload Race"),
        "Arena Victoriae"
        | "Gogadoro"
        | "Place Lacroix"
        | "Redwood Dam"
        | "Serenza"
        | "Wuxing University - Water College" => Some("Stadium"),
        "Workshop Chamber" | "Workshop Expanse" | "Workshop Green Screen" | "Workshop Island" => {
            Some("Workshop")
        }
        "Mastery Course" | "Practice Range" | "Tutorial" => Some("Practice"),
        "Château Guillard" | "Kanezaka" | "Malevento" | "Petra" => Some("Deathmatch"),
        _ => None,
    }
}

/// Fuzzy threshold for a map name of `len` non-space chars. Short names get a
/// touch more slack: a single wrong glyph in a 5-char name (BUSAN→BUSVN) is a
/// 0.80 score, which the strict long-name bar would reject. Kept close to the
/// long-name bar so player names / stat fragments still don't cross it — the
/// King's-Row false-positive that justified the strict bar is a substring trap
/// (guarded by dropping bare "king" from MAPS), not a fuzzy near-miss.
pub fn map_fuzzy_threshold(len: usize) -> f64 {
    if len <= 6 { 0.80 } else { 0.85 }
}

/// Lead the best fuzzy map must hold over the next different map. A smaller
/// gap is a tie, and the read stays unknown. Exact whole-word hits do not
/// use this: Temple of Anubis and Throne of Anubis still resolve by table
/// order when the key is present.
const MAP_FUZZY_MARGIN: f64 = 0.05;

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
        // Whole word or phrase. A short key must not fire inside a longer
        // word: "paris" inside "comparison", "petra" inside "competra",
        // "practice" inside "inpractice".
        if contains_map_key(text, pattern) {
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
/// long-name threshold and lead the other by [`MAP_FUZZY_MARGIN`]. `6` folds
/// to `o` only here, so Route 66 is left alone. The next two tokens are also
/// joined, so "GRIMS VOTN" can still hit Grímsvötn.
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
    if best_grim >= threshold && best_grim - best_gib >= MAP_FUZZY_MARGIN {
        WatchpointFamily::Named(GRIMSVOTN_NAME)
    } else if best_gib >= threshold && best_gib - best_grim >= MAP_FUZZY_MARGIN {
        WatchpointFamily::Named(GIBRALTAR_NAME)
    } else {
        WatchpointFamily::Undecided
    }
}

/// `pattern` is a map key. It matches only when every character of the key
/// is bounded by a non-alphanumeric edge, so a shorter key cannot hide
/// inside a longer word.
fn contains_map_key(text: &str, pattern: &str) -> bool {
    let pattern = normalize_ocr_glyphs(pattern);
    if pattern.is_empty() {
        return false;
    }
    let mut rest = text;
    while let Some(at) = rest.find(&pattern) {
        let before_ok = rest[..at]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric());
        let after = at + pattern.len();
        let after_ok = rest[after..]
            .chars()
            .next()
            .is_none_or(|c| !c.is_alphanumeric());
        if before_ok && after_ok {
            return true;
        }
        let step = rest[at..].chars().next().map(char::len_utf8).unwrap_or(1);
        rest = &rest[at + step..];
    }
    false
}

fn find_map(lines: &[&str]) -> Option<String> {
    let text = normalize_ocr_glyphs(&lines.join(" ").to_lowercase());
    map_from_normalized(&text, true)
}

/// Score for an OCR tail cut off the end of a long map name (`GRiMsVO` for
/// Grímsvötn). Short keys stay on the whole-word rule. The score is the
/// average of a full match and the covered fraction, so two missing letters
/// on a 9-letter name clear the long-name floor and a stub does not.
fn prefix_truncation_score(word: &str, pattern: &str) -> f64 {
    let n = pattern.chars().count();
    let k = word.chars().count();
    if n <= 6 || k < 6 || k >= n || !pattern.starts_with(word) {
        return 0.0;
    }
    (n + k) as f64 / (2.0 * n as f64)
}

/// Closest map at or above [`map_fuzzy_threshold`], when it leads the next
/// different display name by [`MAP_FUZZY_MARGIN`]. Below the floor, or inside
/// that margin, the map is unknown. Aliases of one display name (Lijiang's
/// `liang` / `lulang` keys) only raise that name's score.
fn fuzzy_match_map(text: &str) -> Option<String> {
    // `text` is expected to already be glyph-normalized by the caller.
    let text = normalize_ocr_glyphs(text);
    let words: Vec<&str> = text.split_whitespace().collect();

    let mut best_map: Option<&str> = None;
    let mut best_score: f64 = 0.0;
    let mut second_score: f64 = 0.0;

    for &(display_name, pattern) in MAPS {
        let pattern = normalize_ocr_glyphs(pattern);
        let pattern_parts: Vec<&str> = pattern.split_whitespace().collect();
        let threshold = map_fuzzy_threshold(pattern.chars().filter(|c| !c.is_whitespace()).count());
        let mut pattern_best = 0.0;

        if pattern_parts.len() == 1 {
            for &word in &words {
                let token = word.trim_matches(|c: char| !c.is_alphanumeric());
                if token.is_empty() {
                    continue;
                }
                let score = normalized_levenshtein(token, &pattern)
                    .max(prefix_truncation_score(token, &pattern));
                if score > pattern_best {
                    pattern_best = score;
                }
            }
        } else {
            for window in words.windows(pattern_parts.len()) {
                let candidate = window.join(" ");
                let score = normalized_levenshtein(&candidate, &pattern);
                if score > pattern_best {
                    pattern_best = score;
                }
            }
        }

        if pattern_best < threshold {
            continue;
        }
        // A second key for the map already in front only improves its score.
        if best_map == Some(display_name) {
            if pattern_best > best_score {
                best_score = pattern_best;
            }
            continue;
        }
        if best_map.is_none() || pattern_best > best_score {
            if best_map.is_some() {
                second_score = second_score.max(best_score);
            }
            best_score = pattern_best;
            best_map = Some(display_name);
        } else if pattern_best > second_score {
            second_score = pattern_best;
        }
    }

    let map_name = best_map?;
    if best_score - second_score < MAP_FUZZY_MARGIN {
        tracing::debug!(
            map = map_name,
            score = best_score,
            runner_up = second_score,
            "fuzzy map match held: top scores are too close"
        );
        return None;
    }

    tracing::debug!(map = map_name, score = best_score, "fuzzy matched map name");
    Some(map_name.to_string())
}

#[cfg(test)]
mod tests_board;

#[cfg(test)]
mod tests_header;

#[cfg(test)]
mod tests_maps;
