//! Reading a scoreboard: per-cell rows into a match, the raw-text
//! fallback, row and board checks, the stat ceilings, and the result word
//! in the board header.

use crate::model::{PersonalMatch, RowOcrResult};
use chrono::Utc;
use surrealdb_types::Datetime as SurrealDatetime;

use super::catalog::{guess_role, map_mode};
use super::find_hero;
use super::map_text::find_map;

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
