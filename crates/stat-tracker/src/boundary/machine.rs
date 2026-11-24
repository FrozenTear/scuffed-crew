//! [`BoundaryState`], [`phase_of`] and the one [`transition`] function with
//! its per-input effects.

use std::time::Instant;

use super::types::{
    BoardObs, CloseReason, Effect, MapRelation, Obs, Phase, ResultMark, StartScreen, Transition,
    screen_reason,
};
use crate::capture_gate::{Counters, GateState};
use crate::model::MatchOutcome;

/// Outcome to write when a session closes, if it does not already have one.
///
/// An idle close passes no seal. A stored result is left as it is.
pub fn outcome_sealed_on_close(
    stored: MatchOutcome,
    explicit_seal: Option<MatchOutcome>,
) -> Option<MatchOutcome> {
    if stored.is_decided() {
        None
    } else {
        explicit_seal.filter(|outcome| outcome.is_decided())
    }
}

/// Boundary fields the poller and the Tab path share.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundaryState {
    pub map: Option<String>,
    pub outcome: MatchOutcome,
    pub outcome_at: Option<Instant>,
    pub result: Option<ResultMark>,
    pub reset_streak: u32,
    pub reset_baseline: Option<GateState>,
    /// Player row that owns the baseline. A later plausible row replaces it.
    pub baseline_row: Option<u32>,
    /// Last stored scoreboard. A deferred reset board does not move this.
    pub last_board_at: Option<Instant>,
    /// Hero select after a board-followed hint. The first reset signal.
    pub pending_boundary: bool,
    /// Progressed boards accepted since the current hint. The second one drops it.
    pub progressed_boards: u8,
    /// When the reset baseline was taken. Rate checks measure from here.
    pub baseline_at: Option<Instant>,
    /// Opened by a start screen; the first board has not been accepted.
    pub awaiting_first_board: bool,
    pub hero: Option<String>,
    /// First fresh-match board, held off the current session.
    pub deferred: Option<Counters>,
    pub gate: Option<GateState>,
    /// A different result replaced the hint. Stays set so the confirming
    /// read cannot relabel a text-fallback map.
    pub text_fallback_locked: bool,
}

impl BoundaryState {
    pub fn new(map: Option<String>) -> Self {
        Self {
            map,
            outcome: MatchOutcome::Unknown,
            outcome_at: None,
            result: None,
            reset_streak: 0,
            reset_baseline: None,
            baseline_row: None,
            last_board_at: None,
            pending_boundary: false,
            progressed_boards: 0,
            baseline_at: None,
            awaiting_first_board: false,
            hero: None,
            deferred: None,
            gate: None,
            text_fallback_locked: false,
        }
    }
}

pub fn phase_of(state: &BoundaryState) -> Phase {
    if state.outcome.is_decided() {
        return Phase::PostMatch;
    }
    if state.awaiting_first_board {
        return Phase::NewGameStarting;
    }
    let hint = state
        .result
        .is_some_and(|mark| !mark.confirmed && mark.outcome.is_decided());
    if hint {
        return Phase::PostResultStreak;
    }
    if state.gate.is_some() || state.map.is_some() || state.hero.is_some() {
        return Phase::LiveMatch;
    }
    Phase::Idle
}

/// The one transition. Poll and capture both call this.
pub fn transition(state: &BoundaryState, obs: &Obs<'_>) -> Transition {
    let phase = phase_of(state);
    let effect = match obs {
        Obs::ConfirmedWord { outcome } => confirmed_word(phase, *outcome),
        Obs::UnconfirmedWord { outcome, relation } => {
            unconfirmed_word(state, phase, *outcome, *relation)
        }
        Obs::StartScreen {
            screen,
            block_map_vote,
        } => start_screen(state, phase, screen, *block_map_vote),
        Obs::Board(board) => board_effect(state, phase, board),
        Obs::Gap { frame_outcome } => gap_effect(state, phase, *frame_outcome),
        Obs::EndScreenDifferentMap { outcome, .. } => end_screen(state, phase, *outcome),
        // Same close as a gap: seal the hint, keep this frame's header.
        Obs::HintedDifferentMap { frame_outcome } => gap_effect(state, phase, *frame_outcome),
    };
    Transition { effect, phase }
}

fn sealable_hint(state: &BoundaryState) -> Option<MatchOutcome> {
    state
        .result
        .filter(|mark| !mark.confirmed && mark.outcome.is_decided())
        .map(|mark| mark.outcome)
}

fn confirmed_word(phase: Phase, outcome: MatchOutcome) -> Effect {
    match phase {
        Phase::PostMatch => Effect::Ignore,
        Phase::Idle | Phase::LiveMatch | Phase::PostResultStreak | Phase::NewGameStarting => {
            Effect::Seal { outcome }
        }
    }
}

/// An unconfirmed word replaces the hint when the maps match, or when the
/// boundary is not armed and either side has no trusted name. A different
/// map never replaces it.
pub(super) fn unconfirmed_replaces(pending_boundary: bool, relation: MapRelation) -> bool {
    match relation {
        MapRelation::Matches => true,
        MapRelation::Differs => false,
        MapRelation::Absent => !pending_boundary,
    }
}

fn unconfirmed_word(
    state: &BoundaryState,
    phase: Phase,
    outcome: MatchOutcome,
    relation: MapRelation,
) -> Effect {
    if !unconfirmed_replaces(state.pending_boundary, relation) {
        return Effect::Ignore;
    }
    match phase {
        Phase::PostMatch => Effect::Ignore,
        Phase::Idle | Phase::LiveMatch | Phase::PostResultStreak | Phase::NewGameStarting => {
            Effect::RememberHint { outcome }
        }
    }
}

fn start_screen(
    state: &BoundaryState,
    phase: Phase,
    screen: &StartScreen,
    block_map_vote: bool,
) -> Effect {
    let split_unsealed = |screen: &StartScreen| Effect::Split {
        reason: screen_reason(screen).0,
        seal: None,
        new_outcome: MatchOutcome::Unknown,
    };
    let split_sealed = |screen: &StartScreen, state: &BoundaryState| Effect::Split {
        reason: screen_reason(screen).0,
        seal: sealable_hint(state),
        new_outcome: MatchOutcome::Unknown,
    };
    match phase {
        Phase::PostMatch => match screen {
            StartScreen::MapVote { .. } if block_map_vote => Effect::Ignore,
            _ => split_unsealed(screen),
        },
        Phase::PostResultStreak => match screen {
            StartScreen::MapVote { .. } if block_map_vote => Effect::Ignore,
            StartScreen::MapVote { .. } | StartScreen::HeroBan => split_sealed(screen, state),
            StartScreen::HeroSelect if state.gate.is_some() => Effect::ArmPending,
            StartScreen::HeroSelect => split_sealed(screen, state),
        },
        Phase::Idle | Phase::LiveMatch => match screen {
            StartScreen::MapVote { .. } if block_map_vote => Effect::Ignore,
            StartScreen::MapVote { .. } => split_unsealed(screen),
            // A ban after one deferred misread is not the second signal.
            StartScreen::HeroBan if state.deferred.is_some() => Effect::Ignore,
            StartScreen::HeroBan => split_unsealed(screen),
            // Select after a deferral does not split. Select before a fresh
            // board is the first signal.
            StartScreen::HeroSelect if state.deferred.is_some() => Effect::Ignore,
            StartScreen::HeroSelect => Effect::PrimeReset,
        },
        // A stable start screen repeats every tick. The debounce blocks a
        // repeated vote or ban. A hero select before the first Tab is a swap
        // on the game this screen just opened, including after the debounce.
        Phase::NewGameStarting => match screen {
            StartScreen::HeroSelect => Effect::Ignore,
            StartScreen::MapVote { .. } | StartScreen::HeroBan if block_map_vote => Effect::Ignore,
            _ => split_unsealed(screen),
        },
    }
}

fn gap_effect(state: &BoundaryState, phase: Phase, frame_outcome: MatchOutcome) -> Effect {
    match phase {
        // Idle has no board to regress from. NewGameStarting's first Tab
        // belongs to the session the start screen opened.
        Phase::Idle | Phase::NewGameStarting | Phase::PostMatch => Effect::Ignore,
        Phase::LiveMatch | Phase::PostResultStreak => Effect::Split {
            reason: CloseReason::StatRegression,
            seal: sealable_hint(state),
            new_outcome: if frame_outcome.is_decided() {
                frame_outcome
            } else {
                MatchOutcome::Unknown
            },
        },
    }
}

fn end_screen(state: &BoundaryState, phase: Phase, outcome: MatchOutcome) -> Effect {
    match phase {
        Phase::PostMatch | Phase::Idle | Phase::NewGameStarting => Effect::Ignore,
        // Both names are trusted and they differ. Armed or not, this word
        // belongs to the next session. A word with no trusted name never
        // reaches here: it is a confirmed word and seals here.
        Phase::LiveMatch | Phase::PostResultStreak => Effect::Split {
            reason: CloseReason::EndScreenMap,
            seal: sealable_hint(state),
            new_outcome: outcome,
        },
    }
}

fn board_effect(state: &BoundaryState, _phase: Phase, board: &BoardObs) -> Effect {
    if board.fresh_reset {
        // A trusted board splits on one earlier signal. A suspect row needs
        // two, or a trusted fresh board already held: it never commits the
        // split by itself (`plan_capture_held`).
        let commits = if board.suspect_row {
            state.reset_streak >= 2 || state.deferred.is_some()
        } else {
            state.reset_streak >= 1 || state.pending_boundary
        };
        if commits {
            return Effect::Split {
                reason: CloseReason::StatReset,
                seal: sealable_hint(state),
                new_outcome: MatchOutcome::Unknown,
            };
        }
        return Effect::Defer;
    }
    let hint_open = sealable_hint(state).is_some() && !state.outcome.is_decided();
    if hint_open && board.progressed && !board.implausible && !board.frame_outcome.is_decided() {
        // An arm is already the first "the match may be over" signal, so one
        // progressed board drops the hint. Otherwise the first progressed
        // board keeps it and the second drops it.
        if state.pending_boundary || state.progressed_boards >= 1 {
            return Effect::ClearHintAndAppend;
        }
        return Effect::CountProgress;
    }
    Effect::Append
}
