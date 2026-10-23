//! Tests of the transition function: the `transition_table`, the hint and
//! arm rules, and the end-screen split log.

use super::tests::{
    board, counters, expect_transition, hint_only_state, idle_state, live_state, post_state,
    starting_state, streak_state, t0,
};
use super::*;

#[test]
fn an_end_screen_map_split_is_not_logged_as_a_stat_regression() {
    let end = Obs::EndScreenDifferentMap {
        outcome: MatchOutcome::Victory,
        map: "Ilios",
    };
    let mut armed = streak_state();
    armed.pending_boundary = true;
    for (label, state) in [
        ("live", live_state()),
        ("streak", streak_state()),
        ("armed", armed),
    ] {
        match transition(&state, &end).effect {
            Effect::Split { reason, seal, .. } => {
                assert_eq!(reason, CloseReason::EndScreenMap, "{label}");
                assert_eq!(reason.log(), "superseded by end-screen map", "{label}");
                assert_ne!(reason.log(), CloseReason::StatRegression.log());
                let sealed = outcome_sealed_on_close(state.outcome, seal);
                let open = OpenNew {
                    reason,
                    seal_outcome: seal,
                    new_outcome: MatchOutcome::Victory,
                    new_map: Some("Ilios".into()),
                    candidates: Vec::new(),
                    new_result: None,
                };
                let (armed, map) = end_screen_log_fields(state.pending_boundary, &open)
                    .expect("an end-screen split carries its accolade map");
                let line = format_end_screen_split(armed, sealed, &map);
                let hint = match sealed {
                    Some(outcome) => outcome.to_string(),
                    None => "none".to_string(),
                };
                assert_eq!(
                    line,
                    format!(
                        "armed={} sealed_hint={hint} accolade_map=Ilios",
                        state.pending_boundary
                    ),
                    "{label}"
                );
            }
            other => panic!("{label} end screen should split, got {other:?}"),
        }
    }
    match transition(
        &live_state(),
        &Obs::Gap {
            frame_outcome: MatchOutcome::Defeat,
        },
    )
    .effect
    {
        Effect::Split { reason, .. } => {
            assert_eq!(reason, CloseReason::StatRegression);
            assert_eq!(reason.log(), "superseded by stat regression");
        }
        other => panic!("a gap stays a stat regression, got {other:?}"),
    }
}

#[test]
fn idle_close_does_not_seal_a_provisional_word() {
    let hint = ResultMark {
        outcome: MatchOutcome::Defeat,
        confirmed: false,
        seen_at: t0(),
    };
    assert_eq!(
        outcome_sealed_on_close(MatchOutcome::Unknown, None),
        None,
        "an idle Tab passes no seal; the stray word must not become the outcome"
    );
    assert_eq!(
        outcome_sealed_on_close(MatchOutcome::Unknown, Some(hint.outcome)),
        Some(MatchOutcome::Defeat),
        "a start screen is the explicit second signal that may seal the hint"
    );
    assert_eq!(
        outcome_sealed_on_close(MatchOutcome::Victory, Some(MatchOutcome::Defeat)),
        None,
        "a stored outcome is not sealed a second time"
    );
}

#[test]
fn confirmed_grace_starts_at_the_confirming_tick() {
    let now = t0();
    let mut state = BoundaryState::new(Some("Busan".into()));
    state.result = Some(ResultMark {
        outcome: MatchOutcome::Defeat,
        confirmed: false,
        seen_at: now - Duration::from_secs(200),
    });
    let decision = decide_poll(&PollInput {
        outcome: MatchOutcome::Unknown,
        result: state.result,
        pending_boundary: false,
        has_board: false,
        reset_streak: 0,
        block_map_vote: false,
        awaiting_first_board: false,
        map: Some("Busan"),
        map_trusted: true,
        hero: None,
        signal: Some(MatchOutcome::Defeat),
        signal_confirmed: true,
        accolade_map: None,
        start_screen: None,
        now,

        deferred: false,
        text_fallback_locked: false,
    });
    let commit = commit_poll(&mut state, decision, now);
    assert_eq!(commit.recorded_outcome, Some(MatchOutcome::Defeat));
    assert_eq!(
        state.outcome_at,
        Some(now),
        "grace starts when the result is confirmed, not at the first sighting"
    );
}

#[test]
fn transition_table() {
    let hero = StartScreen::HeroSelect;
    let ban = StartScreen::HeroBan;
    let vote = StartScreen::MapVote {
        candidates: vec!["Junkertown".into(), "Ilios".into()],
    };
    let word = MatchOutcome::Defeat;
    let stay = board(false, false, MatchOutcome::Unknown);
    let progressed = board(false, true, MatchOutcome::Unknown);
    let header = board(false, true, MatchOutcome::Victory);
    let reset = board(true, false, MatchOutcome::Victory);
    let implausible = BoardObs {
        fresh_reset: false,
        progressed: false,
        implausible: true,
        frame_outcome: MatchOutcome::Unknown,
        suspect_row: false,
    };
    let gap = Obs::Gap {
        frame_outcome: MatchOutcome::Defeat,
    };
    let end = Obs::EndScreenDifferentMap {
        outcome: word,
        map: "Junkertown",
    };
    let confirmed = Obs::ConfirmedWord { outcome: word };
    let unconfirmed = Obs::UnconfirmedWord {
        outcome: MatchOutcome::Victory,
        relation: MapRelation::Absent,
    };
    let hero_select = Obs::StartScreen {
        screen: &hero,
        block_map_vote: false,
    };
    let hero_ban = Obs::StartScreen {
        screen: &ban,
        block_map_vote: false,
    };
    let map_vote = Obs::StartScreen {
        screen: &vote,
        block_map_vote: false,
    };
    let map_blocked = Obs::StartScreen {
        screen: &vote,
        block_map_vote: true,
    };

    // Idle and LiveMatch share the poll rows. LiveMatch is the one with
    // a mature board, so only that state defers a fresh-match read.
    let cases: &[(&str, BoundaryState, &Obs<'_>, &str)] = &[
        ("idle confirmed", idle_state(), &confirmed, "seal:defeat"),
        ("live confirmed", live_state(), &confirmed, "seal:defeat"),
        (
            "idle unconfirmed",
            idle_state(),
            &unconfirmed,
            "hint:victory",
        ),
        (
            "live unconfirmed",
            live_state(),
            &unconfirmed,
            "hint:victory",
        ),
        ("idle hero select", idle_state(), &hero_select, "prime"),
        ("live hero select", live_state(), &hero_select, "prime"),
        ("idle hero ban", idle_state(), &hero_ban, "split:unknown:-"),
        ("live hero ban", live_state(), &hero_ban, "split:unknown:-"),
        ("idle map vote", idle_state(), &map_vote, "split:unknown:-"),
        ("live map vote", live_state(), &map_vote, "split:unknown:-"),
        (
            "live map vote blocked",
            live_state(),
            &map_blocked,
            "ignore",
        ),
        ("idle board", idle_state(), &Obs::Board(stay), "append"),
        ("live board", live_state(), &Obs::Board(stay), "append"),
        (
            "live progressed",
            live_state(),
            &Obs::Board(progressed),
            "append",
        ),
        (
            "live implausible still stored",
            live_state(),
            &Obs::Board(implausible),
            "append",
        ),
        (
            "live fresh reset",
            live_state(),
            &Obs::Board(reset),
            "defer",
        ),
        ("live gap", live_state(), &gap, "split:defeat:-"),
        ("live end screen", live_state(), &end, "split:defeat:-"),
        (
            "streak confirmed",
            streak_state(),
            &confirmed,
            "seal:defeat",
        ),
        (
            "streak unconfirmed",
            streak_state(),
            &unconfirmed,
            "hint:victory",
        ),
        (
            "streak unarmed different map",
            streak_state(),
            &Obs::UnconfirmedWord {
                outcome: MatchOutcome::Victory,
                relation: MapRelation::Differs,
            },
            "ignore",
        ),
        (
            "streak start after a board",
            streak_state(),
            &hero_select,
            "arm",
        ),
        (
            "hint with no board then start",
            hint_only_state(),
            &hero_select,
            "split:unknown:defeat",
        ),
        (
            "streak first progressed board keeps the hint",
            streak_state(),
            &Obs::Board(progressed),
            "progress",
        ),
        (
            "streak same totals",
            streak_state(),
            &Obs::Board(stay),
            "append",
        ),
        (
            "streak header does not clear",
            streak_state(),
            &Obs::Board(header),
            "append",
        ),
        (
            "streak implausible stays stored",
            streak_state(),
            &Obs::Board(implausible),
            "append",
        ),
        (
            "streak fresh reset",
            streak_state(),
            &Obs::Board(reset),
            "defer",
        ),
        (
            "streak gap seals the hint",
            streak_state(),
            &gap,
            "split:defeat:defeat",
        ),
        (
            "streak end screen",
            streak_state(),
            &end,
            "split:defeat:defeat",
        ),
        ("post confirmed", post_state(), &confirmed, "ignore"),
        ("post unconfirmed", post_state(), &unconfirmed, "ignore"),
        ("post start", post_state(), &hero_select, "split:unknown:-"),
        ("post board", post_state(), &Obs::Board(stay), "append"),
        (
            "post implausible still stored",
            post_state(),
            &Obs::Board(implausible),
            "append",
        ),
        (
            "post fresh reset",
            post_state(),
            &Obs::Board(reset),
            "defer",
        ),
        ("post gap stays", post_state(), &gap, "ignore"),
        ("post end screen", post_state(), &end, "ignore"),
        (
            "starting confirmed",
            starting_state(),
            &confirmed,
            "seal:defeat",
        ),
        (
            "starting unconfirmed",
            starting_state(),
            &unconfirmed,
            "hint:victory",
        ),
        (
            "starting start",
            starting_state(),
            &map_vote,
            "split:unknown:-",
        ),
        (
            "starting board",
            starting_state(),
            &Obs::Board(stay),
            "append",
        ),
        ("starting gap", starting_state(), &gap, "ignore"),
        (
            "streak vote after a board seals the hint",
            streak_state(),
            &map_vote,
            "split:unknown:defeat",
        ),
        (
            "streak ban after a board seals the hint",
            streak_state(),
            &hero_ban,
            "split:unknown:defeat",
        ),
        (
            "streak vote blocked",
            streak_state(),
            &map_blocked,
            "ignore",
        ),
        ("post vote blocked", post_state(), &map_blocked, "ignore"),
        (
            "starting ban",
            starting_state(),
            &hero_ban,
            "split:unknown:-",
        ),
        ("idle gap", idle_state(), &gap, "ignore"),
        ("idle end screen", idle_state(), &end, "ignore"),
        (
            "starting hero select",
            starting_state(),
            &hero_select,
            "ignore",
        ),
        (
            "starting start still inside the debounce",
            starting_state(),
            &map_blocked,
            "ignore",
        ),
        ("starting end screen", starting_state(), &end, "ignore"),
    ];
    for (label, state, obs, tag) in cases {
        expect_transition(label, state, obs, tag);
    }

    let mut second = post_state();
    second.reset_streak = 1;
    expect_transition(
        "post second fresh-match board",
        &second,
        &Obs::Board(reset),
        "split:unknown:-",
    );
    let mut second_hint = streak_state();
    second_hint.reset_streak = 1;
    expect_transition(
        "streak second fresh-match board seals the hint",
        &second_hint,
        &Obs::Board(reset),
        "split:unknown:defeat",
    );
    expect_transition(
        "hero select after a primed streak arms and does not split",
        &second_hint,
        &hero_select,
        "arm",
    );
    let mut deferred = live_state();
    deferred.deferred = Some(counters(1, 0, 0, 10, 0, 0));
    deferred.reset_streak = 1;
    expect_transition(
        "hero select after a deferred board does not split",
        &deferred,
        &hero_select,
        "ignore",
    );
    expect_transition(
        "hero ban after a deferred board does not split",
        &deferred,
        &hero_ban,
        "ignore",
    );
    let mut armed = streak_state();
    armed.pending_boundary = true;
    expect_transition(
        "armed end screen on a different map seals the old hint",
        &armed,
        &end,
        "split:defeat:defeat",
    );
    let mut pending = streak_state();
    pending.pending_boundary = true;
    expect_transition(
        "armed boundary plus a continuation drops the hint",
        &pending,
        &Obs::Board(progressed),
        "clear",
    );
    expect_transition(
        "armed boundary plus the first reset board splits and seals",
        &pending,
        &Obs::Board(reset),
        "split:unknown:defeat",
    );
    let mut live_primed = live_state();
    live_primed.reset_streak = 1;
    expect_transition(
        "live second fresh board",
        &live_primed,
        &Obs::Board(reset),
        "split:unknown:-",
    );
    let hinted = Obs::HintedDifferentMap {
        frame_outcome: MatchOutcome::Victory,
    };
    expect_transition(
        "hinted different map seals the hint",
        &streak_state(),
        &hinted,
        "split:victory:defeat",
    );
    expect_transition(
        "armed word with no map keeps the hint",
        &pending,
        &Obs::UnconfirmedWord {
            outcome: MatchOutcome::Victory,
            relation: MapRelation::Absent,
        },
        "ignore",
    );
    expect_transition(
        "armed word on a different map keeps the hint",
        &pending,
        &Obs::UnconfirmedWord {
            outcome: MatchOutcome::Victory,
            relation: MapRelation::Differs,
        },
        "ignore",
    );
    let starting_ban_blocked = Obs::StartScreen {
        screen: &ban,
        block_map_vote: true,
    };
    expect_transition(
        "starting ban inside the debounce",
        &starting_state(),
        &starting_ban_blocked,
        "ignore",
    );
}

#[test]
fn new_game_starting_exits_on_a_word_and_a_start_screen_and_ignores_a_gap() {
    let now = t0();
    let mut state = starting_state();
    let decision = decide_poll(&PollInput {
        outcome: MatchOutcome::Unknown,
        result: None,
        pending_boundary: false,
        awaiting_first_board: true,
        has_board: false,
        reset_streak: 0,
        map: None,
        map_trusted: false,
        hero: None,
        signal: Some(MatchOutcome::Defeat),
        signal_confirmed: true,
        accolade_map: None,
        start_screen: None,
        block_map_vote: false,
        now,

        deferred: false,
        text_fallback_locked: false,
    });
    commit_poll(&mut state, decision, now);
    assert_eq!(state.outcome, MatchOutcome::Defeat);
    assert!(!state.awaiting_first_board);

    let screen = StartScreen::MapVote {
        candidates: vec!["Ilios".into()],
    };
    expect_transition(
        "second start screen",
        &starting_state(),
        &Obs::StartScreen {
            screen: &screen,
            block_map_vote: false,
        },
        "split:unknown:-",
    );
    expect_transition(
        "starting gap",
        &starting_state(),
        &Obs::Gap {
            frame_outcome: MatchOutcome::Victory,
        },
        "ignore",
    );
}
