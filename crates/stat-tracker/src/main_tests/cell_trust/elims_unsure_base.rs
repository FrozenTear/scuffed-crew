//! Daemon tests: the unsure base (`GateState::unsure_base`) outlives one
//! misread Tab. After a held read below the accepted value drops a row to
//! the accepted value, the unsure value the row before showed stays the
//! base a later refused digit is measured from, across every following
//! misread of the cell, until a capture stores a value from it, the gate
//! accepts a sure read of the cell, or a new game opens. A sibling of
//! `elims_no_fallback.rs`, kept apart so both files stay under the size
//! limit.
//!
//! `Some((v, true))` is a cell the digit reader does not vouch for (held);
//! `sure(v)` is a vouched read; `None` is a cell with no number.

use super::*;

/// The Rialto-shaped other five columns, held still so every elims change
/// is a lone change on a quiet board.
const QUIET: [u32; 5] = [11, 2, 8656, 1263, 2574];

fn board(elims: Option<(u32, bool)>) -> O10Cells {
    let mut cells = [elims, None, None, None, None, None];
    for (cell, value) in cells[1..].iter_mut().zip(QUIET) {
        *cell = sure(value);
    }
    cells
}

/// 25, 29 sure, then a held unsure 33: the opening every series here
/// shares, with the Rialto gaps.
fn opening() -> Vec<(u64, O10Cells)> {
    vec![
        (0, all_sure([25, 9, 2, 6862, 1048, 2029])),
        (111, all_sure([29, 9, 2, 8446, 1263, 2429])),
        (70, board(Some((33, true)))),
    ]
}

/// Run `opening` and then `rest`; the stored rows and the session.
async fn run_series(rest: Vec<(u64, O10Cells)>) -> (SessionState, Vec<storage::PersonalMatch>) {
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let mut tabs = opening();
    tabs.extend(rest);
    let (st, seen) = o10_run(&ctx, tabs).await;
    assert!(seen.iter().all(|tab| tab.recorded), "{seen:?}");
    let rows = ctx.store.get_session_snapshots("o10").await.unwrap();
    (st, rows)
}

fn elims(rows: &[storage::PersonalMatch]) -> Vec<u32> {
    rows.iter().map(|r| r.elims).collect()
}

fn assert_e_flagged(rows: &[storage::PersonalMatch], from: usize) {
    for (i, row) in rows.iter().enumerate() {
        let flagged = row.suspect_fields.iter().any(|f| f == "e");
        assert_eq!(
            flagged,
            i >= from,
            "row {i} elims flag: {:?}",
            row.suspect_fields
        );
    }
}

/// FrozenTear's case: 25, 29, held 33, sure 27, sure 27, sure 35. The two sure
/// 27s are monotonic holds below the accepted 29; both rows drop to 29,
/// flagged, and the unsure 33 stays the base across both. From 33 the
/// final 35 is a +2, so it is stored, flagged. 27 is never stored. Before
/// the base carried over (89676647) this stored 25, 29, 33, 29, 29, 29.
#[tokio::test]
async fn the_unsure_base_survives_two_held_lower_reads() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let (st, rows) = run_series(vec![
        (20, board(sure(27))),
        (30, board(sure(27))),
        (60, board(sure(35))),
    ])
    .await;
    assert_eq!(elims(&rows), vec![25, 29, 33, 29, 29, 35], "stored elims");
    assert!(!elims(&rows).contains(&27), "the held 27 is never stored");
    assert_e_flagged(&rows, 2);
    let gate = st.active_game.as_ref().unwrap().gate.unwrap();
    assert_eq!(gate.accepted.elims, 29, "the gate keeps its accepted value");
    assert!(!gate.has_unsure_base[0], "the base ends once it is used");
    assert!(gate.has_unsure_shown[0]);
    assert_eq!(gate.unsure_shown[0], 35);
}

/// Three held lower reads (two sure 27s and a sure 28), then a sure 35:
/// the base lasts all three.
#[tokio::test]
async fn the_unsure_base_survives_three_held_lower_reads() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let (_, rows) = run_series(vec![
        (20, board(sure(27))),
        (30, board(sure(27))),
        (15, board(sure(28))),
        (60, board(sure(35))),
    ])
    .await;
    assert_eq!(
        elims(&rows),
        vec![25, 29, 33, 29, 29, 29, 35],
        "stored elims"
    );
    assert_e_flagged(&rows, 2);
}

/// A held lower read, then a Tab whose elims cell has no number, then a
/// sure 35: a cell with no read carries the base on.
#[tokio::test]
async fn the_unsure_base_survives_a_held_cell_with_no_read() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let (_, rows) = run_series(vec![
        (20, board(sure(27))),
        (10, board(None)),
        (60, board(sure(35))),
    ])
    .await;
    assert_eq!(elims(&rows), vec![25, 29, 33, 29, 29, 35], "stored elims");
    assert_e_flagged(&rows, 2);
}

/// A held lower read, then an unvouched held 27 (also below the accepted
/// 29), then a sure 35: an unvouched lower read carries the base on too.
#[tokio::test]
async fn the_unsure_base_survives_an_unvouched_held_lower_read() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let (_, rows) = run_series(vec![
        (20, board(sure(27))),
        (10, board(Some((27, true)))),
        (60, board(sure(35))),
    ])
    .await;
    assert_eq!(elims(&rows), vec![25, 29, 33, 29, 29, 35], "stored elims");
    assert!(!elims(&rows).contains(&27), "the held 27 is never stored");
}

/// A held lower read, then a sure 30 the gate accepts (+1 from 29): that
/// confirms the column and ends the base. A later sure 35 is a lone +5
/// jump from the new 30, so the gate holds it and the row keeps 30,
/// flagged. With the base still at 33 it would have stored 35.
#[tokio::test]
async fn a_confirming_sure_read_ends_the_unsure_base() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let (st, rows) = run_series(vec![
        (20, board(sure(27))),
        (30, board(sure(30))),
        (60, board(sure(35))),
    ])
    .await;
    assert_eq!(elims(&rows), vec![25, 29, 33, 29, 30, 30], "stored elims");
    assert!(
        rows[5].suspect_fields.iter().any(|f| f == "e"),
        "the held 35 keeps 30, flagged: {:?}",
        rows[5].suspect_fields
    );
    let gate = st.active_game.as_ref().unwrap().gate.unwrap();
    assert_eq!(gate.accepted.elims, 30);
    assert!(!gate.has_unsure_base[0]);
    assert!(!gate.has_unsure_shown[0]);
}

/// A held lower read, then a sure 40: a lone +11 from the accepted 29 and
/// a +7 from the base 33, past `JUMP_MAX` from both. The gate holds it and
/// the row stays at 29, flagged, as the gate decided. The read neither
/// confirms nor rules out the base, so the base carries on (a misread
/// jump), and a later sure 35, plausible from 33, is still stored.
#[tokio::test]
async fn a_read_implausible_from_the_base_too_stays_held() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let (st, rows) = run_series(vec![(20, board(sure(27))), (30, board(sure(40)))]).await;
    assert_eq!(elims(&rows), vec![25, 29, 33, 29, 29], "stored elims");
    assert!(!elims(&rows).contains(&40), "the held 40 is never stored");
    assert_e_flagged(&rows, 2);
    let gate = st.active_game.as_ref().unwrap().gate.unwrap();
    assert!(gate.has_unsure_base[0], "the base carries on");
    assert_eq!(gate.unsure_base[0], 33);
    drop(_serial);

    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let (_, rows) = run_series(vec![
        (20, board(sure(27))),
        (30, board(sure(40))),
        (60, board(sure(35))),
    ])
    .await;
    assert_eq!(elims(&rows), vec![25, 29, 33, 29, 29, 35], "stored elims");
}

/// Vision case 2: the drop to the accepted value applies only to a held
/// read below the accepted value. Accepted 29, shown 36 (held 33, then a
/// held 36 plausible from it): a sure 34 between them is stored, flagged
/// (lower than the row before), and leaves no base. A sure 27 below the
/// accepted 29 drops the row to 29 and keeps 36 as the base.
#[tokio::test]
async fn the_drop_applies_only_below_the_accepted_value() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let (st, rows) = run_series(vec![(7, board(Some((36, true)))), (20, board(sure(34)))]).await;
    assert_eq!(elims(&rows), vec![25, 29, 33, 36, 34], "stored elims");
    assert!(rows[4].suspect_fields.iter().any(|f| f == "e"));
    let gate = st.active_game.as_ref().unwrap().gate.unwrap();
    assert_eq!(gate.accepted.elims, 29);
    assert!(!gate.has_unsure_base[0]);
    assert_eq!(gate.unsure_shown[0], 34);
    drop(_serial);

    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let (st, rows) = run_series(vec![(7, board(Some((36, true)))), (20, board(sure(27)))]).await;
    assert_eq!(elims(&rows), vec![25, 29, 33, 36, 29], "stored elims");
    assert!(rows[4].suspect_fields.iter().any(|f| f == "e"));
    let gate = st.active_game.as_ref().unwrap().gate.unwrap();
    assert!(gate.has_unsure_base[0]);
    assert_eq!(gate.unsure_base[0], 36);
}

/// Vision case 3: an unvouched held 27 right after the stored unsure 33
/// keeps 33 (a held cell with no fresh read keeps what the row showed; no
/// drop, no base), and a sure 35 after it is stored from 33.
#[tokio::test]
async fn an_unvouched_held_lower_read_keeps_the_stored_unsure_read() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let (st, rows) = run_series(vec![(20, board(Some((27, true))))]).await;
    assert_eq!(elims(&rows), vec![25, 29, 33, 33], "stored elims");
    assert!(rows[3].suspect_fields.iter().any(|f| f == "e"));
    let gate = st.active_game.as_ref().unwrap().gate.unwrap();
    assert!(!gate.has_unsure_base[0]);
    assert_eq!(gate.unsure_shown[0], 33);
    drop(_serial);

    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let (_, rows) = run_series(vec![(20, board(Some((27, true)))), (60, board(sure(35)))]).await;
    assert_eq!(elims(&rows), vec![25, 29, 33, 33, 35], "stored elims");
}

/// The base keeps its own age: the seconds since a stored row last showed
/// it, growing by each carried capture's gap.
#[tokio::test]
async fn the_unsure_base_ages_from_the_row_that_showed_it() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let (st, _) = run_series(vec![(20, board(sure(27)))]).await;
    let gate = st.active_game.as_ref().unwrap().gate.unwrap();
    assert!(gate.has_unsure_base[0]);
    assert_eq!(gate.unsure_base_secs[0], 20, "shown on the capture before");
    drop(_serial);

    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let (st, _) = run_series(vec![(20, board(sure(27))), (30, board(None))]).await;
    let gate = st.active_game.as_ref().unwrap().gate.unwrap();
    assert!(gate.has_unsure_base[0]);
    assert_eq!(gate.unsure_base_secs[0], 50, "20 + 30");
}

/// A gate state with a damage base of 5000, `base_secs` old, on a quiet
/// board whose accepted damage is 1000 and whose accepted value is 600 s
/// old (`held_secs`).
fn damage_base_state(base_secs: u32) -> capture_gate::GateState {
    let accepted = capture_gate::Counters {
        elims: 10,
        assists: 5,
        deaths: 3,
        damage: 1000,
        healing: 800,
        mitigation: 400,
    };
    let mut state = capture_gate::GateState {
        accepted,
        last_raw: accepted,
        ..capture_gate::GateState::default()
    };
    state.held_secs[3] = 600;
    state.unsure_base[3] = 5000;
    state.has_unsure_base[3] = true;
    state.unsure_base_secs[3] = base_secs;
    state
}

/// Run one capture on `prior` with damage held and its unsure read 8900
/// (+3900 from the base), 5 s after `prior`; the stored damage and the
/// gate outcome.
fn damage_capture(prior: capture_gate::GateState, split: bool) -> (u32, capture_gate::GateOutcome) {
    damage_capture_with(prior, split, None, Some(8900))
}

/// [`damage_capture`] with the board carried onto a capture that opens a
/// game (`carried`) and the reader's unsure damage read (`guess`).
fn damage_capture_with(
    prior: capture_gate::GateState,
    split: bool,
    carried: Option<(capture_gate::Counters, Duration)>,
    guess: Option<u32>,
) -> (u32, capture_gate::GateOutcome) {
    let mut parsed = test_match("s", "unknown");
    parsed.elims = 10;
    parsed.assists = 5;
    parsed.deaths = 3;
    parsed.damage = 0;
    parsed.healing = 800;
    parsed.mitigation = 400;
    let clean = [false; capture_gate::GATE_COLS];
    let mut held_cells = clean;
    held_cells[3] = true;
    let mut guesses = [None; capture_gate::GATE_COLS];
    guesses[3] = guess;
    let out = hold_new_reader_stats_with_guesses(
        &mut parsed,
        Some((prior, Duration::from_secs(5))),
        clean,
        held_cells,
        split,
        true,
        carried,
        guesses,
    );
    (parsed.damage, out)
}

/// The filter measures from the base over the base's own age plus the
/// gap, not over the older accepted value's age. A +3900 damage read is
/// past the wide rate cap over 10 + 5 s (80/s + 2500 = 3700) but within
/// it over 20 + 5 s (4500). Measured over the accepted value's 605 s it
/// would pass both times.
#[test]
fn the_base_filter_uses_the_time_since_the_base_was_shown() {
    let (fresh_base, out) = damage_capture(damage_base_state(10), false);
    assert_eq!(fresh_base, 1000, "a +3900 in 15 s is past the rate cap");
    assert!(
        out.state.has_unsure_base[3],
        "a refused jump carries the base"
    );
    assert_eq!(out.state.unsure_base[3], 5000);
    assert_eq!(out.state.unsure_base_secs[3], 15);
    let (older_base, out) = damage_capture(damage_base_state(20), false);
    assert_eq!(older_base, 8900, "a +3900 in 25 s is within it");
    assert!(!out.state.has_unsure_base[3], "a used base ends");
    assert!(out.state.has_unsure_shown[3]);
}

/// The filter counts this capture's own gap on top of the base's age.
/// With a 13 s old base and a 5 s gap, a +3900 damage read is within the
/// wide rate cap over 18 s (80/s + 2500 = 3940) and is stored. Over the
/// base's 13 s alone the cap would be 3540 and the read refused.
#[test]
fn the_base_filter_counts_this_captures_gap() {
    let (stored, out) = damage_capture(damage_base_state(13), false);
    assert_eq!(stored, 8900, "a +3900 in 13 + 5 s is within the rate cap");
    assert!(!out.state.has_unsure_base[3], "a used base ends");
    assert!(out.state.has_unsure_shown[3]);
    assert_eq!(out.state.unsure_shown[3], 8900);
}

/// A capture that opens a new game (a split) ignores the base: the same
/// capture on the same state with and without a base gives the same row
/// and the same gate state, and leaves no base. Without the split the
/// base is used (8900 stored), so the base would have mattered.
#[test]
fn a_new_game_ignores_the_unsure_base() {
    let (unsplit, _) = damage_capture(damage_base_state(20), false);
    assert_eq!(unsplit, 8900);
    let mut no_base = damage_base_state(20);
    no_base.unsure_base = [0; capture_gate::GATE_COLS];
    no_base.has_unsure_base = [false; capture_gate::GATE_COLS];
    no_base.unsure_base_secs = [0; capture_gate::GATE_COLS];
    let (with, with_out) = damage_capture(damage_base_state(20), true);
    let (without, without_out) = damage_capture(no_base, true);
    assert_eq!(with, without, "the split stores the same either way");
    assert_ne!(with, 8900, "the base is not used on a split");
    assert_eq!(with_out.state, without_out.state);
    assert!(!with_out.state.has_unsure_base.iter().any(|&b| b));
}

/// A split that carries the new game's first board onto this capture,
/// with the damage cell held: the gate takes the carried 1000 for the held
/// column, so the column is read and the base paths are reachable. The
/// prior game's base is neither used nor carried into the new game. An
/// unsure read at or above the carried value (8900) is the new game's own
/// fresh read and is stored the same with or without the old base, with
/// the same gate state. A held cell with no read, or an unsure read below
/// the carried value (900), keeps the carried 1000 and starts the new game
/// with no base; without the new-game guard both would carry the old base
/// 5000 on.
#[test]
fn a_new_game_with_a_carried_board_drops_the_unsure_base() {
    let carried = capture_gate::Counters {
        damage: 1000,
        ..damage_base_state(20).accepted
    };
    let carried = Some((carried, Duration::from_secs(3)));
    let mut no_base = damage_base_state(20);
    no_base.unsure_base = [0; capture_gate::GATE_COLS];
    no_base.has_unsure_base = [false; capture_gate::GATE_COLS];
    no_base.unsure_base_secs = [0; capture_gate::GATE_COLS];
    for guess in [Some(8900), None, Some(900)] {
        let (with, with_out) = damage_capture_with(damage_base_state(20), true, carried, guess);
        let (without, without_out) = damage_capture_with(no_base, true, carried, guess);
        assert!(
            !with_out.state.unread[3],
            "{guess:?}: the carried board reads it"
        );
        assert_eq!(with, without, "{guess:?}: the old game's base is not used");
        assert_eq!(with_out.state, without_out.state, "{guess:?}");
        assert!(
            !with_out.state.has_unsure_base.iter().any(|&b| b),
            "{guess:?}: the old game's base is not carried into the new one"
        );
        let want = if guess == Some(8900) { 8900 } else { 1000 };
        assert_eq!(with, want, "{guess:?}");
    }
}

/// An `active_game.json` from before the base fields loads with no base
/// and a 0 age; a state with a base round-trips.
#[test]
fn a_save_without_the_base_fields_loads_as_no_base() {
    let state = damage_base_state(42);
    let mut old = serde_json::to_value(state).unwrap();
    for field in ["unsure_base", "has_unsure_base", "unsure_base_secs"] {
        old.as_object_mut()
            .unwrap()
            .remove(field)
            .expect("the field is saved");
    }
    let loaded: capture_gate::GateState = serde_json::from_value(old).unwrap();
    assert!(!loaded.has_unsure_base.iter().any(|&b| b));
    assert_eq!(loaded.unsure_base, [0; capture_gate::GATE_COLS]);
    assert_eq!(loaded.unsure_base_secs, [0; capture_gate::GATE_COLS]);
    assert_eq!(loaded.accepted, state.accepted);
    let back: capture_gate::GateState =
        serde_json::from_value(serde_json::to_value(state).unwrap()).unwrap();
    assert_eq!(back, state);
}

/// Pinned as-is, not a decision: after the stored unsure 33 (accepted 29),
/// a Tab whose own row is suspect reads a sure 30, between the accepted and
/// the shown value. A trusted 30 there is stored (see
/// `the_drop_applies_only_below_the_accepted_value`). A suspect row gives
/// no refused digit, so nothing of it is stored, the gate holds the 30
/// below the shown 33, and the row drops to the accepted 29, flagged, with
/// 33 kept as the base: the reverse of Vision case 2 for a suspect row.
/// Whether a suspect row should keep the shown value instead, like an
/// unvouched read, belongs in the follow-up PR about vouched and suspect
/// lower reads; change this test there, not here.
#[tokio::test]
async fn a_suspect_row_between_accepted_and_shown_drops_to_accepted() {
    use detect::own_row::combine;
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let sure_row = Some(combine(Some(0), o9_lit(0, 1.5), true));
    let suspect_row = Some(combine(Some(0), None, true));
    let mut tabs: Vec<_> = opening()
        .into_iter()
        .map(|(gap, cells)| (gap, cells, sure_row))
        .collect();
    tabs.push((20, board(sure(30)), suspect_row));
    let (st, seen) = o10_run_rows(&ctx, tabs).await;
    assert!(seen.iter().all(|tab| tab.recorded), "{seen:?}");
    let rows = ctx.store.get_session_snapshots("o10").await.unwrap();
    assert_eq!(elims(&rows), vec![25, 29, 33, 29], "stored elims");
    assert!(!elims(&rows).contains(&30), "the suspect 30 is not stored");
    assert_e_flagged(&rows, 2);
    let gate = st.active_game.as_ref().unwrap().gate.unwrap();
    assert_eq!(gate.accepted.elims, 29);
    assert!(!gate.has_unsure_shown[0]);
    assert!(gate.has_unsure_base[0], "the shown 33 is kept as the base");
    assert_eq!(gate.unsure_base[0], 33);
}
