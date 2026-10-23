//! Tests of map adoption on the poll path: accolade maps, trusted vs
//! text-fallback maps, and the text-fallback lock.

use super::tests::{CLEAN, counters, gate, streak_state, t0};
use super::*;

#[test]
fn first_accolade_fills_an_empty_map_and_does_not_split() {
    let now = t0();
    let decision = decide_poll(&PollInput {
        outcome: MatchOutcome::Unknown,
        result: None,
        pending_boundary: false,
        has_board: false,
        reset_streak: 0,
        block_map_vote: false,
        awaiting_first_board: false,
        map: None,
        map_trusted: false,
        hero: None,
        signal: Some(MatchOutcome::Defeat),
        signal_confirmed: true,
        accolade_map: Some("Busan"),
        start_screen: None,
        now,

        deferred: false,
        text_fallback_locked: false,
    });
    match decision {
        PollDecision::Update(u) => {
            assert_eq!(u.record_outcome, Some(MatchOutcome::Defeat));
            assert_eq!(u.adopt_map.as_deref(), Some("Busan"));
        }
        other => panic!("expected update, got {other:?}"),
    }
}

#[test]
fn post_match_adopts_an_accolade_map_without_changing_the_outcome() {
    let now = t0();
    let mut state = BoundaryState::new(None);
    state.outcome = MatchOutcome::Defeat;
    state.outcome_at = Some(now);
    let decision = decide_poll(&PollInput {
        outcome: MatchOutcome::Defeat,
        result: None,
        pending_boundary: false,
        awaiting_first_board: false,
        has_board: true,
        reset_streak: 0,
        map: None,
        map_trusted: false,
        hero: None,
        signal: Some(MatchOutcome::Victory),
        signal_confirmed: false,
        accolade_map: Some("Busan"),
        start_screen: None,
        block_map_vote: false,
        now,

        deferred: false,
        text_fallback_locked: false,
    });
    let commit = commit_poll(&mut state, decision, now);
    assert!(commit.recorded_outcome.is_none());
    assert_eq!(commit.adopted_map.as_deref(), Some("Busan"));
    assert_eq!(state.outcome, MatchOutcome::Defeat);
    assert_eq!(state.map.as_deref(), Some("Busan"));
}

fn hinted_map_plan(source: Option<MapSource>, age_secs: u64, incoming: &str) -> CapturePlan {
    let prev = gate(counters(14, 22, 6, 2400, 9800, 400));
    plan_capture(&CapturePlanInput {
        prev_gate: Some(&prev),
        baseline: Some(&prev),
        streak: 0,
        cur: counters(16, 23, 6, 2600, 9900, 500),
        suspect: CLEAN,
        create_session: false,
        suppress_same_unfinished: false,
        age: Some(Duration::from_secs(age_secs)),
        min_gap: Duration::from_secs(120),
        classic_regressed: false,
        row_counts: true,
        row_id: Some(2),
        baseline_row: Some(2),
        confirmed_end: false,
        inherited_outcome: MatchOutcome::Unknown,
        frame_outcome: MatchOutcome::Unknown,
        session_hero: Some("Zenyatta"),
        hint: Some(MatchOutcome::Defeat),
        pending_boundary: false,
        awaiting_first_board: false,
        baseline_age: Some(Duration::from_secs(age_secs)),
        progressed_boards: 0,
        session_map: Some("Busan"),
        session_map_source: source,
        incoming_map: Some(incoming),
    })
}

#[test]
fn a_trusted_map_splits_after_the_gap_and_a_text_fallback_does_not() {
    assert!(
        hinted_map_plan(Some(MapSource::TopBar), 150, "Junkertown").split,
        "a top-bar map and a later different Tab close the hinted session"
    );
    assert!(
        hinted_map_plan(Some(MapSource::Accolade), 150, "Junkertown").split,
        "an accolade map is the same kind of read"
    );
    assert!(
        !hinted_map_plan(Some(MapSource::TextFallback), 150, "Junkertown").split,
        "a full-board text fallback is not the match's map"
    );
    assert!(
        !hinted_map_plan(Some(MapSource::TopBar), 30, "Junkertown").split,
        "inside the gap a different top-bar read stays this match"
    );
    assert!(
        !hinted_map_plan(Some(MapSource::TopBar), 150, "Busan").split,
        "the same map after the gap is still this match"
    );
}

#[test]
fn an_armed_unconfirmed_word_on_a_different_map_keeps_the_hint() {
    let now = t0();
    let mut state = streak_state();
    state.pending_boundary = true;
    let decision = decide_poll(&PollInput {
        outcome: MatchOutcome::Unknown,
        result: state.result,
        pending_boundary: true,
        awaiting_first_board: false,
        has_board: true,
        reset_streak: 0,
        map: Some("Busan"),
        map_trusted: true,
        hero: Some("Zenyatta"),
        signal: Some(MatchOutcome::Victory),
        signal_confirmed: false,
        accolade_map: Some("Junkertown"),
        start_screen: None,
        block_map_vote: false,
        deferred: false,
        text_fallback_locked: false,
        now,
    });
    assert_eq!(decision, PollDecision::Keep);
    let confirmed = decide_poll(&PollInput {
        outcome: MatchOutcome::Unknown,
        result: state.result,
        pending_boundary: true,
        awaiting_first_board: false,
        has_board: true,
        reset_streak: 0,
        map: Some("Busan"),
        map_trusted: true,
        hero: Some("Zenyatta"),
        signal: Some(MatchOutcome::Victory),
        signal_confirmed: true,
        accolade_map: Some("Junkertown"),
        start_screen: None,
        block_map_vote: false,
        deferred: false,
        text_fallback_locked: false,
        now,
    });
    match confirmed {
        PollDecision::Open(open) => {
            assert_eq!(open.seal_outcome, Some(MatchOutcome::Defeat));
            assert_eq!(open.new_outcome, MatchOutcome::Victory);
            assert_eq!(open.new_map.as_deref(), Some("Junkertown"));
        }
        other => panic!("expected the confirming read to split, got {other:?}"),
    }
}

fn armed_busan_word(accolade: Option<&str>, confirmed: bool) -> PollDecision {
    let now = t0();
    decide_poll(&PollInput {
        outcome: MatchOutcome::Unknown,
        result: Some(ResultMark {
            outcome: MatchOutcome::Defeat,
            confirmed: false,
            seen_at: now,
        }),
        pending_boundary: true,
        awaiting_first_board: false,
        has_board: true,
        reset_streak: 0,
        map: Some("Busan"),
        map_trusted: true,
        hero: Some("Zenyatta"),
        signal: Some(MatchOutcome::Victory),
        signal_confirmed: confirmed,
        accolade_map: accolade,
        start_screen: None,
        block_map_vote: false,
        deferred: false,
        text_fallback_locked: false,
        now,
    })
}

#[test]
fn an_armed_word_with_no_map_keeps_the_hint() {
    assert_eq!(
        armed_busan_word(None, false),
        PollDecision::Keep,
        "an end title before the accolade must not replace A's hint"
    );
    assert_eq!(
        armed_busan_word(Some("unknown"), false),
        PollDecision::Keep,
        "an unnamed accolade is not a different map and does not replace the hint while armed"
    );
}

#[test]
fn an_unarmed_different_map_word_does_not_replace_the_hint() {
    let now = t0();
    let hint = Some(ResultMark {
        outcome: MatchOutcome::Defeat,
        confirmed: false,
        seen_at: now,
    });
    let word = |accolade: Option<&str>| {
        decide_poll(&PollInput {
            outcome: MatchOutcome::Unknown,
            result: hint,
            pending_boundary: false,
            awaiting_first_board: false,
            has_board: true,
            reset_streak: 0,
            map: Some("Busan"),
            map_trusted: true,
            hero: Some("Zenyatta"),
            signal: Some(MatchOutcome::Victory),
            signal_confirmed: false,
            accolade_map: accolade,
            start_screen: None,
            block_map_vote: false,
            deferred: false,
            text_fallback_locked: false,
            now,
        })
    };
    assert_eq!(
        word(Some("Junkertown")),
        PollDecision::Keep,
        "a different map does not replace the hint, armed or not"
    );
    match word(Some("Busan")) {
        PollDecision::Update(update) => {
            assert_eq!(
                update.result.map(|mark| mark.outcome),
                Some(MatchOutcome::Victory),
                "the same map still replaces the hint"
            );
        }
        other => panic!("expected the same map to replace the hint, got {other:?}"),
    }
    match word(None) {
        PollDecision::Update(update) => {
            assert_eq!(
                update.result.map(|mark| mark.outcome),
                Some(MatchOutcome::Victory),
                "an unarmed word with no map still replaces the hint"
            );
        }
        other => panic!("expected a mapless word to replace the hint, got {other:?}"),
    }
}

#[test]
fn an_untrusted_text_map_is_absent_on_the_poll_path() {
    let now = t0();
    let mut state = BoundaryState::new(Some("Dorado".into()));
    state.result = Some(ResultMark {
        outcome: MatchOutcome::Defeat,
        confirmed: false,
        seen_at: now,
    });
    let decision = decide_poll(&PollInput {
        outcome: MatchOutcome::Unknown,
        result: state.result,
        pending_boundary: false,
        awaiting_first_board: false,
        has_board: true,
        reset_streak: 0,
        map: Some("Dorado"),
        map_trusted: false,
        hero: None,
        signal: Some(MatchOutcome::Victory),
        signal_confirmed: true,
        accolade_map: Some("Junkertown"),
        start_screen: None,
        block_map_vote: false,
        deferred: false,
        text_fallback_locked: false,
        now,
    });
    match &decision {
        PollDecision::Open(_) => {
            panic!("an untrusted map must not split on a different accolade")
        }
        PollDecision::Update(update) => {
            assert_eq!(update.record_outcome, Some(MatchOutcome::Victory));
            assert!(
                update.adopt_map.is_none(),
                "a different result is the next game, so the accolade must not relabel the text map"
            );
        }
        other => panic!("expected the word to seal without a new map, got {other:?}"),
    }
    let commit = commit_poll(&mut state, decision, now);
    assert!(commit.adopted_map.is_none());
    assert_eq!(state.map.as_deref(), Some("Dorado"));
    assert_eq!(state.outcome, MatchOutcome::Victory);
}

#[test]
fn a_text_fallback_adopts_an_accolade_before_a_boundary_signal() {
    let now = t0();
    let open = decide_poll(&PollInput {
        outcome: MatchOutcome::Unknown,
        result: None,
        pending_boundary: false,
        awaiting_first_board: false,
        has_board: true,
        reset_streak: 0,
        map: Some("Dorado"),
        map_trusted: false,
        hero: None,
        signal: Some(MatchOutcome::Defeat),
        signal_confirmed: false,
        accolade_map: Some("Busan"),
        start_screen: None,
        block_map_vote: false,
        deferred: false,
        text_fallback_locked: false,
        now,
    });
    match open {
        PollDecision::Update(update) => {
            assert_eq!(update.adopt_map.as_deref(), Some("Busan"));
        }
        other => panic!("expected an open text map to adopt the accolade, got {other:?}"),
    }
    let armed = decide_poll(&PollInput {
        outcome: MatchOutcome::Unknown,
        result: None,
        pending_boundary: true,
        awaiting_first_board: false,
        has_board: true,
        reset_streak: 0,
        map: Some("Dorado"),
        map_trusted: false,
        hero: None,
        signal: Some(MatchOutcome::Defeat),
        signal_confirmed: false,
        accolade_map: Some("Busan"),
        start_screen: None,
        block_map_vote: false,
        deferred: false,
        text_fallback_locked: false,
        now,
    });
    match &armed {
        PollDecision::Keep => {}
        PollDecision::Update(update) if update.adopt_map.is_none() => {}
        other => panic!("an armed boundary must not relabel the text map, got {other:?}"),
    }
    let finished = decide_poll(&PollInput {
        outcome: MatchOutcome::Victory,
        result: None,
        pending_boundary: false,
        awaiting_first_board: false,
        has_board: true,
        reset_streak: 0,
        map: Some("Dorado"),
        map_trusted: false,
        hero: None,
        signal: Some(MatchOutcome::Defeat),
        signal_confirmed: false,
        accolade_map: Some("Junkertown"),
        start_screen: None,
        block_map_vote: false,
        deferred: false,
        text_fallback_locked: false,
        now,
    });
    match finished {
        PollDecision::Keep | PollDecision::IgnoreContradictory { .. } => {}
        PollDecision::Update(update) => assert!(update.adopt_map.is_none()),
        other => panic!("a recorded outcome must not take the next map, got {other:?}"),
    }
}

#[test]
fn a_replaced_hint_keeps_the_text_fallback_locked() {
    let now = t0();
    let mut state = BoundaryState::new(Some("Busan".into()));
    state.result = Some(ResultMark {
        outcome: MatchOutcome::Defeat,
        confirmed: false,
        seen_at: now,
    });
    state.gate = Some(gate(counters(14, 22, 6, 2400, 9800, 400)));
    fn input<'a>(state: &'a BoundaryState, confirmed: bool, now: Instant) -> PollInput<'a> {
        PollInput {
            outcome: state.outcome,
            result: state.result,
            pending_boundary: state.pending_boundary,
            awaiting_first_board: false,
            has_board: state.gate.is_some(),
            reset_streak: state.reset_streak,
            map: state.map.as_deref(),
            map_trusted: false,
            hero: None,
            signal: Some(MatchOutcome::Victory),
            signal_confirmed: confirmed,
            accolade_map: Some("Junkertown"),
            start_screen: None,
            block_map_vote: false,
            deferred: false,
            text_fallback_locked: state.text_fallback_locked,
            now,
        }
    }
    let first = decide_poll(&input(&state, false, now));
    let commit = commit_poll(&mut state, first, now);
    assert!(commit.adopted_map.is_none());
    assert!(
        state.text_fallback_locked,
        "replacing the hint must keep the text-fallback window shut"
    );
    assert_eq!(
        state.result.map(|mark| mark.outcome),
        Some(MatchOutcome::Victory)
    );
    let second = decide_poll(&input(&state, true, now));
    match &second {
        PollDecision::Update(update) => {
            assert_eq!(update.record_outcome, Some(MatchOutcome::Victory));
            assert!(
                update.adopt_map.is_none(),
                "the confirming read must not relabel the text map"
            );
        }
        other => {
            panic!("expected the confirming read to seal without a new map, got {other:?}")
        }
    }
    commit_poll(&mut state, second, now);
    assert_eq!(state.map.as_deref(), Some("Busan"));
    assert_eq!(state.outcome, MatchOutcome::Victory);
}

#[test]
fn an_arm_or_a_primed_reset_keeps_a_same_result_text_map() {
    let now = t0();
    let hint = ResultMark {
        outcome: MatchOutcome::Victory,
        confirmed: false,
        seen_at: now,
    };
    // A same-totals refresh can clear the streak and leave the arm.
    // The confirmed word has to reach `adopt_map`: an unconfirmed
    // absent word returns Keep before that.
    let armed = PollInput {
        outcome: MatchOutcome::Unknown,
        result: Some(hint),
        pending_boundary: true,
        awaiting_first_board: false,
        has_board: true,
        reset_streak: 0,
        map: Some("Dorado"),
        map_trusted: false,
        hero: None,
        signal: Some(MatchOutcome::Victory),
        signal_confirmed: true,
        accolade_map: Some("Busan"),
        start_screen: None,
        block_map_vote: false,
        deferred: false,
        text_fallback_locked: false,
        now,
    };
    match decide_poll(&armed) {
        PollDecision::Update(update) => {
            assert_eq!(update.record_outcome, Some(MatchOutcome::Victory));
            assert!(
                update.adopt_map.is_none(),
                "an arm with no streak still blocks the accolade"
            );
        }
        other => panic!("expected a seal without a new map, got {other:?}"),
    }
    let primed = PollInput {
        pending_boundary: false,
        reset_streak: 1,
        ..armed
    };
    match decide_poll(&primed) {
        PollDecision::Update(update) => {
            assert_eq!(update.record_outcome, Some(MatchOutcome::Victory));
            assert!(
                update.adopt_map.is_none(),
                "a primed or held reset with no arm still blocks the accolade"
            );
        }
        other => panic!("expected a seal without a new map, got {other:?}"),
    }
}

#[test]
fn clearing_the_hint_clears_the_text_fallback_lock() {
    let now = t0();
    let mut state = BoundaryState::new(Some("Dorado".into()));
    state.text_fallback_locked = true;
    state.result = Some(ResultMark {
        outcome: MatchOutcome::Victory,
        confirmed: false,
        seen_at: now,
    });
    state.pending_boundary = true;
    state.gate = Some(gate(counters(14, 22, 6, 2400, 9800, 400)));
    let plan = CapturePlan {
        split: false,
        defer: false,
        ignore_row: false,
        skip_store: false,
        clear_hint: true,
        reset_streak: 0,
        reset_baseline: None,
        baseline_row: Some(2),
        refresh_baseline: true,
        count_progress: false,
        stored_outcome: MatchOutcome::Unknown,
        deferred_counters: None,
        seal: None,
        close_reason: None,
    };
    note_accepted_capture(
        &mut state,
        &plan,
        gate(counters(16, 24, 6, 2800, 10200, 500)),
        now,
    );
    assert!(state.result.is_none(), "progressed boards clear the hint");
    assert!(!state.pending_boundary);
    assert!(
        !state.text_fallback_locked,
        "clearing the hint on a capture clears the text-fallback lock"
    );
}

fn adopted_map(decision: &PollDecision) -> Option<&str> {
    match decision {
        PollDecision::Update(update) => update.adopt_map.as_deref(),
        PollDecision::Open(open) => open.new_map.as_deref(),
        PollDecision::Keep | PollDecision::IgnoreContradictory { .. } => None,
    }
}

#[test]
fn a_blocked_start_screen_or_a_held_board_keeps_the_text_map() {
    let now = t0();
    let screen = StartScreen::MapVote {
        candidates: vec!["Busan".into(), "Ilios".into()],
    };
    let blocked = PollInput {
        outcome: MatchOutcome::Unknown,
        result: None,
        pending_boundary: false,
        awaiting_first_board: false,
        has_board: true,
        reset_streak: 0,
        map: Some("Dorado"),
        map_trusted: false,
        hero: None,
        signal: Some(MatchOutcome::Victory),
        signal_confirmed: false,
        accolade_map: Some("Busan"),
        start_screen: Some(screen),
        block_map_vote: true,
        deferred: false,
        text_fallback_locked: false,
        now,
    };
    assert!(
        adopted_map(&decide_poll(&blocked)).is_none(),
        "a blocked start screen on an unconfirmed word must not adopt the accolade, got {:?}",
        decide_poll(&blocked)
    );
    let held = PollInput {
        start_screen: None,
        block_map_vote: false,
        deferred: true,
        ..blocked
    };
    match decide_poll(&held) {
        PollDecision::Update(update) => {
            assert!(
                update.adopt_map.is_none(),
                "a held board with no streak must not adopt the accolade"
            );
            assert_eq!(
                update.result.map(|mark| mark.outcome),
                Some(MatchOutcome::Victory),
                "the unconfirmed word is still recorded"
            );
        }
        other => panic!("expected the word to update without a new map, got {other:?}"),
    }
}
