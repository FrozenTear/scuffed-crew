//! Grabbing a Tab frame: the attempt, the retry ranking, the reused-frame
//! skip log, and the test hooks that inject a capture or a grab error.

use crate::*;

/// A prepared analysis for tests of [`handle_capture`]. Production never sets it.
#[cfg(test)]
pub(crate) struct InjectedCapture {
    pub(crate) analysis: FrameAnalysis,
    pub(crate) board: Option<shadow::BoardRead>,
}

#[cfg(test)]
pub(crate) static INJECTED_CAPTURE: std::sync::Mutex<Option<InjectedCapture>> =
    std::sync::Mutex::new(None);

/// Held by each test that injects a capture, so two such tests never take
/// each other's injection.
#[cfg(test)]
pub(crate) static INJECTED_CAPTURE_SERIAL: tokio::sync::Mutex<()> =
    tokio::sync::Mutex::const_new(());

pub(crate) fn take_injected_capture() -> Option<(FrameAnalysis, Option<shadow::BoardRead>)> {
    #[cfg(test)]
    {
        return INJECTED_CAPTURE
            .lock()
            .unwrap()
            .take()
            .map(|item| (item.analysis, item.board));
    }
    #[cfg(not(test))]
    None
}

/// A screen grab error for tests of [`handle_capture`]'s missed-capture
/// arms, returned by the next [`grab_tab_attempt`]. Production never sets it.
#[cfg(test)]
pub(crate) static INJECTED_GRAB_ERROR: std::sync::Mutex<Option<String>> =
    std::sync::Mutex::new(None);

pub(crate) fn take_injected_grab_error() -> Option<String> {
    #[cfg(test)]
    {
        return INJECTED_GRAB_ERROR.lock().unwrap().take();
    }
    #[cfg(not(test))]
    None
}

/// Why the new reader refuses an analysed Tab frame's board: the trust
/// gate `handle_capture` runs on it ([`reader_apply::board_refusal`]), here
/// inside the retried attempt so a refused board is grabbed again while
/// Tab is held (S5-CAP-2). `None` under ocr-v1, for a board read with
/// ocr-v1 instead, and for a board the reader vouches for.
pub(crate) fn reader_refusal(
    analysis: &FrameAnalysis,
    use_new_reader: bool,
) -> Option<&'static str> {
    if !use_new_reader || analysis.ocr_fallback {
        return None;
    }
    match &analysis.board {
        Some(board) => reader_apply::board_refusal(board, analysis.team_size),
        None => reader_apply::board_refusal(
            &shadow::reader::not_found(shadow::BoardStatus::NotFound),
            analysis.team_size,
        ),
    }
}

/// How a Tab grab ended, for the retry loop. A frame with no full
/// scoreboard (the preflight's, or a board the new reader refuses), a Tab
/// that got no fresh stream frame, and a share that is still opening are
/// tried again ([`stat_tracker::tab_retry::TabResult::retries`]).
/// A declined share, a team size the pitch does not settle, and an analysed
/// frame the reader takes are not.
pub(crate) fn tab_result(
    outcome: &FrameAnalysisOutcome,
    use_new_reader: bool,
) -> stat_tracker::tab_retry::TabResult {
    use stat_tracker::tab_retry::TabResult;
    match outcome {
        FrameAnalysisOutcome::Analyzed(a) if reader_refusal(a, use_new_reader).is_some() => {
            TabResult::BoardRefused
        }
        FrameAnalysisOutcome::Analyzed(_) => TabResult::Accepted,
        FrameAnalysisOutcome::NotAScoreboard { .. } => TabResult::NotAScoreboard,
        FrameAnalysisOutcome::StreamMiss
        | FrameAnalysisOutcome::ShareOpening
        | FrameAnalysisOutcome::CaptureBusy => TabResult::NoFrameYet,
        FrameAnalysisOutcome::Declined
        | FrameAnalysisOutcome::PortalPaused
        | FrameAnalysisOutcome::TeamSizeUncertain { .. } => TabResult::Final,
    }
}

/// Why a Tab grab is tried again, for the log.
pub(crate) fn tab_retry_reason(
    outcome: &FrameAnalysisOutcome,
    use_new_reader: bool,
) -> &'static str {
    match outcome {
        FrameAnalysisOutcome::Analyzed(a) if reader_refusal(a, use_new_reader).is_some() => {
            "the scoreboard reader refused the board (see board_refusal above)"
        }
        FrameAnalysisOutcome::NotAScoreboard { .. } => {
            "rejected by the scoreboard check (the frame shows no full scoreboard)"
        }
        FrameAnalysisOutcome::StreamMiss => "no fresh stream frame arrived",
        FrameAnalysisOutcome::ShareOpening => "the share is still opening",
        FrameAnalysisOutcome::CaptureBusy => "the previous screenshot was still running",
        FrameAnalysisOutcome::Declined
        | FrameAnalysisOutcome::PortalPaused
        | FrameAnalysisOutcome::TeamSizeUncertain { .. }
        | FrameAnalysisOutcome::Analyzed(_) => "not retried",
    }
}

/// How useful a Tab outcome is, so a later grab never replaces a better
/// one. Higher is better: analysed and taken by the reader (5), analysed
/// but refused by the reader (4), team size unsure (3), not a scoreboard
/// (2), stream miss (1), share still opening or declined, a busy lock or a
/// paused portal (0). Any analysed board outranks the rejects, as in rc.7:
/// it carries the header outcome fallback and the own-seed verdict, which
/// a reject does not. All of them carry the frame's outcome (the report
/// back-fills an end-of-match VICTORY or DEFEAT from a reject too), and the
/// rank does not look at it: a refused board with no outcome outranks an
/// unsure team size that read one, and that read is lost. An outcome-aware
/// tie-break is left for 0.6.
pub(crate) fn tab_outcome_rank(outcome: &FrameAnalysisOutcome, use_new_reader: bool) -> u8 {
    match outcome {
        FrameAnalysisOutcome::Analyzed(a) if reader_refusal(a, use_new_reader).is_some() => 4,
        FrameAnalysisOutcome::ShareOpening
        | FrameAnalysisOutcome::Declined
        | FrameAnalysisOutcome::CaptureBusy
        | FrameAnalysisOutcome::PortalPaused => 0,
        FrameAnalysisOutcome::StreamMiss => 1,
        FrameAnalysisOutcome::NotAScoreboard { .. } => 2,
        FrameAnalysisOutcome::TeamSizeUncertain { .. } => 3,
        FrameAnalysisOutcome::Analyzed(_) => 5,
    }
}

pub(crate) fn log_reused_frame_skip() {
    static LAST: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
    let debug = {
        let mut last = LAST.lock().unwrap_or_else(|err| err.into_inner());
        let now = Instant::now();
        let debug =
            capture::reused_frame_skip_is_debug(*last, now, capture::REUSED_FRAME_LOG_WINDOW);
        if debug {
            *last = Some(now);
        }
        debug
    };
    if debug {
        tracing::debug!(target: "scuffed_stat_tracker", "poll skipped: stream frame already seen");
    } else {
        tracing::trace!(target: "scuffed_stat_tracker", "poll skipped: stream frame already seen");
    }
}

pub(crate) async fn grab_tab_attempt(
    backend: &capture::CaptureBackend,
    output: Option<&str>,
    attempt: usize,
) -> Result<capture::CapturedFrame, Box<dyn std::error::Error + Send + Sync>> {
    if let Some(err) = take_injected_grab_error() {
        return Err(err.into());
    }
    if attempt != 0 {
        // A retry is not a new key press: on the stream it takes the next
        // buffer that arrives, with no second overlay delay.
        let press = capture::tab_retry_press(std::time::Instant::now());
        let grab = capture::capture_screen_output_for(backend, output, capture::CaptureKind::Tab);
        return capture::with_tab_press(press, grab).await;
    }
    // The first attempt keeps the original key, unless it starts so late
    // (a press queued behind another capture) that the key's frame budget
    // is gone: then it takes the next buffer, as a retry does. While the
    // share is still opening it waits up to 5 seconds for a fresh frame, not
    // the 1200 ms frame budget. If that open uses up the frame budget, the
    // grab runs again from now.
    let key = capture::tab_press().unwrap_or_else(std::time::Instant::now);
    let press = capture::first_grab_press(key, std::time::Instant::now());
    capture::with_tab_press(
        press,
        capture::first_tab_grab(
            press,
            || press.elapsed(),
            std::time::Instant::now,
            || capture::capture_screen_output_for(backend, output, capture::CaptureKind::Tab),
            || tokio::time::sleep(std::time::Duration::from_millis(50)),
        ),
    )
    .await
}
