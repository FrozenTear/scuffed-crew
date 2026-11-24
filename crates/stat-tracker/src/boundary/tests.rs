//! Shared fixtures for the `boundary` tests: the clock and counter helpers,
//! the state builders and `expect_transition`.

use super::*;

pub(super) fn t0() -> Instant {
    Instant::now() + Duration::from_secs(3_600)
}

pub(super) fn counters(e: u32, a: u32, d: u32, dmg: u32, hlg: u32, mit: u32) -> Counters {
    Counters {
        elims: e,
        assists: a,
        deaths: d,
        damage: dmg,
        healing: hlg,
        mitigation: mit,
    }
}

pub(super) fn gate(c: Counters) -> GateState {
    GateState {
        accepted: c,
        last_raw: c,
        ..GateState::default()
    }
}

pub(super) const CLEAN: [bool; GATE_COLS] = [false; GATE_COLS];

fn effect_tag(effect: &Effect) -> String {
    match effect {
        Effect::Ignore => "ignore".into(),
        Effect::Append => "append".into(),
        Effect::Seal { outcome } => format!("seal:{outcome}"),
        Effect::RememberHint { outcome } => format!("hint:{outcome}"),
        Effect::ClearHintAndAppend => "clear".into(),
        Effect::Defer => "defer".into(),
        Effect::ArmPending => "arm".into(),
        Effect::PrimeReset => "prime".into(),
        Effect::CountProgress => "progress".into(),
        Effect::Split {
            new_outcome, seal, ..
        } => format!(
            "split:{new_outcome}:{}",
            seal.map(|outcome| outcome.to_string())
                .unwrap_or_else(|| "-".into())
        ),
    }
}

pub(super) fn idle_state() -> BoundaryState {
    BoundaryState::new(None)
}

pub(super) fn live_state() -> BoundaryState {
    let mut state = BoundaryState::new(Some("Busan".into()));
    state.gate = Some(gate(counters(14, 22, 6, 2400, 9800, 400)));
    state.hero = Some("Zenyatta".into());
    state.baseline_row = Some(2);
    state
}

pub(super) fn streak_state() -> BoundaryState {
    let mut state = live_state();
    state.result = Some(ResultMark {
        outcome: MatchOutcome::Defeat,
        confirmed: false,
        seen_at: t0(),
    });
    state
}

pub(super) fn hint_only_state() -> BoundaryState {
    let mut state = BoundaryState::new(Some("Busan".into()));
    state.result = Some(ResultMark {
        outcome: MatchOutcome::Defeat,
        confirmed: false,
        seen_at: t0(),
    });
    state
}

pub(super) fn post_state() -> BoundaryState {
    let mut state = live_state();
    state.outcome = MatchOutcome::Defeat;
    state.outcome_at = Some(t0());
    state
}

pub(super) fn starting_state() -> BoundaryState {
    let mut state = BoundaryState::new(None);
    state.awaiting_first_board = true;
    state
}

pub(super) fn expect_transition(label: &str, state: &BoundaryState, obs: &Obs<'_>, tag: &str) {
    let got = effect_tag(&transition(state, obs).effect);
    assert_eq!(got, tag, "{label}");
}

pub(super) fn board(fresh_reset: bool, progressed: bool, frame: MatchOutcome) -> BoardObs {
    BoardObs {
        fresh_reset,
        progressed,
        implausible: false,
        frame_outcome: frame,
        suspect_row: false,
    }
}
