use super::outcome_from_board_header;
use crate::model::MatchOutcome;

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
