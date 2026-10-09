//! Saved-stat merge for `reader = "new"`.
//!
//! ocr-v1 still decides game start and end, screen detection, and the
//! plausibility holds. This module only chooses the values stored on the
//! member's own row. A stat, hero, or result from [`BoardRead`] is used when
//! that field was read and is not suspect. Anything else keeps that field's
//! ocr-v1 value. A field is listed in `suspect_fields` only when the new
//! reader actually attempted it and did not supply a value to store.
//!
//! The map and mode already chosen by capture policy stay as they are. A
//! confident or suspect board map that names a different map adds `map` to
//! `suspect_fields` and does not replace the policy map or mode.
//!
//! The whole ocr-v1 read is kept, with recognizer `ocr-v1` and no suspect
//! list, when the board is missing, the team size is unknown or disagrees,
//! the member's own row was not identified, the row index is outside the
//! board, or the board is marked read but has no digit values (the digit
//! pass can miss its time budget after the columns are found). Each of
//! those full fallbacks logs one info line with the reason. A blank map, a
//! hero key that is not the display name, and a hero that came from the
//! career panel or the portrait matcher are per-field: confident digits
//! stay on this reader. The recognizer is also `ocr-v1` when every stored
//! field fell back to ocr-v1.

use scuffed_types::{RECOGNIZER_OCR_V1, SUSPECT_FIELD_NAMES};

use crate::shadow::digits::RECOGNIZER_ID;
use crate::shadow::{BoardRead, BoardStatus, FieldRead, Value};

/// ocr-v1 values for the row that will be stored, after holds and map policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrSnapshot {
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
    let named = player_name.is_some_and(|name| !name.trim().is_empty());
    let index = if named {
        name_match.or(fallback_row)
    } else {
        fallback_row
    };
    match index {
        Some(index) => OwnRow::Identified { index, team_size },
        None => OwnRow::Uncertain,
    }
}

/// Merge one accepted board.
///
/// `board == None` is the switch off: the snapshot is returned unchanged,
/// tagged `ocr-v1`, with no suspect list.
pub fn merge_saved(ocr: &OcrSnapshot, own: OwnRow, board: Option<&BoardRead>) -> SavedRead {
    let keep = || SavedRead {
        map: ocr.map.clone(),
        mode: ocr.mode.clone(),
        result: ocr.result.clone(),
        hero: ocr.hero.clone(),
        elims: ocr.elims,
        assists: ocr.assists,
        deaths: ocr.deaths,
        damage: ocr.damage,
        healing: ocr.healing,
        mitigation: ocr.mitigation,
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
    let map = ocr.map.clone();
    let mode = ocr.mode.clone();
    if let Some(board_map) = text_on(board, "map")
        && board_map.trim() != ocr.map.trim()
    {
        push_suspect(&mut suspect, "map");
    }
    let result = match take_text(board, "result", &mut suspect, "result") {
        Some(value) => {
            used_cv = true;
            value
        }
        None => ocr.result.clone(),
    };
    let hero_name = format!("r{index}.hero");
    let hero = match take_text(board, &hero_name, &mut suspect, "hero") {
        Some(value) => {
            used_cv = true;
            canonical_board_hero(&value)
        }
        None => ocr.hero.clone(),
    };
    let stats = [
        ("e", ocr.elims),
        ("a", ocr.assists),
        ("d", ocr.deaths),
        ("dmg", ocr.damage),
        ("h", ocr.healing),
        ("mit", ocr.mitigation),
    ];
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
        recognizer: if used_cv {
            RECOGNIZER_ID
        } else {
            RECOGNIZER_OCR_V1
        },
        suspect_fields: order_suspects(suspect),
    }
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
    match board.status {
        BoardStatus::Read => {}
        BoardStatus::NotFound => return Some("board was not found"),
        BoardStatus::TeamSizeUnknown => return Some("team size is unknown"),
    }
    if !has_digit_value(board) {
        return Some("read board has no digit values");
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

/// Template keys (`wrecking-ball`, `wrecking_ball`) store the display name.
///
/// A key and the career-panel name are the same hero. Leaving the key on the
/// row made cv-v5 saves and later ocr-v1 saves look like two heroes, and a
/// raw string compare is not a reason to drop the board.
fn canonical_board_hero(raw: &str) -> String {
    let spaced = raw.replace(['_', '-'], " ");
    crate::parse::canonical_hero(&spaced)
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

    fn ocr() -> OcrSnapshot {
        OcrSnapshot {
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
        }
    }

    fn identified(index: usize, team_size: usize) -> OwnRow {
        OwnRow::Identified { index, team_size }
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
        assert_eq!(saved.deaths, ocr().deaths, "suspect digit stays on ocr-v1");
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
