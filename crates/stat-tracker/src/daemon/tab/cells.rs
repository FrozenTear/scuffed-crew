//! Cell trust on a Tab board: the gate's held cells and suspects, and the
//! new reader's stats, holds and unsure reads.

use crate::*;

/// Put the game's last accepted value in each held stat column of `parsed`
/// (0 when the game has none yet).
///
/// A belt, not a rule: nothing that runs before the gate reads a held
/// column. The boundary plan leaves it out of every comparison
/// (`boundary::plan_capture_held`; `the_plan_does_not_read_a_held_cell`
/// pins that), the split vote skips a suspect column, both gates ignore
/// the number, and the report kit's field is overwritten after the gate.
/// It stays so that a later reader of these counters finds the game's own
/// value and not a 0 that looks like a read.
pub(crate) fn fill_held_cells(
    parsed: &mut storage::PersonalMatch,
    held: [bool; capture_gate::GATE_COLS],
    prior: Option<capture_gate::Counters>,
) {
    let prior = prior.unwrap_or_default();
    let prior = [
        prior.elims,
        prior.assists,
        prior.deaths,
        prior.damage,
        prior.healing,
        prior.mitigation,
    ];
    let cells = [
        &mut parsed.elims,
        &mut parsed.assists,
        &mut parsed.deaths,
        &mut parsed.damage,
        &mut parsed.healing,
        &mut parsed.mitigation,
    ];
    for (col, cell) in cells.into_iter().enumerate() {
        if held[col] {
            *cell = prior[col];
        }
    }
}

/// Run the capture gate on the new reader's stats.
///
/// `apply_new_reader` puts the sure digit reads back on the row. A drop is
/// clamped back and flagged. A single-field jump past the capture gate's
/// limit waits until the next read matches. A `held` column (the digit
/// reader did not vouch for the cell) keeps the game's last accepted value
/// and is flagged; whatever number sits in it on `parsed` is ignored.
/// `suspect` and `trusted` are this capture's marks. `carried` is the board
/// written onto the same session just before this row ([`carried_board`]);
/// it seeds the held columns of a capture that opens the game. The returned
/// gate is what the next capture compares against.
///
/// Every cell whose stored value is not this capture's read is flagged,
/// whatever kept it out (`capture_gate::held_fields`): under this reader a
/// rate-cap or trailing-digit hold after a run of held captures keeps a
/// number that can be minutes old (F266-3). So is every stat that is lower
/// than on the row before it (`capture_gate::revised_fields`,
/// `capture_gate::lowered_fields`).
///
/// The capture path calls [`hold_new_reader_stats_with_guesses`]; this is
/// that call with no digit guesses, kept for the gate tests.
#[cfg(test)]
pub(crate) fn hold_new_reader_stats(
    parsed: &mut storage::PersonalMatch,
    prior: Option<(capture_gate::GateState, std::time::Duration)>,
    suspect: [bool; capture_gate::GATE_COLS],
    held_cells: [bool; capture_gate::GATE_COLS],
    split: bool,
    trusted: bool,
    carried: Option<(Counters, std::time::Duration)>,
) -> capture_gate::GateOutcome {
    hold_new_reader_stats_with_guesses(
        parsed,
        prior,
        suspect,
        held_cells,
        split,
        trusted,
        carried,
        [None; capture_gate::GATE_COLS],
    )
}

/// The six gate columns of `c`, in gate order.
pub(crate) fn gate_cols(c: Counters) -> [u32; capture_gate::GATE_COLS] {
    [
        c.elims,
        c.assists,
        c.deaths,
        c.damage,
        c.healing,
        c.mitigation,
    ]
}

/// [`Counters`] from six values in gate order.
pub(crate) fn counters_from_cols(a: [u32; capture_gate::GATE_COLS]) -> Counters {
    Counters {
        elims: a[0],
        assists: a[1],
        deaths: a[2],
        damage: a[3],
        healing: a[4],
        mitigation: a[5],
    }
}

/// [`hold_new_reader_stats`] with the digit reader's own read of every cell,
/// sure or not (`reader_apply::own_row_guesses`).
///
/// A held cell used to store the game's last accepted value. When no later
/// Tab read the cell for sure, that old number was the game's final value:
/// a healing of 3143 saved under a board that showed 5298 (report a6f3).
/// Now a held cell whose unsure read the gate would have taken, had the
/// reader vouched for it ([`fresh_unsure_reads`]), stores that read. It
/// stays flagged in `suspect_fields`. The gate state is not touched: the
/// accepted value, the previous raw read and every streak stay as the gate
/// left them, so the next capture is judged exactly as before. Only the
/// stored row changes, and the state remembers what that row shows
/// ([`capture_gate::GateState::unsure_shown`]) so the next row cannot go
/// below it unflagged. A held cell with no read, or with a read the gate
/// would hold (lower than the accepted value, a jump, an injected digit,
/// past the rate cap), keeps the accepted value, flagged, as before.
#[allow(clippy::too_many_arguments)]
pub(crate) fn hold_new_reader_stats_with_guesses(
    parsed: &mut storage::PersonalMatch,
    prior: Option<(capture_gate::GateState, std::time::Duration)>,
    suspect: [bool; capture_gate::GATE_COLS],
    held_cells: [bool; capture_gate::GATE_COLS],
    split: bool,
    trusted: bool,
    carried: Option<(Counters, std::time::Duration)>,
    guesses: [Option<u32>; capture_gate::GATE_COLS],
) -> capture_gate::GateOutcome {
    let seed = carried.filter(|_| held_cells.iter().any(|&cell| cell));
    // The row stored just before this one in the same game: the game's last
    // accepted counters once it has a row, or the carried board on the
    // capture that opens the game with one. A column whose stored row
    // showed an unsure read instead of the accepted value compares with
    // that read.
    let opens_game = split || prior.is_none();
    let before = if opens_game {
        carried.map(|(board, _)| board)
    } else {
        prior
            .map(|(state, _)| state)
            .filter(|state| state.unread_cols().is_empty())
            .map(|state| {
                let mut shown = gate_cols(state.accepted);
                for (col, value) in shown.iter_mut().enumerate() {
                    if state.has_unsure_shown[col] {
                        *value = state.unsure_shown[col];
                    }
                }
                counters_from_cols(shown)
            })
    };
    let read = capture_gate::Counters {
        elims: parsed.elims,
        assists: parsed.assists,
        deaths: parsed.deaths,
        damage: parsed.damage,
        healing: parsed.healing,
        mitigation: parsed.mitigation,
    };
    let mut held =
        capture_gate::apply_gate_seeded(prior, read, suspect, held_cells, split, trusted, seed);
    let fresh = fresh_unsure_reads(
        prior,
        read,
        suspect,
        held_cells,
        guesses,
        split,
        seed,
        held.accepted,
    );
    let mut stored = gate_cols(held.accepted);
    for col in 0..capture_gate::GATE_COLS {
        held.state.has_unsure_shown[col] = false;
        held.state.unsure_shown[col] = 0;
        // A column the game has never read stores no row at all (F266-1).
        if let Some(value) = fresh[col].filter(|_| !held.state.unread[col]) {
            if value != stored[col] {
                tracing::info!(
                    target: "scuffed_stat_tracker",
                    col,
                    read = value,
                    kept = stored[col],
                    "held stat cell stores the reader's unsure read, flagged, not the older accepted value"
                );
            }
            stored[col] = value;
            held.state.unsure_shown[col] = value;
            held.state.has_unsure_shown[col] = true;
        }
    }
    let stored = counters_from_cols(stored);
    parsed.elims = stored.elims;
    parsed.assists = stored.assists;
    parsed.deaths = stored.deaths;
    parsed.damage = stored.damage;
    parsed.healing = stored.healing;
    parsed.mitigation = stored.mitigation;
    // Flagged: every cell the gate kept at another value than the read,
    // every cell an un-latch revised down, and any stat lower than on the
    // row before, whatever let it down (R3-1: a stat never goes down
    // unflagged in a stored row). A held cell is a gate hold, so a stored
    // unsure read is flagged too.
    let mut unsure = capture_gate::held_fields(&held.holds);
    let lowered = before
        .map(|before| capture_gate::lowered_fields(before, stored))
        .unwrap_or_default();
    for name in capture_gate::revised_fields(&held.unlatches)
        .into_iter()
        .chain(lowered)
    {
        if !unsure.contains(&name) {
            unsure.push(name);
        }
    }
    note_gate_suspects(parsed, stored, &unsure);
    held
}

/// The unsure digit reads of held cells that may be stored in place of the
/// game's older accepted value (see [`hold_new_reader_stats_with_guesses`]).
///
/// A held column gets `Some(read)` only when the reader gave a number for
/// it, the number is not below `kept` (the accepted value the gate kept),
/// and the gate, run once more on the same capture with that read as an
/// untrusted, suspect read, keeps the read: no monotonic, rate-cap, jump or
/// injected-digit hold, and no refused first read. Every other column is
/// `None`. This is a dry run; its gate state is thrown away.
#[allow(clippy::too_many_arguments)]
pub(crate) fn fresh_unsure_reads(
    prior: Option<(capture_gate::GateState, std::time::Duration)>,
    read: Counters,
    suspect: [bool; capture_gate::GATE_COLS],
    held_cells: [bool; capture_gate::GATE_COLS],
    guesses: [Option<u32>; capture_gate::GATE_COLS],
    split: bool,
    seed: Option<(Counters, std::time::Duration)>,
    kept: Counters,
) -> [Option<u32>; capture_gate::GATE_COLS] {
    let candidate: [bool; capture_gate::GATE_COLS] =
        std::array::from_fn(|col| held_cells[col] && guesses[col].is_some());
    if !candidate.iter().any(|&c| c) {
        return [None; capture_gate::GATE_COLS];
    }
    let mut trial = gate_cols(read);
    let mut trial_held = held_cells;
    let mut trial_suspect = suspect;
    for col in 0..capture_gate::GATE_COLS {
        if let (true, Some(value)) = (candidate[col], guesses[col]) {
            trial[col] = value;
            trial_held[col] = false;
            trial_suspect[col] = true;
        }
    }
    let dry = capture_gate::apply_gate_seeded(
        prior,
        counters_from_cols(trial),
        trial_suspect,
        trial_held,
        split,
        false,
        seed,
    );
    let kept = gate_cols(kept);
    let accepted = gate_cols(dry.accepted);
    std::array::from_fn(|col| {
        let value = guesses[col]?;
        let held_again = dry.holds.iter().any(|hold| hold.col == col);
        (candidate[col]
            && value >= kept[col]
            && accepted[col] == value
            && !held_again
            && !dry.state.unread[col])
            .then_some(value)
    })
}

/// A gate hold keeps the old number and names the field.
///
/// `unsure` is the fields to name: ocr-v1 passes a drop and a single-field
/// jump (`capture_gate::unsure_fields`), the new reader every hold
/// (`capture_gate::held_fields`). The flag stays when the stored number is
/// the one the gate kept.
pub(crate) fn note_gate_suspects(
    parsed: &mut storage::PersonalMatch,
    held: capture_gate::Counters,
    unsure: &[&'static str],
) {
    let stored = [
        parsed.elims,
        parsed.assists,
        parsed.deaths,
        parsed.damage,
        parsed.healing,
        parsed.mitigation,
    ];
    let kept = [
        held.elims,
        held.assists,
        held.deaths,
        held.damage,
        held.healing,
        held.mitigation,
    ];
    for &name in unsure {
        let Some(col) = capture_gate::COL_FIELD
            .iter()
            .position(|field| *field == name)
        else {
            continue;
        };
        if stored[col] != kept[col] {
            continue;
        }
        if !parsed.suspect_fields.iter().any(|field| field == name) {
            parsed.suspect_fields.push(name.to_string());
        }
    }
    parsed.suspect_fields = reader_apply::upload_suspect_fields(&parsed.suspect_fields);
}

/// Replace stored fields from the new reader's board.
///
/// `board` is the board the capture path already accepted
/// (`reader_apply::board_refusal`, `reader_apply::read_own_row_stats`), and
/// `parsed` is the policy row built from it: its stats are the capture
/// gate's (a held cell carries the game's last accepted value), so a field
/// the board does not supply never takes an OCR number. The map and mode
/// already chosen by capture policy stay even when the board names another
/// map; under this reader that policy map already took the banner read as
/// its top-bar map ([`choose_top_bar_map`]). This does not touch the capture
/// gate, the session map, or the report.
///
/// The row is the one [`find_own_row`] found from the own-name seeds and the
/// highlight (alpha.7 O9); no name is matched here. `own_row_suspect` says
/// only one of the two named it where both could have: the row is used, and
/// every field read off it is marked suspect.
pub(crate) fn apply_new_reader(
    parsed: &mut storage::PersonalMatch,
    evidence: &shadow::result::ResultEvidence,
    own: reader_apply::OwnRow,
    own_row_suspect: bool,
    mut board: shadow::BoardRead,
    authority: Option<&str>,
) {
    board.set_result(shadow::result_field(evidence));
    let policy = reader_apply::PolicyRow {
        map: parsed.map_name.clone(),
        mode: parsed.game_mode.clone(),
        result: parsed.outcome.clone(),
        hero: parsed.hero.clone(),
        elims: parsed.elims,
        assists: parsed.assists,
        deaths: parsed.deaths,
        damage: parsed.damage,
        healing: parsed.healing,
        mitigation: parsed.mitigation,
    };
    // The own row's hero is stored only when the Tab's hero authority (own
    // row and career panel portrait) names it too.
    let saved =
        reader_apply::merge_saved_with_authority(&policy, own, Some(&board), Some(authority));
    parsed.map_name = saved.map;
    parsed.game_mode = saved.mode;
    parsed.outcome = saved.result;
    parsed.hero = saved.hero;
    parsed.role = parse::guess_role_public(&parsed.hero);
    parsed.elims = saved.elims;
    parsed.assists = saved.assists;
    parsed.deaths = saved.deaths;
    parsed.damage = saved.damage;
    parsed.healing = saved.healing;
    parsed.mitigation = saved.mitigation;
    parsed.recognizer = if saved.recognizer == shadow::digits::RECOGNIZER_ID {
        shadow::digits::live_recognizer_id().to_string()
    } else {
        saved.recognizer.to_string()
    };
    parsed.suspect_fields = saved.suspect_fields;
    if own_row_suspect {
        reader_apply::mark_own_row_suspect(&mut parsed.suspect_fields);
    }
    tracing::info!(
        target: "scuffed_stat_tracker",
        recognizer = parsed.recognizer,
        suspects = ?parsed.suspect_fields,
        "saved scoreboard reader"
    );
}
