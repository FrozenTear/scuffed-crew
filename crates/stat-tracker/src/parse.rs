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
pub(crate) const MAX_ELIMS: u32 = 99;
/// Assists past this are a damage or timer digit in the assists column.
pub(crate) const MAX_ASSISTS: u32 = 99;
/// Deaths past this are a damage or timer digit in the deaths column.
pub(crate) const MAX_DEATHS: u32 = 50;

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
/// `|`. Icon junk and the mode word are on the left and are not matched.
/// This split happens before glyph folding, which would turn `|` into `i`
/// and glue it onto the map (`|ILIOS` becomes a word the matcher misses).
/// With no bar, a mode word in the first few tokens is skipped the same way,
/// and so is the junk in front of it.
fn tab_banner_map_text(text: &str) -> &str {
    let text = text.trim();
    if let Some((_, right)) = text.split_once('|') {
        return right.trim();
    }
    strip_icon_and_mode(text)
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
pub(crate) fn map_fuzzy_threshold(len: usize) -> f64 {
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
                let score = normalized_levenshtein(word, &pattern);
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
    }

    #[test]
    fn seven_numbers_and_a_shifted_extra_stat_are_refused() {
        let seven = "FROZEN 8 1 2 3,993 989 1,583 35";
        assert_eq!(
            read_scoreboard(&[], None, seven, "unknown", Some("FROZEN")).unwrap_err(),
            ScoreboardMiss::CellsUnreadable
        );
        // Damage dropped, and an accuracy percent supplies the sixth number.
        let shifted = "FROZEN 8 1 2 989 1,583 ACCURACY 35%";
        assert_eq!(
            read_scoreboard(&[], None, shifted, "unknown", Some("FROZEN")).unwrap_err(),
            ScoreboardMiss::CellsUnreadable
        );
        // Last six of this line are in the ceilings (E1 A2 D30 DMG989 H1583
        // MIT35). Taking the last six would accept it. The first six of the
        // line above fail the deaths ceiling, so that line does not pin this.
        let last_six = "FROZEN 8 1 2 30 989 1,583 35";
        assert_eq!(
            read_scoreboard(&[], None, last_six, "unknown", Some("FROZEN")).unwrap_err(),
            ScoreboardMiss::CellsUnreadable
        );
        // Six numbers and a percent. The run length is fine. The percent
        // after the run is what refuses it.
        let percent = "FROZEN 8 1 2 989 1,583 35%";
        assert_eq!(
            read_scoreboard(&[], None, percent, "unknown", Some("FROZEN")).unwrap_err(),
            ScoreboardMiss::CellsUnreadable
        );
        // A hero-panel label after a correct row. Refusing it is intentional.
        let labeled = "FROZEN 8 1 2 3,993 989 1,583 OBJ CONTEST TIME 00:02";
        assert_eq!(
            read_scoreboard(&[], None, labeled, "unknown", Some("FROZEN")).unwrap_err(),
            ScoreboardMiss::CellsUnreadable
        );
        // The label and the percent are both gone. Six numbers are still
        // accepted, shifted. Nothing in the line says 35 is not mitigation.
        let unlabeled = "FROZEN 8 1 2 989 1,583 35";
        let read = read_scoreboard(&[], None, unlabeled, "unknown", Some("FROZEN")).unwrap();
        assert_eq!(read.matched.elims, 8);
        assert_eq!(read.matched.damage, 989);
        assert_eq!(read.matched.mitigation, 35);
        let overflow = "FROZEN 2 0 0 1,105 259 450 99999999999";
        assert_eq!(
            read_scoreboard(&[], None, overflow, "unknown", Some("FROZEN")).unwrap_err(),
            ScoreboardMiss::CellsUnreadable
        );
    }

    #[test]
    fn a_short_or_empty_name_does_not_read_another_row() {
        let banana = "BANANA 9 9 9 9999 9999 9999";
        assert_eq!(
            read_scoreboard(&[], None, banana, "unknown", Some("Ana")).unwrap_err(),
            ScoreboardMiss::PlayerRowNotFound
        );
        let someone = "SOMEONE 1 2 3 4 5 6";
        assert_eq!(
            read_scoreboard(&[], None, someone, "unknown", Some("")).unwrap_err(),
            ScoreboardMiss::PlayerRowNotFound
        );
        assert_eq!(
            read_scoreboard(&[], None, someone, "unknown", Some("   ")).unwrap_err(),
            ScoreboardMiss::PlayerRowNotFound
        );
    }

    #[test]
    fn a_join_line_does_not_hide_the_stat_line() {
        let raw = "FROZEN joined the game\nFROZEN 8 1 2 3,993 989 1,583";
        let read = read_scoreboard(&[], None, raw, "unknown", Some("FROZEN")).unwrap();
        assert!(!read.trusted_cells);
        assert_eq!(read.matched.elims, 8);
        assert_eq!(read.matched.damage, 3993);
        assert_eq!(read.matched.mitigation, 1583);
    }

    #[test]
    fn a_chat_line_does_not_hide_the_stat_line() {
        // The row's own line failed, and a chat line before it has six
        // numbers after the name. Those are not the scoreboard.
        let raw = "[Team] FROZEN: 1 2 3 4 5 6\nFROZEN 8 1 2 3,993 989 1,583";
        let read = read_scoreboard(&[], None, raw, "unknown", Some("FROZEN")).unwrap();
        assert_eq!(read.matched.elims, 8);
        assert_eq!(read.matched.assists, 1);
        assert_eq!(read.matched.damage, 3993);
        assert_eq!(read.matched.mitigation, 1583);
    }

    #[test]
    fn suffix_after_a_name_keeps_char_boundaries_when_casefold_widths_cancel() {
        // İ (U+0130) lowercases to two code points. K (U+212A) lowercases to
        // one byte. The old fast path treated equal byte lengths as aligned
        // indexes and sliced into the stats.
        let raw = "İİFROZEN K 12 0 0 100 200 300";
        let read = read_scoreboard(&[], None, raw, "unknown", Some("FROZEN")).unwrap();
        assert_eq!(read.matched.elims, 12);
        assert_eq!(read.matched.assists, 0);
        assert_eq!(read.matched.mitigation, 300);
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

    #[test]
    fn known_map_names_keep_the_table_spellings() {
        let names = known_map_names();
        assert_eq!(names.first().copied(), Some("King's Row"));
        assert_eq!(
            names.iter().filter(|name| **name == "Eichenwalde").count(),
            1
        );
        assert!(names.contains(&"Paraiso"));
        assert!(names.contains(&"Esperanca"));
        assert!(!names.contains(&"Paraíso"));
        assert!(!names.contains(&"Esperança"));
        assert!(names.contains(&"Watchpoint: Grímsvötn"));
        assert!(names.contains(&"Château Guillard"));
        assert!(names.contains(&"King's Row"));
        for name in &names {
            assert!(map_is_known(name), "{name}");
        }
    }

    #[test]
    fn blank_or_unknown_map_mode_and_hero_are_suspect() {
        assert_eq!(review_suspect_fields("", "", "Ana"), vec!["map", "mode"]);
        assert_eq!(
            review_suspect_fields("Not a map", "", "Ana"),
            vec!["map", "mode"]
        );
        assert_eq!(
            review_suspect_fields("Not a map", "Escort", "Ana"),
            vec!["map"]
        );
        assert_eq!(
            review_suspect_fields("Busan", "", "Ana"),
            Vec::<&str>::new()
        );
        assert_eq!(review_suspect_fields("Busan", "", "Unknown"), vec!["hero"]);
        assert_eq!(review_suspect_fields("Busan", "", "unknown"), vec!["hero"]);
        assert_eq!(review_suspect_fields("Busan", "", "  "), vec!["hero"]);
        assert!(review_suspect_fields("Busan", "", "Ana").is_empty());
        assert_eq!(uploaded_game_mode("Busan", ""), "Control");
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
    fn liang_tower_is_lijiang() {
        assert_eq!(
            match_map_in_text("LIANG TOWER").as_deref(),
            Some("Lijiang Tower")
        );
    }

    #[test]
    fn lulang_tower_is_lijiang() {
        assert_eq!(
            match_map_in_text("LULANG TOWER").as_deref(),
            Some("Lijiang Tower")
        );
        assert_eq!(
            match_map_in_text("Lulang tower").as_deref(),
            Some("Lijiang Tower")
        );
    }

    #[test]
    fn lang_tower_line_is_lijiang() {
        assert_eq!(
            match_map_in_text("Q control | Lang Tower Jjjf").as_deref(),
            Some("Lijiang Tower")
        );
    }

    #[test]
    fn llang_tower_line_is_lijiang() {
        assert_eq!(
            match_map_in_text("Q control | LlanG Tower Jjj").as_deref(),
            Some("Lijiang Tower")
        );
    }

    #[test]
    fn luang_tower_line_is_lijiang() {
        assert_eq!(
            match_map_in_text("Q control | Luang tower [jj").as_deref(),
            Some("Lijiang Tower")
        );
    }

    #[test]
    fn lulang_tower_line_is_lijiang() {
        assert_eq!(
            match_map_in_text("Q control | Lulang tower Jj").as_deref(),
            Some("Lijiang Tower")
        );
    }

    #[test]
    fn tab_banner_matches_the_map_side_and_ignores_the_timer() {
        // The crop used to include the start of the match timer, so the read
        // was `CONTROL | ILIOS TIM` and the folded bar glued onto the name.
        for raw in [
            "CONTROL | ILIOS TIM",
            "control | ilios tim",
            "Control | Ilios Tim",
            "CONTROL|ILIOS TIM",
            "CONTROL |ILIOS TIM",
            "Q CONTROL | ILIOS TIM",
            "@@ CONTROL | ILIOS TIM",
        ] {
            assert_eq!(match_map_in_text(raw).as_deref(), Some("Ilios"), "{raw}");
        }
        // No bar: junk before the mode word is not a map, even when that
        // junk is itself a map name earlier in the table than Ilios.
        assert_eq!(
            match_map_in_text("BUSAN CONTROL ILIOS TIM").as_deref(),
            Some("Ilios")
        );
        assert_eq!(match_map_in_text("CONTROL | TIM"), None);
        assert_eq!(match_map_in_text("TIM"), None);
    }

    #[test]
    fn tab_banner_keeps_anubis_and_antarctica_apart() {
        for (raw, display) in [
            ("ASSAULT | TEMPLE OF ANUBIS", "Temple of Anubis"),
            ("xqz assault | temple of anubis", "Temple of Anubis"),
            ("CLASH | THRONE OF ANUBIS", "Throne of Anubis"),
            ("clash | throne of anubis", "Throne of Anubis"),
            ("CLASH | ANUBIS", "Throne of Anubis"),
            ("ELIMINATION | ECOPOINT: ANTARCTICA", "Ecopoint: Antarctica"),
            ("elimination | ecopoint: antarctica", "Ecopoint: Antarctica"),
            ("ELIMINATION | ANTARCTICA", "Ecopoint: Antarctica"),
            ("CONTROL | ANTARCTIC PENINSULA", "Antarctic Peninsula"),
            ("control | antarctic peninsula", "Antarctic Peninsula"),
            ("CONTROL | ANTARCTIC", "Antarctic Peninsula"),
            (
                "OASIS ELIMINATION ECOPOINT: ANTARCTICA",
                "Ecopoint: Antarctica",
            ),
            ("FLASHPOINT NEW JUNK CITY", "New Junk City"),
        ] {
            assert_eq!(match_map_in_text(raw).as_deref(), Some(display), "{raw}");
        }
    }

    #[test]
    fn lijiang_ocr_aliases_do_not_steal_other_maps() {
        let liang = normalize_ocr_glyphs("liang tower");
        let lulang = normalize_ocr_glyphs("lulang tower");
        for &(display, pattern) in MAPS {
            if pattern == "liang tower" || pattern == "lulang tower" {
                continue;
            }
            assert_eq!(
                match_map_in_text(display).as_deref(),
                Some(display),
                "{display} no longer resolves to itself"
            );
            let folded = normalize_ocr_glyphs(&display.to_lowercase());
            assert!(
                !folded.contains(&liang),
                "{display} contains the LIANG TOWER alias"
            );
            assert!(
                !folded.contains(&lulang),
                "{display} contains the LULANG TOWER alias"
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
    fn anubis_and_antarctica_names_stay_on_their_own_maps() {
        // Stored spellings of the maps that were already in the table do
        // not move. Esperanca and Paraiso stay unaccented.
        assert_eq!(canonical_map("Esperanca").as_deref(), Some("Esperanca"));
        assert_eq!(canonical_map("Esperança").as_deref(), Some("Esperanca"));
        assert_eq!(canonical_map("Paraiso").as_deref(), Some("Paraiso"));
        assert_eq!(canonical_map("Paraíso").as_deref(), Some("Paraiso"));
        assert_eq!(
            canonical_map("Throne of Anubis").as_deref(),
            Some("Throne of Anubis")
        );
        assert_eq!(
            canonical_map("Antarctic Peninsula").as_deref(),
            Some("Antarctic Peninsula")
        );

        for (raw, display, mode) in [
            ("Temple of Anubis", "Temple of Anubis", "Assault"),
            ("TEMPLE OF ANUBIS", "Temple of Anubis", "Assault"),
            ("temple of anubis", "Temple of Anubis", "Assault"),
            ("Throne of Anubis", "Throne of Anubis", "Clash"),
            ("THRONE OF ANUBIS", "Throne of Anubis", "Clash"),
            ("ANUBIS", "Throne of Anubis", "Clash"),
            (
                "Ecopoint: Antarctica",
                "Ecopoint: Antarctica",
                "Elimination",
            ),
            (
                "ECOPOINT: ANTARCTICA",
                "Ecopoint: Antarctica",
                "Elimination",
            ),
            ("ANTARCTICA", "Ecopoint: Antarctica", "Elimination"),
            ("ECOPOINT", "Ecopoint: Antarctica", "Elimination"),
            ("Antarctic Peninsula", "Antarctic Peninsula", "Control"),
            ("ANTARCTIC PENINSULA", "Antarctic Peninsula", "Control"),
            ("ANTARCTIC", "Antarctic Peninsula", "Control"),
        ] {
            let name = canonical_map(raw).unwrap_or_else(|| panic!("{raw}"));
            assert_eq!(name, display, "{raw}");
            assert_eq!(map_mode(&name), Some(mode), "{raw}");
            assert_eq!(
                exact_map_in_text(raw).as_deref(),
                Some(display),
                "accolade {raw}"
            );
        }
    }

    #[test]
    fn maps_missing_from_the_ocr_table_canonicalize_with_their_mode() {
        // Assault, Elimination, Capture the Flag, Payload Race, Workshop,
        // and Stadium stay local. `tracked` is false for every one of them.
        let cases = [
            ("Hanamura", "Hanamura", "Assault", false),
            ("HANAMURA", "Hanamura", "Assault", false),
            (
                "Horizon Lunar Colony",
                "Horizon Lunar Colony",
                "Assault",
                false,
            ),
            ("HORIZON", "Horizon Lunar Colony", "Assault", false),
            ("LUNAR COLONY", "Horizon Lunar Colony", "Assault", false),
            ("Paris", "Paris", "Assault", false),
            ("PARIS", "Paris", "Assault", false),
            (
                "Volskaya Industries",
                "Volskaya Industries",
                "Assault",
                false,
            ),
            ("VOLSKAYA", "Volskaya Industries", "Assault", false),
            ("Black Forest", "Black Forest", "Elimination", false),
            ("Castillo", "Castillo", "Elimination", false),
            ("Necropolis", "Necropolis", "Elimination", false),
            ("Ayutthaya", "Ayutthaya", "Capture the Flag", false),
            ("Arena Victoriae", "Arena Victoriae", "Stadium", false),
            ("VICTORIAE", "Arena Victoriae", "Stadium", false),
            ("Gogadoro", "Gogadoro", "Stadium", false),
            (
                "Wuxing University - Water College",
                "Wuxing University - Water College",
                "Stadium",
                false,
            ),
            (
                "WUXING",
                "Wuxing University - Water College",
                "Stadium",
                false,
            ),
            (
                "WATER COLLEGE",
                "Wuxing University - Water College",
                "Stadium",
                false,
            ),
            ("Place Lacroix", "Place Lacroix", "Stadium", false),
            ("LACROIX", "Place Lacroix", "Stadium", false),
            ("Redwood Dam", "Redwood Dam", "Stadium", false),
            ("REDWOOD", "Redwood Dam", "Stadium", false),
            ("Serenza", "Serenza", "Stadium", false),
            ("Powder Keg Mine", "Powder Keg Mine", "Payload Race", false),
            ("POWDER KEG MINES", "Powder Keg Mine", "Payload Race", false),
            ("Thames District", "Thames District", "Payload Race", false),
            ("THAMES", "Thames District", "Payload Race", false),
            ("Workshop Chamber", "Workshop Chamber", "Workshop", false),
            ("Workshop Expanse", "Workshop Expanse", "Workshop", false),
            (
                "Workshop Green Screen",
                "Workshop Green Screen",
                "Workshop",
                false,
            ),
            ("Workshop Island", "Workshop Island", "Workshop", false),
            ("Kanezaka", "Kanezaka", "Deathmatch", false),
            ("Malevento", "Malevento", "Deathmatch", false),
            ("Petra", "Petra", "Deathmatch", false),
            ("Practice Range", "Practice Range", "Practice", false),
            ("PRACTICE RANGE", "Practice Range", "Practice", false),
            ("PRACTICE", "Practice Range", "Practice", false),
            ("Mastery Course", "Mastery Course", "Practice", false),
            ("Tutorial", "Tutorial", "Practice", false),
        ];
        for (raw, display, mode, tracked) in cases {
            let name = canonical_map(raw).unwrap_or_else(|| panic!("{raw}"));
            assert_eq!(name, display, "{raw}");
            assert_eq!(map_mode(&name), Some(mode), "{raw}");
            assert_eq!(map_is_untracked(&name), !tracked, "{raw}");
            assert_eq!(stats_row_is_tracked(&name, mode), tracked, "{raw}");
        }
        // Hanaoka is not Hanamura. Paraiso is not Paris.
        assert_eq!(canonical_map("Hanaoka").as_deref(), Some("Hanaoka"));
        assert_eq!(canonical_map("HANAOKA").as_deref(), Some("Hanaoka"));
        assert_eq!(canonical_map("Paraiso").as_deref(), Some("Paraiso"));
        assert_eq!(canonical_map("PARAISO").as_deref(), Some("Paraiso"));
        assert!(!stats_row_is_tracked("Hanamura", "Assault"));
        assert!(!stats_row_is_tracked("Practice Range", "Practice"));
        assert!(!stats_row_is_tracked("Busan", "Practice"));
        assert!(stats_row_is_tracked("Busan", "Control"));
        assert!(stats_row_is_tracked("King's Row", "Hybrid"));
    }

    #[test]
    fn arcade_and_stadium_modes_stay_local() {
        for (map, mode) in [
            ("Hanamura", "Assault"),
            ("Temple of Anubis", "Assault"),
            ("Black Forest", "Elimination"),
            ("Ecopoint: Antarctica", "Elimination"),
            ("Ayutthaya", "Capture the Flag"),
            ("Powder Keg Mine", "Payload Race"),
            ("Thames District", "Payload Race"),
            ("Workshop Chamber", "Workshop"),
            ("Arena Victoriae", "Stadium"),
            ("Place Lacroix", "Stadium"),
            ("Château Guillard", "Deathmatch"),
            ("Practice Range", "Practice"),
        ] {
            assert_eq!(map_mode(map), Some(mode), "{map}");
            assert!(map_is_untracked(map), "{map}");
            assert!(!stats_row_is_tracked(map, mode), "{map}");
            assert!(
                review_suspect_fields(map, mode, "Ana").is_empty(),
                "{map} is a known map with a mode, so the review hold does not keep it"
            );
            assert!(
                !stats_row_is_tracked("Busan", mode),
                "{mode} on another map stays local"
            );
        }
    }

    #[test]
    fn short_map_keys_match_whole_words_and_do_not_block_a_real_upload() {
        // "paris" is a letter run inside "comparison".
        assert_eq!(canonical_map("comparison"), None);
        assert_eq!(canonical_map("COMPARISON").as_deref(), None);
        let ilios = canonical_map("Ilios comparison").expect("ilios");
        assert_eq!(ilios, "Ilios");
        assert!(stats_row_is_tracked(&ilios, &stored_game_mode(&ilios)));

        // "petra" inside a longer word must not become the deathmatch map.
        assert_eq!(canonical_map("competra"), None);
        let kings = canonical_map("King's Row competra").expect("kings");
        assert_eq!(kings, "King's Row");
        assert!(stats_row_is_tracked(&kings, "Hybrid"));

        // "practice" inside a longer word must not become Practice Range.
        assert_eq!(canonical_map("inpractice"), None);
        let busan = canonical_map("Busan inpractice").expect("busan");
        assert_eq!(busan, "Busan");
        assert!(stats_row_is_tracked(&busan, "Control"));

        // The keys still match when they are the whole word.
        assert_eq!(canonical_map("PARIS").as_deref(), Some("Paris"));
        assert_eq!(canonical_map("PETRA").as_deref(), Some("Petra"));
        assert_eq!(canonical_map("PRACTICE").as_deref(), Some("Practice Range"));
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
    fn fuzzy_map_holds_junk_and_close_scores_and_keeps_canonical_names() {
        // Below the floor: not the closest name.
        for raw in ["PARXQ", "BUSXY", "TIM", "zzzzz", "not-a-map", "qqqqq"] {
            assert_eq!(match_map_in_text(raw), None, "{raw}");
            assert_eq!(exact_map_in_text(raw), None, "{raw}");
        }
        // One edit still clears the short-name floor and has no close second.
        assert_eq!(match_map_in_text("PARIZ").as_deref(), Some("Paris"));
        assert_eq!(match_map_in_text("BUSVN").as_deref(), Some("Busan"));

        // Oasis and Paris both score 0.80 on these. Antarctic and Antarctica
        // tie, or lead by less than the margin. Either way the map is unknown.
        for raw in ["oaris", "pasis", "antarcticx", "antarctia"] {
            assert_eq!(match_map_in_text(raw), None, "{raw}");
        }

        // Timer digits stuck on the end of the banner name.
        for raw in [
            "ILIOS 0",
            "ILIOS 7/M",
            "CONTROL | ILIOS 0",
            "CONTROL | ILIOS 7/M",
            "CONTROL|ILIOS 0",
        ] {
            assert_eq!(match_map_in_text(raw).as_deref(), Some("Ilios"), "{raw}");
        }
        // I/L fold, and a single u/i swap that still clears the floor alone.
        assert_eq!(match_map_in_text("ILios").as_deref(), Some("Ilios"));
        assert_eq!(match_map_in_text("iuios").as_deref(), Some("Ilios"));

        // #201: a short key does not match inside a longer word.
        for raw in ["comparison", "parisian", "inparis", "COMPARISON"] {
            assert_eq!(canonical_map(raw), None, "{raw}");
        }
        assert_eq!(canonical_map("PARIS").as_deref(), Some("Paris"));

        let grim = canonical_map("GRIMSVOTN").expect("grimsvotn");
        assert_eq!(grim, "Watchpoint: Grímsvötn");
        assert!(grim.contains('í') && grim.contains('ö'), "{grim}");
        let chateau = canonical_map("Chateau Guillard").expect("chateau");
        assert_eq!(chateau, "Château Guillard");
        assert!(chateau.contains('â'), "{chateau}");
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
