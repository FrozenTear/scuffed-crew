//! Saved-stat merge for `reader = "new"`.
//!
//! Under `reader = "new"` the six stats of the member's row come from the
//! digit reader only (alpha.7 O10). The capture path refuses a board this
//! reader cannot vouch for ([`board_refusal`], [`read_own_row_stats`]) and stores
//! nothing from that Tab. On an accepted board each stat cell is either
//! sure, and stored, or held ([`crate::shadow::confirm::CellTrust`]): a
//! suspect or empty digit read is never stored and no OCR number takes its
//! place. A held cell keeps the game's last accepted value through the
//! capture gate (`capture_gate::apply_gate_held`), is listed in
//! `suspect_fields`, and is replaced by the next sure read. A column the
//! game has never read has no value to keep, and a placeholder is not a
//! number: the capture path stores no row for the game until every column
//! has had a sure read (`capture_gate::GateState::unread_cols`).
//!
//! [`merge_saved`] then chooses the values stored on the member's own row.
//! It starts from the row capture policy built ([`PolicyRow`]: the gated
//! stats, the policy map and mode, the hero authority's hero, the game's
//! result). A stat, hero, or result from [`BoardRead`] is used when that
//! field was read and is not suspect. Anything else keeps the policy row's
//! value. A field is listed in `suspect_fields` only when the new reader
//! actually attempted it and did not supply a value to store.
//!
//! The map and mode already chosen by capture policy stay as they are. Under
//! `reader = "new"` that policy map already comes from the map pack: the
//! capture path takes a sure banner read as the top-bar map
//! (`boundary::MapSource::TopBar`, see `choose_top_bar_map` in main.rs) and
//! uses the top-bar OCR only when the banner reader abstains. The policy then
//! keeps a session's first trusted map, applies the vote-candidate veto, and
//! keeps untracked maps local, exactly as for the OCR label. A sure banner read
//! may also replace a text-fallback session map while that game shows no
//! boundary signal (`banner_replaces_text_fallback` in main.rs). A confident or suspect
//! board map that names a different map adds `map` to `suspect_fields` and
//! does not replace the policy map or mode.
//!
//! [`merge_saved`] keeps the whole policy row, with recognizer `ocr-v1` and
//! no suspect list, when the board is missing, the team size is unknown or
//! disagrees, the member's own row was not identified, the row index is
//! outside the board, or the board is marked read but has no digit values
//! (the digit pass can miss its time budget after the columns are found).
//! The capture path refuses every one of those boards before it gets here,
//! so under `reader = "new"` that branch stores nothing; it stays as the
//! switch-off answer and as a guard. A blank map, a hero key that is not
//! the display name, and a hero that came from the career panel or the
//! portrait matcher are per-field: confident digits stay on this reader.
//!
//! The recognizer tag describes the stat, hero and result fields only, never
//! the map: a `cv-v5` row may carry a map from the top-bar OCR.

use scuffed_types::{RECOGNIZER_OCR_V1, SUSPECT_FIELD_NAMES};

use crate::parse::ScoreboardMiss;
use crate::shadow::confirm::{CellTrust, cell_trust};
use crate::shadow::digits::RECOGNIZER_ID;
use crate::shadow::{BoardRead, BoardStatus, FieldRead, Value};
use crate::storage::PersonalMatch;

/// The six stat fields of a board row, in capture gate column order.
pub const STAT_SUFFIXES: [&str; 6] = ["e", "a", "d", "dmg", "h", "mit"];

/// The row capture policy built for this Tab, after holds and map policy:
/// what is stored for every field the new reader does not supply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyRow {
    pub map: String,
    pub mode: String,
    pub result: String,
    pub hero: String,
    pub elims: u32,
    pub assists: u32,
    pub deaths: u32,
    pub damage: u32,
    pub healing: u32,
    pub mitigation: u32,
}

/// Whether ocr-v1 confidently identified the member's own row.
///
/// `Identified` is that row's index on a board of `team_size`. The new
/// reader's row index agrees only when its measured team size is the same
/// and the index falls inside `0..2 * team_size`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnRow {
    Uncertain,
    Identified { index: usize, team_size: usize },
}

/// Values to store, plus the recognizer tag and flat suspect names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedRead {
    pub map: String,
    pub mode: String,
    pub result: String,
    pub hero: String,
    pub elims: u32,
    pub assists: u32,
    pub deaths: u32,
    pub damage: u32,
    pub healing: u32,
    pub mitigation: u32,
    pub recognizer: &'static str,
    pub suspect_fields: Vec<String>,
}

/// Own row from the same match ocr-v1 uses.
///
/// A configured player name wins when that name matches a row. When it does
/// not, the brightness row ocr-v1 already stored is the own row. The row is
/// uncertain only when neither one names a row. A name miss after the
/// highlight row is known is not a failed board: that used to drop every
/// confident digit and tag the save `ocr-v1` with an empty suspect list.
pub fn own_row(
    player_name: Option<&str>,
    name_match: Option<usize>,
    fallback_row: Option<usize>,
    team_size: usize,
) -> OwnRow {
    let index = if player_name_is_set(player_name) {
        name_match.or(fallback_row)
    } else {
        fallback_row
    };
    match index {
        Some(index) => OwnRow::Identified { index, team_size },
        None => OwnRow::Uncertain,
    }
}

/// A player name is configured and is not blank. [`own_row`] takes a name
/// match only then. The daemon's one call passes no name: under
/// `reader = "new"` the row is `detect::own_row`'s, and ocr-v1 matches the
/// name in the vision pass.
pub fn player_name_is_set(player_name: Option<&str>) -> bool {
    player_name.is_some_and(|name| !name.trim().is_empty())
}

/// Why the capture path stores nothing from this board under
/// `reader = "new"`, before the member's row is looked at. `None` is a board
/// the digit reader vouches for: found, with the capture path's team size,
/// and with digit values (the reader's own check, 3 rows with 4 read cells,
/// is behind [`BoardStatus::Read`]). This is the trust gate ocr-v1 runs on
/// its cell text (`parse::looks_like_scoreboard`, OCR site T3).
pub fn board_refusal(board: &BoardRead, team_size: usize) -> Option<&'static str> {
    if let Some(reason) = board_fallback_reason(board) {
        return Some(reason);
    }
    match board.team_size {
        None => Some("team size is missing"),
        Some(read_size) if read_size != team_size => Some("team size disagrees"),
        Some(_) => None,
    }
}

/// The stats of the member's row off an accepted board ([`board_refusal`]
/// is `None`). Which row that is, and how sure, is not decided here: that
/// is `detect::own_row` (`OwnRow`, `OwnRowRead`, `read_own_row`), whose
/// names these are kept apart from.
#[derive(Debug)]
pub struct OwnRowStats {
    /// A fresh row with the sure stats. A held cell is 0 here: the capture
    /// gate puts the game's last accepted value in its place.
    pub matched: PersonalMatch,
    /// Per stat column: the digit reader did not vouch for the cell.
    pub held: [bool; 6],
}

/// Trust in each stat cell of board row `index`, in gate column order. A
/// field the board does not have is held.
pub fn row_cells(board: &BoardRead, index: usize) -> [CellTrust; 6] {
    STAT_SUFFIXES.map(|suffix| match board.get(&format!("r{index}.{suffix}")) {
        Some(field) => match field.value {
            Some(Value::Int(n)) => cell_trust(Some(n), field.suspect),
            _ => CellTrust::Held,
        },
        None => CellTrust::Held,
    })
}

/// Sure stat cells a row needs before it is read at all: the same four the
/// team rule ([`both_teams_have_stats`]) and the reader's own board check
/// ask for. Under it a single cell just above the reader's suspect line
/// would be the only read of the row (F266-6).
pub const MIN_SURE_CELLS: usize = 4;

/// Read the member's stats off an accepted board, with no OCR text.
///
/// The refusals are ocr-v1's two (`parse::read_scoreboard`), decided on the
/// digit reads: no identified row is [`ScoreboardMiss::PlayerRowNotFound`],
/// and a row with fewer than [`MIN_SURE_CELLS`] sure cells is
/// [`ScoreboardMiss::CellsUnreadable`]. There is no third path: a row is
/// never rebuilt from text.
pub fn read_own_row_stats(
    board: &BoardRead,
    own: OwnRow,
    outcome: &str,
) -> Result<OwnRowStats, ScoreboardMiss> {
    let OwnRow::Identified { index, team_size } = own else {
        return Err(ScoreboardMiss::PlayerRowNotFound);
    };
    if index >= team_size.saturating_mul(2) {
        return Err(ScoreboardMiss::PlayerRowNotFound);
    }
    let cells = row_cells(board, index);
    if sure_cells(&cells) < MIN_SURE_CELLS {
        return Err(ScoreboardMiss::CellsUnreadable);
    }
    let stats = cells.map(|cell| match cell {
        CellTrust::Sure(n) => n,
        CellTrust::Held => 0,
    });
    Ok(OwnRowStats {
        matched: crate::parse::match_from_stats(stats, outcome),
        held: cells.map(|cell| cell == CellTrust::Held),
    })
}

/// Both teams show a row with four sure stat cells: ocr-v1's
/// `parse::both_teams_have_stats` on the digit reads.
pub fn both_teams_have_stats(board: &BoardRead, team_size: usize) -> bool {
    let plausible = |row: usize| sure_cells(&row_cells(board, row)) >= MIN_SURE_CELLS;
    team_size > 0 && (0..team_size).any(plausible) && (team_size..2 * team_size).any(plausible)
}

fn sure_cells(cells: &[CellTrust; 6]) -> usize {
    cells
        .iter()
        .filter(|cell| matches!(cell, CellTrust::Sure(_)))
        .count()
}

/// Merge one accepted board.
///
/// `board == None` is the switch off: the policy row is returned unchanged,
/// tagged `ocr-v1`, with no suspect list.
pub fn merge_saved(policy: &PolicyRow, own: OwnRow, board: Option<&BoardRead>) -> SavedRead {
    merge_saved_with_authority(policy, own, board, None)
}

/// [`merge_saved`] with the Tab's hero authority
/// ([`crate::shadow::TabHeroes::authority`] for the own row, `Some(None)`
/// when it abstained). The own row's hero is taken only when the authority
/// names that same hero: the authority is the own row's read checked against
/// the career panel's, so a row read the panel contradicts keeps the earlier
/// hero and marks `hero` suspect. `None` (no authority known) behaves as
/// [`merge_saved`].
pub fn merge_saved_with_authority(
    policy: &PolicyRow,
    own: OwnRow,
    board: Option<&BoardRead>,
    authority: Option<Option<&str>>,
) -> SavedRead {
    let keep = || SavedRead {
        map: policy.map.clone(),
        mode: policy.mode.clone(),
        result: policy.result.clone(),
        hero: canonical_board_hero(&policy.hero),
        elims: policy.elims,
        assists: policy.assists,
        deaths: policy.deaths,
        damage: policy.damage,
        healing: policy.healing,
        mitigation: policy.mitigation,
        recognizer: RECOGNIZER_OCR_V1,
        suspect_fields: Vec::new(),
    };
    let Some(board) = board else {
        return keep();
    };
    if let Some(reason) = full_fallback_reason(board, own) {
        tracing::info!(reason, "new reader full fallback");
        return keep();
    }
    let OwnRow::Identified { index, .. } = own else {
        return keep();
    };

    let mut suspect = Vec::new();
    let mut used_cv = false;
    // Capture policy already chose the map and mode (including Deathmatch,
    // which must stay local). The board may disagree, and that is the only
    // reason `map` is marked. The stored mode stays with the policy map.
    let map = policy.map.clone();
    let mode = policy.mode.clone();
    if let Some(board_map) = text_on(board, "map")
        && board_map.trim() != policy.map.trim()
    {
        push_suspect(&mut suspect, "map");
    }
    let result = match take_text(board, "result", &mut suspect, "result") {
        Some(value) => {
            used_cv = true;
            value
        }
        None => policy.result.clone(),
    };
    let hero_name = format!("r{index}.hero");
    let hero = match take_text(board, &hero_name, &mut suspect, "hero") {
        // A placeholder icon is an empty or unknown slot. It is not a hero
        // read, so the career panel or portrait hero already on the snapshot
        // stays.
        Some(value) if scuffed_types::is_placeholder_hero(&value) => {
            canonical_board_hero(&policy.hero)
        }
        Some(value)
            if authority.is_some_and(|a| {
                a.is_none_or(|a| canonical_board_hero(a) != canonical_board_hero(&value))
            }) =>
        {
            push_suspect(&mut suspect, "hero");
            canonical_board_hero(&policy.hero)
        }
        Some(value) => {
            used_cv = true;
            canonical_board_hero(&value)
        }
        None => canonical_board_hero(&policy.hero),
    };
    let stats = [
        ("e", policy.elims),
        ("a", policy.assists),
        ("d", policy.deaths),
        ("dmg", policy.damage),
        ("h", policy.healing),
        ("mit", policy.mitigation),
    ];
    // A stat the digit reader does not vouch for keeps the policy row's
    // number. Under `reader = "new"` that number is the capture gate's held
    // value (the game's last accepted one), never an OCR read.
    let row_attempted = stat_row_attempted(board, index);
    let mut numbers = [0u32; 6];
    for (slot, (suffix, fallback)) in stats.into_iter().enumerate() {
        let name = format!("r{index}.{suffix}");
        match take_int(board, &name, &mut suspect, suffix, row_attempted) {
            Some(value) => {
                used_cv = true;
                numbers[slot] = value;
            }
            None => numbers[slot] = fallback,
        }
    }
    SavedRead {
        map,
        mode,
        result,
        hero,
        elims: numbers[0],
        assists: numbers[1],
        deaths: numbers[2],
        damage: numbers[3],
        healing: numbers[4],
        mitigation: numbers[5],
        // One tag for the whole row. cv-v5 when any stored field came from
        // this reader. ocr-v1 only when every field fell back.
        recognizer: if used_cv {
            RECOGNIZER_ID
        } else {
            RECOGNIZER_OCR_V1
        },
        suspect_fields: dimmed_board_suspects(board, suspect),
    }
}

/// The suspect list of a row read off `board`. A board whose columns were
/// found only on a dimmed header strip ([`BoardRead::header_dimmed`]) still
/// saves its numbers, but every own-row field is listed, the same mark
/// [`mark_own_row_suspect`] gives a row only one signal named: the read must
/// not look as solid as a clean one. The mark applies when the stored row
/// came from this reader and when every field fell back to ocr-v1. A clean
/// board's list is unchanged.
fn dimmed_board_suspects(board: &BoardRead, suspect: Vec<String>) -> Vec<String> {
    let mut fields = order_suspects(suspect);
    if board.header_dimmed {
        mark_own_row_suspect(&mut fields);
    }
    fields
}

/// The digit reader's number for each own-row stat (`e a d dmg h mit`),
/// sure or not: the best guess for a column that never gets a sure read
/// (alpha.8). `None` where the cell has no number or the row is unknown.
pub fn own_row_guesses(board: &BoardRead, own: OwnRow) -> [Option<u32>; 6] {
    let OwnRow::Identified { index, .. } = own else {
        return [None; 6];
    };
    std::array::from_fn(|slot| {
        let suffix = ["e", "a", "d", "dmg", "h", "mit"][slot];
        match board.get(&format!("r{index}.{suffix}"))?.value {
            Some(Value::Int(n)) => Some(n),
            _ => None,
        }
    })
}

/// Flat suspect names safe to upload.
///
/// Row names such as `r3.dmg` become `dmg`. Unknown names, duplicates, and
/// anything that is not in the server's allowlist are dropped. Order follows
/// [`SUSPECT_FIELD_NAMES`].
pub fn upload_suspect_fields(stored: &[String]) -> Vec<String> {
    let mut seen = [false; SUSPECT_FIELD_NAMES.len()];
    for name in stored {
        let Some(flat) = flat_suspect_name(name) else {
            continue;
        };
        if let Some(idx) = SUSPECT_FIELD_NAMES
            .iter()
            .position(|allowed| *allowed == flat)
        {
            seen[idx] = true;
        }
    }
    SUSPECT_FIELD_NAMES
        .iter()
        .enumerate()
        .filter(|(idx, _)| seen[*idx])
        .map(|(_, name)| (*name).to_string())
        .collect()
}

/// The fields read off the member's own row: the hero and the six stats.
/// The map, the mode and the result are the same on every row.
pub const OWN_ROW_FIELDS: [&str; 7] = ["hero", "e", "a", "d", "dmg", "h", "mit"];

/// Mark every own-row field suspect.
///
/// `reader = "new"` finds the own row from the own-name seeds, the
/// highlight and the career hero's row ([`crate::detect::own_row`]). When
/// only one of them named the row where two could have, or the career hero
/// alone did, the row is still used, but the values on it may be another
/// player's: each of them is marked. The upload allowlist
/// has no name for the row itself, so the row's fields carry the mark.
pub fn mark_own_row_suspect(fields: &mut Vec<String>) {
    for name in OWN_ROW_FIELDS {
        push_suspect(fields, name);
    }
    *fields = upload_suspect_fields(fields);
}

/// Drop suspect names cleared by a manual edit.
///
/// `storage_field` is the tracker's edit name (`elims`, `map_name`, ...).
/// Both the flat API name and a leftover `rN.` form of that name are removed.
pub fn clear_edited_suspects(fields: &mut Vec<String>, storage_field: &str) {
    let drop_names = suspect_names_for_edit(storage_field);
    if drop_names.is_empty() {
        return;
    }
    fields.retain(|name| {
        let Some(flat) = flat_suspect_name(name) else {
            return true;
        };
        !drop_names.contains(&flat)
    });
}

/// Why a provided board cannot replace any field.
///
/// `None` means per-field merge. A blank map, a hero template key, and which
/// older source named the hero are not reasons: those fields fall back on
/// their own and confident digits keep [`RECOGNIZER_ID`].
fn full_fallback_reason(board: &BoardRead, own: OwnRow) -> Option<&'static str> {
    if let Some(reason) = board_fallback_reason(board) {
        return Some(reason);
    }
    let OwnRow::Identified { index, team_size } = own else {
        return Some("own row is uncertain");
    };
    let Some(read_size) = board.team_size else {
        return Some("team size is missing");
    };
    if read_size != team_size {
        return Some("team size disagrees");
    }
    if index >= team_size.saturating_mul(2) {
        return Some("row index is outside the board");
    }
    None
}

/// Why a Tab under `reader = "new"` is read with ocr-v1 instead, if it is:
/// the reader found nothing on the board (not found, team size unknown, no
/// digit values) and classic OCR may run (`ocr_off` false). A board it found
/// and doubts (a team size that disagrees, an unsure own row) is not handed
/// to OCR: it is refused or held as before. With OCR off nothing falls back.
pub fn ocr_fallback_reason(board: &BoardRead, ocr_off: bool) -> Option<&'static str> {
    if ocr_off {
        return None;
    }
    board_fallback_reason(board)
}

/// Why the board itself gave no read: it was not found, its team size is
/// unknown, or it has no digit value.
fn board_fallback_reason(board: &BoardRead) -> Option<&'static str> {
    match board.status {
        BoardStatus::Read => {}
        BoardStatus::NotFound => return Some("board was not found"),
        BoardStatus::TeamSizeUnknown => return Some("team size is unknown"),
    }
    if !has_digit_value(board) {
        return Some("read board has no digit values");
    }
    None
}

/// Template keys (`wrecking-ball`, `wrecking_ball`) store the display name.
///
/// A key and the career-panel name are the same hero. Leaving the key on the
/// row made cv-v5 saves and later ocr-v1 saves look like two heroes, and a
/// raw string compare is not a reason to drop the board.
fn canonical_board_hero(raw: &str) -> String {
    crate::parse::canonical_hero(raw)
}

fn suspect_names_for_edit(storage_field: &str) -> &'static [&'static str] {
    match storage_field {
        "hero" => &["hero"],
        "map_name" => &["map", "mode"],
        "outcome" => &["result"],
        "elims" => &["e"],
        "assists" => &["a"],
        "deaths" => &["d"],
        "damage" => &["dmg"],
        "healing" => &["h"],
        "mitigation" => &["mit"],
        _ => &[],
    }
}

fn flat_suspect_name(name: &str) -> Option<&str> {
    if SUSPECT_FIELD_NAMES.contains(&name) {
        return Some(name);
    }
    let (row, field) = name.split_once('.')?;
    let digits = row.strip_prefix('r')?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    SUSPECT_FIELD_NAMES.contains(&field).then_some(field)
}

/// Non-empty text on a field, including a suspect read. Missing and blank
/// fields are not a second map name.
fn text_on(board: &BoardRead, name: &str) -> Option<String> {
    match board.get(name).and_then(|field| field.value.clone()) {
        Some(Value::Text(text)) if !text.trim().is_empty() => Some(text),
        _ => None,
    }
}

fn take_text(
    board: &BoardRead,
    name: &str,
    suspect: &mut Vec<String>,
    flat: &str,
) -> Option<String> {
    let field = board.get(name)?;
    if let Some(Value::Text(text)) = field.value.clone()
        && !field.suspect
        && !text.trim().is_empty()
    {
        return Some(text);
    }
    // Unread placeholders (no value, confidence 0, suspect) were not
    // attempted: no hero pack, no result evidence, no map pack.
    if field_attempted(field) {
        push_suspect(suspect, flat);
    }
    None
}

fn take_int(
    board: &BoardRead,
    name: &str,
    suspect: &mut Vec<String>,
    flat: &str,
    row_attempted: bool,
) -> Option<u32> {
    let field = board.get(name)?;
    if let Some(Value::Int(n)) = field.value
        && !field.suspect
    {
        return Some(n);
    }
    if row_attempted {
        push_suspect(suspect, flat);
    }
    None
}

/// At least one stat cell has a number. Unread placeholders, including a
/// `Read` board whose digit pass returned no values, do not.
fn has_digit_value(board: &BoardRead) -> bool {
    board
        .fields
        .iter()
        .any(|field| matches!(field.value, Some(Value::Int(_))) && is_stat_field(&field.name))
}

fn is_stat_field(name: &str) -> bool {
    let Some((row, suffix)) = name.split_once('.') else {
        return false;
    };
    let Some(digits) = row.strip_prefix('r') else {
        return false;
    };
    !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
        && matches!(suffix, "e" | "a" | "d" | "dmg" | "h" | "mit")
}

/// The digit reader ran on this row. A row of unread placeholders did not.
fn stat_row_attempted(board: &BoardRead, index: usize) -> bool {
    ["e", "a", "d", "dmg", "h", "mit"].iter().any(|suffix| {
        board
            .get(&format!("r{index}.{suffix}"))
            .is_some_and(field_attempted)
    })
}

/// The new reader's confidence for each report kit read key (`mode`,
/// `hero`, `e`, `a`, `d`, `dmg`, `h`, `mit`) of the own row. A field the
/// reader did not attempt has no entry. `result` is not on the board until
/// [`crate::shadow::result_field`] sets it, so the caller adds it.
/// Each entry carries the reader's own value, so the kit can tell a read
/// the capture gate replaced from one it kept.
pub fn kit_confidences(
    board: &BoardRead,
    own: OwnRow,
) -> std::collections::BTreeMap<String, crate::report::kit::ReaderRead> {
    let mut out = std::collections::BTreeMap::new();
    let mut put = |key: &str, name: &str| {
        if let Some(f) = board.get(name).filter(|f| field_attempted(f)) {
            out.insert(
                key.to_string(),
                crate::report::kit::ReaderRead::from_field(f),
            );
        }
    };
    put("mode", "mode");
    if let OwnRow::Identified { index, .. } = own {
        put("hero", &format!("r{index}.hero"));
        for suffix in STAT_SUFFIXES {
            put(suffix, &format!("r{index}.{suffix}"));
        }
    }
    out
}

/// A field the new reader wrote, as opposed to the unread placeholder
/// (`value` none, confidence 0, suspect).
fn field_attempted(field: &FieldRead) -> bool {
    field.value.is_some() || field.confidence > 0.0 || !field.suspect
}

fn push_suspect(suspect: &mut Vec<String>, flat: &str) {
    if SUSPECT_FIELD_NAMES.contains(&flat) && !suspect.iter().any(|name| name == flat) {
        suspect.push(flat.to_string());
    }
}

fn order_suspects(names: Vec<String>) -> Vec<String> {
    upload_suspect_fields(&names)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shadow::{FieldRead, Value};

    /// Only a board the new reader found nothing on falls back to ocr-v1,
    /// and only while classic OCR may run.
    #[test]
    fn ocr_fallback_only_for_a_board_found_empty_with_ocr_on() {
        let not_found = crate::shadow::reader::not_found(BoardStatus::NotFound);
        let size_unknown = crate::shadow::reader::not_found(BoardStatus::TeamSizeUnknown);
        let no_digits = BoardRead {
            status: BoardStatus::Read,
            team_size: Some(5),
            fields: vec![FieldRead {
                name: "r0.e".into(),
                value: None,
                confidence: 0.0,
                suspect: true,
            }],
            elapsed_ms: 1,
            header_dimmed: false,
        };
        let read = BoardRead {
            status: BoardStatus::Read,
            team_size: Some(5),
            fields: vec![FieldRead {
                name: "r0.e".into(),
                value: Some(Value::Int(7)),
                confidence: 0.9,
                suspect: false,
            }],
            elapsed_ms: 1,
            header_dimmed: false,
        };
        for empty in [&not_found, &size_unknown, &no_digits] {
            assert!(
                ocr_fallback_reason(empty, false).is_some(),
                "{:?}",
                empty.status
            );
            assert_eq!(
                ocr_fallback_reason(empty, true),
                None,
                "OCR off: no fallback"
            );
        }
        assert_eq!(
            ocr_fallback_reason(&read, false),
            None,
            "a found board stays"
        );
        let mut six = read.clone();
        six.team_size = Some(6);
        assert_eq!(
            ocr_fallback_reason(&six, false),
            None,
            "a found board that disagrees is refused, not handed to OCR"
        );
        assert!(board_refusal(&six, 5).is_some());
    }

    fn ocr() -> PolicyRow {
        PolicyRow {
            map: "Busan".into(),
            mode: "Control".into(),
            result: "victory".into(),
            hero: "Ana".into(),
            elims: 10,
            assists: 4,
            deaths: 2,
            damage: 4000,
            healing: 8000,
            mitigation: 100,
        }
    }

    fn text_field(name: &str, value: &str, suspect: bool) -> FieldRead {
        FieldRead {
            name: name.into(),
            value: Some(Value::Text(value.into())),
            confidence: if suspect { 0.1 } else { 0.9 },
            suspect,
        }
    }

    fn int_field(name: &str, value: u32, suspect: bool) -> FieldRead {
        FieldRead {
            name: name.into(),
            value: Some(Value::Int(value)),
            confidence: if suspect { 0.1 } else { 0.9 },
            suspect,
        }
    }

    fn unread(name: &str) -> FieldRead {
        FieldRead {
            name: name.into(),
            value: None,
            confidence: 0.0,
            suspect: true,
        }
    }

    #[test]
    fn kit_confidences_carry_the_own_rows_reads_and_skip_unread_fields() {
        let mut board = confident_board(5, 2);
        if let Some(f) = board.fields.iter_mut().find(|f| f.name == "r2.mit") {
            *f = unread("r2.mit");
        }
        let c = kit_confidences(
            &board,
            OwnRow::Identified {
                index: 2,
                team_size: 5,
            },
        );
        assert_eq!(c["mode"].confidence, f64::from(0.9f32));
        assert_eq!(c["e"].confidence, f64::from(0.9f32));
        assert_eq!(c["hero"].confidence, f64::from(0.9f32));
        assert_eq!(c["mode"].value.as_deref(), Some("Control"));
        assert!(
            c["e"].value.is_some(),
            "the reader's own number rides along"
        );
        assert!(!c.contains_key("mit"), "an unread cell has no confidence");
        let uncertain = kit_confidences(&board, OwnRow::Uncertain);
        assert_eq!(uncertain.keys().collect::<Vec<_>>(), vec!["mode"]);
    }

    /// A Read board whose row `index` is fully confident, plus match fields.
    fn confident_board(team_size: usize, index: usize) -> BoardRead {
        let mut fields = vec![
            text_field("map", "Ilios", false),
            text_field("mode", "Control", false),
            text_field("result", "defeat", false),
            text_field(&format!("r{index}.hero"), "Kiriko", false),
        ];
        let stats = [
            ("e", 21u32),
            ("a", 9),
            ("d", 3),
            ("dmg", 9100),
            ("h", 1200),
            ("mit", 50),
        ];
        for (suffix, value) in stats {
            fields.push(int_field(&format!("r{index}.{suffix}"), value, false));
        }
        BoardRead {
            status: BoardStatus::Read,
            team_size: Some(team_size),
            fields,
            elapsed_ms: 1,
            header_dimmed: false,
        }
    }

    fn identified(index: usize, team_size: usize) -> OwnRow {
        OwnRow::Identified { index, team_size }
    }

    /// A board read only through the dimmed header path still saves every
    /// sure number, but the row lists every own-row field as suspect. The
    /// same board read clean lists none, and a later clean read in the same
    /// game is the row the game keeps.
    #[test]
    fn a_dimmed_header_read_saves_marked_unsure_and_a_later_clean_read_replaces_it() {
        let clean_board = confident_board(5, 2);
        let mut dimmed_board = clean_board.clone();
        dimmed_board.header_dimmed = true;

        // The capture gate still accepts the dimmed read: every cell is sure.
        let stats = read_own_row_stats(&dimmed_board, identified(2, 5), "victory").unwrap();
        assert_eq!(stats.held, [false; 6]);

        let dimmed = merge_saved(&ocr(), identified(2, 5), Some(&dimmed_board));
        assert_eq!(dimmed.recognizer, RECOGNIZER_ID);
        assert_eq!(
            (dimmed.elims, dimmed.assists, dimmed.deaths),
            (21, 9, 3),
            "the numbers are saved"
        );
        assert_eq!(
            (dimmed.damage, dimmed.healing, dimmed.mitigation),
            (9100, 1200, 50)
        );
        // The clean read lists only the board map, which names a different
        // map than the policy row; the dimmed read adds every own-row field.
        let clean = merge_saved(&ocr(), identified(2, 5), Some(&clean_board));
        assert_eq!(clean.suspect_fields, vec!["map".to_string()]);
        let mut want = clean.suspect_fields.clone();
        mark_own_row_suspect(&mut want);
        assert_eq!(dimmed.suspect_fields, want, "every own-row field is unsure");
        for name in ["e", "a", "d", "dmg", "h", "mit", "hero"] {
            assert!(dimmed.suspect_fields.iter().any(|f| f == name), "{name}");
        }

        // Two snapshots of one game, newest first (the store's order): the
        // later clean read wins.
        let row = |saved: &SavedRead| {
            let mut m = crate::parse::match_from_stats(
                [
                    saved.elims,
                    saved.assists,
                    saved.deaths,
                    saved.damage,
                    saved.healing,
                    saved.mitigation,
                ],
                "victory",
            );
            m.session_id = "game".into();
            m.suspect_fields = saved.suspect_fields.clone();
            m
        };
        let mut later = clean.clone();
        later.elims += 1;
        let games = crate::storage::latest_per_game(vec![row(&later), row(&dimmed)]);
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].elims, 22);
        assert_eq!(games[0].suspect_fields, vec!["map".to_string()]);
    }

    /// A dimmed header whose cells all fell back to the policy row (ocr-v1)
    /// still lists every own-row field. An unread hero is not an attempted
    /// read, so only the dimmed mark puts `hero` on the list.
    #[test]
    fn a_dimmed_header_marks_the_row_when_ocr_v1_kept_every_field() {
        let mut board = confident_board(5, 2);
        board.header_dimmed = true;
        for field in &mut board.fields {
            if field.name == "r2.hero" {
                *field = unread("r2.hero");
            } else if is_stat_field(&field.name) || field.name == "result" {
                field.suspect = true;
            }
        }
        let saved = merge_saved(&ocr(), identified(2, 5), Some(&board));
        assert_eq!(saved.recognizer, RECOGNIZER_OCR_V1);
        assert_eq!(saved.hero, "Ana", "the unread hero stays on the policy row");
        assert!(
            !board.get("r2.hero").is_some_and(field_attempted),
            "the hero cell was not a cv read"
        );
        for name in ["hero", "e", "a", "d", "dmg", "h", "mit"] {
            assert!(
                saved.suspect_fields.iter().any(|f| f == name),
                "{name} missing from {:?}",
                saved.suspect_fields
            );
        }
    }

    // ----- alpha.7 O10: the trust gate and the stats without OCR -----

    #[test]
    fn board_refusal_names_every_board_the_reader_does_not_vouch_for() {
        assert_eq!(board_refusal(&confident_board(5, 2), 5), None);
        let mut gone = confident_board(5, 2);
        gone.status = BoardStatus::NotFound;
        assert_eq!(board_refusal(&gone, 5), Some("board was not found"));
        let mut unsized_board = confident_board(5, 2);
        unsized_board.status = BoardStatus::TeamSizeUnknown;
        assert_eq!(
            board_refusal(&unsized_board, 5),
            Some("team size is unknown")
        );
        // a Read board whose digit pass returned nothing
        let mut empty = confident_board(5, 2);
        for field in &mut empty.fields {
            if matches!(field.value, Some(Value::Int(_))) {
                *field = unread(&field.name.clone());
            }
        }
        assert_eq!(
            board_refusal(&empty, 5),
            Some("read board has no digit values")
        );
        // the capture path measured another team size: row 2 is another row
        assert_eq!(
            board_refusal(&confident_board(6, 2), 5),
            Some("team size disagrees")
        );
        let mut sizeless = confident_board(5, 2);
        sizeless.team_size = None;
        assert_eq!(board_refusal(&sizeless, 5), Some("team size is missing"));
    }

    #[test]
    fn read_own_row_takes_sure_cells_and_holds_the_rest() {
        let mut board = confident_board(5, 2);
        for field in &mut board.fields {
            match field.name.as_str() {
                // a plausible suspect number, and a cell with no number
                "r2.h" => *field = int_field("r2.h", 1250, true),
                "r2.mit" => *field = unread("r2.mit"),
                _ => {}
            }
        }
        let read = read_own_row_stats(&board, identified(2, 5), "defeat").unwrap();
        assert_eq!(read.held, [false, false, false, false, true, true]);
        let m = &read.matched;
        assert_eq!(
            (
                m.elims,
                m.assists,
                m.deaths,
                m.damage,
                m.healing,
                m.mitigation
            ),
            (21, 9, 3, 9100, 0, 0),
            "a held cell carries no number, least of all the suspect one"
        );
        assert_eq!((m.hero.as_str(), m.map_name.as_str()), ("Unknown", ""));
        assert_eq!(m.outcome, "defeat");
        assert_eq!(
            row_cells(&board, 2),
            [
                CellTrust::Sure(21),
                CellTrust::Sure(9),
                CellTrust::Sure(3),
                CellTrust::Sure(9100),
                CellTrust::Held,
                CellTrust::Held
            ]
        );
        // a row the board does not have is held throughout
        assert_eq!(row_cells(&board, 7), [CellTrust::Held; 6]);
    }

    #[test]
    fn read_own_row_refuses_without_a_row_or_with_under_four_sure_cells() {
        let board = confident_board(5, 2);
        assert_eq!(
            read_own_row_stats(&board, OwnRow::Uncertain, "unknown").unwrap_err(),
            ScoreboardMiss::PlayerRowNotFound
        );
        assert_eq!(
            read_own_row_stats(&board, identified(10, 5), "unknown").unwrap_err(),
            ScoreboardMiss::PlayerRowNotFound
        );
        // the row is on the board, but every cell of it is suspect
        let mut unsure = confident_board(5, 2);
        for field in &mut unsure.fields {
            if matches!(field.value, Some(Value::Int(_))) {
                field.suspect = true;
            }
        }
        assert_eq!(
            read_own_row_stats(&unsure, identified(2, 5), "unknown").unwrap_err(),
            ScoreboardMiss::CellsUnreadable
        );
        // one, two and three sure cells are still not a row (F266-6): a
        // single read just over the suspect line is not stored alone
        let sure = |board: &mut BoardRead, suffix: &str, value: u32| {
            let name = format!("r2.{suffix}");
            board.fields.retain(|f| f.name != name);
            board.fields.push(int_field(&name, value, false));
        };
        for (suffix, value) in [("e", 4), ("a", 2), ("d", 1)] {
            sure(&mut unsure, suffix, value);
            assert_eq!(
                read_own_row_stats(&unsure, identified(2, 5), "unknown").unwrap_err(),
                ScoreboardMiss::CellsUnreadable,
                "sure up to {suffix}"
            );
        }
        // the fourth makes it a row: the other two are held
        sure(&mut unsure, "dmg", 900);
        let read = read_own_row_stats(&unsure, identified(2, 5), "unknown").unwrap();
        assert_eq!(read.held, [false, false, false, false, true, true]);
        assert_eq!(
            (
                read.matched.elims,
                read.matched.damage,
                read.matched.healing
            ),
            (4, 900, 0)
        );
        assert_eq!(MIN_SURE_CELLS, 4);
    }

    #[test]
    fn a_blank_player_name_is_no_name() {
        assert!(!player_name_is_set(None));
        assert!(!player_name_is_set(Some("")));
        assert!(!player_name_is_set(Some("  \t")));
        assert!(player_name_is_set(Some("FrozenTear")));
        // and it does not pick a row either: the highlight row stands
        assert_eq!(
            own_row(Some(" "), Some(7), Some(2), 5),
            OwnRow::Identified {
                index: 2,
                team_size: 5
            }
        );
    }

    #[test]
    fn both_teams_need_a_row_with_four_sure_cells() {
        let row = |board: &mut BoardRead, index: usize, sure: usize| {
            for (k, suffix) in STAT_SUFFIXES.iter().enumerate() {
                board
                    .fields
                    .push(int_field(&format!("r{index}.{suffix}"), 5, k >= sure));
            }
        };
        let mut board = confident_board(5, 2);
        assert!(!both_teams_have_stats(&board, 5), "team 2 has no row");
        row(&mut board, 7, 3);
        assert!(
            !both_teams_have_stats(&board, 5),
            "three sure cells are not a row"
        );
        row(&mut board, 5, 4);
        assert!(both_teams_have_stats(&board, 5));
        // on a 6v6 board row 5 is team 1, and row 7 is still not a row
        assert!(!both_teams_have_stats(&board, 6));
        assert!(!both_teams_have_stats(&board, 0));
    }

    #[test]
    fn switch_off_keeps_ocr_v1_values_and_omits_suspects() {
        let before = ocr();
        let saved = merge_saved(&before, identified(2, 5), None);
        assert_eq!(saved.map, before.map);
        assert_eq!(saved.mode, before.mode);
        assert_eq!(saved.result, before.result);
        assert_eq!(saved.hero, before.hero);
        assert_eq!(
            (
                saved.elims,
                saved.assists,
                saved.deaths,
                saved.damage,
                saved.healing,
                saved.mitigation
            ),
            (
                before.elims,
                before.assists,
                before.deaths,
                before.damage,
                before.healing,
                before.mitigation
            )
        );
        assert_eq!(saved.recognizer, RECOGNIZER_OCR_V1);
        assert!(saved.suspect_fields.is_empty());
    }

    #[test]
    fn confident_own_row_uses_new_values_and_cv_recognizer() {
        let saved = merge_saved(&ocr(), identified(2, 5), Some(&confident_board(5, 2)));
        assert_eq!(saved.map, "Busan", "policy map stays");
        assert_eq!(saved.mode, "Control", "policy mode stays");
        assert_eq!(saved.suspect_fields, vec!["map".to_string()]);
        assert_eq!(saved.result, "defeat");
        assert_eq!(saved.hero, "Kiriko");
        assert_eq!(saved.elims, 21);
        assert_eq!(saved.assists, 9);
        assert_eq!(saved.deaths, 3);
        assert_eq!(saved.damage, 9100);
        assert_eq!(saved.healing, 1200);
        assert_eq!(saved.mitigation, 50);
        assert_eq!(saved.recognizer, RECOGNIZER_ID);
        assert!(!saved.recognizer.contains(' '), "{:?}", saved.recognizer);
    }

    /// The own row's hero is stored only when the Tab's hero authority
    /// names it too: a row read the career panel contradicts, or an
    /// abstaining authority, keeps the earlier hero and flags `hero`.
    #[test]
    fn the_own_row_hero_needs_the_hero_authority() {
        let board = confident_board(5, 2);
        let with = |a: Option<Option<&str>>| {
            merge_saved_with_authority(&ocr(), identified(2, 5), Some(&board), a)
        };
        for agree in [Some(Some("Kiriko")), Some(Some("kiriko")), None] {
            let saved = with(agree);
            assert_eq!(saved.hero, "Kiriko", "{agree:?}");
            assert!(
                !saved.suspect_fields.contains(&"hero".to_string()),
                "{agree:?}"
            );
        }
        for disagree in [Some(Some("zenyatta")), Some(None)] {
            let saved = with(disagree);
            assert_eq!(
                saved.hero,
                canonical_board_hero(&ocr().hero),
                "{disagree:?}"
            );
            assert!(
                saved.suspect_fields.contains(&"hero".to_string()),
                "{disagree:?}"
            );
            // the stats still come from the row
            assert_eq!(saved.elims, 21, "{disagree:?}");
        }
    }

    #[test]
    fn suspect_digit_falls_back_for_that_field_only() {
        let mut board = confident_board(5, 1);
        let map = board
            .fields
            .iter_mut()
            .find(|field| field.name == "map")
            .unwrap();
        map.value = Some(Value::Text("Busan".into()));
        let deaths = board
            .fields
            .iter_mut()
            .find(|field| field.name == "r1.d")
            .unwrap();
        deaths.suspect = true;
        deaths.value = Some(Value::Int(99));
        let saved = merge_saved(&ocr(), identified(1, 5), Some(&board));
        assert_eq!(
            saved.deaths,
            ocr().deaths,
            "suspect digit keeps the policy row"
        );
        assert_eq!(saved.elims, 21);
        assert_eq!(saved.damage, 9100);
        assert_eq!(saved.hero, "Kiriko");
        assert_eq!(saved.recognizer, RECOGNIZER_ID);
        assert_eq!(saved.suspect_fields, vec!["d".to_string()]);
    }

    #[test]
    fn read_board_with_no_digit_values_keeps_the_whole_ocr_v1_read() {
        // Columns were found, so the reader marks the board read, but the
        // digit pass produced no numbers (its time budget ran out). A
        // confident map, mode, result, or hero on that board is not a read.
        let mut board = BoardRead {
            status: BoardStatus::Read,
            team_size: Some(5),
            fields: crate::shadow::field_names(5)
                .into_iter()
                .map(|name| unread(&name))
                .collect(),
            elapsed_ms: 1,
            header_dimmed: false,
        };
        for (name, value) in [
            ("map", "Ilios"),
            ("mode", "Escort"),
            ("result", "defeat"),
            ("r0.hero", "Kiriko"),
        ] {
            let field = board
                .fields
                .iter_mut()
                .find(|field| field.name == name)
                .unwrap();
            field.value = Some(Value::Text(value.into()));
            field.confidence = 0.95;
            field.suspect = false;
        }
        let saved = merge_saved(&ocr(), identified(0, 5), Some(&board));
        let before = ocr();
        assert_eq!(saved.map, before.map);
        assert_eq!(saved.mode, before.mode);
        assert_eq!(saved.result, before.result);
        assert_eq!(saved.hero, before.hero);
        assert_eq!(saved.elims, before.elims);
        assert_eq!(saved.assists, before.assists);
        assert_eq!(saved.deaths, before.deaths);
        assert_eq!(saved.damage, before.damage);
        assert_eq!(saved.healing, before.healing);
        assert_eq!(saved.mitigation, before.mitigation);
        assert_eq!(saved.recognizer, RECOGNIZER_OCR_V1);
        assert!(saved.suspect_fields.is_empty());

        // A stored 0 is a digit value, so this path does not apply.
        let zero = board
            .fields
            .iter_mut()
            .find(|field| field.name == "r0.e")
            .unwrap();
        zero.value = Some(Value::Int(0));
        zero.confidence = 0.9;
        zero.suspect = false;
        let with_zero = merge_saved(&ocr(), identified(0, 5), Some(&board));
        assert_eq!(with_zero.elims, 0);
        assert_ne!(with_zero.recognizer, RECOGNIZER_OCR_V1);
    }

    #[test]
    fn not_found_keeps_the_whole_ocr_v1_read() {
        // A frame with no board can still carry match slots. None of them
        // are stored, even when a slot looks confident.
        let mut board = BoardRead {
            status: BoardStatus::NotFound,
            team_size: None,
            fields: vec![unread("map"), unread("mode"), unread("result")],
            elapsed_ms: 1,
            header_dimmed: false,
        };
        board.fields[0] = FieldRead {
            name: "map".into(),
            value: Some(Value::Text("Ilios".into())),
            confidence: 0.99,
            suspect: false,
        };
        let saved = merge_saved(&ocr(), identified(0, 5), Some(&board));
        assert_eq!(saved.map, "Busan");
        assert_eq!(saved.mode, "Control");
        assert_eq!(saved.result, "victory");
        assert_eq!(saved.hero, "Ana");
        assert_eq!(saved.elims, 10);
        assert_eq!(saved.recognizer, RECOGNIZER_OCR_V1);
        assert!(saved.suspect_fields.is_empty());
    }

    #[test]
    fn team_size_unknown_keeps_the_whole_ocr_v1_read() {
        // A later reader must not be trusted to guess 5 or 6. Even if row
        // fields are present, TeamSizeUnknown keeps ocr-v1 for the game.
        let mut board = confident_board(5, 0);
        board.status = BoardStatus::TeamSizeUnknown;
        board.team_size = None;
        let saved = merge_saved(&ocr(), identified(0, 5), Some(&board));
        assert_eq!(saved.map, "Busan");
        assert_eq!(saved.hero, "Ana");
        assert_eq!(saved.elims, 10);
        assert_eq!(saved.damage, 4000);
        assert_eq!(saved.recognizer, RECOGNIZER_OCR_V1);
        assert!(saved.suspect_fields.is_empty());
    }

    #[test]
    fn read_board_with_uncertain_own_row_keeps_the_full_ocr_v1_read() {
        let saved = merge_saved(&ocr(), OwnRow::Uncertain, Some(&confident_board(5, 0)));
        assert_eq!(saved.map, "Busan");
        assert_eq!(saved.mode, "Control");
        assert_eq!(saved.result, "victory");
        assert_eq!(saved.hero, "Ana");
        assert_eq!(saved.elims, 10);
        assert_eq!(saved.assists, 4);
        assert_eq!(saved.deaths, 2);
        assert_eq!(saved.damage, 4000);
        assert_eq!(saved.healing, 8000);
        assert_eq!(saved.mitigation, 100);
        assert_eq!(saved.recognizer, RECOGNIZER_OCR_V1);
        assert!(saved.suspect_fields.is_empty());
    }

    #[test]
    fn own_row_that_disagrees_with_the_new_reader_keeps_ocr_v1() {
        let saved = merge_saved(&ocr(), identified(0, 5), Some(&confident_board(6, 0)));
        assert_eq!(saved.recognizer, RECOGNIZER_OCR_V1);
        assert_eq!(saved.hero, "Ana");
        assert_eq!(saved.elims, 10);
        assert!(saved.suspect_fields.is_empty());

        let out_of_range = merge_saved(&ocr(), identified(10, 5), Some(&confident_board(5, 0)));
        assert_eq!(out_of_range.recognizer, RECOGNIZER_OCR_V1);
        assert_eq!(out_of_range.map, "Busan");
    }

    #[test]
    fn blank_map_mode_or_hero_falls_back_instead_of_clearing_ocr() {
        let mut board = confident_board(5, 0);
        for name in ["map", "mode", "r0.hero"] {
            let field = board
                .fields
                .iter_mut()
                .find(|field| field.name == name)
                .unwrap();
            field.value = Some(Value::Text("  ".into()));
            field.suspect = false;
        }
        let saved = merge_saved(&ocr(), identified(0, 5), Some(&board));
        assert_eq!(saved.map, "Busan");
        assert_eq!(saved.mode, "Control");
        assert_eq!(saved.hero, "Ana");
        assert_eq!(saved.elims, 21);
        assert_eq!(saved.suspect_fields, vec!["hero".to_string()]);
        assert_eq!(saved.recognizer, RECOGNIZER_ID);
    }

    #[test]
    fn agreeing_map_is_not_suspect_and_mode_stays_on_policy() {
        let mut board = confident_board(5, 0);
        let map = board
            .fields
            .iter_mut()
            .find(|field| field.name == "map")
            .unwrap();
        map.value = Some(Value::Text("Busan".into()));
        let mode = board
            .fields
            .iter_mut()
            .find(|field| field.name == "mode")
            .unwrap();
        mode.value = Some(Value::Text("Escort".into()));
        let saved = merge_saved(&ocr(), identified(0, 5), Some(&board));
        assert_eq!(saved.map, "Busan");
        assert_eq!(saved.mode, "Control");
        assert!(!saved.suspect_fields.iter().any(|name| name == "map"));
        assert!(!saved.suspect_fields.iter().any(|name| name == "mode"));
        assert_eq!(saved.recognizer, RECOGNIZER_ID);
    }

    #[test]
    fn deathmatch_policy_stays_when_the_new_reader_names_another_map() {
        let mut policy = ocr();
        policy.map = "Château Guillard".into();
        policy.mode = "Deathmatch".into();
        let saved = merge_saved(&policy, identified(0, 5), Some(&confident_board(5, 0)));
        assert_eq!(saved.map, "Château Guillard");
        assert_eq!(saved.mode, "Deathmatch");
        assert!(saved.suspect_fields.iter().any(|name| name == "map"));
        assert!(!saved.suspect_fields.iter().any(|name| name == "mode"));
        assert!(!crate::parse::stats_row_is_tracked(&saved.map, &saved.mode));
        assert!(crate::parse::map_is_untracked(&saved.map));
    }

    #[test]
    fn every_fallback_is_tagged_ocr_v1() {
        let mut board = confident_board(5, 0);
        for field in &mut board.fields {
            field.suspect = true;
        }
        let saved = merge_saved(&ocr(), identified(0, 5), Some(&board));
        assert_eq!(saved.map, "Busan");
        assert_eq!(saved.mode, "Control");
        assert_eq!(saved.result, "victory");
        assert_eq!(saved.hero, "Ana");
        assert_eq!(saved.elims, 10);
        assert_eq!(saved.damage, 4000);
        assert_eq!(saved.recognizer, RECOGNIZER_OCR_V1);
        assert!(saved.suspect_fields.iter().any(|name| name == "map"));
        assert!(saved.suspect_fields.iter().any(|name| name == "hero"));
        assert!(saved.suspect_fields.iter().any(|name| name == "e"));
        assert!(!saved.suspect_fields.iter().any(|name| name == "mode"));
    }

    #[test]
    fn unread_match_fields_are_not_suspect() {
        let mut board = confident_board(5, 0);
        for name in ["map", "mode", "result", "r0.hero"] {
            let field = board
                .fields
                .iter_mut()
                .find(|field| field.name == name)
                .unwrap();
            *field = unread(name);
        }
        let saved = merge_saved(&ocr(), identified(0, 5), Some(&board));
        assert_eq!(saved.map, "Busan");
        assert_eq!(saved.mode, "Control");
        assert_eq!(saved.result, "victory");
        assert_eq!(saved.hero, "Ana");
        assert_eq!(saved.elims, 21);
        assert!(saved.suspect_fields.is_empty());
        assert_eq!(saved.recognizer, RECOGNIZER_ID);
    }

    #[test]
    fn name_match_wins_and_a_miss_uses_the_highlight_row() {
        assert_eq!(own_row(Some("Ada"), Some(3), Some(0), 5), identified(3, 5));
        assert_eq!(own_row(Some("Ada"), None, Some(0), 5), identified(0, 5));
        assert_eq!(own_row(Some("Ada"), None, None, 5), OwnRow::Uncertain);
        assert_eq!(own_row(None, None, Some(1), 6), identified(1, 6));
        assert_eq!(own_row(Some("  "), None, Some(1), 6), identified(1, 6));
        assert_eq!(own_row(None, None, None, 5), OwnRow::Uncertain);
    }

    #[test]
    fn highlight_row_keeps_confident_digits_when_the_name_misses() {
        let own = own_row(Some("Ada"), None, Some(2), 5);
        let saved = merge_saved(&ocr(), own, Some(&confident_board(5, 2)));
        assert_eq!(saved.recognizer, RECOGNIZER_ID);
        assert_eq!(saved.elims, 21);
        assert_eq!(saved.damage, 9100);
        assert_eq!(saved.hero, "Kiriko");
        assert!(saved.suspect_fields.iter().any(|name| name == "map"));
    }

    #[test]
    fn blank_map_and_a_hero_key_do_not_drop_confident_digits() {
        let mut board = confident_board(6, 0);
        let hero = board
            .fields
            .iter_mut()
            .find(|field| field.name == "r0.hero")
            .unwrap();
        hero.value = Some(Value::Text("wrecking-ball".into()));
        hero.confidence = 0.95;
        hero.suspect = false;
        let mut snap = ocr();
        snap.map.clear();
        snap.mode.clear();
        snap.hero = "Tracer".into();
        let saved = merge_saved(&snap, identified(0, 6), Some(&board));
        assert_eq!(saved.map, "", "a blank policy map stays");
        assert_eq!(saved.mode, "");
        assert_eq!(saved.hero, "Wrecking Ball");
        assert_eq!(crate::parse::guess_role_public(&saved.hero), "Tank");
        assert_eq!(saved.elims, 21);
        assert_eq!(saved.damage, 9100);
        assert_eq!(saved.recognizer, RECOGNIZER_ID);
        assert!(saved.suspect_fields.iter().any(|name| name == "map"));
        assert!(!saved.suspect_fields.iter().any(|name| name == "hero"));

        let mut unread_map = board.clone();
        for name in ["map", "mode"] {
            let field = unread_map
                .fields
                .iter_mut()
                .find(|field| field.name == name)
                .unwrap();
            *field = unread(name);
        }
        let quiet = merge_saved(&snap, identified(0, 6), Some(&unread_map));
        assert_eq!(quiet.map, "");
        assert_eq!(quiet.hero, "Wrecking Ball");
        assert_eq!(quiet.elims, 21);
        assert_eq!(quiet.recognizer, RECOGNIZER_ID);
        assert!(quiet.suspect_fields.is_empty());
    }

    fn set_hero(board: &mut crate::shadow::BoardRead, value: &str) {
        let hero = board
            .fields
            .iter_mut()
            .find(|field| field.name == "r0.hero")
            .unwrap();
        hero.value = Some(Value::Text(value.into()));
        hero.confidence = 0.95;
        hero.suspect = false;
    }

    #[test]
    fn a_placeholder_match_falls_back_to_career_or_portrait() {
        let mut board = confident_board(5, 0);
        set_hero(&mut board, "empty_01");

        let mut career = ocr();
        career.hero = "Wrecking Ball".into();
        let saved = merge_saved(&career, identified(0, 5), Some(&board));
        assert_eq!(saved.hero, "Wrecking Ball");
        assert_ne!(saved.hero, "empty_01");
        assert!(!saved.suspect_fields.iter().any(|name| name == "hero"));
        assert_eq!(saved.elims, 21);
        assert_eq!(saved.recognizer, RECOGNIZER_ID);

        let mut portrait = ocr();
        portrait.hero = "wrecking_ball".into();
        let saved = merge_saved(&portrait, identified(0, 5), Some(&board));
        assert_eq!(saved.hero, "Wrecking Ball");
        assert!(!saved.suspect_fields.iter().any(|name| name == "hero"));

        set_hero(&mut board, "special/skull_06.png");
        let saved = merge_saved(&career, identified(0, 5), Some(&board));
        assert_eq!(saved.hero, "Wrecking Ball");
        assert_ne!(saved.hero, "skull_06");
    }

    #[test]
    fn a_rejected_board_still_stores_the_pack_key_as_the_display_name() {
        let mut snap = ocr();
        snap.hero = "wrecking-ball".into();
        let saved = merge_saved(&snap, OwnRow::Uncertain, Some(&confident_board(5, 0)));
        assert_eq!(saved.hero, "Wrecking Ball");
        assert_eq!(crate::parse::guess_role_public(&saved.hero), "Tank");
        assert_eq!(saved.recognizer, RECOGNIZER_OCR_V1);
    }

    #[test]
    fn full_fallback_names_the_reason_and_keeps_ocr_v1() {
        let board = confident_board(5, 0);
        assert_eq!(
            full_fallback_reason(&board, OwnRow::Uncertain),
            Some("own row is uncertain")
        );
        let saved = merge_saved(&ocr(), OwnRow::Uncertain, Some(&board));
        assert_eq!(saved.recognizer, RECOGNIZER_OCR_V1);
        assert!(saved.suspect_fields.is_empty());

        let mut missing = board.clone();
        missing.status = BoardStatus::NotFound;
        missing.team_size = None;
        assert_eq!(
            full_fallback_reason(&missing, identified(0, 5)),
            Some("board was not found")
        );
        let mut unknown = board.clone();
        unknown.status = BoardStatus::TeamSizeUnknown;
        unknown.team_size = None;
        assert_eq!(
            full_fallback_reason(&unknown, identified(0, 5)),
            Some("team size is unknown")
        );
        let mut no_digits = board.clone();
        for field in &mut no_digits.fields {
            if is_stat_field(&field.name) {
                field.value = None;
            }
        }
        assert_eq!(
            full_fallback_reason(&no_digits, identified(0, 5)),
            Some("read board has no digit values")
        );
        let mut other_size = board.clone();
        other_size.team_size = Some(6);
        assert_eq!(
            full_fallback_reason(&other_size, identified(0, 5)),
            Some("team size disagrees")
        );
        assert_eq!(
            full_fallback_reason(&board, identified(10, 5)),
            Some("row index is outside the board")
        );
        assert_eq!(full_fallback_reason(&board, identified(0, 5)), None);
    }

    #[test]
    fn upload_names_are_flat_allowed_and_deduped() {
        let flat = upload_suspect_fields(&[
            "r3.dmg".into(),
            "dmg".into(),
            "r0.hero".into(),
            "nope".into(),
            "r12.e".into(),
            "map".into(),
            "r3.nope".into(),
            "hero".into(),
        ]);
        assert_eq!(
            flat,
            vec!["map".to_string(), "hero".into(), "e".into(), "dmg".into()]
        );
        assert!(flat.iter().all(|name| !name.contains('.')));
        assert!(
            flat.iter()
                .all(|name| SUSPECT_FIELD_NAMES.contains(&name.as_str()))
        );
    }

    #[test]
    fn a_suspect_own_row_marks_the_fields_read_off_the_row_and_no_other() {
        let mut fields = Vec::new();
        mark_own_row_suspect(&mut fields);
        assert_eq!(fields, ["hero", "e", "a", "d", "dmg", "h", "mit"]);
        // the map, the mode and the result are not read off the row
        for shared in ["map", "mode", "result"] {
            assert!(!fields.iter().any(|f| f == shared));
        }
        // marks already there stay, nothing repeats, the order is the API's
        let mut fields = vec!["mit".to_string(), "map".into(), "r2.e".into()];
        mark_own_row_suspect(&mut fields);
        assert_eq!(fields, ["map", "hero", "e", "a", "d", "dmg", "h", "mit"]);
        let again = fields.clone();
        mark_own_row_suspect(&mut fields);
        assert_eq!(fields, again);
        // every name is one the server accepts
        assert!(
            OWN_ROW_FIELDS
                .iter()
                .all(|f| SUSPECT_FIELD_NAMES.contains(f))
        );
    }

    #[test]
    fn editing_a_field_clears_its_flat_and_row_names() {
        let mut fields = vec!["r3.dmg".into(), "hero".into(), "e".into(), "map".into()];
        clear_edited_suspects(&mut fields, "damage");
        assert_eq!(fields, vec!["hero".to_string(), "e".into(), "map".into()]);
        clear_edited_suspects(&mut fields, "map_name");
        assert_eq!(fields, vec!["hero".to_string(), "e".into()]);
        clear_edited_suspects(&mut fields, "elims");
        assert_eq!(fields, vec!["hero".to_string()]);
    }
}
