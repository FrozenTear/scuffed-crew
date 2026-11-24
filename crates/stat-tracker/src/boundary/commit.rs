//! Applying decisions to the state: [`commit_poll`] and
//! [`note_accepted_capture`].

use std::time::Instant;

use super::machine::{BoundaryState, outcome_sealed_on_close};
use super::types::{CapturePlan, CloseReason, CloseRecord, PollCommit, PollDecision, ResultMark};
use crate::capture_gate::{self, GateState};
use crate::model::MatchOutcome;

/// Apply a poll decision. This is the state change `main` runs before it
/// writes the store. The decision itself came from [`transition`].
///
/// [`transition`]: super::transition
pub fn commit_poll(state: &mut BoundaryState, decision: PollDecision, now: Instant) -> PollCommit {
    match decision {
        PollDecision::Keep | PollDecision::IgnoreContradictory { .. } => PollCommit {
            recorded_outcome: None,
            adopted_map: None,
            closed: None,
        },
        PollDecision::Update(update) => {
            if update.arm_pending {
                state.pending_boundary = true;
                if state.deferred.is_none() {
                    state.reset_streak = state.reset_streak.max(1);
                }
            }
            if update.arm_reset && state.deferred.is_none() {
                state.reset_streak = state.reset_streak.max(1);
            }
            if update.clear_awaiting {
                state.awaiting_first_board = false;
            }
            if let Some(mark) = update.result {
                // The hint that was replaced was a different result. The
                // accolade belongs to the next game, and the new hint must
                // not reopen the text-fallback window on the confirming tick.
                if state
                    .result
                    .is_some_and(|prev| prev.outcome.is_decided() && prev.outcome != mark.outcome)
                {
                    state.text_fallback_locked = true;
                }
                let replaced = state.result.is_none_or(|prev| {
                    prev.outcome != mark.outcome || prev.confirmed != mark.confirmed
                });
                if replaced {
                    state.progressed_boards = 0;
                }
                state.result = Some(mark);
            }
            let recorded = update
                .record_outcome
                .filter(|outcome| outcome.is_decided() && !state.outcome.is_decided());
            if let Some(outcome) = recorded {
                state.outcome = outcome;
                // Grace starts when the result is recorded, not when the
                // word was first sighted.
                state.outcome_at = Some(now);
                state.pending_boundary = false;
                state.awaiting_first_board = false;
            }
            // `adopt_map` is set when the session has no map, or when a
            // text-fallback name is still open for an accolade. The in-memory
            // name is replaced, not only filled when empty.
            let adopted = update.adopt_map.clone();
            if let Some(map) = adopted.clone() {
                state.map = Some(map);
            }
            PollCommit {
                recorded_outcome: recorded,
                adopted_map: adopted,
                closed: None,
            }
        }
        PollDecision::Open(open) => {
            let previous = std::mem::replace(state, BoundaryState::new(None));
            let seal = outcome_sealed_on_close(previous.outcome, open.seal_outcome);
            state.outcome = open.new_outcome;
            state.outcome_at = open.new_outcome.is_decided().then_some(now);
            state.map = open.new_map;
            state.result = open.new_result;
            state.awaiting_first_board = !open.new_outcome.is_decided()
                && matches!(
                    open.reason,
                    CloseReason::MapVote | CloseReason::HeroSelectOrBan
                );
            PollCommit {
                recorded_outcome: None,
                adopted_map: None,
                closed: Some(CloseRecord {
                    reason: open.reason,
                    seal,
                }),
            }
        }
    }
}

/// Fold a capture that stayed on this session. A deferred board is held and
/// does not move the gate. A stored row always updates the gate.
pub fn note_accepted_capture(
    state: &mut BoundaryState,
    plan: &CapturePlan,
    accepted: GateState,
    now: Instant,
) {
    if plan.ignore_row {
        return;
    }
    if plan.defer {
        // A deferral on a game with no baseline yet takes the gate as the
        // baseline. Its time is the gate's, so the reset age is measured
        // from that board (`compare_age`).
        if state.reset_baseline.is_none() && plan.reset_baseline.is_some() {
            state.baseline_at = state.last_board_at;
        }
        state.reset_streak = plan.reset_streak;
        state.reset_baseline = plan.reset_baseline;
        state.baseline_row = plan.baseline_row;
        state.deferred = plan.deferred_counters;
        return;
    }
    if plan.refresh_baseline || plan.count_progress || plan.clear_hint {
        state.deferred = None;
    }
    // A discarded suspect row does not move the monotonic gate. Its raw
    // counters can still anchor the reset baseline, which is what a later
    // fresh board is compared with.
    let mut monotonic = accepted;
    let anchor = monotonic.boundary_anchor.take();
    state.gate = Some(monotonic);
    state.reset_streak = plan.reset_streak;
    state.reset_baseline = if plan.refresh_baseline {
        Some(anchor.map_or(monotonic, |raw| capture_gate::GateState {
            accepted: raw,
            last_raw: raw,
            ..capture_gate::GateState::default()
        }))
    } else {
        plan.reset_baseline
    };
    state.baseline_row = plan.baseline_row;
    state.last_board_at = Some(now);
    state.awaiting_first_board = false;
    if plan.refresh_baseline {
        state.baseline_at = Some(now);
    }
    if plan.count_progress {
        state.progressed_boards = state.progressed_boards.saturating_add(1);
    }
    let confirmed_mark =
        state.outcome.is_decided() || state.result.is_some_and(|mark| mark.confirmed);
    if plan.clear_hint && !confirmed_mark {
        state.result = None;
        state.pending_boundary = false;
        state.progressed_boards = 0;
        // Progressed boards are the capture path that drops the hint.
        // Drop the lock with it, or a later accolade still cannot correct
        // a text-fallback name after the hint is gone.
        state.text_fallback_locked = false;
    }
}

pub fn has_post_result(outcome: MatchOutcome, result: Option<ResultMark>) -> bool {
    outcome.is_decided() || result.is_some_and(|mark| mark.outcome.is_decided())
}
