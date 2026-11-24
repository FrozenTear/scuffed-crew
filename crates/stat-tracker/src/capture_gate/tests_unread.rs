//! Tests of held (unread) cells: alpha.7 O10.

use super::tests::{CLEAN, c, secs, state};
use super::*;

// ----- alpha.7 O10: cells the reader did not vouch for (`held`) -----
/// Column 4 (healing) held, nothing else.
const HELD_H: [bool; GATE_COLS] = [false, false, false, false, true, false];

#[test]
fn a_held_column_keeps_the_accepted_value_whatever_the_read_says() {
    let prev = state(c(11, 4, 2, 3000, 800, 400));
    // the reader's unsure guess for healing is 8000; it is not a number
    let gate = apply_gate_held(
        Some((prev, secs(20))),
        c(12, 4, 2, 3200, 8000, 450),
        CLEAN,
        HELD_H,
        false,
        true,
    );
    assert_eq!(gate.accepted, c(12, 4, 2, 3200, 800, 450));
    assert_eq!(
        gate.holds,
        vec![Hold {
            col: 4,
            kind: HoldKind::Unread,
            raw: 8000,
            held: 800,
        }]
    );
    assert_eq!(unsure_fields(&gate.holds), vec!["h"], "the cell is flagged");
    // the guess is not the previous raw read of the next capture
    assert_eq!(gate.state.last_raw, c(12, 4, 2, 3200, 800, 450));
    assert!(!gate.state.last_raw_suspect[4]);
    assert!(!gate.state.low_trust && gate.state.unread == [false; GATE_COLS]);
}

#[test]
fn a_held_read_never_corroborates_the_next_jump() {
    let prev = state(c(11, 4, 2, 3000, 800, 400));
    let held = apply_gate_held(
        Some((prev, secs(20))),
        c(11, 4, 2, 3000, 5000, 400),
        CLEAN,
        HELD_H,
        false,
        true,
    );
    // the next sure read is the same 5000, alone on a quiet board: with
    // the held 5000 as its previous raw read it would be corroborated
    let next = apply_gate(
        Some((held.state, secs(20))),
        c(11, 4, 2, 3000, 5000, 400),
        CLEAN,
        false,
    );
    assert_eq!(next.accepted.healing, 800, "one sure 5000 still waits");
    assert_eq!(next.holds[0].kind, HoldKind::Jump);
    // and a second sure 5000 is what stores it, as for any other jump
    let third = apply_gate(
        Some((next.state, secs(20))),
        c(11, 4, 2, 3000, 5000, 400),
        CLEAN,
        false,
    );
    assert_eq!(third.accepted.healing, 5000);
}

#[test]
fn a_first_capture_stores_zero_for_a_held_column_and_then_its_first_read() {
    let held = [true, false, false, false, true, false];
    let first = apply_gate_held(None, c(7, 1, 0, 900, 351, 0), CLEAN, held, false, true);
    assert_eq!(first.accepted, c(0, 1, 0, 900, 0, 0), "no guess is stored");
    assert_eq!(first.state.unread, held);
    assert!(first.holds.is_empty() && !first.state.low_trust);
    // 20 s later both cells read sure. Against the placeholder 0 the 25
    // elims are past the rate cap and the 3511 healing is a lone jump.
    let second = apply_gate(
        Some((first.state, secs(20))),
        c(25, 1, 0, 950, 3511, 0),
        CLEAN,
        false,
    );
    assert_eq!(second.accepted, c(25, 1, 0, 950, 3511, 0));
    assert!(second.holds.is_empty(), "{:?}", second.holds);
    assert_eq!(second.state.unread, [false; GATE_COLS]);
    assert_eq!(second.state.row_shift, [false; GATE_COLS]);
    // from here the column is an ordinary one: a drop is held
    let third = apply_gate(
        Some((second.state, secs(20))),
        c(25, 1, 0, 950, 35, 0),
        CLEAN,
        false,
    );
    assert_eq!(third.accepted.healing, 3511);
}

#[test]
fn a_column_held_twice_stays_unread_until_it_is_read() {
    let first = apply_gate_held(None, c(3, 1, 0, 900, 351, 0), CLEAN, HELD_H, false, true);
    let again = apply_gate_held(
        Some((first.state, secs(20))),
        c(3, 1, 0, 950, 77, 0),
        CLEAN,
        HELD_H,
        false,
        true,
    );
    assert_eq!(again.accepted.healing, 0);
    assert!(again.state.unread[4]);
    let read = apply_gate(
        Some((again.state, secs(20))),
        c(3, 1, 0, 950, 3511, 0),
        CLEAN,
        false,
    );
    assert_eq!(read.accepted.healing, 3511);
}

#[test]
fn the_first_read_of_an_unread_column_is_not_a_move_on_the_board() {
    // healing was never read; damage is at 3000
    let first = apply_gate_held(None, c(3, 1, 0, 3000, 0, 0), CLEAN, HELD_H, false, true);
    // healing reads 3000 for the first time while damage alone jumps
    // 5000: the first read does not make that jump one of two
    let second = apply_gate(
        Some((first.state, secs(20))),
        c(3, 1, 0, 8000, 3000, 0),
        CLEAN,
        false,
    );
    assert_eq!(second.accepted, c(3, 1, 0, 3000, 3000, 0));
    assert_eq!(
        second.holds,
        vec![Hold {
            col: 3,
            kind: HoldKind::Jump,
            raw: 8000,
            held: 3000,
        }]
    );
    // elims unread, then read as 20 on the frame where assists drop:
    // with a column down the first read is not taken at all (F266-2),
    // so it is no row-shift rise next to that drop either
    let held_e = [true, false, false, false, false, false];
    let first = apply_gate_held(None, c(0, 10, 2, 3000, 0, 0), CLEAN, held_e, false, true);
    let second = apply_gate(
        Some((first.state, secs(20))),
        c(20, 2, 2, 3000, 0, 0),
        CLEAN,
        false,
    );
    assert_eq!((second.accepted.elims, second.accepted.assists), (0, 10));
    assert!(second.state.unread[0], "still unread");
    assert!(
        !second.state.row_shift[0],
        "a first read is not a shifted row"
    );
    // the next board, with assists back, gives elims its first read
    let third = apply_gate(
        Some((second.state, secs(20))),
        c(21, 10, 2, 3100, 0, 0),
        CLEAN,
        false,
    );
    assert_eq!((third.accepted.elims, third.accepted.assists), (21, 10));
    assert!(third.holds.is_empty(), "{:?}", third.holds);
    assert!(!third.state.row_shift[0]);
}

#[test]
fn an_untrusted_first_read_is_unconfirmed_and_leaves_no_floor() {
    let first = apply_gate_held(None, c(3, 1, 0, 900, 0, 0), CLEAN, HELD_H, false, true);
    let second = apply_gate_with_trust(
        Some((first.state, secs(20))),
        c(3, 1, 0, 900, 500, 0),
        CLEAN,
        false,
        false,
    );
    assert_eq!(second.accepted.healing, 500);
    assert!(second.state.unconfirmed[4] && second.state.low_trust);
    assert!(
        !second.state.has_confirmed_floor[4],
        "the placeholder 0 is not a confirmed value"
    );
}

#[test]
fn a_held_column_keeps_its_row_shift_tag() {
    let mut prev = state(c(30, 4, 2, 3000, 800, 400));
    prev.row_shift[0] = true;
    let held_e = [true, false, false, false, false, false];
    let held = apply_gate_held(
        Some((prev, secs(20))),
        c(30, 4, 2, 3000, 800, 400),
        CLEAN,
        held_e,
        false,
        true,
    );
    assert!(
        held.state.row_shift[0],
        "no read, so nothing clears the tag"
    );
    // a real read of the same number, with no drop held, does clear it
    let read = apply_gate(
        Some((prev, secs(20))),
        c(30, 4, 2, 3000, 800, 400),
        CLEAN,
        false,
    );
    assert!(!read.state.row_shift[0]);
}

#[test]
fn a_split_does_not_carry_a_held_column_into_the_new_game() {
    let prev = state(c(30, 12, 9, 14000, 5000, 900));
    // the caller fills a held column with the old game's value
    let gate = apply_gate_held(
        Some((prev, secs(200))),
        c(1, 0, 0, 300, 5000, 0),
        CLEAN,
        HELD_H,
        true,
        true,
    );
    assert_eq!(gate.accepted, c(1, 0, 0, 300, 0, 0));
    assert!(gate.state.unread[4]);
}

#[test]
fn a_held_column_is_not_a_clean_read_for_an_unconfirmed_latch() {
    // a fallback latched assists 40. A board with a held cell is not a
    // clean read, so it does not replace the latch. The same numbers
    // with every cell read do.
    let first = apply_gate_with_trust(None, c(2, 40, 1, 450, 0, 2), CLEAN, false, false);
    let held = apply_gate_held(
        Some((first.state, secs(20))),
        c(2, 0, 1, 500, 0, 2),
        CLEAN,
        HELD_H,
        false,
        true,
    );
    assert_eq!(
        held.accepted.assists, 40,
        "a board with a held cell replaces nothing"
    );
    let clean = apply_gate(
        Some((first.state, secs(20))),
        c(2, 0, 1, 500, 0, 2),
        CLEAN,
        false,
    );
    assert_eq!(
        clean.accepted.assists, 0,
        "the same read, fully clean, does"
    );
}

#[test]
fn a_game_with_no_held_cell_never_uses_the_held_state() {
    // ocr-v1 has a number for every column on every capture. Through a
    // noisy game (drops, a rate cap, a fallback, an inject, an un-latch)
    // nothing of the held machinery may switch on: no column is unread,
    // no held time builds up behind a gate hold, no cell is `Unread`.
    let reads = [
        (c(3, 1, 0, 900, 0, 120), CLEAN, true),
        (c(5, 1, 1, 1500, 0, 300), CLEAN, true),
        (
            c(1, 1, 1, 15031, 0, 300),
            [false, false, false, true, false, false],
            true,
        ),
        (c(40, 18, 1, 1600, 0, 310), CLEAN, false),
        (c(6, 2, 1, 1700, 0, 22994), CLEAN, true),
        (c(6, 2, 1, 1700, 0, 350), CLEAN, true),
        (c(6, 2, 1, 1700, 0, 350), CLEAN, true),
    ];
    let mut state: Option<GateState> = None;
    let mut kinds = Vec::new();
    for (i, (raw, suspect, trusted)) in reads.into_iter().enumerate() {
        let prev = state.map(|s| (s, secs(20)));
        let out = apply_gate_with_trust(prev, raw, suspect, false, trusted);
        assert_eq!(out.state.unread, [false; GATE_COLS], "step {i}");
        assert_eq!(out.state.held_secs, [0; GATE_COLS], "step {i}");
        assert!(out.state.unread_cols().is_empty(), "step {i}");
        assert!(
            out.holds.iter().all(|hold| hold.kind != HoldKind::Unread),
            "step {i}: {:?}",
            out.holds
        );
        // ocr-v1 flags a drop and a lone jump only; the new reader's
        // list is that plus the rate-cap and inject holds
        let unsure = unsure_fields(&out.holds);
        let all = held_fields(&out.holds);
        assert!(unsure.iter().all(|name| all.contains(name)), "step {i}");
        kinds.extend(out.holds.iter().map(|hold| hold.kind));
        state = Some(out.state);
    }
    // the game above really does exercise the holds this is about
    for kind in [
        HoldKind::Monotonic,
        HoldKind::RateCap,
        HoldKind::DigitInject,
    ] {
        assert!(kinds.contains(&kind), "{kind:?} never fired: {kinds:?}");
    }
}

/// Healing held over a 5 minute run, the rest of the board read.
fn after_a_held_healing_run() -> GateState {
    let first = apply_gate(None, c(11, 4, 2, 3000, 800, 400), CLEAN, false);
    let held = apply_gate_held(
        Some((first.state, secs(300))),
        c(20, 8, 4, 9000, 0, 1500),
        CLEAN,
        HELD_H,
        false,
        true,
    );
    assert_eq!(held.accepted, c(20, 8, 4, 9000, 800, 1500));
    assert_eq!(held.state.held_secs, [0, 0, 0, 0, 300, 0]);
    held.state
}

/// F266-3 (R266-A), the reviewer's first trace: 800, held, then a sure
/// 8,500 after 20 s. Against one 20 s interval that is a trailing-digit
/// inject, held with no flag and with no way out. The accepted 800 is
/// 320 s old, and over 320 s the rise is ordinary.
#[test]
fn a_sure_read_after_a_held_run_is_not_a_trailing_digit_inject() {
    let run = after_a_held_healing_run();
    // the board moved on this capture: the read is stored at once
    let busy = apply_gate(
        Some((run, secs(20))),
        c(21, 8, 4, 9600, 8500, 1500),
        CLEAN,
        false,
    );
    assert_eq!(busy.accepted.healing, 8500, "{:?}", busy.holds);
    assert!(busy.holds.is_empty());
    assert_eq!(busy.state.held_secs, [0; GATE_COLS]);
    // on a quiet board it is a lone jump: kept one capture, FLAGGED,
    // and the held time keeps counting behind that hold
    let quiet = apply_gate(
        Some((run, secs(20))),
        c(20, 8, 4, 9100, 8500, 1500),
        CLEAN,
        false,
    );
    assert_eq!(quiet.accepted.healing, 800);
    assert_eq!(
        quiet.holds,
        vec![Hold {
            col: 4,
            kind: HoldKind::Jump,
            raw: 8500,
            held: 800,
        }]
    );
    assert_eq!(unsure_fields(&quiet.holds), vec!["h"]);
    assert_eq!(quiet.state.held_secs[4], 320);
    // the next sure read agrees and is stored: no inject hold repeats
    let next = apply_gate(
        Some((quiet.state, secs(20))),
        c(20, 8, 4, 9200, 8700, 1500),
        CLEAN,
        false,
    );
    assert_eq!(next.accepted.healing, 8700, "{:?}", next.holds);
    assert_eq!(next.state.held_secs, [0; GATE_COLS]);

    // the same reads with no held run are an inject, as on ocr-v1; the
    // new reader's list flags that hold, ocr-v1's does not
    let prev = state(c(20, 8, 4, 9000, 800, 1500));
    let inject = apply_gate(
        Some((prev, secs(20))),
        c(20, 8, 4, 9100, 8500, 1500),
        CLEAN,
        false,
    );
    assert_eq!(inject.accepted.healing, 800);
    assert_eq!(inject.holds[0].kind, HoldKind::DigitInject);
    assert!(unsure_fields(&inject.holds).is_empty());
    assert_eq!(held_fields(&inject.holds), vec!["h"]);
    assert_eq!(inject.state.held_secs, [0; GATE_COLS]);
}

/// F266-3, the second trace: elims 10, held, then a sure 25 after 20 s.
/// One interval allows 10 + 20/5 + 8 = 22, a rate-cap hold with no
/// flag. The 10 is 320 s old.
#[test]
fn a_sure_read_after_a_held_run_is_not_over_the_kill_rate_cap() {
    let held_e = [true, false, false, false, false, false];
    let first = apply_gate(None, c(10, 4, 2, 3000, 800, 400), CLEAN, false);
    let held = apply_gate_held(
        Some((first.state, secs(300))),
        c(0, 9, 5, 9000, 4000, 1500),
        CLEAN,
        held_e,
        false,
        true,
    );
    assert_eq!(held.accepted.elims, 10);
    assert_eq!(held.state.held_secs[0], 300);
    // a second held capture adds its own gap
    let again = apply_gate_held(
        Some((held.state, secs(15))),
        c(0, 9, 5, 9100, 4000, 1500),
        CLEAN,
        held_e,
        false,
        true,
    );
    assert_eq!(again.state.held_secs[0], 315);
    // assists moved too: stored at once
    let busy = apply_gate(
        Some((held.state, secs(20))),
        c(25, 11, 5, 9300, 4000, 1500),
        CLEAN,
        false,
    );
    assert_eq!(busy.accepted.elims, 25, "{:?}", busy.holds);
    // alone on a quiet board: a flagged lone jump, then stored
    let quiet = apply_gate(
        Some((held.state, secs(20))),
        c(25, 9, 5, 9100, 4000, 1500),
        CLEAN,
        false,
    );
    assert_eq!(quiet.accepted.elims, 10);
    assert_eq!(quiet.holds[0].kind, HoldKind::Jump);
    assert_eq!(unsure_fields(&quiet.holds), vec!["e"]);
    let next = apply_gate(
        Some((quiet.state, secs(20))),
        c(25, 9, 5, 9200, 4000, 1500),
        CLEAN,
        false,
    );
    assert_eq!(next.accepted.elims, 25);

    // no held run: 25 is past 22, a rate cap; only the new reader's
    // list flags it
    let prev = state(c(10, 9, 5, 9000, 4000, 1500));
    let capped = apply_gate(
        Some((prev, secs(20))),
        c(25, 9, 5, 9100, 4000, 1500),
        CLEAN,
        false,
    );
    assert_eq!(capped.accepted.elims, 10);
    assert_eq!(capped.holds[0].kind, HoldKind::RateCap);
    assert!(unsure_fields(&capped.holds).is_empty());
    assert_eq!(held_fields(&capped.holds), vec!["e"]);
}

/// A column back from a held run moved over that whole run. Its rise
/// must not make another column's lone jump look like one of two.
#[test]
fn a_column_back_from_a_held_run_does_not_vouch_for_another_jump() {
    let run = after_a_held_healing_run();
    // healing reads 3,800 (+3,000 over 320 s) while damage alone jumps
    // 9,000 to 15,000 in 20 s
    let out = apply_gate(
        Some((run, secs(20))),
        c(20, 8, 4, 15000, 3800, 1500),
        CLEAN,
        false,
    );
    assert_eq!(out.accepted.damage, 9000, "the damage jump still waits");
    assert!(
        out.holds
            .iter()
            .any(|hold| hold.col == 3 && hold.kind == HoldKind::Jump),
        "{:?}",
        out.holds
    );
}

/// F266-2: a game whose healing was never read, its end missed, and the
/// next game's first board 60 s later. Every read column drops. The
/// healing on that board is the next game's and is not taken into this
/// one, where nothing would ever flag it.
#[test]
fn the_first_read_of_an_unread_column_is_not_taken_when_the_board_drops() {
    let first = apply_gate_held(
        None,
        c(30, 12, 9, 14000, 0, 900),
        CLEAN,
        HELD_H,
        false,
        true,
    );
    assert_eq!(first.state.unread_cols(), vec!["h"]);
    let next_game = apply_gate(
        Some((first.state, secs(60))),
        c(1, 0, 0, 300, 100, 0),
        CLEAN,
        false,
    );
    assert_eq!(next_game.accepted, c(30, 12, 9, 14000, 0, 900));
    assert_eq!(
        next_game.state.unread_cols(),
        vec!["h"],
        "the column is still unread, so the game still has no row"
    );
    assert!(next_game.holds.contains(&Hold {
        col: 4,
        kind: HoldKind::Unread,
        raw: 100,
        held: 0,
    }));
    assert!(held_fields(&next_game.holds).contains(&"h"));
    // the refused read is not the previous raw read either
    assert_eq!(next_game.state.last_raw.healing, 0);
    // and the state remembers it was seen, for the not-stored line
    assert_eq!(next_game.state.refused_cols(), vec!["h"]);
    assert!(
        first.state.refused_cols().is_empty(),
        "never read is not refused"
    );
    // one dropped column is enough to refuse it
    let one_drop = apply_gate(
        Some((first.state, secs(60))),
        c(31, 12, 9, 300, 100, 900),
        CLEAN,
        false,
    );
    assert!(one_drop.state.unread[4]);
    assert_eq!(one_drop.accepted.healing, 0);
    // a board of this game, no column down: the first read is stored
    let same_game = apply_gate(
        Some((first.state, secs(60))),
        c(31, 12, 9, 14200, 100, 900),
        CLEAN,
        false,
    );
    assert_eq!(same_game.accepted.healing, 100);
    assert!(same_game.state.unread_cols().is_empty());
    assert!(same_game.holds.is_empty());
    // a refused read that is taken later is no longer "refused"
    let taken = apply_gate(
        Some((next_game.state, secs(60))),
        c(31, 12, 9, 14200, 100, 900),
        CLEAN,
        false,
    );
    assert!(taken.state.refused_cols().is_empty());
    assert_eq!(taken.state.refused_first_read, [false; GATE_COLS]);
}

/// The board carried onto a new session is a row of that game. A cell
/// the splitting capture has no number for takes the carried board's:
/// not a 0 under the number on the row before, and not the previous
/// game's value the caller filled it with.
#[test]
fn a_seeded_first_capture_takes_the_carried_board_in_a_held_column() {
    let old_game = state(c(30, 12, 9, 14000, 5000, 900));
    let carried = c(1, 0, 0, 300, 120, 0);
    let split = apply_gate_seeded(
        Some((old_game, secs(200))),
        c(3, 1, 0, 900, 5000, 0),
        CLEAN,
        HELD_H,
        true,
        true,
        Some((carried, secs(50))),
    );
    assert_eq!(split.accepted, c(3, 1, 0, 900, 120, 0));
    assert!(split.state.unread_cols().is_empty(), "the game has a read");
    assert_eq!(
        split.holds,
        vec![Hold {
            col: 4,
            kind: HoldKind::Unread,
            raw: 5000,
            held: 120,
        }]
    );
    assert_eq!(split.state.held_secs, [0, 0, 0, 0, 50, 0]);
    assert_eq!(split.state.last_raw.healing, 120);
    assert!(!split.state.last_raw_suspect[4]);
    // the same on a session's first capture (no split, no gate yet)
    let first = apply_gate_seeded(
        None,
        c(3, 1, 0, 900, 0, 0),
        CLEAN,
        HELD_H,
        false,
        true,
        Some((carried, secs(50))),
    );
    assert_eq!(first.accepted.healing, 120);
    // without a carried board there is nothing to take: unread
    let bare = apply_gate_held(
        Some((old_game, secs(200))),
        c(3, 1, 0, 900, 5000, 0),
        CLEAN,
        HELD_H,
        true,
        true,
    );
    assert_eq!(bare.accepted.healing, 0);
    assert_eq!(bare.state.unread_cols(), vec!["h"]);
    // the seed is for held columns of a first capture only: a read
    // column keeps its read, and a later capture ignores the seed
    let later = apply_gate_seeded(
        Some((old_game, secs(20))),
        c(31, 12, 9, 14100, 0, 900),
        CLEAN,
        HELD_H,
        false,
        true,
        Some((carried, secs(50))),
    );
    assert_eq!(later.accepted, c(31, 12, 9, 14100, 5000, 900));
}

/// R3-2: a suspect own row opened the game (untrusted, healing held),
/// higher than the member's real row. Trusted reads of the real row then
/// sit below it on every board and never repeat, so they would be
/// monotonic holds for the rest of the game and healing would never get
/// its first read. Nothing of the game is stored yet: the first trusted
/// board starts it over.
#[test]
fn a_trusted_board_restarts_a_game_that_has_only_low_trust_reads() {
    let first = apply_gate_held(
        None,
        c(20, 6, 3, 8000, 77, 450),
        CLEAN,
        HELD_H,
        false,
        false,
    );
    assert!(first.state.only_low_trust_reads());
    assert_eq!(first.state.unread_cols(), vec!["h"]);
    // another low-trust board does not restart anything
    let again = apply_gate_held(
        Some((first.state, secs(30))),
        c(21, 6, 3, 8200, 77, 450),
        CLEAN,
        HELD_H,
        false,
        false,
    );
    assert!(again.state.only_low_trust_reads());
    assert_eq!(again.accepted.elims, 21);
    // ...also not a lower one that reads healing: it is gated against
    // the first, so the game still has no row
    let lower = apply_gate_with_trust(
        Some((first.state, secs(30))),
        c(10, 6, 3, 4000, 500, 450),
        CLEAN,
        false,
        false,
    );
    assert_eq!(lower.accepted, c(20, 6, 3, 8000, 0, 450));
    assert_eq!(lower.state.unread_cols(), vec!["h"]);
    assert_eq!(lower.state.refused_cols(), vec!["h"]);
    // the member's real row, trusted: taken as a first capture
    let real = apply_gate(
        Some((again.state, secs(30))),
        c(12, 6, 3, 3500, 700, 450),
        CLEAN,
        false,
    );
    assert_eq!(real.accepted, c(12, 6, 3, 3500, 700, 450));
    assert!(real.holds.is_empty() && real.unlatches.is_empty());
    assert!(real.state.unread_cols().is_empty());
    assert!(!real.state.low_trust && !real.state.only_low_trust_reads());
    assert_eq!(real.state, state(c(12, 6, 3, 3500, 700, 450)));
    // a trusted board with a held cell restarts too, and stays pending
    let held = apply_gate_held(
        Some((again.state, secs(30))),
        c(12, 6, 3, 3500, 0, 450),
        CLEAN,
        HELD_H,
        false,
        true,
    );
    assert_eq!(held.accepted, c(12, 6, 3, 3500, 0, 450));
    assert_eq!(held.state.unread_cols(), vec!["h"]);
    assert!(!held.state.only_low_trust_reads(), "e..mit are confirmed");

    // a game that has a row is never restarted. An untrusted read in
    // the middle of it leaves low-trust columns. One later clean read
    // replaces them, even when it is lower. It does not start the game
    // over: the columns that were already confirmed keep their floors.
    let game = apply_gate(None, c(12, 5, 3, 3500, 700, 300), CLEAN, false);
    let suspect = apply_gate_with_trust(
        Some((game.state, secs(20))),
        c(20, 9, 5, 8000, 2000, 900),
        CLEAN,
        false,
        false,
    );
    assert!(suspect.state.low_trust && !suspect.state.only_low_trust_reads());
    let lower = apply_gate(
        Some((suspect.state, secs(20))),
        c(13, 5, 3, 3700, 720, 300),
        CLEAN,
        false,
    );
    assert_eq!(lower.accepted, c(13, 5, 3, 3700, 720, 300));
    assert!(!lower.state.low_trust);
    // and a pending game a trusted board opened is not restarted either
    let trusted_first = apply_gate_held(
        None,
        c(30, 12, 9, 14000, 0, 900),
        CLEAN,
        HELD_H,
        false,
        true,
    );
    assert!(!trusted_first.state.only_low_trust_reads());
}

/// Reviewer mutant: `unread` carried across a split. The new game's
/// unread columns are this capture's held cells and nothing else.
#[test]
fn unread_does_not_survive_a_split() {
    let first = apply_gate_held(
        None,
        c(30, 12, 9, 14000, 0, 900),
        CLEAN,
        HELD_H,
        false,
        true,
    );
    assert!(first.state.unread[4]);
    let split = apply_gate(
        Some((first.state, secs(200))),
        c(1, 0, 0, 300, 100, 0),
        CLEAN,
        true,
    );
    assert_eq!(split.accepted, c(1, 0, 0, 300, 100, 0));
    assert_eq!(split.state.unread, [false; GATE_COLS]);
    // a split with another column held: that one, not the old one
    let held_e = [true, false, false, false, false, false];
    let split = apply_gate_held(
        Some((first.state, secs(200))),
        c(7, 0, 0, 300, 100, 0),
        CLEAN,
        held_e,
        true,
        true,
    );
    assert_eq!(split.state.unread, held_e);
    assert_eq!(split.state.held_secs, [0; GATE_COLS]);
}

/// Reviewer mutant: the held guess written into `confirmed_floor`. An
/// untrusted capture leaves a floor under the column it moved, and
/// nothing under a column it has no number for.
#[test]
fn an_untrusted_capture_leaves_no_floor_under_a_held_cell() {
    let prev = state(c(11, 4, 2, 3000, 800, 400));
    let out = apply_gate_held(
        Some((prev, secs(20))),
        c(11, 4, 2, 3300, 8000, 400),
        CLEAN,
        HELD_H,
        false,
        false,
    );
    assert_eq!(out.accepted, c(11, 4, 2, 3300, 800, 400));
    // damage moved on an untrusted read: unconfirmed, floor 3000
    assert!(out.state.unconfirmed[3] && out.state.has_confirmed_floor[3]);
    assert_eq!(out.state.confirmed_floor[3], 3000);
    // healing was not read: still confirmed at 800, no floor, no guess
    assert!(!out.state.unconfirmed[4]);
    assert!(!out.state.has_confirmed_floor[4]);
    assert_eq!(out.state.confirmed_floor[4], 0);
}

#[test]
fn a_save_from_before_the_unread_column_loads_as_all_read() {
    let now = serde_json::to_value(state(c(11, 4, 2, 3000, 800, 400))).unwrap();
    let mut old = now.clone();
    for field in ["unread", "held_secs", "refused_first_read"] {
        old.as_object_mut()
            .unwrap()
            .remove(field)
            .expect("the field is saved");
    }
    let loaded: GateState = serde_json::from_value(old).unwrap();
    assert_eq!(loaded.unread, [false; GATE_COLS]);
    assert_eq!(loaded.held_secs, [0; GATE_COLS]);
    assert_eq!(loaded, state(c(11, 4, 2, 3000, 800, 400)));
    // a pending game survives a restart: both fields round-trip
    let mut pending = state(c(30, 12, 9, 14000, 0, 900));
    pending.unread[4] = true;
    pending.held_secs[0] = 315;
    let back: GateState = serde_json::from_value(serde_json::to_value(pending).unwrap()).unwrap();
    assert_eq!(back, pending);
    assert_eq!(back.unread_cols(), vec!["h"]);
}
