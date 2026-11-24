//! Map-suspect checks in [`super::merge_saved`]: a board name that
//! canonicalizes to the held map must clear the warning, and a different
//! map must keep it.

use crate::shadow::Value;

use super::tests::{confident_board, identified, ocr};
use super::*;

#[test]
fn a_kings_row_alias_on_the_board_does_not_keep_the_map_warning() {
    // The policy map is the display name. The board often has the OCR
    // form with no apostrophe, or the banner in all caps. A raw string
    // compare treated those as a different map, so the map warning
    // stayed up for the rest of the game.
    // KINGS R0W is a glyph-fold / fuzzy near-miss (0 reads as O) that
    // canonical_map still resolves to King's Row.
    let aliases = [
        "Kings Row",
        "KING'S ROW",
        "kings row",
        "King’s Row",
        "KINGS R0W",
    ];
    for board_name in aliases {
        let mut policy = ocr();
        policy.map = "King's Row".into();
        policy.mode = "Hybrid".into();
        let mut board = confident_board(5, 0);
        let map = board
            .fields
            .iter_mut()
            .find(|field| field.name == "map")
            .unwrap();
        map.value = Some(Value::Text(board_name.into()));
        let saved = merge_saved(&policy, identified(0, 5), Some(&board));
        assert_eq!(saved.map, "King's Row", "{board_name}");
        assert_eq!(saved.mode, "Hybrid", "{board_name}");
        assert!(
            !saved.suspect_fields.iter().any(|name| name == "map"),
            "board {board_name:?} must clear the map warning"
        );
    }

    // A genuinely different map still flags the field and keeps policy.
    let mut policy = ocr();
    policy.map = "King's Row".into();
    policy.mode = "Hybrid".into();
    let saved = merge_saved(&policy, identified(0, 5), Some(&confident_board(5, 0)));
    assert_eq!(saved.map, "King's Row");
    assert!(saved.suspect_fields.iter().any(|name| name == "map"));
}

#[test]
fn adlersbrunn_on_the_board_matches_a_held_eichenwalde() {
    // Adlersbrunn is the Halloween label of Eichenwalde. Both names
    // canonicalize to Eichenwalde, so a later Tab must not keep the
    // map warning up. A genuinely different map still flags it.
    let mut policy = ocr();
    policy.map = "Eichenwalde".into();
    policy.mode = "Hybrid".into();
    let mut board = confident_board(5, 0);
    let map = board
        .fields
        .iter_mut()
        .find(|field| field.name == "map")
        .unwrap();
    map.value = Some(Value::Text("Adlersbrunn".into()));
    let saved = merge_saved(&policy, identified(0, 5), Some(&board));
    assert_eq!(saved.map, "Eichenwalde");
    assert_eq!(saved.mode, "Hybrid");
    assert!(
        !saved.suspect_fields.iter().any(|name| name == "map"),
        "Adlersbrunn must not keep the map warning against Eichenwalde"
    );

    let saved = merge_saved(&policy, identified(0, 5), Some(&confident_board(5, 0)));
    assert_eq!(saved.map, "Eichenwalde");
    assert!(saved.suspect_fields.iter().any(|name| name == "map"));
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
