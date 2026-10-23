//! The rc.9 split guard: a Tab past a finished game's grace window
//! ([`finished_game_tab`]).

use std::time::Duration;

use super::board::{implausible_growth, named_map};
use crate::capture_gate::{Counters, GATE_COLS, GateState};

/// What a Tab past the post-match grace window of a finished game shows
/// ([`finished_game_tab`]). 0.5.0-rc.9, R9-2.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FinishedGameTab {
    /// Not shown to be the same game: the next game, as before rc.9.
    NewGame,
    /// The same game's board with nothing moved on, or only one column (one
    /// column can be a misread): a late look at the same board. The game
    /// keeps its result, and the board is not stored (it adds nothing, and
    /// a one-column rise may be a misread).
    SameBoard,
    /// The same game, still being played: a board accepted after the result
    /// was recorded has since moved on in two or more columns, and none fell
    /// except at most one by an OCR-sized step. The recorded result was
    /// false (a mid-match frame taken for an end).
    StillPlaying,
}

/// The last accepted board is far enough into a match that neither a new
/// game nor a late quick-play backfill can be mistaken for it soon after:
/// elims at least 4, deaths at least 2 and damage at least 1500, all with a
/// number. Stricter than the reset rule's [`mature_board`], because a wrong
/// merge here clears a real result.
///
/// [`mature_board`]: super::board::mature_board
fn probe_mature(prev: &Counters, known: &[bool; GATE_COLS]) -> bool {
    known[0] && known[2] && known[3] && prev.elims >= 4 && prev.deaths >= 2 && prev.damage >= 1500
}

/// Whether a Tab past the grace window of a finished game reads that same
/// game's board. `prev` is the game's last accepted board, `elapsed` the time
/// since it was accepted, `board_after_result` whether that board was
/// accepted after the result was recorded.
///
/// On 2026-10-22 a red mid-match frame was taken for DEFEAT, and the first
/// Tab after the grace window opened a new session for a board on the same
/// map whose elims and deaths were still rising (9/8 taken after the false
/// result, then 10/9): one game split in two. A board is the same game only
/// when every check below holds; anything that cannot be compared is
/// [`FinishedGameTab::NewGame`], the rule before rc.9:
///
/// * the map read on this Tab names the game's map;
/// * the own row is identified, its cells parsed, and not suspect;
/// * elims, deaths and damage have a number on both boards: read on this
///   Tab (not held, not edge-ink suspect) and confirmed on the last
///   accepted board;
/// * the last accepted board is mature ([`probe_mature`]), so a new game's
///   board, a quick-play backfill included, can never look like it;
/// * the board is judged as a set, not on one value: no compared column is
///   below the last accepted board, except that one column may be a little
///   below it (one kill-column count, or a tenth of a wide column), an OCR
///   misread, when two or more others moved on. A new game starts every
///   column back near zero, so it fails this on several columns at once;
/// * no growth above the rate ceiling for `elapsed`.
///
/// A board that moved on in two or more columns is
/// [`FinishedGameTab::StillPlaying`] only when the board it is compared with
/// was accepted after the result. Compared with a board from before the
/// result, the rise is what any real end looks like (the last fight between
/// that Tab and the end), so it is not evidence against the result:
/// [`FinishedGameTab::NewGame`], as before rc.9, which keeps the result.
///
/// The caller asks only while the last accepted board is recent (main's
/// `FINISHED_GAME_TAB_MAX_GAP`).
#[allow(clippy::too_many_arguments)]
pub fn finished_game_tab(
    prev: &GateState,
    session_map: Option<&str>,
    incoming_map: Option<&str>,
    cur: &Counters,
    suspect: &[bool; GATE_COLS],
    held: &[bool; GATE_COLS],
    own_row_trusted: bool,
    elapsed: Duration,
    board_after_result: bool,
) -> FinishedGameTab {
    let same_map = matches!(
        (named_map(session_map), named_map(incoming_map)),
        (Some(session), Some(incoming)) if session.eq_ignore_ascii_case(incoming)
    );
    if !same_map || !own_row_trusted {
        return FinishedGameTab::NewGame;
    }
    let known: [bool; GATE_COLS] = std::array::from_fn(|col| {
        !held[col] && !suspect[col] && !prev.unread[col] && !prev.unconfirmed[col]
    });
    if !probe_mature(&prev.accepted, &known) {
        return FinishedGameTab::NewGame;
    }
    let before = prev.accepted.to_array();
    let now = cur.to_array();
    let fell: Vec<usize> = (0..GATE_COLS)
        .filter(|&col| known[col] && now[col] < before[col])
        .collect();
    // An OCR-sized step down: one count in elims, assists or deaths, a
    // tenth of damage, healing or mitigation.
    let small_step = |col: usize| {
        let drop = before[col] - now[col];
        if col <= 2 {
            drop <= 1
        } else {
            drop.saturating_mul(10) <= before[col]
        }
    };
    if fell.len() > 1 || fell.iter().any(|&col| !small_step(col)) {
        return FinishedGameTab::NewGame;
    }
    // A column only this Tab has no number for is left out of the ceiling
    // too: `implausible_growth` skips held and never-read columns.
    let skipped: [bool; GATE_COLS] = std::array::from_fn(|col| !known[col]);
    if implausible_growth(prev, cur, &skipped, elapsed) {
        return FinishedGameTab::NewGame;
    }
    let grew = (0..GATE_COLS)
        .filter(|&col| known[col] && now[col] > before[col])
        .count();
    match (grew >= 2, fell.is_empty()) {
        (true, _) if board_after_result => FinishedGameTab::StillPlaying,
        // Rising past a board from before the result: the game's own final
        // board, or another game. Not evidence against the result.
        (true, _) => FinishedGameTab::NewGame,
        (false, true) => FinishedGameTab::SameBoard,
        // A small drop with nothing else moving is not shown to be anything.
        (false, false) => FinishedGameTab::NewGame,
    }
}
