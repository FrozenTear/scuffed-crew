//! Board-shape rules: fresh resets, implausible growth, mature boards and
//! the reset age. Pure arithmetic on counters.

use std::time::Duration;

use super::types::CapturePlanInput;
use crate::capture_gate::{self, Counters, GATE_COLS, GateState};

/// The first fresh-match board has to land at least this long after the
/// board it is compared with (the reset baseline, or the gate when there is
/// none yet). A drop a few seconds later is still this match. A row of
/// another kind stored in between (an unidentified row, another player's)
/// does not restart this clock: it says nothing about the own row.
pub const RESET_MIN_AGE: Duration = Duration::from_secs(45);

/// Whether `cur` is a real, sharp fall from the accepted counter: under half,
/// past the OCR jitter band, and not an edge-ink suspect read.
fn sharp_vote(acc: u32, raw: u32, cur: u32, cur_suspect: bool, raw_suspect: bool) -> bool {
    if cur_suspect || acc < 4 || cur.saturating_mul(2) >= acc {
        return false;
    }
    if raw_suspect {
        true
    } else {
        capture_gate::raw_dropped(raw, cur)
    }
}

/// After an end screen, a sharp drop across several cumulative columns is a
/// new game. Two of elims/deaths/damage, or any three of the six counters.
/// One column is still an OCR miss — the capture gate holds that cell.
///
/// Reset detection itself uses the fresh-match thresholds in [`plan_capture`].
/// This is the multi-column drop the 120-second gap compares.
///
/// [`plan_capture`]: super::plan_capture
pub fn post_result_stat_reset(
    prev: &GateState,
    cur: Counters,
    cur_suspect: [bool; GATE_COLS],
) -> bool {
    let acc = prev.accepted.to_array();
    let raw = prev.last_raw.to_array();
    let now = cur.to_array();
    let mut sharp = [false; GATE_COLS];
    for col in 0..GATE_COLS {
        sharp[col] = sharp_vote(
            acc[col],
            raw[col],
            now[col],
            cur_suspect[col],
            prev.last_raw_suspect[col],
        );
    }
    // edd order matches `Counters::edd`: elims, deaths, damage = cols 0, 2, 3.
    let edd = u8::from(sharp[0]) + u8::from(sharp[2]) + u8::from(sharp[3]);
    let n = sharp.iter().filter(|s| **s).count();
    edd >= 2 || n >= 3
}

/// A confident map name. Empty and `unknown` are not a map.
pub(super) fn named_map(map: Option<&str>) -> Option<&str> {
    map.map(str::trim)
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("unknown"))
}

/// Growth that must not become the reset baseline.
///
/// A single column past the time-scaled ceiling is an implausible jump. A
/// drop is not. Wide columns use the ceiling even when the read is clean;
/// the store gate still leaves clean wide columns uncapped.
///
/// A row whose mature columns all landed between 2x and 4x inside
/// [`SHORT_ALL_INCREASE`] is the residual all-increase misread. The same
/// shape over a longer stretch is ordinary growth. The row is stored either
/// way, and a short-age hit does not replace the baseline.
const SHORT_ALL_INCREASE: Duration = Duration::from_secs(60);

pub(super) fn implausible_growth(
    prev: &GateState,
    cur: &Counters,
    held: &[bool; GATE_COLS],
    elapsed: Duration,
) -> bool {
    let known = known_cols(prev, held);
    if elapsed < SHORT_ALL_INCREASE && uniform_two_to_four_times(&prev.accepted, cur, &known) {
        return true;
    }
    let acc = prev.accepted.to_array();
    let now = cur.to_array();
    for col in 0..GATE_COLS {
        // A column one side has no number for did not grow: a held cell
        // repeats the accepted value, and the first read of a never-read
        // column rises from a placeholder, not from a number.
        if !known[col] || now[col] <= acc[col] {
            continue;
        }
        // The accepted value is as old as the column's last accepted read.
        let secs = elapsed
            .as_secs()
            .saturating_add(u64::from(prev.held_secs[col]));
        let ceiling = if col <= 2 {
            ((secs / capture_gate::KILL_RATE_DIVISOR_SECS) as u32)
                .saturating_add(capture_gate::KILL_RATE_SLACK)
        } else {
            (secs as u32)
                .saturating_mul(capture_gate::WIDE_RATE_PER_SEC)
                .saturating_add(capture_gate::WIDE_RATE_SLACK)
        };
        if now[col] - acc[col] > ceiling {
            return true;
        }
    }
    false
}

/// Columns both boards have a number for: read on this capture (`held` is
/// false) and read at some point on the board it is compared with. All six
/// under ocr-v1.
pub(super) fn known_cols(prev: &GateState, held: &[bool; GATE_COLS]) -> [bool; GATE_COLS] {
    std::array::from_fn(|col| !held[col] && !prev.unread[col])
}

/// Every mature column (accepted ≥ 4) grew by 2x to 4x. One spiked column
/// is ordinary play. This band is under the rate ceiling and used to replace
/// the baseline.
fn uniform_two_to_four_times(prev: &Counters, cur: &Counters, known: &[bool; GATE_COLS]) -> bool {
    let pairs = [
        (prev.elims, cur.elims),
        (prev.assists, cur.assists),
        (prev.deaths, cur.deaths),
        (prev.damage, cur.damage),
        (prev.healing, cur.healing),
        (prev.mitigation, cur.mitigation),
    ];
    let mut comparable = 0u32;
    for (col, (before, after)) in pairs.into_iter().enumerate() {
        if before < 4 || !known[col] {
            continue;
        }
        comparable += 1;
        let lo = before.saturating_mul(2);
        let hi = before.saturating_mul(4);
        if after < lo || after > hi {
            return false;
        }
    }
    comparable >= 3
}

/// Elims, deaths and damage: the columns a reset is read from.
const EDD_COLS: [usize; 3] = [0, 2, 3];

pub(super) fn counters_progressed(
    prev: &Counters,
    cur: &Counters,
    known: &[bool; GATE_COLS],
) -> bool {
    let before = prev.to_array();
    let now = cur.to_array();
    let grew = (0..GATE_COLS).any(|col| known[col] && now[col] > before[col]);
    let dropped = EDD_COLS
        .iter()
        .any(|&col| known[col] && now[col] < before[col]);
    grew && !dropped
}

/// The earlier board is far enough into a match to reset from, judged on a
/// column both boards have a number for.
pub(super) fn mature_board(prev: &Counters, known: &[bool; GATE_COLS]) -> bool {
    (known[0] && prev.elims >= 4) || (known[3] && prev.damage >= 400)
}

/// Live-match fresh-start thresholds. A real slot change across a reset is
/// near zero. Another player's totals fail this and re-anchor instead. A
/// column one board has no number for is not compared.
pub(super) fn under_fresh_match(
    prev: &Counters,
    cur: &Counters,
    known: &[bool; GATE_COLS],
) -> bool {
    let elims_max = 2.max(prev.elims / 4);
    let deaths_max = 1.max(prev.deaths / 4);
    let damage_max = prev.damage / 4;
    (!known[0] || cur.elims <= elims_max)
        && (!known[2] || cur.deaths <= deaths_max)
        && (!known[3] || cur.damage <= damage_max)
}

/// No edge-ink suspect read in elims, deaths or damage. A held cell is not
/// such a read: it has no number, and abstains instead of vetoing.
fn edd_clean(suspect: &[bool; GATE_COLS], held: &[bool; GATE_COLS]) -> bool {
    EDD_COLS.iter().all(|&col| !suspect[col] || held[col])
}

pub(super) fn same_baseline_row(baseline_row: Option<u32>, row_id: Option<u32>) -> bool {
    matches!((baseline_row, row_id), (Some(baseline), Some(row)) if baseline == row)
}

pub(super) fn is_fresh_reset(
    input: &CapturePlanInput<'_>,
    held: &[bool; GATE_COLS],
    compare: Option<&GateState>,
    implausible: bool,
) -> bool {
    let Some(prev) = compare else {
        return false;
    };
    let known = known_cols(prev, held);
    // One column falling is a misread cell. Two of the three reset columns
    // have to be there to say the board fell.
    let enough = EDD_COLS.iter().filter(|&&col| known[col]).count() >= 2;
    same_baseline_row(input.baseline_row, input.row_id)
        && input.row_counts
        && edd_clean(&input.suspect, held)
        && enough
        && !implausible
        && mature_board(&prev.accepted, &known)
        && compare_age(input).is_some_and(|age| age >= RESET_MIN_AGE)
        && under_fresh_match(&prev.accepted, &input.cur, &known)
}

/// Age of the board a reset is measured from: the baseline's own time when
/// there is a baseline, else the gate's. Before CAP-8 this was the time since
/// the last stored row of any kind, so an unidentified row stored between two
/// fresh boards made the second one "too young", and it became the baseline.
/// A baseline with no recorded time falls back to the gate's age, which is
/// never older than the baseline.
fn compare_age(input: &CapturePlanInput<'_>) -> Option<Duration> {
    if input.baseline.is_some() {
        input.baseline_age.or(input.age)
    } else {
        input.age
    }
}
