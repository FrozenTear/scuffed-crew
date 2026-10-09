//! Saved-stat merge for `reader = "new"`.
//!
//! ocr-v1 still decides game start and end, screen detection, and the
//! plausibility holds. This module only chooses the values stored on the
//! member's own row. A field from [`BoardRead`] is used when it has a value
//! and is not suspect. Anything else keeps that field's ocr-v1 value and is
//! listed in `suspect_fields` under the flat names the server accepts.
//!
//! The whole ocr-v1 read is kept, with recognizer `ocr-v1` and no suspect
//! list, when the board is missing, the team size is unknown, or the member's
//! own row is uncertain or does not agree with the new reader's row index.

use scuffed_types::{RECOGNIZER_OCR_V1, SUSPECT_FIELD_NAMES};

use crate::shadow::digits::RECOGNIZER_ID;
use crate::shadow::{BoardRead, BoardStatus, Value};

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
/// A configured player name counts only when that name matches a row. With
/// no name, the brightness row ocr-v1 already fell back to is the own row.
/// A name that does not match is uncertain even if a brightness row exists.
pub fn own_row(
    player_name: Option<&str>,
    name_match: Option<usize>,
    fallback_row: Option<usize>,
    team_size: usize,
) -> OwnRow {
    let named = player_name.is_some_and(|name| !name.trim().is_empty());
    let index = if named { name_match } else { fallback_row };
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
    if board.status != BoardStatus::Read {
        return keep();
    }
    let OwnRow::Identified { index, team_size } = own else {
        return keep();
    };
    // Do not guess 5 or 6. A missing or different team size means the new
    // reader's row index is not the member's row.
    let Some(read_size) = board.team_size else {
        return keep();
    };
    if read_size != team_size || index >= team_size.saturating_mul(2) {
        return keep();
    }

    let mut suspect = Vec::new();
    let map = take_text(board, "map", &mut suspect, "map").unwrap_or_else(|| ocr.map.clone());
    let mode = take_text(board, "mode", &mut suspect, "mode").unwrap_or_else(|| ocr.mode.clone());
    let result =
        take_text(board, "result", &mut suspect, "result").unwrap_or_else(|| ocr.result.clone());
    let hero_name = format!("r{index}.hero");
    let hero =
        take_text(board, &hero_name, &mut suspect, "hero").unwrap_or_else(|| ocr.hero.clone());
    let stats = [
        ("e", ocr.elims),
        ("a", ocr.assists),
        ("d", ocr.deaths),
        ("dmg", ocr.damage),
        ("h", ocr.healing),
        ("mit", ocr.mitigation),
    ];
    let mut numbers = [0u32; 6];
    for (slot, (suffix, fallback)) in stats.into_iter().enumerate() {
        let name = format!("r{index}.{suffix}");
        numbers[slot] = take_int(board, &name, &mut suspect, suffix).unwrap_or(fallback);
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
        recognizer: RECOGNIZER_ID,
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

fn take_text(
    board: &BoardRead,
    name: &str,
    suspect: &mut Vec<String>,
    flat: &str,
) -> Option<String> {
    match confident_value(board, name) {
        Some(Value::Text(text)) if !text.trim().is_empty() => Some(text),
        _ => {
            push_suspect(suspect, flat);
            None
        }
    }
}

fn take_int(board: &BoardRead, name: &str, suspect: &mut Vec<String>, flat: &str) -> Option<u32> {
    match confident_value(board, name) {
        Some(Value::Int(n)) => Some(n),
        _ => {
            push_suspect(suspect, flat);
            None
        }
    }
}

fn confident_value(board: &BoardRead, name: &str) -> Option<Value> {
    let field = board.get(name)?;
    if field.suspect {
        return None;
    }
    field.value.clone()
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
        assert_eq!(saved.map, "Ilios");
        assert_eq!(saved.mode, "Control");
        assert_eq!(saved.result, "defeat");
        assert_eq!(saved.hero, "Kiriko");
        assert_eq!(saved.elims, 21);
        assert_eq!(saved.assists, 9);
        assert_eq!(saved.deaths, 3);
        assert_eq!(saved.damage, 9100);
        assert_eq!(saved.healing, 1200);
        assert_eq!(saved.mitigation, 50);
        assert_eq!(saved.recognizer, RECOGNIZER_ID);
        assert!(saved.suspect_fields.is_empty());
        assert!(!saved.recognizer.contains(' '), "{:?}", saved.recognizer);
    }

    #[test]
    fn suspect_digit_falls_back_for_that_field_only() {
        let mut board = confident_board(5, 1);
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
        assert_eq!(
            saved.suspect_fields,
            vec!["map".to_string(), "mode".into(), "hero".into()]
        );
        assert_eq!(saved.recognizer, RECOGNIZER_ID);
    }

    #[test]
    fn name_match_is_the_own_row_and_a_miss_is_uncertain() {
        assert_eq!(own_row(Some("Ada"), Some(3), Some(0), 5), identified(3, 5));
        assert_eq!(own_row(Some("Ada"), None, Some(0), 5), OwnRow::Uncertain);
        assert_eq!(own_row(None, None, Some(1), 6), identified(1, 6));
        assert_eq!(own_row(Some("  "), None, Some(1), 6), identified(1, 6));
        assert_eq!(own_row(None, None, None, 5), OwnRow::Uncertain);
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
