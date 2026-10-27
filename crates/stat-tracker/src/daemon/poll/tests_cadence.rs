//! Tests for the poll cadence and its stack-size guard, next to `cadence.rs`.

use crate::{main_tests::*, *};

#[test]
fn slow_mode_only_for_mature_unfinished_game() {
    let now = test_now();
    // No game open → full cadence (start screens could show any tick).
    assert!(!poll_slow_mode(&session(None, now), now));
    // Mature unfinished game, nothing pending → slow. (`game()` opens
    // 300s ago; `last_game_open: None` models a recovered session.)
    let mut st = session(Some(game(detect::MatchOutcome::Unknown, None, now)), now);
    assert!(poll_slow_mode(&st, now));
    // Freshly opened game → full cadence (start screens still live).
    st.last_game_open = Some(now - Duration::from_secs(30));
    assert!(!poll_slow_mode(&st, now));
    st.last_game_open = Some(now - SLOW_AFTER_GAME_OPEN);
    assert!(poll_slow_mode(&st, now));
}

#[test]
fn slow_mode_drops_on_end_evidence() {
    let now = test_now();
    // Outcome recorded (banner / confirmed word) → full cadence for the
    // between-games window where the next start screens appear.
    let st = session(Some(game(detect::MatchOutcome::Victory, Some(5), now)), now);
    assert!(!poll_slow_mode(&st, now));
    // A fresh unconfirmed word read holds full cadence while it waits for
    // its agreeing partner...
    let mut st = session(Some(game(detect::MatchOutcome::Unknown, None, now)), now);
    st.word_outcome_streak = Some(word_streak(
        detect::MatchOutcome::Defeat,
        now - Duration::from_secs(10),
    ));
    assert!(!poll_slow_mode(&st, now));
    // ...but a streak past the confirmation window no longer does.
    st.word_outcome_streak = Some(word_streak(
        detect::MatchOutcome::Defeat,
        now - OUTCOME_CONFIRM_WINDOW - Duration::from_secs(1),
    ));
    assert!(poll_slow_mode(&st, now));
    // The OCR streak expired, but the session still has a sealable hint.
    st.word_outcome_streak = None;
    st.active_game.as_mut().unwrap().result_mark = Some(ResultMark {
        outcome: detect::MatchOutcome::Defeat,
        confirmed: false,
        seen_at: now - Duration::from_secs(90),
    });
    assert!(
        !poll_slow_mode(&st, now),
        "an open hint keeps full poll cadence"
    );
    st.active_game.as_mut().unwrap().result_mark = None;
    st.active_game.as_mut().unwrap().pending_boundary = true;
    assert!(
        !poll_slow_mode(&st, now),
        "a pending boundary keeps full poll cadence"
    );
}

#[test]
fn slow_mode_drops_on_end_reel_wake_then_returns() {
    let now = test_now();
    let mut st = session(Some(game(detect::MatchOutcome::Unknown, None, now)), now);
    assert!(poll_slow_mode(&st, now), "mature unfinished → slow");

    st.end_reel_wake_until = Some(now + END_REEL_WAKE);
    assert!(!poll_slow_mode(&st, now), "POTG wake → full cadence");
    assert!(
        !poll_slow_mode(&st, now + Duration::from_secs(44)),
        "still inside the 45s window"
    );
    assert!(
        poll_slow_mode(&st, now + END_REEL_WAKE),
        "at expiry → slow again"
    );
    assert!(
        poll_slow_mode(&st, now + END_REEL_WAKE + Duration::from_secs(1)),
        "after expiry → slow"
    );

    // Outcome streak / finished still win over a leftover wake field.
    st.end_reel_wake_until = None;
    st.word_outcome_streak = Some(word_streak(
        detect::MatchOutcome::Victory,
        now - Duration::from_secs(5),
    ));
    assert!(!poll_slow_mode(&st, now));
    let finished = session(Some(game(detect::MatchOutcome::Defeat, Some(2), now)), now);
    assert!(!poll_slow_mode(&finished, now));
}

/// Debug builds run the Tab capture future on a test thread's 2 MiB
/// stack; an unboxed grab future grew it past that once (#432). These
/// bounds sit at about twice today's sizes, so boxing removed from the
/// capture path fails here first.
#[cfg(debug_assertions)]
#[tokio::test]
async fn capture_futures_stay_small() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let now = test_now();
    let g = game(detect::MatchOutcome::Unknown, None, now);
    let req = build_capture_request(&g, false, now);
    let handle = handle_capture(&ctx, req);
    let handle_size = std::mem::size_of_val(&handle);
    drop(handle);
    let backend = capture::CaptureBackend::None;
    let grab = capture::capture_screen_output_for(&backend, None, capture::CaptureKind::Tab);
    let grab_size = std::mem::size_of_val(&grab);
    drop(grab);
    let poll = capture::poll_grab(&backend, None, true);
    let poll_size = std::mem::size_of_val(&poll);
    drop(poll);
    assert!(poll_size <= CAPTURE_FUTURE_MAX, "poll grab {poll_size}");
    assert!(handle_size <= HANDLE_CAPTURE_FUTURE_MAX, "{handle_size}");
    assert!(grab_size <= CAPTURE_FUTURE_MAX, "{grab_size}");
}

/// #433 review: the poll arm passes its own outcome-only flag to the
/// screenshot lock wait, and only a grab that ran resets the cadence.
#[test]
fn the_poll_arm_passes_its_outcome_only_flag() {
    // In src/daemon/event_loop.rs since 0.6 S11: found by name, wherever it
    // lives, with its whitespace removed.
    let run_loop: String = fn_source("run_loop").split_whitespace().collect();
    assert!(run_loop.contains("capture::poll_grab(backend,capture_output,outcome_only)"));
    assert!(run_loop.contains("ifpoll_resets_cadence(&grabbed){"));
}

/// #433 review: shutdown flushes perf.log, bounded by the deadline.
#[test]
fn shutdown_flushes_perf_log_within_the_deadline() {
    // In src/daemon/shutdown.rs since 0.6 S7: found by name, wherever it
    // lives, with its whitespace removed.
    let shutdown: String = fn_source("shutdown_from_signal")
        .split_whitespace()
        .collect();
    assert!(
        shutdown
            .contains("letwait=shutdown_flush_wait(stat_tracker::replace::shutdown_time_left());")
    );
    assert!(shutdown.contains("stat_tracker::perf_log::flush(wait)"));
}

#[test]
fn the_shutdown_flush_wait_is_the_smaller_of_500_ms_and_what_is_left() {
    assert_eq!(
        shutdown_flush_wait(std::time::Duration::from_secs(4)),
        std::time::Duration::from_millis(500)
    );
    assert_eq!(
        shutdown_flush_wait(std::time::Duration::from_millis(120)),
        std::time::Duration::from_millis(120)
    );
    assert_eq!(
        shutdown_flush_wait(std::time::Duration::ZERO),
        std::time::Duration::ZERO
    );
}

#[test]
fn a_poll_turned_away_by_the_shot_lock_keeps_the_cadence_due() {
    type Grab = Result<Result<u8, String>, ()>;
    let busy: Grab = Ok(Err(format!(
        "portal screenshot: {}",
        "a previous capture request is still running"
    )));
    assert!(!poll_resets_cadence(&busy), "busy: the next tick polls");
    let kwin_busy: Grab = Ok(Err(capture::kwin::KwinError::Busy.to_string()));
    assert!(!poll_resets_cadence(&kwin_busy));
    let frame: Grab = Ok(Ok(1));
    assert!(poll_resets_cadence(&frame));
    let failed: Grab = Ok(Err("screencast stream ended".into()));
    assert!(poll_resets_cadence(&failed), "a grab that ran counts");
}

#[test]
fn poll_failures_warn_on_the_first_then_every_five_minutes() {
    let t0 = Instant::now();
    let at = |secs| t0 + std::time::Duration::from_secs(secs);
    let mut streak = PollFailureStreak::default();
    assert_eq!(streak.recovered(t0), None, "no run, nothing to say");
    assert!(streak.failed(at(0)), "the first failure warns");
    assert!(!streak.failed(at(1)));
    assert!(!streak.failed(at(299)));
    assert!(streak.failed(at(300)), "five minutes on, it warns again");
    assert!(!streak.failed(at(301)));
    assert_eq!(streak.count, 5);
    assert_eq!(
        streak.recovered(at(310)),
        Some((5, std::time::Duration::from_secs(310)))
    );
    // A new run inside five minutes of the last WARN stays quiet, and
    // its end is not logged either: flapping grabs do not flood.
    assert!(!streak.failed(at(311)));
    assert_eq!(streak.count, 1);
    assert_eq!(streak.recovered(at(312)), None);
    // Five minutes after the last WARN a new run warns at once.
    assert!(streak.failed(at(600)));
    assert_eq!(
        streak.recovered(at(601)),
        Some((1, std::time::Duration::from_secs(1)))
    );
    assert_eq!(POLL_FAILURE_WARN_EVERY, std::time::Duration::from_secs(300));
}

/// Review of #437: a share that is opening (every game start on KDE
/// and GNOME), declined, or closed until the game runs is no failure,
/// and a run does not carry into the next game.
#[test]
fn known_poll_states_never_warn_and_the_run_ends_with_the_game() {
    let t0 = Instant::now();
    let mut streak = PollFailureStreak::default();
    for err in [
        "screencast open in progress",
        "Screen share was declined: no captures until Overwatch is restarted",
        "screencast share stays closed until the game is running",
        "portal screenshot: a previous capture request is still running",
    ] {
        assert_eq!(streak.on_error(err, t0), PollErrorLog::Expected, "{err}");
    }
    assert_eq!(streak.count, 0, "nothing counted");
    assert_eq!(streak.recovered(t0), None);
    assert_eq!(
        streak.on_error("screencast stream ended", t0),
        PollErrorLog::Warn
    );
    assert_eq!(
        streak.on_error(
            "screencast stream ended",
            t0 + std::time::Duration::from_secs(1)
        ),
        PollErrorLog::Trace
    );
    streak.game_stopped();
    assert_eq!(streak.count, 0);
    // The next game's first failure warns at once.
    assert_eq!(
        streak.on_error(
            "screencast stream ended",
            t0 + std::time::Duration::from_secs(2)
        ),
        PollErrorLog::Warn
    );
}

#[test]
fn idle_shot_floor_keeps_start_screens_inside_their_window() {
    let new_reader = stat_tracker::config::NEW_READER_POLL_INTERVAL;
    let classic = stat_tracker::config::CLASSIC_POLL_INTERVAL;
    assert_eq!(idle_shot_divisor(new_reader), 2, "1.5 s -> every 3 s");
    assert_eq!(idle_shot_divisor(classic), 1, "4 s is already slower");
    assert_eq!(idle_shot_divisor(Duration::from_millis(500)), 6);
    assert_eq!(idle_shot_divisor(Duration::from_secs(10)), 1);
    // An interval at or above the floor is never stretched. Below it,
    // the cadence is at most the floor (faster than the classic
    // reader's 4 s), so the ~15 s map vote sees five ticks and two
    // consecutive ones pass the stability gate.
    for ms in (500..=10_000).step_by(250) {
        let interval = Duration::from_millis(ms);
        let divisor = idle_shot_divisor(interval);
        if interval >= IDLE_SHOT_POLL_FLOOR {
            assert_eq!(divisor, 1, "{ms} ms");
            continue;
        }
        let cadence = interval * divisor;
        assert!(cadence <= IDLE_SHOT_POLL_FLOOR, "{ms} ms");
        assert!(cadence * 2 > IDLE_SHOT_POLL_FLOOR, "{ms} ms: {cadence:?}");
        assert!(
            Duration::from_secs(15) >= cadence * 5,
            "{ms} ms: map vote gets too few ticks at {cadence:?}"
        );
    }
}

/// A retry that finds the screenshot lock busy keeps the frame the Tab
/// already has; it is never lost to "busy".
#[tokio::test]
async fn a_busy_retry_keeps_the_best_tab_frame() {
    let scan = detect::hero_portrait::RowScan {
        dip_count: 2,
        median_pitch: None,
        spectral_pitch: None,
    };
    // Popped from the end: a rejected frame first, then busy grabs.
    let attempts = std::sync::Mutex::new(vec![
        FrameAnalysisOutcome::CaptureBusy,
        FrameAnalysisOutcome::NotAScoreboard {
            outcome: detect::MatchOutcome::Unknown,
            frame: image::DynamicImage::new_rgb8(4, 4),
            scan,
        },
    ]);
    let flight = stat_tracker::tab_retry::TabFlight::new(Instant::now());
    let (best, retries) = stat_tracker::tab_retry::with_retries(
        &flight,
        &stat_tracker::tab_retry::TAB_RETRY_WAITS,
        |outcome| tab_result(outcome, true),
        |outcome| tab_outcome_rank(outcome, true),
        |outcome| tab_retry_reason(outcome, true),
        |_| {
            let next = attempts.lock().unwrap().pop();
            async move { Ok::<_, ()>(next.unwrap_or(FrameAnalysisOutcome::CaptureBusy)) }
        },
        |_| async {},
    )
    .await
    .unwrap();
    assert_eq!(retries, 2, "busy is retried while Tab is held");
    assert!(
        matches!(best, FrameAnalysisOutcome::NotAScoreboard { .. }),
        "the grabbed frame survives the busy retries"
    );
    // Both routes' busy errors are recognised.
    assert!(capture::is_capture_busy(
        "portal screenshot: a previous capture request is still running"
    ));
    assert!(capture::is_capture_busy(
        &capture::kwin::KwinError::Busy.to_string()
    ));
    assert!(!capture::is_capture_busy("screencast stream ended"));
}

#[test]
fn shot_routes_poll_less_only_out_of_a_match() {
    use capture::ShotRoute::{Kwin, None as Stream, Portal};
    let now = test_now();
    let interval = stat_tracker::config::NEW_READER_POLL_INTERVAL;
    // No game open: the shot routes slow down, the others do not.
    let idle = session(None, now);
    assert_eq!(poll_divisor(&idle, now, Portal, interval), 2);
    assert_eq!(poll_divisor(&idle, now, Kwin, interval), 2);
    assert_eq!(poll_divisor(&idle, now, Stream, interval), 1);
    // A finished game (result in): between games, same.
    let finished = session(Some(game(detect::MatchOutcome::Victory, Some(5), now)), now);
    assert_eq!(poll_divisor(&finished, now, Portal, interval), 2);
    assert_eq!(poll_divisor(&finished, now, Stream, interval), 1);
    // A fresh game, start screens still live: full cadence everywhere.
    let mut fresh = session(Some(game(detect::MatchOutcome::Unknown, None, now)), now);
    fresh.last_game_open = Some(now - Duration::from_secs(30));
    assert_eq!(poll_divisor(&fresh, now, Portal, interval), 1);
    // Mid-match slow mode is unchanged on every route.
    let mid = session(Some(game(detect::MatchOutcome::Unknown, None, now)), now);
    assert_eq!(poll_divisor(&mid, now, Portal, interval), SLOW_POLL_DIVISOR);
    assert_eq!(poll_divisor(&mid, now, Stream, interval), SLOW_POLL_DIVISOR);
    // A wake (POTG / end reel, or a fresh result word) is full cadence.
    let mut woke = session(None, now);
    woke.end_reel_wake_until = Some(now + END_REEL_WAKE);
    assert_eq!(poll_divisor(&woke, now, Portal, interval), 1);
    let mut word = session(None, now);
    word.word_outcome_streak = Some(word_streak(
        detect::MatchOutcome::Defeat,
        now - Duration::from_secs(2),
    ));
    assert_eq!(poll_divisor(&word, now, Portal, interval), 1);
    // A boundary waiting on the next start screen: full cadence.
    let mut pending = session(Some(game(detect::MatchOutcome::Victory, Some(5), now)), now);
    pending.active_game.as_mut().unwrap().pending_boundary = true;
    assert_eq!(poll_divisor(&pending, now, Portal, interval), 1);
}

#[test]
fn cadence_wake_active_covers_streak_and_end_reel() {
    let now = test_now();
    let mut st = session(Some(game(detect::MatchOutcome::Unknown, None, now)), now);
    assert!(!cadence_wake_active(&st, now));
    st.word_outcome_streak = Some(word_streak(
        detect::MatchOutcome::Victory,
        now - Duration::from_secs(1),
    ));
    assert!(cadence_wake_active(&st, now));
    st.word_outcome_streak = None;
    st.end_reel_wake_until = Some(now + Duration::from_secs(10));
    assert!(cadence_wake_active(&st, now));
    st.end_reel_wake_until = Some(now);
    assert!(!cadence_wake_active(&st, now), "deadline is exclusive");
}

#[test]
fn skip_poll_for_tab_in_flight_mid_match_but_not_during_end_reel_wake() {
    let now = test_now();
    let mut st = session(Some(game(detect::MatchOutcome::Unknown, None, now)), now);
    assert!(
        skip_poll_for_tab_in_flight(true, &st, now),
        "Tab busy, no wake → skip (mid-match GPU/OCR contention)"
    );
    assert!(
        !skip_poll_for_tab_in_flight(false, &st, now),
        "Tab idle → never skip"
    );

    st.end_reel_wake_until = Some(now + END_REEL_WAKE);
    assert!(
        !skip_poll_for_tab_in_flight(true, &st, now),
        "wake + Tab busy → do not skip (cheap outcome poll)"
    );
    assert!(
        !skip_poll_for_tab_in_flight(true, &st, now + Duration::from_secs(44)),
        "still inside the 45s wake"
    );
    assert!(
        skip_poll_for_tab_in_flight(true, &st, now + END_REEL_WAKE),
        "wake expired → skip again"
    );

    // Word streak alone is not the Tab-busy exception, mid-match skip
    // stays unless end-reel wake is hot.
    st.end_reel_wake_until = None;
    st.word_outcome_streak = Some(word_streak(
        detect::MatchOutcome::Victory,
        now - Duration::from_secs(1),
    ));
    assert!(
        skip_poll_for_tab_in_flight(true, &st, now),
        "word streak without end-reel wake still skips"
    );
}

#[test]
fn poll_outcome_only_only_when_tab_busy_and_end_reel_wake() {
    let now = test_now();
    let mut st = session(Some(game(detect::MatchOutcome::Unknown, None, now)), now);
    assert!(!poll_outcome_only_while_tab_busy(false, &st, now));
    assert!(!poll_outcome_only_while_tab_busy(true, &st, now));

    st.end_reel_wake_until = Some(now + END_REEL_WAKE);
    assert!(
        poll_outcome_only_while_tab_busy(true, &st, now),
        "Tab busy + wake → cheap outcome path"
    );
    assert!(
        !poll_outcome_only_while_tab_busy(false, &st, now),
        "wake but Tab idle → full poll, not the cheap subset"
    );
    assert!(
        !poll_outcome_only_while_tab_busy(true, &st, now + END_REEL_WAKE),
        "wake expired → not cheap-path"
    );

    // Mutually exclusive with the skip helper.
    assert!(!skip_poll_for_tab_in_flight(true, &st, now));
    assert!(poll_outcome_only_while_tab_busy(true, &st, now));
    assert!(skip_poll_for_tab_in_flight(true, &st, now + END_REEL_WAKE));
    assert!(!poll_outcome_only_while_tab_busy(
        true,
        &st,
        now + END_REEL_WAKE
    ));
}
