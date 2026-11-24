//! Tests of the pure row rules: `latest_per_game` and the majority hero/map.

use super::tests::snap;
use super::*;

/// cv-v7 reads Tab boards with a dimmed header (a dimmed scene, or the
/// board behind an overlay). Such a read is a snapshot like any other
/// Tab: a clean board read later in the same game is newer and wins, so
/// the dimmed read is the game's row only when no later Tab was read.
#[test]
fn a_later_clean_board_replaces_a_dimmed_read_in_the_same_game() {
    let mut dimmed = snap("g", 9);
    dimmed.suspect_fields = vec!["deaths".into()];
    let clean = snap("g", 12);
    let games = latest_per_game(vec![clean, dimmed]);
    assert_eq!(games.len(), 1);
    assert_eq!(games[0].elims, 12);
    assert!(games[0].suspect_fields.is_empty());
}

#[test]
fn latest_per_game_keeps_final_snapshot_per_session() {
    // Newest-first input: the first snapshot per session is the final one.
    let rows = vec![snap("b", 30), snap("a", 20), snap("a", 10), snap("a", 5)];
    let games = latest_per_game(rows);
    assert_eq!(games.len(), 2);
    assert_eq!(games[0].session_id, "b");
    assert_eq!(games[1].elims, 20);
}

/// A later snapshot that did not read a name must not drop the recognizer
/// an earlier snapshot of the same match stored. The uploaded row is the
/// newest one, so the recognizer is copied onto it.
#[test]
fn latest_per_game_keeps_an_earlier_name_recognizer() {
    let mut older = snap("s", 4);
    older.name_recognizer = Some("names-v1".into());
    let newer = snap("s", 9);
    let games = latest_per_game(vec![newer, older]);
    assert_eq!(games.len(), 1);
    assert_eq!(games[0].elims, 9);
    assert_eq!(games[0].name_recognizer.as_deref(), Some("names-v1"));
    let mut named = snap("s2", 1);
    named.name_recognizer = Some("ocr-v1".into());
    let games = latest_per_game(vec![named]);
    assert_eq!(games[0].name_recognizer.as_deref(), Some("ocr-v1"));
}

#[test]
fn majority_map_ignores_empty_and_picks_dominant() {
    let mut rows: Vec<PersonalMatch> = (0..5).map(|_| snap("a", 1)).collect();
    rows[0].map_name = "King's Row".into();
    for r in rows.iter_mut().skip(2) {
        r.map_name = "Circuit Royal".into();
    }
    // rows[1] stays empty — doesn't vote
    assert_eq!(majority_map(&rows).as_deref(), Some("Circuit Royal"));
    assert_eq!(majority_map(&[snap("a", 1)]), None);
}

#[test]
fn majority_hero_ignores_unknown_and_picks_dominant() {
    let mut rows: Vec<PersonalMatch> = (0..5).map(|_| snap("a", 1)).collect();
    rows[0].hero = "illari".into();
    rows[1].hero = "Unknown".into();
    for r in rows.iter_mut().skip(2) {
        r.hero = "Wrecking Ball".into();
    }
    assert_eq!(majority_hero(&rows).as_deref(), Some("Wrecking Ball"));
    assert_eq!(majority_hero(&[]), None);
}

#[test]
fn latest_per_game_passes_legacy_rows_through() {
    let rows = vec![snap("", 1), snap("", 2), snap("a", 3), snap("a", 4)];
    assert_eq!(latest_per_game(rows).len(), 3);
}
