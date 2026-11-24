//! The Tab path: [`plan_capture`] and [`plan_capture_held`].

use std::time::{Duration, Instant};

use super::board::{
    counters_progressed, implausible_growth, is_fresh_reset, known_cols, mature_board, named_map,
    same_baseline_row, under_fresh_match,
};
use super::machine::{BoundaryState, transition};
use super::types::{
    BoardDoubt, BoardObs, CapturePlan, CapturePlanInput, Effect, MapSource, Obs, ResultMark,
};
use crate::capture_gate::Counters;

/// A hinted session whose Tab names a different map is the next game.
///
/// A session with no board of its own splits on that first board. Its map
/// came from the accolade on the hint tick; without that read there is no
/// stored map to differ from. A session that already has a board waits out
/// the gap, and only when the stored map was the top bar or the accolade.
/// A full-board text fallback is not that map. The same map does not split:
/// a late Tab of this match names the map already stored.
pub(super) fn hinted_different_map(input: &CapturePlanInput<'_>) -> bool {
    if input.confirmed_end || input.hint.filter(|outcome| outcome.is_decided()).is_none() {
        return false;
    }
    match (named_map(input.session_map), named_map(input.incoming_map)) {
        (Some(session), Some(incoming)) if !session.eq_ignore_ascii_case(incoming) => {
            if input.prev_gate.is_none() {
                return true;
            }
            input
                .session_map_source
                .is_some_and(MapSource::trusted_for_board_split)
                && input.age.is_some_and(|age| age >= input.min_gap)
        }
        _ => false,
    }
}

/// Decide whether this capture closes the session, and which outcome the
/// landing session stores. The decision is [`transition`].
pub fn plan_capture(input: &CapturePlanInput<'_>) -> CapturePlan {
    plan_capture_held(input, BoardDoubt::default())
}

/// [`plan_capture`] for a board with cells the reader did not vouch for
/// (`reader = "new"`).
///
/// `held[col]` means this capture has no number for the column;
/// `input.cur` carries the game's last accepted value there, and
/// `input.suspect` has the column set. A column the game has never read
/// ([`GateState::unread`] on the board it is compared with) has no number on
/// that side either. Such a column abstains: it is left out of every
/// comparison, so it neither shows a move nor vetoes one.
///
/// * A fresh-match reset is decided on the elims, deaths and damage columns
///   that both boards have a number for. At least two of the three are
///   needed, all of them under the fresh-match thresholds, and the earlier
///   board has to be mature on one of them. A held column no longer blocks
///   the reset, and a board with a held cell re-anchors the reset baseline
///   like any other identified row. Before this, one held cell anywhere kept
///   the baseline unset for the whole game, and a missed game end could not
///   be recovered by the reset rule at all (F266-2).
/// * Growth is measured over the time since the column's last accepted read
///   ([`GateState::held_secs`]), and a never-read column is not growth: its
///   placeholder 0 is not a number (F266-5).
/// * A fresh-match board with a held cell is held off this session like any
///   other, and counts as the first reset signal, but it is not carried
///   onto the next session as a row: it has no number for that cell.
///
/// # A suspect own row
///
/// A suspect own row ([`BoardDoubt::suspect_row`]) may be another player's.
/// It takes part in the plan like this:
///
/// * **It anchors.** On the baseline's own row, or when the game has no
///   baseline yet, it re-anchors the reset baseline like any identified row.
///   Without that, a game whose every Tab has a suspect row has no baseline,
///   no reset can fire for it, and the next game's boards are appended to it
///   (R3-1). It never moves the baseline to another row.
/// * **It can say "reset", never alone.** A fresh-match board on a suspect
///   row is held off the game and counts as a reset signal, but it is not
///   carried as a row and it does not commit the split by itself: it splits
///   only after two earlier signals, or when a trusted fresh-match board is
///   already held ([`BoardDoubt::board_deferred`]). A trusted board splits on
///   one earlier signal, as before. So one wrong suspect row cannot split a
///   real game, also not after a hero-select screen primed the streak.
/// * **It is not progress.** It does not count toward dropping a result
///   hint.
///
/// With nothing held, nothing unread and no suspect row this is exactly
/// [`plan_capture`].
///
/// [`GateState::unread`]: crate::capture_gate::GateState::unread
/// [`GateState::held_secs`]: crate::capture_gate::GateState::held_secs
pub fn plan_capture_held(input: &CapturePlanInput<'_>, doubt: BoardDoubt) -> CapturePlan {
    let held = doubt.held;
    let mut state = BoundaryState::new(None);
    if doubt.board_deferred {
        // Only its presence is read (`board_effect`), as in `decide_poll`.
        state.deferred = Some(Counters::default());
    }
    state.outcome = input.inherited_outcome;
    state.reset_streak = input.streak;
    state.reset_baseline = input.baseline.copied();
    state.baseline_row = input.baseline_row;
    state.gate = input.prev_gate.copied();
    state.hero = input.session_hero.map(str::to_string);
    state.pending_boundary = input.pending_boundary;
    state.progressed_boards = input.progressed_boards;
    state.awaiting_first_board = input.awaiting_first_board;
    if let Some(outcome) = input.hint.filter(|outcome| outcome.is_decided())
        && !input.confirmed_end
    {
        state.result = Some(ResultMark {
            outcome,
            confirmed: false,
            seen_at: Instant::now(),
        });
    }

    // The first Tab of a hinted session that names a different map is the
    // next game. A late Tab of the same match names the same map and stays.
    // A session that already has a board uses the same rule only after the
    // gap, and only when its stored map came from the top bar or the accolade.
    if hinted_different_map(input) {
        let decided = transition(
            &state,
            &Obs::HintedDifferentMap {
                frame_outcome: input.frame_outcome,
            },
        );
        if let Effect::Split { .. } = &decided.effect {
            return plan_from_effect(input, doubt, &decided.effect, false);
        }
    }

    // A session a start screen just opened has no board of its own. The
    // previous game is not an anchor, and a gap is not applicable there.
    let gap = !input.confirmed_end
        && !input.suppress_same_unfinished
        && input.age.is_some_and(|age| age >= input.min_gap)
        && input.classic_regressed
        && !input.create_session
        && !input.awaiting_first_board;
    if gap {
        let decided = transition(
            &state,
            &Obs::Gap {
                frame_outcome: input.frame_outcome,
            },
        );
        if let Effect::Split { .. } = &decided.effect {
            return plan_from_effect(input, doubt, &decided.effect, false);
        }
    }

    let compare = input.baseline.or(input.prev_gate);
    let elapsed = input.baseline_age.or(input.age).unwrap_or(Duration::ZERO);
    let implausible =
        compare.is_some_and(|gate| implausible_growth(gate, &input.cur, &held, elapsed));
    let fresh_reset = is_fresh_reset(input, &held, compare, implausible);
    let progressed = !implausible
        && compare.is_some_and(|gate| {
            counters_progressed(&gate.accepted, &input.cur, &known_cols(gate, &held))
        });
    let decided = transition(
        &state,
        &Obs::Board(BoardObs {
            fresh_reset,
            progressed,
            implausible,
            frame_outcome: input.frame_outcome,
            suspect_row: doubt.suspect_row,
        }),
    );
    plan_from_effect(input, doubt, &decided.effect, implausible)
}

fn plan_from_effect(
    input: &CapturePlanInput<'_>,
    doubt: BoardDoubt,
    effect: &Effect,
    implausible: bool,
) -> CapturePlan {
    let held = doubt.held;
    let stored_if_stay = if input.inherited_outcome.is_decided() {
        input.inherited_outcome
    } else {
        input.frame_outcome
    };
    let anchor = input.baseline.or(input.prev_gate);
    let fresh_shaped = anchor.is_some_and(|gate| {
        under_fresh_match(&gate.accepted, &input.cur, &known_cols(gate, &held))
    });
    let same_or_unset =
        input.baseline_row.is_none() || same_baseline_row(input.baseline_row, input.row_id);
    // The baseline's own row fell under the fresh-match thresholds of a
    // mature board: the shape of a new game. Taken as the baseline, the new
    // game's low counters would leave nothing mature to reset from, and the
    // two games would merge (CAP-8). It is stored, and the baseline, the
    // streak and a held board stay, so the reset can still fire.
    let reset_shaped = same_baseline_row(input.baseline_row, input.row_id)
        && anchor.is_some_and(|gate| {
            let known = known_cols(gate, &held);
            mature_board(&gate.accepted, &known)
                && under_fresh_match(&gate.accepted, &input.cur, &known)
        });
    // A different row re-anchors only when the stats continue. A low-total
    // teammate row is fresh-shaped: it is stored, and it neither replaces
    // the baseline nor counts toward the reset streak.
    // A suspect row anchors its own row's baseline, or the first one. It
    // does not take the baseline over from another row.
    let refresh = input.row_id.is_some()
        && input.row_counts
        && !implausible
        && !reset_shaped
        && (same_or_unset || (!fresh_shaped && !doubt.suspect_row));
    // A suspect row is not progress: it may be another player's.
    let counts = input.row_counts && !doubt.suspect_row;
    // An unidentified or implausible row is stored and leaves a held board
    // where it is. Only a refresh, or a counted progression, ends the streak.
    let counted_progress =
        counts && matches!(effect, Effect::CountProgress | Effect::ClearHintAndAppend);
    let drop_streak = refresh || counted_progress;
    match effect {
        Effect::Ignore
        | Effect::Seal { .. }
        | Effect::RememberHint { .. }
        | Effect::ArmPending
        | Effect::PrimeReset => {
            unreachable!("poll effects are not capture plans")
        }
        Effect::Append | Effect::ClearHintAndAppend | Effect::CountProgress => CapturePlan {
            split: false,
            defer: false,
            ignore_row: false,
            skip_store: false,
            clear_hint: matches!(effect, Effect::ClearHintAndAppend) && counts,
            count_progress: matches!(effect, Effect::CountProgress) && counts,
            reset_streak: if drop_streak { 0 } else { input.streak },
            reset_baseline: if refresh {
                None
            } else {
                input.baseline.copied()
            },
            baseline_row: if refresh {
                input.row_id
            } else {
                input.baseline_row
            },
            refresh_baseline: refresh,
            stored_outcome: stored_if_stay,
            deferred_counters: None,
            seal: None,
            close_reason: None,
        },
        Effect::Defer => CapturePlan {
            split: false,
            defer: true,
            ignore_row: false,
            skip_store: true,
            clear_hint: false,
            reset_streak: input.streak.saturating_add(1),
            reset_baseline: input.baseline.copied().or(input.prev_gate.copied()),
            baseline_row: input.baseline_row.or(input.row_id),
            refresh_baseline: false,
            count_progress: false,
            stored_outcome: stored_if_stay,
            // A board with a held cell is not a row: `input.cur` has the
            // previous game's number in that column. A suspect row may be
            // another player's. Either still counts as a reset signal;
            // nothing is carried onto the next session.
            deferred_counters: (!held.iter().any(|&cell| cell) && !doubt.suspect_row)
                .then_some(input.cur),
            seal: None,
            close_reason: None,
        },
        Effect::Split {
            new_outcome,
            seal,
            reason,
        } => CapturePlan {
            split: true,
            defer: false,
            ignore_row: false,
            skip_store: false,
            clear_hint: false,
            reset_streak: 0,
            reset_baseline: None,
            baseline_row: input.row_id,
            refresh_baseline: false,
            count_progress: false,
            stored_outcome: *new_outcome,
            deferred_counters: None,
            seal: *seal,
            close_reason: Some(*reason),
        },
    }
}
