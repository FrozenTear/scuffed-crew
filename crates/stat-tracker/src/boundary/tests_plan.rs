//! Tests of the Tab capture plan (`plan_capture`, `plan_capture_held`): held,
//! suspect and unread cells, fresh resets, growth and the stat gap.

use super::tests::{CLEAN, counters, gate, t0};
use super::*;
use crate::capture_gate::{HoldKind, apply_gate};

#[allow(clippy::too_many_arguments)]
fn plan_at(
    prev: &GateState,
    cur: Counters,
    after_end: bool,
    streak: u32,
    classic: bool,
    age_secs: u64,
    suppress: bool,
    row_counts: bool,
    inherited: MatchOutcome,
) -> CapturePlan {
    plan_capture(&CapturePlanInput {
        prev_gate: Some(prev),
        baseline: None,
        streak,
        cur,
        suspect: CLEAN,
        create_session: false,
        suppress_same_unfinished: suppress,
        age: Some(Duration::from_secs(age_secs)),
        min_gap: Duration::from_secs(120),
        classic_regressed: classic,
        row_counts,
        row_id: row_counts.then_some(0),
        baseline_row: Some(0),
        confirmed_end: after_end && inherited.is_decided(),
        inherited_outcome: inherited,
        frame_outcome: MatchOutcome::Unknown,
        session_hero: None,
        hint: (after_end && !inherited.is_decided()).then_some(MatchOutcome::Defeat),
        pending_boundary: false,
        awaiting_first_board: false,
        baseline_age: None,
        progressed_boards: 0,
        session_map: None,
        incoming_map: None,
        session_map_source: None,
    })
}

// ---- alpha.7 O10: cells the new reader held, and never-read columns.
/// One board against `prev`, the same identified row, 60 s later.
fn held_plan(prev: &GateState, cur: Counters, held: [bool; GATE_COLS]) -> CapturePlan {
    held_plan_at(prev, cur, held, 60)
}

fn held_input<'a>(
    prev: &'a GateState,
    cur: Counters,
    suspect: [bool; GATE_COLS],
    age_secs: u64,
) -> CapturePlanInput<'a> {
    CapturePlanInput {
        prev_gate: Some(prev),
        baseline: Some(prev),
        streak: 0,
        cur,
        suspect,
        create_session: false,
        suppress_same_unfinished: false,
        age: Some(Duration::from_secs(age_secs)),
        min_gap: Duration::from_secs(120),
        classic_regressed: false,
        row_counts: true,
        row_id: Some(0),
        baseline_row: Some(0),
        confirmed_end: false,
        inherited_outcome: MatchOutcome::Unknown,
        frame_outcome: MatchOutcome::Unknown,
        session_hero: None,
        hint: None,
        pending_boundary: false,
        awaiting_first_board: false,
        baseline_age: Some(Duration::from_secs(age_secs)),
        progressed_boards: 0,
        session_map: None,
        incoming_map: None,
        session_map_source: None,
    }
}

fn held_plan_at(
    prev: &GateState,
    cur: Counters,
    held: [bool; GATE_COLS],
    age_secs: u64,
) -> CapturePlan {
    // main passes the held cells as the suspect mask
    plan_capture_held(
        &held_input(prev, cur, held, age_secs),
        BoardDoubt {
            held,
            ..BoardDoubt::default()
        },
    )
}

/// A board on a suspect own row against `prev`, with `streak` earlier
/// reset signals and maybe a trusted fresh board already held.
fn suspect_plan(
    prev: &GateState,
    cur: Counters,
    row: u32,
    streak: u32,
    board_deferred: bool,
) -> CapturePlan {
    let mut input = held_input(prev, cur, CLEAN, 60);
    input.row_id = Some(row);
    input.streak = streak;
    plan_capture_held(
        &input,
        BoardDoubt {
            suspect_row: true,
            board_deferred,
            ..BoardDoubt::default()
        },
    )
}

/// R3-1: a suspect own row anchors the reset baseline, and can say
/// "reset" but never commit the split alone.
#[test]
fn a_suspect_row_anchors_the_baseline_and_never_splits_alone() {
    let prev = gate(counters(30, 12, 9, 14000, 5000, 900));
    // the game goes on, on the baseline's row: the baseline moves along
    let on = suspect_plan(&prev, counters(31, 12, 9, 14400, 5100, 900), 0, 0, false);
    assert!(on.refresh_baseline && !on.skip_store, "{on:?}");
    // a game with no baseline yet gets one from a suspect row
    let mut first = held_input(&prev, counters(31, 12, 9, 14400, 5100, 900), CLEAN, 60);
    first.baseline = None;
    first.baseline_row = None;
    let doubt = BoardDoubt {
        suspect_row: true,
        ..BoardDoubt::default()
    };
    let plan = plan_capture_held(&first, doubt);
    assert!(plan.refresh_baseline, "{plan:?}");
    assert_eq!(plan.baseline_row, Some(0));
    // another row's numbers do not take the baseline over (a trusted
    // row that continues does)
    let other = suspect_plan(&prev, counters(34, 14, 10, 15000, 0, 2000), 1, 0, false);
    assert!(!other.refresh_baseline && !other.skip_store, "{other:?}");
    assert_eq!(other.baseline_row, Some(0));
    let mut trusted = held_input(&prev, counters(34, 14, 10, 15000, 0, 2000), CLEAN, 60);
    trusted.row_id = Some(1);
    assert!(plan_capture(&trusted).refresh_baseline);

    // a fresh-match board on the baseline's row: a reset signal, held
    // off the game, not carried
    let fresh = counters(1, 0, 0, 300, 100, 0);
    let one = suspect_plan(&prev, fresh, 0, 0, false);
    assert!(one.defer && one.skip_store && !one.split, "{one:?}");
    assert_eq!((one.reset_streak, one.deferred_counters), (1, None));
    // one earlier signal (a second suspect board, or a hero select that
    // primed the streak) is still not enough for a suspect row...
    let two = suspect_plan(&prev, fresh, 0, 1, false);
    assert!(two.defer && !two.split, "{two:?}");
    assert_eq!(two.reset_streak, 2);
    // ...where a trusted row splits
    let mut sure = held_input(&prev, fresh, CLEAN, 60);
    sure.streak = 1;
    assert!(plan_capture(&sure).split);
    // two earlier signals, or a trusted fresh board already held: split
    assert!(suspect_plan(&prev, fresh, 0, 2, false).split);
    assert!(suspect_plan(&prev, fresh, 0, 1, true).split);
    assert!(suspect_plan(&prev, fresh, 0, 0, true).split);
    // on another row a fresh-shaped suspect board is no signal at all
    let stray = suspect_plan(&prev, fresh, 1, 2, true);
    assert!(
        !stray.split && !stray.defer && !stray.refresh_baseline,
        "{stray:?}"
    );
}

/// A suspect row is not progress: two of them do not drop a result hint.
#[test]
fn a_suspect_row_does_not_count_as_progress() {
    let prev = gate(counters(30, 12, 9, 14000, 5000, 900));
    let cur = counters(31, 12, 9, 14400, 5100, 900);
    let mut input = held_input(&prev, cur, CLEAN, 60);
    input.hint = Some(MatchOutcome::Defeat);
    input.progressed_boards = 1;
    let sure = plan_capture(&input);
    assert!(sure.clear_hint, "{sure:?}");
    let doubt = BoardDoubt {
        suspect_row: true,
        ..BoardDoubt::default()
    };
    let suspect = plan_capture_held(&input, doubt);
    assert!(
        !suspect.clear_hint && !suspect.count_progress,
        "{suspect:?}"
    );
    assert!(suspect.refresh_baseline, "it still anchors");
}

/// F266-2: a held cell abstains from the reset, it does not veto it.
/// Before, any held cell made the board "not a reset row", so a missed
/// game end could not be recovered while the reader held anything.
#[test]
fn a_held_cell_abstains_from_the_fresh_reset() {
    let prev = gate(counters(30, 12, 9, 14000, 5000, 900));
    // healing held: main fills the last accepted 5000 in
    let held_h = [false, false, false, false, true, false];
    let plan = held_plan(&prev, counters(1, 0, 0, 300, 5000, 0), held_h);
    assert!(plan.defer && plan.skip_store, "{plan:?}");
    assert_eq!(plan.reset_streak, 1);
    assert_eq!(
        plan.deferred_counters, None,
        "a board with a held cell is not carried as a row"
    );
    // elims held (the old game's 30 sits in it): deaths and damage fell
    let held_e = [true, false, false, false, false, false];
    let plan = held_plan(&prev, counters(30, 0, 0, 300, 100, 0), held_e);
    assert!(plan.defer, "two of the three reset columns are enough");
    // one reset column left is one cell, and one cell is a misread
    let held_ed = [true, false, true, false, false, false];
    let plan = held_plan(&prev, counters(30, 0, 9, 300, 100, 0), held_ed);
    assert!(!plan.defer && !plan.split && !plan.skip_store, "{plan:?}");
    // a sure column that did not fall still says "same game"
    let plan = held_plan(&prev, counters(1, 0, 0, 13000, 5000, 0), held_h);
    assert!(!plan.defer, "damage is still this game's");
    // the same complete board is a reset, and is carried
    let plan = held_plan(&prev, counters(1, 0, 0, 300, 100, 0), CLEAN);
    assert!(plan.defer);
    assert_eq!(plan.deferred_counters, Some(counters(1, 0, 0, 300, 100, 0)));
    // maturity has to show on a column both boards have: with elims
    // held, damage 300 on the earlier board is not a mature board
    let early = gate(counters(6, 2, 1, 300, 100, 0));
    let plan = held_plan(&early, counters(6, 0, 0, 20, 100, 0), held_e);
    assert!(!plan.defer, "{plan:?}");
    // ocr-v1's edge-ink suspect column still vetoes: that is a number
    // off the board which may be wrong, not a missing one
    let ink = plan_capture(&held_input(
        &prev,
        counters(1, 0, 0, 300, 100, 0),
        held_e,
        60,
    ));
    assert!(!ink.defer && !ink.split, "{ink:?}");
}

/// A board with a held cell re-anchors the reset baseline like any
/// other identified row. Without a baseline row no reset can fire.
#[test]
fn a_board_with_a_held_cell_refreshes_the_reset_baseline() {
    let prev = gate(counters(30, 12, 9, 14000, 5000, 900));
    let held_h = [false, false, false, false, true, false];
    let plan = held_plan(&prev, counters(31, 12, 9, 14400, 5000, 900), held_h);
    assert!(plan.refresh_baseline, "{plan:?}");
    assert_eq!(plan.baseline_row, Some(0));
    assert!(!plan.skip_store && !plan.split);
}

/// F266-5: the placeholder 0 of a never-read column is not a number.
#[test]
fn a_never_read_column_is_not_a_number_to_the_plan() {
    // damage never read: with the placeholder as a real 0 no board can
    // be under a quarter of it, and the reset never fires
    let mut prev = gate(counters(20, 5, 6, 0, 3000, 500));
    prev.unread[3] = true;
    let plan = held_plan(&prev, counters(1, 0, 0, 300, 100, 0), CLEAN);
    assert!(plan.defer, "{plan:?}");
    // elims never read, then read as 25 after 20 s: not growth from 0,
    // so the row still re-anchors the baseline
    let mut prev = gate(counters(0, 5, 6, 9000, 3000, 500));
    prev.unread[0] = true;
    let cur = counters(25, 5, 6, 9100, 3000, 500);
    assert!(held_plan_at(&prev, cur, CLEAN, 20).refresh_baseline);
    // the same numbers on a column that WAS read are an implausible jump
    prev.unread[0] = false;
    assert!(!held_plan_at(&prev, cur, CLEAN, 20).refresh_baseline);
    // a first read alone is not "the board progressed" either
    let mut prev = gate(counters(20, 5, 6, 9000, 0, 500));
    prev.unread[4] = true;
    let known = known_cols(&prev, &CLEAN);
    assert!(!counters_progressed(
        &prev.accepted,
        &counters(20, 5, 6, 9000, 700, 500),
        &known
    ));
    assert!(counters_progressed(
        &prev.accepted,
        &counters(21, 5, 6, 9000, 700, 500),
        &known
    ));
}

/// Growth is measured from the column's last accepted read: a column
/// back from a held run has an accepted value older than the baseline.
#[test]
fn growth_after_a_held_run_is_measured_from_the_last_accepted_read() {
    let mut prev = gate(counters(10, 5, 6, 9000, 3000, 500));
    // +30 elims in 20 s is past 20/5 + 8
    let cur = counters(40, 5, 6, 9100, 3000, 500);
    assert!(!held_plan_at(&prev, cur, CLEAN, 20).refresh_baseline);
    // ...and inside 320/5 + 8 when the 10 is 320 s old
    prev.held_secs[0] = 300;
    assert!(held_plan_at(&prev, cur, CLEAN, 20).refresh_baseline);
}

#[test]
fn single_cell_ocr_noise_is_held_and_does_not_split() {
    let prev_c = counters(22, 7, 5, 6400, 11000, 800);
    let prev = gate(prev_c);
    let mut cur = prev_c;
    cur.elims = 2;
    assert!(
        !post_result_stat_reset(&prev, cur, CLEAN),
        "one sharp column is an OCR miss, not a new game"
    );
    let held = plan_at(
        &prev,
        cur,
        true,
        0,
        false,
        5,
        true,
        true,
        MatchOutcome::Defeat,
    );
    assert!(
        !held.split,
        "after an end screen a single-column drop still does not split"
    );
    let early = plan_at(
        &prev,
        counters(1, 1, 0, 100, 100, 100),
        false,
        0,
        true,
        5,
        false,
        true,
        MatchOutcome::Unknown,
    );
    assert!(
        !early.split,
        "mid-match, even a classic regression waits out the gap"
    );
    let gated = apply_gate(Some((prev, Duration::from_secs(30))), cur, CLEAN, false);
    assert!(
        gated
            .holds
            .iter()
            .any(|h| h.col == 0 && h.kind == HoldKind::Monotonic && h.raw == 2),
        "the gate still holds the noisy cell: {holds:?}",
        holds = gated.holds
    );
    assert_eq!(gated.accepted.elims, 22);
}

/// CAP-8, at the plan: [`RESET_MIN_AGE`] is measured from the baseline,
/// not from the last stored row, and a fresh-shaped baseline-row board
/// never refreshes the baseline or drops the streak.
#[test]
fn the_reset_age_is_the_baselines_and_a_fresh_board_is_not_a_baseline() {
    let mature = gate(counters(20, 8, 8, 8000, 4000, 2000));
    let low = counters(2, 1, 1, 900, 300, 100);
    // Last stored row 10 s ago (an unidentified row), baseline 60 s.
    let mut input = held_input(&mature, low, CLEAN, 10);
    input.baseline_age = Some(Duration::from_secs(60));
    input.streak = 1;
    let plan = plan_capture(&input);
    assert!(plan.split, "{plan:?}");
    input.streak = 0;
    let plan = plan_capture(&input);
    assert!(plan.defer && !plan.refresh_baseline, "{plan:?}");
    // Baseline itself only 10 s old: not a reset yet, and not a baseline.
    let mut young = held_input(&mature, low, CLEAN, 10);
    young.streak = 1;
    let plan = plan_capture(&young);
    assert!(!plan.split && !plan.defer, "{plan:?}");
    assert!(!plan.refresh_baseline, "{plan:?}");
    assert_eq!(plan.reset_streak, 1, "the streak stays: {plan:?}");
    assert_eq!(plan.reset_baseline, Some(mature));
    // No baseline yet: the gate's own age is the reset age.
    let mut no_baseline = held_input(&mature, low, CLEAN, 60);
    no_baseline.baseline = None;
    no_baseline.baseline_age = Some(Duration::from_secs(5));
    assert!(plan_capture(&no_baseline).defer);
    // A continuing board on the same row still refreshes.
    let cont = held_input(&mature, counters(21, 8, 8, 8100, 4000, 2000), CLEAN, 10);
    assert!(plan_capture(&cont).refresh_baseline);
    // A fresh-shaped board against an immature baseline still refreshes.
    let early = gate(counters(1, 0, 0, 100, 0, 0));
    let start = held_input(&early, counters(1, 0, 0, 20, 0, 0), CLEAN, 10);
    assert!(plan_capture(&start).refresh_baseline);
}

#[test]
fn post_result_reset_is_several_columns_and_ignores_the_mid_match_gap() {
    let prev = gate(counters(18, 7, 9, 6400, 11000, 800));
    let cur = counters(1, 3, 0, 220, 80, 400);
    assert!(post_result_stat_reset(&prev, cur, CLEAN));
    let first = plan_at(
        &prev,
        cur,
        true,
        0,
        true,
        200,
        true,
        true,
        MatchOutcome::Defeat,
    );
    assert!(
        !first.split,
        "one sharp drop does not split, even past the mid-match gap"
    );
    assert!(
        first.defer,
        "the first drop is not written onto the finished game"
    );
    assert_eq!(first.reset_streak, 1);
    assert_eq!(first.stored_outcome, MatchOutcome::Defeat);
    let second = plan_capture(&CapturePlanInput {
        prev_gate: Some(&prev),
        baseline: first.reset_baseline.as_ref(),
        streak: first.reset_streak,
        cur: counters(3, 4, 1, 500, 200, 600),
        suspect: CLEAN,
        create_session: false,
        suppress_same_unfinished: true,
        age: Some(Duration::from_secs(50)),
        min_gap: Duration::from_secs(120),
        classic_regressed: true,
        row_counts: true,
        row_id: Some(0),
        baseline_row: Some(0),
        confirmed_end: true,
        inherited_outcome: MatchOutcome::Defeat,
        frame_outcome: MatchOutcome::Victory,
        session_hero: None,
        hint: None,
        pending_boundary: false,
        awaiting_first_board: false,
        baseline_age: None,
        progressed_boards: 0,
        session_map: None,
        incoming_map: None,
        session_map_source: None,
    });
    assert!(second.split, "the second consecutive drop splits");
    assert_eq!(
        second.stored_outcome,
        MatchOutcome::Unknown,
        "a split does not keep the header result from the board being closed"
    );
    assert!(
        second.seal.is_none(),
        "a confirmed result is already stored"
    );
    // Healing + damage + elims, deaths not sharp (9 → 6).
    let cur = counters(4, 8, 6, 900, 400, 900);
    assert!(post_result_stat_reset(&prev, cur, CLEAN));
}

#[test]
fn garbage_and_unidentified_rows_do_not_count_toward_a_reset() {
    let prev = gate(counters(18, 7, 9, 6400, 11000, 800));
    let garbage = counters(9, 11, 11, 61029, 100, 100);
    let ignored = plan_at(
        &prev,
        garbage,
        true,
        1,
        true,
        86,
        false,
        false,
        MatchOutcome::Defeat,
    );
    assert!(!ignored.split);
    assert!(!ignored.defer);
    assert_eq!(
        ignored.reset_streak, 1,
        "an unidentified row leaves the streak and a held board alone"
    );
    let one = plan_at(
        &prev,
        counters(1, 3, 0, 220, 80, 400),
        true,
        0,
        true,
        86,
        true,
        true,
        MatchOutcome::Defeat,
    );
    assert!(!one.split);
    assert!(one.defer);
    // 2026-07-14 field case. The garbage row reads as an increase on
    // damage and a drop on elims. It must not become the baseline, so
    // the next real board is not "one capture later" from a split.
    let mixed = plan_capture(&CapturePlanInput {
        prev_gate: Some(&prev),
        baseline: Some(&prev),
        streak: 0,
        cur: garbage,
        suspect: CLEAN,
        create_session: false,
        suppress_same_unfinished: false,
        age: Some(Duration::from_secs(86)),
        min_gap: Duration::from_secs(120),
        classic_regressed: true,
        row_counts: true,
        row_id: Some(0),
        baseline_row: Some(0),
        confirmed_end: true,
        inherited_outcome: MatchOutcome::Defeat,
        frame_outcome: MatchOutcome::Defeat,
        session_hero: None,
        hint: None,
        pending_boundary: false,
        awaiting_first_board: false,
        baseline_age: None,
        progressed_boards: 0,
        session_map: None,
        incoming_map: None,
        session_map_source: None,
    });
    assert!(
        !mixed.ignore_row && !mixed.skip_store,
        "an implausible row is stored"
    );
    assert!(!mixed.refresh_baseline, "it does not become the baseline");
    assert!(!mixed.split);
    assert_eq!(mixed.reset_streak, 0);
    let follow = plan_capture(&CapturePlanInput {
        prev_gate: Some(&prev),
        baseline: Some(&prev),
        streak: 0,
        cur: counters(20, 8, 10, 7000, 12000, 900),
        suspect: CLEAN,
        create_session: false,
        suppress_same_unfinished: false,
        age: Some(Duration::from_secs(90)),
        min_gap: Duration::from_secs(120),
        classic_regressed: false,
        row_counts: true,
        row_id: Some(0),
        baseline_row: Some(0),
        confirmed_end: true,
        inherited_outcome: MatchOutcome::Defeat,
        frame_outcome: MatchOutcome::Unknown,
        session_hero: None,
        hint: None,
        pending_boundary: false,
        awaiting_first_board: false,
        baseline_age: None,
        progressed_boards: 0,
        session_map: None,
        incoming_map: None,
        session_map_source: None,
    });
    assert!(!follow.split);
    assert_eq!(follow.reset_streak, 0);
    // A low-total teammate row is a different slot. It is stored, it does
    // not count as a fresh reset, and it does not replace the baseline.
    let moved = plan_capture(&CapturePlanInput {
        prev_gate: Some(&prev),
        baseline: Some(&prev),
        streak: 0,
        cur: counters(1, 3, 0, 220, 80, 400),
        suspect: CLEAN,
        create_session: false,
        suppress_same_unfinished: true,
        age: Some(Duration::from_secs(50)),
        min_gap: Duration::from_secs(120),
        classic_regressed: true,
        row_counts: true,
        row_id: Some(4),
        baseline_row: Some(0),
        confirmed_end: true,
        inherited_outcome: MatchOutcome::Defeat,
        frame_outcome: MatchOutcome::Victory,
        session_hero: None,
        hint: None,
        pending_boundary: false,
        awaiting_first_board: false,
        baseline_age: None,
        progressed_boards: 0,
        session_map: None,
        incoming_map: None,
        session_map_source: None,
    });
    assert!(
        !moved.split && !moved.defer,
        "a different row is not a fresh reset"
    );
    assert!(!moved.refresh_baseline);
    assert_eq!(moved.baseline_row, Some(0));
    assert!(!moved.ignore_row);
    assert_eq!(moved.reset_streak, 0);
    let moved_again = plan_capture(&CapturePlanInput {
        prev_gate: Some(&prev),
        baseline: Some(&prev),
        streak: 1,
        cur: counters(0, 1, 0, 80, 20, 100),
        suspect: CLEAN,
        create_session: false,
        suppress_same_unfinished: true,
        age: Some(Duration::from_secs(70)),
        min_gap: Duration::from_secs(120),
        classic_regressed: true,
        row_counts: true,
        row_id: Some(4),
        baseline_row: Some(0),
        confirmed_end: true,
        inherited_outcome: MatchOutcome::Defeat,
        frame_outcome: MatchOutcome::Victory,
        session_hero: None,
        hint: None,
        pending_boundary: false,
        awaiting_first_board: false,
        baseline_age: None,
        progressed_boards: 0,
        session_map: None,
        incoming_map: None,
        session_map_source: None,
    });
    assert!(
        !moved_again.split && !moved_again.defer,
        "a second read of a different row is still not a reset"
    );
    assert_eq!(moved_again.baseline_row, Some(0));
    assert_eq!(moved_again.stored_outcome, MatchOutcome::Defeat);
    // The garbage row must not become the baseline, so the next real
    // drop is the first of the pair, not a split one capture later.
    let mut state = BoundaryState::new(Some("Busan".into()));
    state.outcome = MatchOutcome::Defeat;
    state.gate = Some(prev);
    state.reset_baseline = Some(prev);
    state.baseline_row = Some(0);
    note_accepted_capture(&mut state, &mixed, prev, t0());
    assert_eq!(state.reset_baseline, Some(prev));
    assert_eq!(state.gate, Some(prev));
    assert_eq!(state.reset_streak, 0);
    let later = plan_capture(&CapturePlanInput {
        prev_gate: state.gate.as_ref(),
        baseline: state.reset_baseline.as_ref(),
        streak: state.reset_streak,
        cur: counters(1, 3, 0, 220, 80, 400),
        suspect: CLEAN,
        create_session: false,
        suppress_same_unfinished: true,
        age: Some(Duration::from_secs(50)),
        min_gap: Duration::from_secs(120),
        classic_regressed: true,
        row_counts: true,
        row_id: Some(0),
        baseline_row: state.baseline_row,
        confirmed_end: true,
        inherited_outcome: MatchOutcome::Defeat,
        frame_outcome: MatchOutcome::Victory,
        session_hero: None,
        hint: None,
        pending_boundary: false,
        awaiting_first_board: false,
        baseline_age: None,
        progressed_boards: 0,
        session_map: None,
        incoming_map: None,
        session_map_source: None,
    });
    assert!(later.defer, "the real drop after garbage only arms");
    assert!(!later.split, "one capture later is not a new session");
    assert_eq!(later.stored_outcome, MatchOutcome::Defeat);
}

#[test]
fn hint_only_session_still_splits_on_the_stat_gap() {
    let prev = gate(counters(29, 8, 5, 9242, 1000, 200));
    let plan = plan_capture(&CapturePlanInput {
        prev_gate: Some(&prev),
        baseline: None,
        streak: 0,
        cur: counters(3, 1, 0, 200, 50, 10),
        suspect: CLEAN,
        create_session: false,
        suppress_same_unfinished: false,
        age: Some(Duration::from_secs(120) + Duration::from_secs(5)),
        min_gap: Duration::from_secs(120),
        classic_regressed: true,
        row_counts: true,
        row_id: Some(0),
        baseline_row: Some(0),
        confirmed_end: false,
        inherited_outcome: MatchOutcome::Unknown,
        frame_outcome: MatchOutcome::Defeat,
        session_hero: None,
        hint: Some(MatchOutcome::Defeat),
        pending_boundary: false,
        awaiting_first_board: false,
        baseline_age: None,
        progressed_boards: 0,
        session_map: None,
        incoming_map: None,
        session_map_source: None,
    });
    assert!(
        plan.split,
        "a hint-only session keeps the mid-match gap split"
    );
    assert_eq!(
        plan.stored_outcome,
        MatchOutcome::Defeat,
        "a gap split keeps this frame's header, not an inherited outcome"
    );
    assert_eq!(plan.seal, Some(MatchOutcome::Defeat));
}

#[test]
fn all_increase_garbage_does_not_become_the_baseline() {
    let prev = gate(counters(18, 7, 9, 6400, 11000, 800));
    let garbage = counters(90, 40, 40, 30000, 50000, 5000);
    let ignored = plan_capture(&CapturePlanInput {
        prev_gate: Some(&prev),
        baseline: Some(&prev),
        streak: 0,
        cur: garbage,
        suspect: CLEAN,
        create_session: false,
        suppress_same_unfinished: false,
        age: Some(Duration::from_secs(40)),
        min_gap: Duration::from_secs(120),
        classic_regressed: false,
        row_counts: true,
        row_id: Some(0),
        baseline_row: Some(0),
        confirmed_end: true,
        inherited_outcome: MatchOutcome::Defeat,
        frame_outcome: MatchOutcome::Defeat,
        session_hero: None,
        hint: None,
        pending_boundary: false,
        awaiting_first_board: false,
        baseline_age: None,
        progressed_boards: 0,
        session_map: None,
        incoming_map: None,
        session_map_source: None,
    });
    assert!(!ignored.ignore_row);
    assert!(!ignored.skip_store);
    assert!(!ignored.refresh_baseline);
    assert!(!ignored.split);
    assert_eq!(ignored.reset_streak, 0);
    let follow = plan_capture(&CapturePlanInput {
        prev_gate: Some(&prev),
        baseline: Some(&prev),
        streak: 0,
        cur: counters(20, 8, 10, 7000, 12000, 900),
        suspect: CLEAN,
        create_session: false,
        suppress_same_unfinished: true,
        age: Some(Duration::from_secs(50)),
        min_gap: Duration::from_secs(120),
        classic_regressed: false,
        row_counts: true,
        row_id: Some(0),
        baseline_row: Some(0),
        confirmed_end: true,
        inherited_outcome: MatchOutcome::Defeat,
        frame_outcome: MatchOutcome::Unknown,
        session_hero: None,
        hint: None,
        pending_boundary: false,
        awaiting_first_board: false,
        baseline_age: None,
        progressed_boards: 0,
        session_map: None,
        incoming_map: None,
        session_map_source: None,
    });
    assert!(
        !follow.split,
        "a continuation above the real baseline does not split"
    );
    assert!(!follow.ignore_row);
}

#[test]
fn two_to_four_x_all_increase_is_stored_and_not_the_baseline() {
    let prev = gate(counters(10, 8, 6, 2000, 3000, 500));
    let jumped = counters(24, 20, 14, 5000, 7500, 1200);
    let plan = plan_capture(&CapturePlanInput {
        prev_gate: Some(&prev),
        baseline: Some(&prev),
        streak: 0,
        cur: jumped,
        suspect: CLEAN,
        create_session: false,
        suppress_same_unfinished: true,
        age: Some(Duration::from_secs(30)),
        min_gap: Duration::from_secs(120),
        classic_regressed: false,
        row_counts: true,
        row_id: Some(0),
        baseline_row: Some(0),
        confirmed_end: false,
        inherited_outcome: MatchOutcome::Unknown,
        frame_outcome: MatchOutcome::Unknown,
        session_hero: None,
        hint: None,
        pending_boundary: false,
        awaiting_first_board: false,
        baseline_age: Some(Duration::from_secs(30)),
        progressed_boards: 0,
        session_map: None,
        incoming_map: None,
        session_map_source: None,
    });
    assert!(!plan.ignore_row && !plan.skip_store && !plan.split);
    assert!(
        !plan.refresh_baseline,
        "a 2-4x all-increase inside a minute must not replace the baseline"
    );
    assert_eq!(plan.reset_streak, 0);
    let uneven = plan_capture(&CapturePlanInput {
        prev_gate: Some(&prev),
        baseline: Some(&prev),
        streak: 0,
        cur: counters(12, 9, 7, 6000, 3400, 560),
        suspect: CLEAN,
        create_session: false,
        suppress_same_unfinished: true,
        age: Some(Duration::from_secs(90)),
        min_gap: Duration::from_secs(120),
        classic_regressed: false,
        row_counts: true,
        row_id: Some(0),
        baseline_row: Some(0),
        confirmed_end: false,
        inherited_outcome: MatchOutcome::Unknown,
        frame_outcome: MatchOutcome::Unknown,
        session_hero: None,
        hint: None,
        pending_boundary: false,
        awaiting_first_board: false,
        baseline_age: None,
        progressed_boards: 0,
        session_map: None,
        incoming_map: None,
        session_map_source: None,
    });
    assert!(
        uneven.refresh_baseline,
        "one wide-column burst still refreshes the baseline"
    );
}

#[test]
fn gap_split_ignores_a_changed_row_and_keeps_the_frame_header() {
    let prev = gate(counters(29, 8, 5, 9242, 1000, 200));
    let plan = plan_capture(&CapturePlanInput {
        prev_gate: Some(&prev),
        baseline: Some(&prev),
        streak: 0,
        cur: counters(3, 1, 0, 200, 50, 10),
        suspect: CLEAN,
        create_session: false,
        suppress_same_unfinished: false,
        age: Some(Duration::from_secs(125)),
        min_gap: Duration::from_secs(120),
        classic_regressed: true,
        row_counts: true,
        row_id: Some(1),
        baseline_row: Some(0),
        confirmed_end: false,
        inherited_outcome: MatchOutcome::Unknown,
        frame_outcome: MatchOutcome::Defeat,
        session_hero: Some("Zenyatta"),
        hint: None,
        pending_boundary: false,
        awaiting_first_board: false,
        baseline_age: None,
        progressed_boards: 0,
        session_map: None,
        incoming_map: None,
        session_map_source: None,
    });
    assert!(plan.split);
    assert_eq!(plan.stored_outcome, MatchOutcome::Defeat);
}

#[test]
fn confirmed_post_match_screen_does_not_split_on_the_stat_gap() {
    let prev = gate(counters(18, 7, 9, 6400, 11000, 800));
    let plan = plan_capture(&CapturePlanInput {
        prev_gate: Some(&prev),
        baseline: Some(&prev),
        streak: 0,
        cur: counters(20, 8, 10, 7000, 12000, 900),
        suspect: CLEAN,
        create_session: false,
        suppress_same_unfinished: false,
        age: Some(Duration::from_secs(200)),
        min_gap: Duration::from_secs(120),
        classic_regressed: true,
        row_counts: true,
        row_id: Some(0),
        baseline_row: Some(0),
        confirmed_end: true,
        inherited_outcome: MatchOutcome::Defeat,
        frame_outcome: MatchOutcome::Victory,
        session_hero: None,
        hint: None,
        pending_boundary: false,
        awaiting_first_board: false,
        baseline_age: None,
        progressed_boards: 0,
        session_map: None,
        incoming_map: None,
        session_map_source: None,
    });
    assert!(!plan.split);
    assert_eq!(plan.stored_outcome, MatchOutcome::Defeat);
}

#[test]
fn clean_new_row_reanchors_the_baseline() {
    let prev = gate(counters(18, 7, 9, 6400, 11000, 800));
    let plan = plan_capture(&CapturePlanInput {
        prev_gate: Some(&prev),
        baseline: Some(&prev),
        streak: 0,
        cur: counters(19, 8, 9, 6600, 11200, 820),
        suspect: CLEAN,
        create_session: false,
        suppress_same_unfinished: true,
        age: Some(Duration::from_secs(15)),
        min_gap: Duration::from_secs(120),
        classic_regressed: false,
        row_counts: true,
        row_id: Some(3),
        baseline_row: Some(0),
        confirmed_end: true,
        inherited_outcome: MatchOutcome::Defeat,
        frame_outcome: MatchOutcome::Defeat,
        session_hero: Some("Zenyatta"),
        hint: None,
        pending_boundary: false,
        awaiting_first_board: false,
        baseline_age: None,
        progressed_boards: 0,
        session_map: None,
        incoming_map: None,
        session_map_source: None,
    });
    assert!(!plan.split);
    assert!(!plan.ignore_row);
    assert_eq!(plan.baseline_row, Some(3));
    assert!(plan.refresh_baseline);
}

#[test]
fn normal_growth_is_folded_and_refreshes_the_baseline() {
    let prev = gate(counters(2, 1, 0, 300, 40, 100));
    let plan = plan_capture(&CapturePlanInput {
        prev_gate: Some(&prev),
        baseline: Some(&prev),
        streak: 0,
        cur: counters(6, 2, 1, 1400, 180, 400),
        suspect: CLEAN,
        create_session: false,
        suppress_same_unfinished: true,
        age: Some(Duration::from_secs(135)),
        min_gap: Duration::from_secs(120),
        classic_regressed: false,
        row_counts: true,
        row_id: Some(0),
        baseline_row: Some(0),
        confirmed_end: false,
        inherited_outcome: MatchOutcome::Unknown,
        frame_outcome: MatchOutcome::Unknown,
        session_hero: None,
        hint: None,
        pending_boundary: false,
        awaiting_first_board: false,
        baseline_age: None,
        progressed_boards: 0,
        session_map: None,
        incoming_map: None,
        session_map_source: None,
    });
    assert!(!plan.split && !plan.defer && !plan.skip_store);
    assert!(plan.refresh_baseline);
}

#[test]
fn one_low_board_does_not_split_and_a_young_drop_does_not_defer() {
    let prev = gate(counters(14, 22, 6, 2400, 9800, 400));
    let low = counters(2, 1, 0, 350, 60, 800);
    let young = plan_at(
        &prev,
        low,
        false,
        0,
        false,
        20,
        true,
        true,
        MatchOutcome::Unknown,
    );
    assert!(
        !young.defer && !young.split,
        "under 45 seconds is still this match"
    );
    let armed = plan_at(
        &prev,
        low,
        false,
        0,
        false,
        50,
        true,
        true,
        MatchOutcome::Unknown,
    );
    assert!(armed.defer && !armed.split);
    assert!(armed.seal.is_none());
    let other_player = plan_at(
        &prev,
        counters(18, 9, 7, 5000, 2000, 800),
        false,
        0,
        false,
        50,
        true,
        true,
        MatchOutcome::Unknown,
    );
    assert!(
        !other_player.defer && !other_player.split,
        "another player's totals are not a fresh match"
    );
    let unidentified = plan_capture(&CapturePlanInput {
        prev_gate: Some(&prev),
        baseline: Some(&prev),
        streak: 0,
        cur: low,
        suspect: CLEAN,
        create_session: false,
        suppress_same_unfinished: true,
        age: Some(Duration::from_secs(80)),
        min_gap: Duration::from_secs(120),
        classic_regressed: false,
        row_counts: false,
        row_id: None,
        baseline_row: Some(0),
        confirmed_end: false,
        inherited_outcome: MatchOutcome::Unknown,
        frame_outcome: MatchOutcome::Unknown,
        session_hero: None,
        hint: None,
        pending_boundary: false,
        awaiting_first_board: false,
        baseline_age: None,
        progressed_boards: 0,
        session_map: None,
        incoming_map: None,
        session_map_source: None,
    });
    assert!(!unidentified.defer && !unidentified.split);
    assert!(!unidentified.refresh_baseline);
    assert!(!unidentified.skip_store);
}

#[test]
fn gap_after_a_hint_seals_it_on_the_old_session() {
    let prev = gate(counters(29, 8, 5, 9242, 1000, 200));
    let plan = plan_capture(&CapturePlanInput {
        prev_gate: Some(&prev),
        baseline: None,
        streak: 0,
        cur: counters(3, 1, 0, 200, 50, 10),
        suspect: CLEAN,
        create_session: false,
        suppress_same_unfinished: false,
        age: Some(Duration::from_secs(130)),
        min_gap: Duration::from_secs(120),
        classic_regressed: true,
        row_counts: true,
        row_id: Some(0),
        baseline_row: Some(0),
        confirmed_end: false,
        inherited_outcome: MatchOutcome::Unknown,
        frame_outcome: MatchOutcome::Defeat,
        session_hero: None,
        hint: Some(MatchOutcome::Defeat),
        pending_boundary: false,
        awaiting_first_board: false,
        baseline_age: None,
        progressed_boards: 0,
        session_map: None,
        incoming_map: None,
        session_map_source: None,
    });
    assert!(plan.split);
    assert_eq!(plan.seal, Some(MatchOutcome::Defeat));
    assert_eq!(plan.stored_outcome, MatchOutcome::Defeat);
    assert_eq!(plan.close_reason, Some(CloseReason::StatRegression));
}

#[test]
fn awaiting_first_board_keeps_its_first_tab() {
    let prev = gate(counters(14, 22, 6, 2400, 9800, 400));
    let plan = plan_capture(&CapturePlanInput {
        prev_gate: Some(&prev),
        baseline: Some(&prev),
        streak: 0,
        cur: counters(2, 1, 0, 350, 60, 800),
        suspect: CLEAN,
        create_session: true,
        suppress_same_unfinished: false,
        age: Some(Duration::from_secs(130)),
        min_gap: Duration::from_secs(120),
        classic_regressed: true,
        row_counts: true,
        row_id: Some(0),
        baseline_row: Some(2),
        confirmed_end: false,
        inherited_outcome: MatchOutcome::Unknown,
        frame_outcome: MatchOutcome::Unknown,
        session_hero: None,
        hint: None,
        pending_boundary: false,
        awaiting_first_board: true,
        baseline_age: Some(Duration::from_secs(130)),
        progressed_boards: 0,
        session_map: None,
        incoming_map: None,
        session_map_source: None,
    });
    assert!(
        !plan.split,
        "the first board belongs to the session the start screen opened"
    );
    assert!(plan.close_reason.is_none());
}

#[test]
fn suspect_or_other_row_is_not_a_fresh_reset() {
    let prev = gate(counters(14, 22, 6, 2400, 9800, 400));
    let low = counters(2, 1, 0, 350, 60, 800);
    let mut suspect = CLEAN;
    suspect[0] = true;
    let flagged = plan_capture(&CapturePlanInput {
        prev_gate: Some(&prev),
        baseline: Some(&prev),
        streak: 0,
        cur: low,
        suspect,
        create_session: false,
        suppress_same_unfinished: true,
        age: Some(Duration::from_secs(60)),
        min_gap: Duration::from_secs(120),
        classic_regressed: false,
        row_counts: true,
        row_id: Some(2),
        baseline_row: Some(2),
        confirmed_end: false,
        inherited_outcome: MatchOutcome::Unknown,
        frame_outcome: MatchOutcome::Unknown,
        session_hero: None,
        hint: None,
        pending_boundary: false,
        awaiting_first_board: false,
        baseline_age: Some(Duration::from_secs(60)),
        progressed_boards: 0,
        session_map: None,
        incoming_map: None,
        session_map_source: None,
    });
    assert!(
        !flagged.defer && !flagged.split,
        "a suspect elim read is not a fresh reset"
    );
    let other_row = plan_capture(&CapturePlanInput {
        prev_gate: Some(&prev),
        baseline: Some(&prev),
        streak: 0,
        cur: low,
        suspect: CLEAN,
        create_session: false,
        suppress_same_unfinished: true,
        age: Some(Duration::from_secs(60)),
        min_gap: Duration::from_secs(120),
        classic_regressed: false,
        row_counts: true,
        row_id: Some(0),
        baseline_row: Some(2),
        confirmed_end: false,
        inherited_outcome: MatchOutcome::Unknown,
        frame_outcome: MatchOutcome::Unknown,
        session_hero: None,
        hint: None,
        pending_boundary: false,
        awaiting_first_board: false,
        baseline_age: Some(Duration::from_secs(60)),
        progressed_boards: 0,
        session_map: None,
        incoming_map: None,
        session_map_source: None,
    });
    assert!(
        !other_row.defer && !other_row.split,
        "a different row never counts as a fresh reset"
    );
    assert!(!other_row.skip_store);
}
