//! The poll path: [`decide_poll`] and the map rules it uses.

use super::board::named_map;
use super::machine::{BoundaryState, transition, unconfirmed_replaces};
use super::types::{
    CloseReason, Effect, MapRelation, Obs, OpenNew, PollDecision, PollInput, ResultMark,
    UpdateCurrent, screen_reason,
};
use crate::capture_gate::{Counters, GateState};
use crate::model::MatchOutcome;

/// One map filter for the relation and for adopt: [`named_map`]. Empty and
/// `unknown` are not names. A looser trim-only check used to accept
/// `unknown`, which let a non-name block a real accolade or get stored.
fn adopt_map(input: &PollInput<'_>) -> Option<String> {
    let accolade = named_map(input.accolade_map)?;
    if named_map(trusted_session_map(input)).is_some() {
        return None;
    }
    if input.map.is_some() && !text_fallback_open_for_accolade(input) {
        return None;
    }
    Some(accolade.to_string())
}

/// A text-fallback name may be replaced only while no boundary signal has
/// been seen yet. An arm, a start screen, a primed or held reset, a recorded
/// outcome, or a hint of a different result closes the window. A hint of
/// the same result, or no hint, still lets the next game's accolade
/// through: the checks cannot tell those apart from this session's own
/// end screen.
fn text_fallback_open_for_accolade(input: &PollInput<'_>) -> bool {
    if input.text_fallback_locked
        || input.pending_boundary
        || input.start_screen.is_some()
        || input.deferred
        || input.reset_streak > 0
        || input.outcome.is_decided()
    {
        return false;
    }
    if let (Some(hint), Some(signal)) = (
        input
            .result
            .filter(|mark| !mark.confirmed && mark.outcome.is_decided()),
        input.signal.filter(|outcome| outcome.is_decided()),
    ) && hint.outcome != signal
    {
        return false;
    }
    true
}

/// Compare two map reads. Empty and `unknown` are not names, so a missing
/// accolade is [`MapRelation::Absent`] rather than a different map.
fn map_relation(session: Option<&str>, accolade: Option<&str>) -> MapRelation {
    match (named_map(session), named_map(accolade)) {
        (Some(session), Some(accolade)) if session.eq_ignore_ascii_case(accolade) => {
            MapRelation::Matches
        }
        (Some(_), Some(_)) => MapRelation::Differs,
        _ => MapRelation::Absent,
    }
}

/// The session map [`decide_poll`] is allowed to compare. A text fallback
/// is absent.
fn trusted_session_map<'a>(input: &'a PollInput<'a>) -> Option<&'a str> {
    input.map.filter(|_| input.map_trusted)
}

/// What this poll tick does to the open session. The rule order is
/// [`transition`].
pub fn decide_poll(input: &PollInput<'_>) -> PollDecision {
    let mut state = BoundaryState::new(input.map.map(str::to_string));
    state.outcome = input.outcome;
    state.result = input.result;
    state.pending_boundary = input.pending_boundary;
    state.awaiting_first_board = input.awaiting_first_board;
    state.reset_streak = input.reset_streak;
    state.hero = input.hero.map(str::to_string);
    if input.has_board {
        state.gate = Some(GateState::default());
    }
    if input.deferred {
        state.deferred = Some(Counters {
            elims: 0,
            assists: 0,
            deaths: 0,
            damage: 0,
            healing: 0,
            mitigation: 0,
        });
    }

    let obs = if let Some(screen) = input.start_screen.as_ref() {
        Obs::StartScreen {
            screen,
            block_map_vote: input.block_map_vote,
        }
    } else if let Some(signal) = input.signal.filter(|outcome| outcome.is_decided()) {
        let session_map = trusted_session_map(input);
        let relation = map_relation(session_map, input.accolade_map);
        if relation == MapRelation::Differs && input.signal_confirmed {
            Obs::EndScreenDifferentMap {
                outcome: signal,
                map: named_map(input.accolade_map).unwrap_or(""),
            }
        } else if input.signal_confirmed {
            Obs::ConfirmedWord { outcome: signal }
        } else {
            Obs::UnconfirmedWord {
                outcome: signal,
                relation,
            }
        }
    } else {
        let adopt = adopt_map(input);
        return if let Some(map) = adopt {
            PollDecision::Update(UpdateCurrent {
                record_outcome: None,
                adopt_map: Some(map),
                result: input.result,
                arm_pending: false,
                arm_reset: false,
                clear_awaiting: false,
            })
        } else {
            PollDecision::Keep
        };
    };

    let decided = transition(&state, &obs);
    match decided.effect {
        Effect::Ignore => {
            // An unconfirmed word that must not replace the hint. This is
            // [`PollDecision::Keep`], reached through [`transition`], so the
            // next game's word is not sealed onto this session.
            if let Obs::UnconfirmedWord { relation, .. } = &obs
                && !unconfirmed_replaces(input.pending_boundary, *relation)
            {
                return PollDecision::Keep;
            }
            if let Some(map) = adopt_map(input) {
                return PollDecision::Update(UpdateCurrent {
                    record_outcome: None,
                    adopt_map: Some(map),
                    result: input.result,
                    arm_pending: false,
                    arm_reset: false,
                    clear_awaiting: false,
                });
            }
            let kept = state
                .outcome
                .is_decided()
                .then_some(state.outcome)
                .or_else(|| {
                    state
                        .result
                        .filter(|mark| mark.outcome.is_decided())
                        .map(|mark| mark.outcome)
                });
            if let Some(signal) = input.signal.filter(|outcome| outcome.is_decided())
                && let Some(kept) = kept
                && kept != signal
            {
                PollDecision::IgnoreContradictory {
                    kept,
                    ignored: signal,
                }
            } else {
                PollDecision::Keep
            }
        }
        Effect::Seal { outcome } => PollDecision::Update(UpdateCurrent {
            record_outcome: Some(outcome),
            adopt_map: adopt_map(input),
            result: Some(confirmed_mark(input, outcome)),
            arm_pending: false,
            arm_reset: false,
            clear_awaiting: true,
        }),
        Effect::RememberHint { outcome } => {
            let seen_at = input
                .result
                .filter(|mark| mark.outcome == outcome)
                .map(|mark| mark.seen_at)
                .unwrap_or(input.now);
            let result = ResultMark {
                outcome,
                confirmed: false,
                seen_at,
            };
            if input.result == Some(result) && !input.awaiting_first_board {
                PollDecision::Keep
            } else {
                PollDecision::Update(UpdateCurrent {
                    record_outcome: None,
                    adopt_map: adopt_map(input),
                    result: Some(result),
                    arm_pending: false,
                    arm_reset: false,
                    clear_awaiting: true,
                })
            }
        }
        Effect::ArmPending => PollDecision::Update(UpdateCurrent {
            record_outcome: None,
            adopt_map: None,
            result: input.result,
            arm_pending: true,
            arm_reset: false,
            clear_awaiting: false,
        }),
        Effect::PrimeReset => PollDecision::Update(UpdateCurrent {
            record_outcome: None,
            adopt_map: None,
            result: input.result,
            arm_pending: false,
            arm_reset: true,
            clear_awaiting: false,
        }),
        Effect::Split {
            reason,
            seal,
            new_outcome,
        } => {
            let (reason, candidates) = if matches!(reason, CloseReason::StatReset) {
                (reason, Vec::new())
            } else if let Some(screen) = input.start_screen.as_ref() {
                screen_reason(screen)
            } else {
                (reason, Vec::new())
            };
            let new_map = match &obs {
                Obs::EndScreenDifferentMap { map, .. } => named_map(Some(map)).map(str::to_string),
                _ => None,
            };
            PollDecision::Open(OpenNew {
                reason,
                seal_outcome: seal,
                new_outcome,
                new_map,
                candidates,
                new_result: None,
            })
        }
        Effect::Append | Effect::ClearHintAndAppend | Effect::CountProgress | Effect::Defer => {
            PollDecision::Keep
        }
    }
}

fn confirmed_mark(input: &PollInput<'_>, outcome: MatchOutcome) -> ResultMark {
    let seen_at = input
        .result
        .filter(|mark| mark.outcome == outcome)
        .map(|mark| mark.seen_at)
        .unwrap_or(input.now);
    ResultMark {
        outcome,
        confirmed: true,
        seen_at,
    }
}
