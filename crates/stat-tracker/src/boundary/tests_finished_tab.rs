//! Tests of the rc.9 split guard: `finished_game_tab` and `probe_mature`.

use super::tests::{CLEAN, counters, gate};
use super::*;

/// R9-2: a Tab past the grace window of a finished game, against its
/// last accepted board, taken after the result. The 2026-10-22 Route 66
/// game: Defeat taken off a red mid-match frame, then a board 9/8, the
/// next Tab 40 s later 10/9.
#[test]
fn a_finished_game_tab_is_the_same_game_only_on_rising_stats_and_the_same_map() {
    use FinishedGameTab::{NewGame, SameBoard, StillPlaying};
    let prev = gate(counters(9, 3, 8, 4200, 0, 3100));
    let route = Some("Route 66");
    let at = |cur: Counters, map: Option<&str>| {
        finished_game_tab(
            &prev,
            route,
            map,
            &cur,
            &CLEAN,
            &CLEAN,
            true,
            Duration::from_secs(40),
            true,
        )
    };
    // the tester's board: still the same game, still being played
    assert_eq!(at(counters(10, 3, 9, 4600, 0, 3400), route), StillPlaying);
    assert_eq!(
        at(counters(10, 3, 9, 4600, 0, 3400), Some("route 66")),
        StillPlaying,
        "map names compare without case"
    );
    // the same board again, or one column moved (could be a misread)
    assert_eq!(at(counters(9, 3, 8, 4200, 0, 3100), route), SameBoard);
    assert_eq!(at(counters(9, 3, 8, 4500, 0, 3100), route), SameBoard);
    // a new game on the same map right after a loss: counters reset
    assert_eq!(at(counters(0, 0, 0, 0, 0, 0), route), NewGame);
    assert_eq!(at(counters(2, 1, 1, 600, 0, 400), route), NewGame);
    // a quick-play backfill into a game in progress: the own row starts
    // at zero, whatever the match clock says
    assert_eq!(at(counters(1, 0, 0, 350, 0, 0), route), NewGame);
    // one column a little below (an OCR misread) while others moved on
    assert_eq!(at(counters(8, 3, 9, 4600, 0, 3400), route), StillPlaying);
    assert_eq!(at(counters(10, 3, 9, 4100, 0, 3400), route), StillPlaying);
    // but not a big drop, two drops, or a drop with nothing else moving
    assert_eq!(at(counters(6, 3, 9, 4600, 0, 3400), route), NewGame);
    assert_eq!(at(counters(10, 3, 9, 3700, 0, 3400), route), NewGame);
    assert_eq!(at(counters(8, 3, 7, 4600, 0, 3400), route), NewGame);
    assert_eq!(at(counters(8, 3, 8, 4200, 0, 3100), route), NewGame);
    // another map, or no map to compare
    assert_eq!(
        at(counters(10, 3, 9, 4600, 0, 3400), Some("Hanaoka")),
        NewGame
    );
    assert_eq!(at(counters(10, 3, 9, 4600, 0, 3400), None), NewGame);
    assert_eq!(
        at(counters(10, 3, 9, 4600, 0, 3400), Some("unknown")),
        NewGame
    );
    assert_eq!(
        finished_game_tab(
            &prev,
            None,
            route,
            &counters(10, 3, 9, 4600, 0, 3400),
            &CLEAN,
            &CLEAN,
            true,
            Duration::from_secs(40),
            true,
        ),
        NewGame,
        "a game with no map of its own"
    );
    // growth above the rate ceiling for 40 s is a misread
    assert_eq!(at(counters(40, 3, 9, 4600, 0, 3400), route), NewGame);
}

/// R9-2 review (H1): the last board was taken before a real result (a
/// Tab in the final fight), and the frozen final board is opened past
/// the grace window. The rise is the last fight, not a live game: the
/// result is not cleared (new game, as before rc.9). The same board is
/// still a late look at it.
#[test]
fn a_rise_past_a_board_from_before_the_result_never_clears_it() {
    use FinishedGameTab::{NewGame, SameBoard};
    let prev = gate(counters(9, 3, 8, 4200, 0, 3100));
    let route = Some("Route 66");
    let at = |cur: Counters| {
        finished_game_tab(
            &prev,
            route,
            route,
            &cur,
            &CLEAN,
            &CLEAN,
            true,
            Duration::from_secs(80),
            false,
        )
    };
    assert_eq!(at(counters(10, 3, 9, 4600, 0, 3400)), NewGame);
    assert_eq!(at(counters(8, 3, 9, 4600, 0, 3400)), NewGame);
    assert_eq!(at(counters(9, 3, 8, 4200, 0, 3100)), SameBoard);
    assert_eq!(at(counters(9, 3, 8, 4500, 0, 3100)), SameBoard);
}

/// R9-2 review (L1): a low-stat last board, such as a late quick-play
/// backfill that ended in a real loss, is never compared: the next game
/// on the same map could rise past it within the window.
#[test]
fn a_low_stat_last_board_is_never_continued() {
    let route = Some("Busan");
    for (prev, cur) in [
        // the reviewers' backfills
        (counters(4, 1, 0, 650, 0, 0), counters(4, 2, 0, 900, 0, 0)),
        (counters(1, 0, 0, 450, 0, 0), counters(2, 1, 0, 900, 0, 300)),
        // each bar alone: elims 4, deaths 2, damage 1500
        (counters(3, 2, 5, 3000, 0, 0), counters(5, 3, 6, 3600, 0, 0)),
        (counters(6, 2, 1, 3000, 0, 0), counters(7, 3, 2, 3600, 0, 0)),
        (counters(6, 2, 5, 1400, 0, 0), counters(7, 3, 6, 1900, 0, 0)),
    ] {
        assert_eq!(
            finished_game_tab(
                &gate(prev),
                route,
                route,
                &cur,
                &CLEAN,
                &CLEAN,
                true,
                Duration::from_secs(60),
                true,
            ),
            FinishedGameTab::NewGame,
            "{prev:?} -> {cur:?}"
        );
    }
}

/// R9-2: what cannot be compared keeps the rule before rc.9.
#[test]
fn a_finished_game_tab_that_cannot_be_compared_is_a_new_game() {
    use FinishedGameTab::{NewGame, StillPlaying};
    let rising = counters(10, 3, 9, 4600, 0, 3400);
    let route = Some("Route 66");
    let check = |prev: &GateState, suspect: [bool; GATE_COLS], held, trusted| {
        finished_game_tab(
            prev,
            route,
            route,
            &rising,
            &suspect,
            &held,
            trusted,
            Duration::from_secs(40),
            true,
        )
    };
    let prev = gate(counters(9, 3, 8, 4200, 0, 3100));
    assert_eq!(check(&prev, CLEAN, CLEAN, true), StillPlaying);
    // the own row was not identified, or is suspect
    assert_eq!(check(&prev, CLEAN, CLEAN, false), NewGame);
    // elims, deaths or damage without a number on this Tab
    for col in [0, 2, 3] {
        let mut flagged = CLEAN;
        flagged[col] = true;
        assert_eq!(check(&prev, flagged, CLEAN, true), NewGame, "suspect {col}");
        assert_eq!(check(&prev, CLEAN, flagged, true), NewGame, "held {col}");
    }
    // or never read, or not confirmed, on the last accepted board
    let mut unread = prev;
    unread.unread[2] = true;
    assert_eq!(check(&unread, CLEAN, CLEAN, true), NewGame);
    let mut unconfirmed = prev;
    unconfirmed.unconfirmed[0] = true;
    assert_eq!(check(&unconfirmed, CLEAN, CLEAN, true), NewGame);
    // another column without a number is left out, not a veto
    let mut healing_held = CLEAN;
    healing_held[4] = true;
    assert_eq!(check(&prev, CLEAN, healing_held, true), StillPlaying);
    // an early board (not mature) is not compared: a new game's board
    // could be above it
    let early = gate(counters(3, 0, 3, 1400, 0, 100));
    assert_eq!(
        finished_game_tab(
            &early,
            route,
            route,
            &counters(4, 0, 4, 1700, 0, 200),
            &CLEAN,
            &CLEAN,
            true,
            Duration::from_secs(40),
            true,
        ),
        NewGame
    );
}
