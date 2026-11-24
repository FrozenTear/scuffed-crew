//! One taken board's read of each own-row stat for the Unsure confirm rule
//! (alpha.3 item 5), and where it goes: onto the current game, or held with
//! a fresh-match board ([`stat_tracker::games::stat_reads`]).

use crate::*;
use stat_tracker::games::stat_reads::{STATS, TabReads};
use stat_tracker::unsure_rule::StatRead;

/// Each own-row stat as the digit reader read it on `board`, before the
/// capture gate: `value` from `reader_apply::own_row_guesses`, the cell's
/// own confidence, and unsure when the cell is suspect, the column is held,
/// the own row is suspect, the header is dimmed or the cells are not
/// trusted. A board with no identified own row, or a cell never attempted,
/// gives `None` (nothing is pushed for it).
pub(crate) fn board_reads(
    board: &shadow::BoardRead,
    own: reader_apply::OwnRow,
    held_cols: [bool; capture_gate::GATE_COLS],
    own_row_suspect: bool,
    trusted: bool,
) -> TabReads {
    let reader_apply::OwnRow::Identified { index, .. } = own else {
        return [None; 6];
    };
    let guesses = reader_apply::own_row_guesses(board, own);
    let row_unsure = own_row_suspect || board.header_dimmed || !trusted;
    std::array::from_fn(|col| {
        let field = board.get(&format!("r{index}.{}", STATS[col]))?;
        reader_apply::field_attempted(field).then_some(StatRead {
            value: guesses[col],
            confidence: field.confidence,
            unsure: field.suspect || held_cols[col] || row_unsure,
            reset: false,
        })
    })
}

/// Mark the reads the capture gate did not vouch for: a held column, a
/// low-trust capture, or a field the stored row lists as suspect (the gate
/// lowered, revised or held it).
pub(crate) fn mark_gate(
    tab: &mut TabReads,
    gate: &capture_gate::GateOutcome,
    suspect_fields: &[String],
) {
    for (col, read) in tab.iter_mut().enumerate() {
        if let Some(read) = read {
            read.unsure |= gate.state.low_trust
                || gate.holds.iter().any(|h| h.col == col)
                || suspect_fields.iter().any(|s| s == STATS[col]);
        }
    }
}

#[cfg(test)]
mod tests;
