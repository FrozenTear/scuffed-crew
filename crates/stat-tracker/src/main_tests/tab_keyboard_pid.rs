//! Daemon tests: tab_keyboard_pid. See `mod.rs`.

use super::*;

#[test]
fn board_names_takes_every_name_window_and_skips_blanks() {
    let cell = |v: &str| ocr::CellOcrResult {
        value: v.into(),
        confidence: 90,
        suspect: false,
    };
    let rows = vec![
        ocr::RowOcrResult {
            name: Some(cell("QXOther")),
            name_alternates: vec![None, Some(cell(" QX0ther "))],
            stats: Vec::new(),
            mean_confidence: 90,
        },
        ocr::RowOcrResult {
            name: Some(cell("  ")),
            name_alternates: Vec::new(),
            stats: Vec::new(),
            mean_confidence: 0,
        },
    ];
    assert_eq!(board_names(&rows), vec!["QXOther", "QX0ther"]);
}

#[test]
fn tab_retry_predicate_retries_only_retryable_results() {
    let scan = detect::hero_portrait::RowScan {
        dip_count: 2,
        median_pitch: None,
        spectral_pitch: None,
    };
    let frame = || image::DynamicImage::new_rgb8(4, 4);
    let rejected = FrameAnalysisOutcome::NotAScoreboard {
        outcome: detect::MatchOutcome::Unknown,
        frame: frame(),
        scan,
    };
    let unsure = FrameAnalysisOutcome::TeamSizeUncertain {
        outcome: detect::MatchOutcome::Unknown,
        frame: frame(),
        scan,
    };
    use stat_tracker::tab_retry::TabResult;
    assert_eq!(tab_result(&rejected, true), TabResult::NotAScoreboard);
    assert!(tab_result(&rejected, true).retries());
    assert!(tab_result(&FrameAnalysisOutcome::StreamMiss, true).retries());
    assert!(tab_result(&FrameAnalysisOutcome::ShareOpening, true).retries());
    assert!(!tab_result(&FrameAnalysisOutcome::Declined, true).retries());
    assert!(!tab_result(&unsure, true).retries());
    assert!(
        tab_outcome_rank(&rejected, true)
            > tab_outcome_rank(&FrameAnalysisOutcome::StreamMiss, true)
    );
    assert!(
        tab_outcome_rank(&FrameAnalysisOutcome::StreamMiss, true)
            > tab_outcome_rank(&FrameAnalysisOutcome::ShareOpening, true)
    );
    assert!(tab_outcome_rank(&unsure, true) > tab_outcome_rank(&rejected, true));
    // A busy lock is retried and never outranks a frame already grabbed.
    assert!(tab_result(&FrameAnalysisOutcome::CaptureBusy, true).retries());
    assert_eq!(
        tab_result(&FrameAnalysisOutcome::CaptureBusy, true),
        TabResult::NoFrameYet,
        "busy is no frame yet, not a rejected board"
    );
    assert!(
        tab_outcome_rank(&FrameAnalysisOutcome::StreamMiss, true)
            > tab_outcome_rank(&FrameAnalysisOutcome::CaptureBusy, true)
    );
    assert!(tab_retry_reason(&FrameAnalysisOutcome::CaptureBusy, true).contains("still running"));
    // The saved rejected frames showed gameplay, killcams, VICTORY, chat
    // or the desktop, never a half-drawn board: the log says so.
    let why = tab_retry_reason(&rejected, true);
    assert!(!why.contains("still be drawing"), "{why}");
    assert!(why.contains("no full scoreboard"), "{why}");
}

/// S5-CAP-2 (journal 2026-10-22 13:29:54, "board was not found" on
/// the end-of-match board, no retry): a Tab frame whose board the new
/// reader refuses is a rejected frame inside the retried attempt, so it
/// is grabbed again while Tab is held, and the flight ends on the board
/// the reader takes.
#[tokio::test]
async fn a_board_the_reader_refuses_is_retried_while_tab_is_held() {
    use stat_tracker::tab_retry::{
        TAB_RETRY_WAITS, TabFlight, TabFlightPhase, TabResult, with_retries,
    };
    let refused = || FrameAnalysisOutcome::Analyzed(Box::new(o8_analysis(None, None)));
    let taken = || {
        let mut analysis = o8_analysis(None, None);
        analysis.board = Some(o8_board(Some("Ana")));
        FrameAnalysisOutcome::Analyzed(Box::new(analysis))
    };
    let refusal = |outcome: &FrameAnalysisOutcome| match outcome {
        FrameAnalysisOutcome::Analyzed(analysis) => reader_refusal(analysis, true),
        _ => panic!("an analysed frame"),
    };
    assert_eq!(refusal(&refused()), Some("board was not found"));
    assert_eq!(refusal(&taken()), None);
    assert_eq!(tab_result(&refused(), true), TabResult::BoardRefused);
    assert!(tab_result(&refused(), true).retries());
    assert_eq!(tab_result(&taken(), true), TabResult::Accepted);
    // ocr-v1, and a board read with ocr-v1 instead, keep their gate
    // where it was.
    assert_eq!(tab_result(&refused(), false), TabResult::Accepted);
    let mut fallback = o8_analysis(None, None);
    fallback.ocr_fallback = true;
    assert_eq!(
        tab_result(&FrameAnalysisOutcome::Analyzed(Box::new(fallback)), true),
        TabResult::Accepted
    );
    assert!(tab_outcome_rank(&taken(), true) > tab_outcome_rank(&refused(), true));
    // A refused board outranks a preflight reject, so a retry's
    // refused board replaces an earlier reject.
    let preflight_reject = FrameAnalysisOutcome::NotAScoreboard {
        outcome: detect::MatchOutcome::Unknown,
        frame: image::DynamicImage::new_rgb8(4, 4),
        scan: detect::hero_portrait::RowScan {
            dip_count: 0,
            median_pitch: None,
            spectral_pitch: None,
        },
    };
    assert!(tab_outcome_rank(&refused(), true) > tab_outcome_rank(&preflight_reject, true));
    assert!(tab_retry_reason(&refused(), true).contains("refused"));

    // Refused, then the retry gets a board the reader takes.
    let flight = TabFlight::new(Instant::now());
    let (best, retries) = with_retries(
        &flight,
        &TAB_RETRY_WAITS,
        |outcome| tab_result(outcome, true),
        |outcome| tab_outcome_rank(outcome, true),
        |outcome| tab_retry_reason(outcome, true),
        |attempt| {
            let outcome = if attempt == 0 { refused() } else { taken() };
            async move { Ok::<_, ()>(outcome) }
        },
        |_| async {},
    )
    .await
    .unwrap();
    assert_eq!(retries, 1);
    assert_eq!(refusal(&best), None, "the taken board is kept");
    assert_eq!(flight.phase(), TabFlightPhase::Accepted);

    // Refused on every grab: the flight ends rejected, never accepted.
    let flight = TabFlight::new(Instant::now());
    let (best, retries) = with_retries(
        &flight,
        &TAB_RETRY_WAITS,
        |outcome| tab_result(outcome, true),
        |outcome| tab_outcome_rank(outcome, true),
        |outcome| tab_retry_reason(outcome, true),
        |_| {
            let outcome = refused();
            async move { Ok::<_, ()>(outcome) }
        },
        |_| async {},
    )
    .await
    .unwrap();
    assert_eq!(retries, TAB_RETRY_WAITS.len());
    assert!(refusal(&best).is_some());
    assert_eq!(flight.phase(), TabFlightPhase::Rejected);
}

/// Final pre-tag review: a board the reader refuses outranks an unsure
/// team size, as any analysed board did in rc.7. When attempt 0 is a
/// refused end-of-match board carrying VICTORY and the retry is unsure
/// with no outcome, the loop keeps the refused board, so the report
/// still back-fills the outcome.
#[tokio::test]
async fn a_refused_board_outranks_an_unsure_team_size_and_keeps_its_outcome() {
    use stat_tracker::tab_retry::{TAB_RETRY_WAITS, TabFlight, with_retries};
    let scan = detect::hero_portrait::RowScan {
        dip_count: 0,
        median_pitch: None,
        spectral_pitch: None,
    };
    let refused_victory = || {
        let mut analysis = o8_analysis(None, None);
        analysis.frame_outcome = detect::MatchOutcome::Victory;
        FrameAnalysisOutcome::Analyzed(Box::new(analysis))
    };
    let unsure = || FrameAnalysisOutcome::TeamSizeUncertain {
        outcome: detect::MatchOutcome::Unknown,
        frame: image::DynamicImage::new_rgb8(4, 4),
        scan,
    };
    let preflight_reject = FrameAnalysisOutcome::NotAScoreboard {
        outcome: detect::MatchOutcome::Unknown,
        frame: image::DynamicImage::new_rgb8(4, 4),
        scan,
    };
    let taken = {
        let mut analysis = o8_analysis(None, None);
        analysis.board = Some(o8_board(Some("Ana")));
        FrameAnalysisOutcome::Analyzed(Box::new(analysis))
    };
    let rank = |outcome: &FrameAnalysisOutcome| tab_outcome_rank(outcome, true);
    // taken > refused > unsure > preflight reject > stream miss > busy.
    assert!(rank(&taken) > rank(&refused_victory()));
    assert!(rank(&refused_victory()) > rank(&unsure()));
    assert!(rank(&unsure()) > rank(&preflight_reject));
    assert!(rank(&preflight_reject) > rank(&FrameAnalysisOutcome::StreamMiss));
    assert!(rank(&FrameAnalysisOutcome::StreamMiss) > rank(&FrameAnalysisOutcome::CaptureBusy));
    assert_eq!(rank(&FrameAnalysisOutcome::PortalPaused), 0);

    let flight = TabFlight::new(Instant::now());
    let (best, _) = with_retries(
        &flight,
        &TAB_RETRY_WAITS,
        |outcome| tab_result(outcome, true),
        |outcome| tab_outcome_rank(outcome, true),
        |outcome| tab_retry_reason(outcome, true),
        |attempt| {
            let outcome = if attempt == 0 {
                refused_victory()
            } else {
                unsure()
            };
            async move { Ok::<_, ()>(outcome) }
        },
        |_| async {},
    )
    .await
    .unwrap();
    match best {
        FrameAnalysisOutcome::Analyzed(analysis) => {
            assert!(reader_refusal(&analysis, true).is_some());
            assert_eq!(analysis.frame_outcome, detect::MatchOutcome::Victory);
        }
        _ => panic!("the refused board with its outcome is kept"),
    }
}

/// Final pre-tag review: a Tab turned away by a paused or unanswering
/// screenshot portal is a quiet missed capture, not a failed cycle, and
/// is not retried.
#[test]
fn a_paused_portal_is_a_quiet_missed_tab() {
    use stat_tracker::tab_retry::TabResult;
    for err in [
        format!("portal screenshot: {}", capture::portal::SHOT_BACKOFF),
        format!("{} (no answer in 60 s)", capture::portal::SHOT_UNANSWERED),
        format!(
            "{} (the portal went away)",
            capture::portal::SHOT_UNANSWERED
        ),
    ] {
        assert!(capture::is_portal_unanswered(&err), "{err}");
        assert!(!capture::is_capture_busy(&err), "{err}");
    }
    assert!(!capture::is_portal_unanswered(
        "portal screenshot: a previous capture request is still running"
    ));
    assert!(!capture::is_portal_unanswered(
        "portal screenshot request failed: x"
    ));
    assert_eq!(
        tab_result(&FrameAnalysisOutcome::PortalPaused, true),
        TabResult::Final
    );
    assert!(!tab_result(&FrameAnalysisOutcome::PortalPaused, true).retries());
}

/// S5-CAP-2: the debounce and a queued press follow the capture
/// report, not the retry loop's verdict. A board refused after the
/// retries (the flight says Accepted) keeps the queued press and arms
/// no debounce; a recorded board arms it and drops the queued press.
#[test]
fn only_a_taken_board_arms_the_debounce_and_drops_a_queued_press() {
    use stat_tracker::tab_retry::{TabFlight, TabKeys, TabResult};
    let refused = rejected_report("s1");
    assert!(!refused.board_taken());
    let mut recorded = rejected_report("s1");
    recorded.recorded = true;
    assert!(recorded.board_taken());
    let mut armed = rejected_report("s1");
    armed.armed_reset = true;
    assert!(armed.board_taken(), "a held fresh-match board is taken");
    let mut pending = rejected_report("s1");
    pending.gate_state = Some(GateState::default());
    assert!(pending.board_taken(), "a pending board is taken");

    let t0 = Instant::now();
    let at = |ms: u64| t0 + Duration::from_millis(ms);
    let mut tabs = TabKeys::default();
    let mut last_accepted = None;
    assert_eq!(tabs.on_press(at(0), last_accepted, false), Some(at(0)));
    let first = TabFlight::new(at(0));
    tabs.started(Arc::clone(&first));
    first.finish(TabResult::Accepted);
    // A press while the capture reads its board is queued, not lost.
    assert_eq!(tabs.on_press(at(500), last_accepted, false), None);
    assert_eq!(tabs.queued_press(), Some(at(500)));
    // The board was refused after the retries: the queued press runs.
    assert_eq!(
        tabs.on_report(&mut last_accepted, refused.board_taken()),
        Some(at(500))
    );
    assert_eq!(last_accepted, None, "a refused board arms no debounce");

    let second = TabFlight::new(at(500));
    tabs.started(Arc::clone(&second));
    second.finish(TabResult::Accepted);
    assert_eq!(tabs.on_press(at(900), last_accepted, false), None);
    // Recorded: the queued press would read the same board.
    assert_eq!(
        tabs.on_report(&mut last_accepted, recorded.board_taken()),
        None
    );
    assert_eq!(last_accepted, Some(at(500)), "the recorded board arms it");
    assert_eq!(tabs.on_press(at(3_000), last_accepted, false), None);
    assert_eq!(
        tabs.on_press(at(3_600), last_accepted, false),
        Some(at(3_600))
    );
}

#[tokio::test]
async fn keyboard_open_retries_then_becomes_ready() {
    let mut waiter = ScriptedWaiter {
        events: vec![KeyboardWait::Retry],
    };
    let mut opens = vec![Err("no keyboard device found"), Ok("kbd")];
    let got = acquire_keyboard(|| opens.remove(0), &mut waiter, KEYBOARD_OPEN_ATTEMPTS)
        .await
        .expect("retry then success");
    assert_eq!(got, KeyboardAcquire::Ready("kbd"));
    assert!(opens.is_empty(), "both open attempts were consumed");
    assert!(waiter.events.is_empty(), "the retry wait was consumed");
}

#[tokio::test]
async fn no_keyboard_is_an_error_after_bounded_retries() {
    // Old startup path returned Ok(()) after Ctrl+C, so Restart=on-failure
    // never ran. Give-up must be Err, and it must not wait again after
    // the last failed open (that wait was the SIGTERM-swallowing hang).
    let retries = KEYBOARD_OPEN_ATTEMPTS.saturating_sub(1) as usize;
    let mut waiter = ScriptedWaiter {
        events: vec![KeyboardWait::Retry; retries],
    };
    let err = acquire_keyboard(
        || Err::<(), _>("no keyboard device found: ensure user is in the 'input' group"),
        &mut waiter,
        KEYBOARD_OPEN_ATTEMPTS,
    )
    .await
    .expect_err("exhausted opens must not look like a clean exit");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("no keyboard available"),
        "give-up is the non-zero exit path, got: {msg}"
    );
    assert!(
        msg.contains("input' group"),
        "the open error is preserved, got: {msg}"
    );
    assert!(
        waiter.events.is_empty(),
        "retried exactly {} time(s), then gave up without another wait",
        retries
    );
}

#[tokio::test]
async fn sigterm_during_keyboard_wait_is_clean_shutdown() {
    // Stop must stay exit 0. Mapping SIGTERM to Err would make
    // Restart=on-failure relaunch a daemon the operator just stopped.
    let mut waiter = ScriptedWaiter {
        events: vec![KeyboardWait::Shutdown],
    };
    let mut opens = 0u32;
    let got = acquire_keyboard(
        || {
            opens += 1;
            Err::<(), _>("no keyboard device found")
        },
        &mut waiter,
        KEYBOARD_OPEN_ATTEMPTS,
    )
    .await
    .expect("shutdown is Ok, not a failure");
    assert_eq!(got, KeyboardAcquire::Shutdown);
    assert_eq!(opens, 1, "do not keep opening after stop is requested");
    assert!(waiter.events.is_empty());
}

#[tokio::test]
async fn signal_waiter_returns_shutdown_on_sigterm() {
    // The scripted tests cover the attempt policy. This one checks the
    // production `select!`: a real SIGTERM must complete the wait as
    // shutdown instead of sitting out `KEYBOARD_OPEN_RETRY`.
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("register SIGTERM");
    let mut waiter = SignalKeyboardWaiter {
        sigterm: &mut sigterm,
    };
    let pid = std::process::id().to_string();
    let killer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        std::process::Command::new("kill")
            .args(["-TERM", &pid])
            .status()
            .expect("spawn kill")
    });
    let ev = tokio::time::timeout(Duration::from_secs(5), waiter.wait())
        .await
        .expect("timed out waiting for SIGTERM")
        .expect("signal listener");
    assert_eq!(ev, KeyboardWait::Shutdown);
    let status = killer.join().expect("killer thread");
    assert!(status.success(), "kill -TERM failed: {status}");
}

#[test]
fn pid_guard_never_blocks_on_self_or_dead_pid() {
    // Our own pid must never count as "another daemon" (PID reuse of a
    // recycled self-pid would otherwise deadlock startup).
    assert!(!stat_tracker::proc_id::pid_is_live_tracker(
        std::process::id()
    ));
    // A pid with no /proc entry is dead → does not block.
    assert!(!stat_tracker::proc_id::pid_is_live_tracker(u32::MAX));
}

#[test]
fn pid_guard_unlinks_only_its_own_pid() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("daemon.pid");
    let owner = 4242u32;

    std::fs::write(&path, owner.to_string()).unwrap();
    assert!(unlink_pid_file_if_owner(&path, owner));
    assert!(!path.exists(), "own pid file should be removed");

    // The record acquire_pid_guard writes: pid plus start ticks.
    std::fs::write(
        &path,
        stat_tracker::proc_id::format_pid_record(owner, Some(999_888)),
    )
    .unwrap();
    assert!(unlink_pid_file_if_owner(&path, owner));
    assert!(
        !path.exists(),
        "a two-line pid record of this process is removed on shutdown"
    );
    std::fs::write(&path, "not a pid\n").unwrap();
    assert!(!unlink_pid_file_if_owner(&path, owner));
    assert!(path.exists(), "a file that is not a pid record stays");

    let replacement = 7777u32;
    std::fs::write(&path, replacement.to_string()).unwrap();
    assert!(
        !unlink_pid_file_if_owner(&path, owner),
        "must not unlink a pid file this process no longer owns"
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap().trim(),
        replacement.to_string(),
        "a newer pid must survive the exiting process"
    );

    // Drop uses the same check. A replaced file stays; our own file goes.
    std::fs::write(&path, "111").unwrap();
    {
        let _guard = PidGuard {
            path: path.clone(),
            pid: owner,
        };
    }
    assert_eq!(std::fs::read_to_string(&path).unwrap().trim(), "111");

    std::fs::write(&path, owner.to_string()).unwrap();
    {
        let _guard = PidGuard {
            path: path.clone(),
            pid: owner,
        };
    }
    assert!(!path.exists());
}
