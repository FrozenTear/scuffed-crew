//! `suspect_fields` names for the holds, un-latches and lowered stats of
//! a capture.

use super::state::{COL_FIELD, Counters, GATE_COLS, Hold, HoldKind, Unlatch};

/// Flat suspect names for holds that kept the previous value.
///
/// A monotonic drop, a single-field jump and a cell the reader did not vouch
/// for are unsure. A rate-cap spike and a trailing-digit inject stay on
/// their own hold kinds and are not added here.
pub fn unsure_fields(holds: &[Hold]) -> Vec<&'static str> {
    let mut names = Vec::new();
    for hold in holds {
        if !matches!(
            hold.kind,
            HoldKind::Monotonic | HoldKind::Jump | HoldKind::Unread
        ) {
            continue;
        }
        let name = COL_FIELD[hold.col];
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// Flat names of every cell whose stored value is not this capture's read,
/// whatever the hold kind.
///
/// `reader = "new"` flags all of them: a rate-cap or trailing-digit hold
/// keeps an older number just as a drop does, and after a run of held
/// captures that number can be minutes old (F266-3). ocr-v1 keeps
/// [`unsure_fields`], which leaves those two kinds unflagged.
pub fn held_fields(holds: &[Hold]) -> Vec<&'static str> {
    let mut names = Vec::new();
    for hold in holds {
        let name = COL_FIELD[hold.col];
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// Flat names of the cells an un-latch revised down on this capture.
///
/// The gate lets one clean read replace an unconfirmed column with a lower
/// number, and that is not a hold. The row that stores the lower number
/// names those fields.
pub fn revised_fields(unlatches: &[Unlatch]) -> Vec<&'static str> {
    let mut names = Vec::new();
    for unlatch in unlatches {
        let name = COL_FIELD[unlatch.col];
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// Flat names of the stats that are lower in `after` than in `before`.
///
/// `before` is the row stored just before in the same game. Whatever let
/// the number down (an un-latch, a restart, a board carried onto a split),
/// a stat that goes down inside one game is flagged under `reader = "new"`.
pub fn lowered_fields(before: Counters, after: Counters) -> Vec<&'static str> {
    let (before, after) = (before.to_array(), after.to_array());
    (0..GATE_COLS)
        .filter(|&col| after[col] < before[col])
        .map(|col| COL_FIELD[col])
        .collect()
}
