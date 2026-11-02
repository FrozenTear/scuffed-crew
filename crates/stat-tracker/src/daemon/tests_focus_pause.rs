//! The alt-tab case: a short focus loss pauses poll reads only. It never
//! ends, resets or splits the game or the session, keeps held state, and
//! is not a missing scoreboard for the game-end logic. Synthetic session
//! state and fake focus reads only.

use super::focus::*;
use super::tests_focus_gate::window;
use crate::main_tests::{game, session, test_now, word_streak};
use crate::*;
use std::time::Duration;

#[test]
fn a_short_alt_tab_pauses_reads_and_resets_nothing_else() {
    let now = test_now();
    let mut active = game(detect::MatchOutcome::Unknown, None, now);
    active.session_id = "alt-tab".into();
    active.gate = Some(GateState::default());
    active.reset_streak = 1;
    let mut st = session(Some(active), now);
    st.word_outcome_streak = Some(word_streak(detect::MatchOutcome::Victory, now));
    st.end_reel_wake_until = Some(now + Duration::from_secs(45));
    st.pending_outcome = Some((detect::MatchOutcome::Defeat, now));
    st.poll_ticks_skipped = 1;
    st.last_tab_capture = Some(now - Duration::from_secs(5));
    st.last_game_open = Some(now - Duration::from_secs(300));
    let before = |g: &ActiveGame| {
        (
            g.session_id.clone(),
            g.last_activity,
            g.opened_at,
            g.outcome,
            g.outcome_recorded_at,
            g.gate,
            g.reset_streak,
            g.pending_boundary,
        )
    };
    let before_game = before(st.active_game.as_ref().unwrap());

    // Overwatch focused, five reads of another window, focused again,
    // through the same gate and pause helper the poll arm uses.
    let mut gate = FocusGate::new(true);
    let mut paused = false;
    let reads = [
        window("overwatch.exe"),
        window("firefox"),
        window("firefox"),
        window("firefox"),
        window("firefox"),
        window("firefox"),
        window("overwatch.exe"),
    ];
    let mut skipped = Vec::new();
    for (i, read) in reads.iter().enumerate() {
        let at = now + Duration::from_millis(1_500 * i as u64);
        // Poll history from before this tick: a stability key seen once.
        st.ocr_stability.repeats("end_title", Some(1));
        let decision = gate.apply(read, at).decision;
        let skip = poll_pause(&mut paused, decision, &mut st);
        skipped.push(skip);
        if i == 1 {
            // The edge into the pause dropped the history.
            assert!(!st.ocr_stability.repeats("end_title", Some(1)));
        }
        if i == 3 {
            // Later pause ticks do not drop it again.
            assert!(st.ocr_stability.repeats("end_title", Some(1)));
        }
    }
    assert_eq!(skipped, [false, true, true, true, true, true, false]);
    assert!(!paused);

    // The game and the session are as they were.
    let after = st.active_game.as_ref().expect("the game is still open");
    // Session id, activity, open time, outcome, held counters, reset
    // streak and pending boundary, all unchanged.
    assert_eq!(before(after), before_game);
    let streak = st.word_outcome_streak.as_ref().expect("word streak kept");
    assert_eq!(
        (streak.outcome, streak.seen_at),
        (detect::MatchOutcome::Victory, now)
    );
    assert_eq!(st.end_reel_wake_until, Some(now + Duration::from_secs(45)));
    assert_eq!(
        st.pending_outcome,
        Some((detect::MatchOutcome::Defeat, now))
    );
    assert_eq!(st.poll_ticks_skipped, 1, "no cadence skip burnt");
    assert_eq!(st.last_tab_capture, Some(now - Duration::from_secs(5)));
    assert_eq!(st.last_game_open, Some(now - Duration::from_secs(300)));

    // No game-end or session timer fires for the pause: no quiet close,
    // and the next Tab joins the same session.
    let later = now + Duration::from_secs(30);
    assert_eq!(
        quiet_close_reason(after, later, Duration::from_secs(90)),
        None
    );
    assert!(!should_start_fresh_session(Some(after), later));
}

#[test]
fn an_unreadable_focus_never_pauses_a_read() {
    let now = test_now();
    let mut st = session(Some(game(detect::MatchOutcome::Unknown, None, now)), now);
    let mut gate = FocusGate::new(true);
    let mut paused = false;
    for i in 0..10 {
        let read = FocusRead::Unreadable("no source".into());
        let decision = gate.apply(&read, now + Duration::from_secs(i)).decision;
        assert!(!poll_pause(&mut paused, decision, &mut st), "tick {i}");
    }
    assert_eq!(gate.status(), FocusGateStatus::Unreadable);
}
