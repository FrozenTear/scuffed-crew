//! Grabbing and reading a Tab frame: `handle_capture`, the attempt, the
//! retry ranking, the reused-frame skip log, and the test hooks that inject
//! a capture or a grab error.

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

/// Upper bounds for [`tests::capture_futures_stay_small`] (debug build,
/// rc.7: 15,168 and 1,640 bytes). Unboxing the KDE direct or portal grab
/// puts `capture_screen_output_for` at 4.6 KB or 28 KB.
#[cfg(all(test, debug_assertions))]
pub(crate) const HANDLE_CAPTURE_FUTURE_MAX: usize = 20 * 1024;

#[cfg(all(test, debug_assertions))]
pub(crate) const CAPTURE_FUTURE_MAX: usize = 3 * 1024;

pub(crate) async fn handle_capture(
    ctx: &DaemonCtx,
    mut req: CaptureRequest,
) -> anyhow::Result<CaptureReport> {
    if ctx.use_new_reader {
        req.member_map_edit = session_has_member_map_edit(&ctx.store, &req).await;
    }
    // QUAL-002 follow-up (multi-file seam split, parked for USER per DR-1 A4)
    // Ergonomic locals over the context/request (the body predates A1).
    let backend = &ctx.backend;
    let store = &ctx.store;
    let player_name = ctx.player_name.as_deref();
    let capture_output = ctx.capture_output.as_deref();
    let collect_portraits = ctx.collect_portraits;
    let data_dir: &std::path::Path = &ctx.data_dir;
    let session_id: &str = &req.session_id;
    let game_outcome = req.game_outcome;
    let session_map = req.session_map.as_deref();
    let allow_banner_recovery = req.allow_banner_recovery;

    let injected = take_injected_capture();
    let (analysis, board_override) = if let Some(injected) = injected {
        injected
    } else {
        tracing::info!(target: "scuffed_stat_tracker", "Tab detected: capturing screen (hold Tab to keep scoreboard visible)");
        // A rejected Tab frame is captured again after a short wait, unless
        // a newer Tab press takes over (`stat_tracker::tab_retry`).
        let own_seeds = own_name_seeds(ctx.own_name.as_ref());
        let names = ctx.names.clone();
        let flight = stat_tracker::tab_retry::current_flight();
        // No more grabs for this press once the retries end: the screen
        // stream may pause while the result is analysed.
        let use_new_reader = ctx.use_new_reader;
        let (analysis, _retries) = then_end_tab_hold(stat_tracker::tab_retry::with_retries(
            &flight,
            stat_tracker::tab_retry::tab_retry_waits(capture::shot_route()),
            |outcome| tab_result(outcome, use_new_reader),
            |outcome| tab_outcome_rank(outcome, use_new_reader),
            |outcome| tab_retry_reason(outcome, use_new_reader),
            |attempt| {
                let matcher = Arc::clone(&ctx.portrait_matcher);
                // Clone player_name so the blocking closure can own it.
                let player_name_owned = player_name.map(|s| s.to_string());
                let flags = FrameFlags {
                    allow_banner_recovery,
                    session_map_known: session_map.is_some(),
                    shadow_on: ctx.shadow.is_some(),
                    use_new_reader: ctx.use_new_reader,
                };
                let own_seeds = own_seeds.clone();
                let names = names.clone();
                async move {
                    let grabbed = grab_tab_attempt(backend, capture_output, attempt).await;
                    let img = match grabbed {
                        Ok(captured) => captured.image,
                        Err(err) if capture::is_stream_miss(&err.to_string()) => {
                            return Ok(FrameAnalysisOutcome::StreamMiss);
                        }
                        Err(err) if capture::is_share_opening(&err.to_string()) => {
                            return Ok(FrameAnalysisOutcome::ShareOpening);
                        }
                        Err(err) if capture::is_declined_capture(&err.to_string()) => {
                            return Ok(FrameAnalysisOutcome::Declined);
                        }
                        Err(err) if capture::is_capture_busy(&err.to_string()) => {
                            return Ok(FrameAnalysisOutcome::CaptureBusy);
                        }
                        Err(err) if capture::is_portal_unanswered(&err.to_string()) => {
                            return Ok(FrameAnalysisOutcome::PortalPaused);
                        }
                        Err(err) => {
                            return Err(
                                anyhow::Error::from_boxed(err).context("screen capture failed")
                            );
                        }
                    };
                    let analysis = tokio::task::spawn_blocking(move || {
                        let started = Instant::now();
                        let cpu_started = stat_tracker::perf::thread_cpu();
                        let analysis = analyze_frame(
                            img,
                            matcher,
                            player_name_owned,
                            game_outcome,
                            flags,
                            own_seeds,
                            &names,
                        );
                        stat_tracker::perf::record_cpu(
                            stat_tracker::perf::Stage::TabAnalyze,
                            stat_tracker::perf::thread_cpu().saturating_sub(cpu_started),
                        );
                        stat_tracker::perf::record(
                            stat_tracker::perf::Stage::TabAnalyze,
                            started.elapsed(),
                        );
                        // With classic OCR off there was no OCR, and no OCR workers.
                        if !ocr::gate::is_off() {
                            stat_tracker::omp::log_threads_after_first_ocr();
                        }
                        analysis
                    })
                    .await?;
                    if let FrameAnalysisOutcome::Analyzed(a) = &analysis
                        && let Some(board_refusal) = reader_refusal(a, use_new_reader)
                    {
                        tracing::info!(target: "scuffed_stat_tracker", board_refusal, "Tab frame refused by the scoreboard reader");
                    }
                    anyhow::Ok(analysis)
                }
            },
            tokio::time::sleep,
        ))
        .await?;
        let analysis = match analysis {
            FrameAnalysisOutcome::Analyzed(a) => Ok(*a),
            FrameAnalysisOutcome::StreamMiss
            | FrameAnalysisOutcome::ShareOpening
            | FrameAnalysisOutcome::Declined
            | FrameAnalysisOutcome::CaptureBusy
            | FrameAnalysisOutcome::PortalPaused => {
                if matches!(analysis, FrameAnalysisOutcome::StreamMiss) {
                    tracing::info!(target: "scuffed_stat_tracker", "Tab capture missed: no fresh stream frame");
                }
                if matches!(analysis, FrameAnalysisOutcome::CaptureBusy) {
                    tracing::info!(target: "scuffed_stat_tracker", "Tab capture missed: the previous screenshot was still running");
                }
                if matches!(analysis, FrameAnalysisOutcome::PortalPaused) {
                    tracing::info!(target: "scuffed_stat_tracker", "Tab capture missed: the screenshot portal is not answering");
                }
                return Ok(CaptureReport {
                    best_guess: None,
                    recorded: false,
                    outcome: game_outcome,
                    map: None,
                    map_source: None,
                    session_id: session_id.to_string(),
                    split: false,
                    armed_reset: false,
                    ignore_row: false,
                    reset_streak: req.reset_streak,
                    reset_baseline: req.reset_baseline,
                    baseline_row: req.baseline_row,
                    refresh_baseline: false,
                    clear_hint: false,
                    count_progress: false,
                    career_panel: false,
                    held_counters: None,
                    held_hero: None,
                    gate_state: None,
                    hero_auth: req.hero_auth.clone(),
                    seal: None,
                    close_reason: None,
                    held_at: None,
                    map_replaces_text_fallback: false,
                    rejoin_carry: None,
                    finished_game_tab: None,
                });
            }
            FrameAnalysisOutcome::NotAScoreboard {
                outcome,
                frame,
                scan,
            } => {
                tracing::warn!(
                    target: "scuffed_stat_tracker",
                    dip_count = scan.dip_count,
                    dip_pitch = ?scan.median_pitch,
                    spectral_pitch = ?scan.spectral_pitch,
                    "capture rejected: not a scoreboard (saved to debug/rejected when debug_ocr is on)"
                );
                Err((outcome, frame, "preflight"))
            }
            FrameAnalysisOutcome::TeamSizeUncertain {
                outcome,
                frame,
                scan,
            } => {
                tracing::warn!(
                    target: "scuffed_stat_tracker",
                    dip_count = scan.dip_count,
                    dip_pitch = ?scan.median_pitch,
                    spectral_pitch = ?scan.spectral_pitch,
                    "capture rejected: row pitch does not settle 5v5 vs 6v6 (saved to debug/rejected when debug_ocr is on)"
                );
                Err((outcome, frame, "teamsize"))
            }
        };
        let analysis = match analysis {
            Ok(a) => a,
            Err((outcome, frame, reason)) => {
                archive_rejected_map_label_crop(
                    ctx.debug_ocr,
                    data_dir,
                    session_id,
                    &frame,
                    reason,
                );
                save_rejected_frame(ctx.debug_ocr, data_dir, frame, reason);
                return Ok(CaptureReport {
                    best_guess: None,
                    recorded: false,
                    outcome,
                    map: None,
                    map_source: None,
                    session_id: session_id.to_string(),
                    split: false,
                    armed_reset: false,
                    ignore_row: false,
                    reset_streak: req.reset_streak,
                    reset_baseline: req.reset_baseline,
                    baseline_row: req.baseline_row,
                    refresh_baseline: false,
                    clear_hint: false,
                    count_progress: false,
                    career_panel: false,
                    held_counters: None,
                    held_hero: None,
                    gate_state: None,
                    hero_auth: req.hero_auth.clone(),
                    seal: None,
                    close_reason: None,
                    held_at: None,
                    map_replaces_text_fallback: false,
                    rejoin_carry: None,
                    finished_game_tab: None,
                });
            }
        };
        (analysis, None)
    };
    let FrameAnalysis {
        mut frame_outcome,
        ocr,
        rows,
        board: analysis_board,
        ocr_fallback,
        portrait_hero,
        career_hero,
        pack_hero,
        map_from_panel,
        map_panel_raw,
        map_from_banner,
        scoreboard: scoreboard_img,
        player_row_idx,
        name_match,
        own_row,
        name_recognizer,
        own_name_candidate,
        own_seed_verdict,
        team_size,
        frame: frame_img,
        shadow_input,
    } = analysis;
    if ctx.use_new_reader {
        // R9-2: a probe Tab's verdict is the finished game's, whose
        // session exists; the fresh id may never be used.
        let seed_session = req
            .finished_game_probe
            .as_ref()
            .map_or(session_id, |old| old.session_id.as_str());
        note_own_seed_verdict(ctx.own_name.as_ref(), own_seed_verdict, seed_session).await;
    }
    let ocr_result = ocr
        .map_err(anyhow::Error::from_boxed)
        .context("full-board OCR failed")?;
    // `reader = "new"`: the board the trust gate and the stats come from.
    // No board (a test injection without one) is a board that was not found.
    // A board found empty with OCR on was read with ocr-v1 instead
    // (`ocr_fallback`): this Tab then takes the ocr-v1 path for its stats
    // and is stored with recognizer `ocr-v1`.
    let board_reader = ctx.use_new_reader && !ocr_fallback;
    let new_board = board_reader.then(|| {
        board_override
            .or(analysis_board)
            .unwrap_or_else(|| shadow::reader::not_found(shadow::BoardStatus::NotFound))
    });
    // The member's row on that board: the row `find_own_row` chose from the
    // own-name seeds and the highlight (alpha.7 O9), or from its one name
    // fallback. No name is matched here.
    let board_row = reader_apply::own_row(None, None, player_row_idx, team_size);

    // Last-resort outcome source: the result header printed inside the
    // scoreboard region itself. Observed 2026-08-16 22:19:24Z (session
    // 018cea57): the full-board OCR of a Tab frame began "~ Defeat" while
    // every dedicated detector returned Unknown and the game stayed unknown.
    // The full-board text is already paid for, so this costs nothing; it is
    // limited to the first lines (header position) so chat or player names
    // deeper in the board can never supply a result (fleet::tracker-wl ET-2).
    if matches!(frame_outcome, detect::MatchOutcome::Unknown) {
        match parse::outcome_from_board_header(&ocr_result.raw_text) {
            detect::MatchOutcome::Unknown => {}
            o => {
                tracing::info!(target: "scuffed_stat_tracker", outcome = ?o, "outcome read from scoreboard OCR header line");
                frame_outcome = o;
            }
        }
    }
    let mut outcome = if game_outcome.is_decided() {
        game_outcome
    } else {
        frame_outcome
    };

    tracing::info!(target: "scuffed_stat_tracker", ?outcome, "frame analysis");
    let preview_end = ocr_result
        .raw_text
        .char_indices()
        .map(|(i, _)| i)
        .take_while(|&i| i <= 120)
        .last()
        .unwrap_or(0);
    // With classic OCR off the board text is not asked for: nothing to log.
    if !ocr::gate::is_off() {
        tracing::info!(
            target: "scuffed_stat_tracker",
            confidence = ocr_result.confidence,
            text_preview = &ocr_result.raw_text[..preview_end],
            "OCR result"
        );
    }

    let player_row_conf = player_row_idx
        .and_then(|i| rows.get(i))
        .map(|r| r.mean_confidence);
    tracing::info!(
        target: "scuffed_stat_tracker",
        ?player_row_idx,
        player_row_conf,
        rows = rows.len(),
        text_confidence = ocr_result.confidence,
        "scoreboard cell OCR complete"
    );

    // Trust gate (OCR site T3): don't parse frames that don't look like a
    // scoreboard (menus, replay browser, desktop). Better to record nothing
    // than to scrape stats out of a random screen. ocr-v1 decides on its
    // cell text. The new reader decides on its own board: found, the same
    // team size, digit values on it. A board it refuses is not handed to OCR.
    let board_refusal = new_board
        .as_ref()
        .and_then(|board| reader_apply::board_refusal(board, team_size));
    let looks_like_scoreboard = match &new_board {
        Some(_) => board_refusal.is_none(),
        None => parse::looks_like_scoreboard(&rows),
    };
    if !looks_like_scoreboard {
        tracing::warn!(
            target: "scuffed_stat_tracker",
            rows = rows.len(),
            board_refusal,
            "capture rejected: frame does not look like a scoreboard (saved to debug/rejected when debug_ocr is on)"
        );
        archive_rejected_map_label_crop(
            ctx.debug_ocr,
            data_dir,
            session_id,
            &frame_img,
            "noscoreboard",
        );
        save_rejected_frame(ctx.debug_ocr, data_dir, frame_img, "noscoreboard");
        return Ok(CaptureReport {
            best_guess: None,
            recorded: false,
            outcome,
            map: None,
            map_source: None,
            session_id: session_id.to_string(),
            split: false,
            armed_reset: false,
            ignore_row: false,
            reset_streak: req.reset_streak,
            reset_baseline: req.reset_baseline,
            baseline_row: req.baseline_row,
            refresh_baseline: false,
            clear_hint: false,
            count_progress: false,
            career_panel: false,
            held_counters: None,
            held_hero: None,
            gate_state: None,
            hero_auth: req.hero_auth.clone(),
            seal: None,
            close_reason: None,
            held_at: None,
            map_replaces_text_fallback: false,
            rejoin_carry: None,
            finished_game_tab: None,
        });
    }

    let mut outcome_label = outcome.to_string();
    // CG-4 C: mutates across this capture; carried back on CaptureReport.
    let mut hero_auth = req.hero_auth.clone();

    // The member's stats. ocr-v1: the cell text, else the name-anchored
    // raw-text fallback. New reader: the digit reads of the own row and
    // nothing else; a cell it does not vouch for is held, and a row with
    // under four sure cells is refused (never a text fallback, never an OCR
    // number). A suspect own row (alpha.7 O9: it may be another player's)
    // stores none of its numbers and does not seed or raise the running
    // totals. The boundary plan still lets it anchor the reset baseline,
    // and it does not split a game by itself (`BoardFacts::suspect_row`).
    let own_row_suspect = own_row.is_some_and(|own| own.suspect());
    let scoreboard_read = match &new_board {
        Some(board) => reader_apply::read_own_row_stats(board, board_row, &outcome_label)
            .map(|read| (read.matched, true, read.held)),
        None => parse::read_scoreboard(
            &rows,
            player_row_idx,
            &ocr_result.raw_text,
            &outcome_label,
            text_fallback_name(board_reader, player_row_idx, player_name),
        )
        .map(|read| {
            (
                read.matched,
                read.trusted_cells,
                [false; capture_gate::GATE_COLS],
            )
        }),
    };
    match scoreboard_read {
        Ok((mut parsed, trusted_cells, held_cols)) => {
            // The own-row name reader for this Tab. Absent when the row
            // came from a highlight, a seed, or the career hero.
            parsed.name_recognizer = name_recognizer.map(|id| id.as_str().to_string());
            // R9-2: a Tab past the grace window of a finished game. This
            // board decides whether it continues that game (then the capture
            // is that game's, with its false result cleared when the board
            // moved on) or is the fresh session's, as before rc.9.
            let mut finished_game_tab = None;
            if let Some(old) = req.finished_game_probe.take() {
                let probe_suspect = if board_reader {
                    held_cols
                } else {
                    parse::player_row_suspect_mask(&rows, player_row_idx)
                };
                let verdict = match old.prev_gate {
                    Some((prev, age)) => boundary::finished_game_tab(
                        &prev,
                        old.session_map.as_deref(),
                        map_from_panel.as_deref(),
                        &Counters {
                            elims: parsed.elims,
                            assists: parsed.assists,
                            deaths: parsed.deaths,
                            damage: parsed.damage,
                            healing: parsed.healing,
                            mitigation: parsed.mitigation,
                        },
                        &probe_suspect,
                        &held_cols,
                        trusted_cells && player_row_idx.is_some() && !own_row_suspect,
                        age,
                        old.last_board_after_result,
                    ),
                    None => boundary::FinishedGameTab::NewGame,
                };
                tracing::info!(
                    target: "scuffed_stat_tracker",
                    ?verdict,
                    finished_session = %old.session_id,
                    map = ?map_from_panel,
                    elims = parsed.elims,
                    deaths = parsed.deaths,
                    "Tab past the grace window: board compared with the finished game"
                );
                if verdict == boundary::FinishedGameTab::SameBoard {
                    // A late look at the finished game's board: the game
                    // and its result stay, and the board is not stored, so
                    // it neither replaces the game's last row nor restarts
                    // the window ([`FINISHED_GAME_TAB_MAX_GAP`]).
                    return Ok(CaptureReport {
                        finished_game_tab: Some(verdict),
                        ..blank_rejected_report(
                            old.game_outcome,
                            &old.session_id,
                            old.hero_auth.clone(),
                            &old,
                        )
                    });
                }
                if verdict == boundary::FinishedGameTab::StillPlaying {
                    req = *old;
                    // The result is cleared when this report lands
                    // (`settle_tab_probe`); this row does not carry it, nor
                    // the result words read for it (`unlatch_false_outcome`).
                    req.game_outcome = detect::MatchOutcome::Unknown;
                    req.hint = None;
                    req.after_end_screen = false;
                    req.result_evidence = Default::default();
                    // A live board and a result word on the same frame
                    // disagree, and this frame was read with the fresh
                    // session's age gate: its outcome is not taken.
                    frame_outcome = detect::MatchOutcome::Unknown;
                    if ctx.use_new_reader {
                        req.member_map_edit = session_has_member_map_edit(&ctx.store, &req).await;
                    }
                    hero_auth = req.hero_auth.clone();
                    outcome = if req.game_outcome.is_decided() {
                        req.game_outcome
                    } else {
                        frame_outcome
                    };
                    outcome_label = outcome.to_string();
                    parsed.outcome = outcome_label.clone();
                }
                finished_game_tab = Some(verdict);
            }
            let session_id: &str = &req.session_id;
            if held_cols.iter().any(|&held| held) {
                // Until the gate runs, a held column reads as the game's
                // last accepted value, so the split and reset checks see no
                // move in it.
                let prior = gate_prev_for_store(req.prev_gate, req.awaiting_first_board)
                    .map(|(state, _)| state.accepted);
                fill_held_cells(&mut parsed, held_cols, prior);
                tracing::info!(
                    target: "scuffed_stat_tracker",
                    held = ?held_cols,
                    "new reader held stat cells it does not vouch for"
                );
            }
            if own_row_suspect {
                tracing::info!(
                    target: "scuffed_stat_tracker",
                    ?player_row_idx,
                    "own row is suspect: none of its stats are stored or used to raise the totals"
                );
            }
            if !trusted_cells {
                tracing::warn!(
                    target: "scuffed_stat_tracker",
                    ?player_row_idx,
                    elims = parsed.elims,
                    assists = parsed.assists,
                    deaths = parsed.deaths,
                    damage = parsed.damage,
                    healing = parsed.healing,
                    mitigation = parsed.mitigation,
                    "stat cells unreadable: using raw-text fallback (low trust)"
                );
            }
            // Hero authority (CG-4 C): career-panel always wins; portrait may
            // confirm current hero but may only switch if career never succeeded
            // this game and ≥2 consecutive matches ≥0.85. See hero_auth::resolve_hero.
            // reader = "new" (O8): the heroes-v2 icon reads (career panel
            // portrait, own row) take the career-panel title's place, and no
            // OCR text names the hero.
            let portrait = portrait_hero
                .as_ref()
                .map(|(name, conf)| (name.as_str(), *conf));
            let (hero, source, next_auth) = if ctx.use_new_reader {
                hero_auth::resolve_hero_new_reader(
                    pack_hero.as_deref(),
                    portrait,
                    &hero_auth,
                    parse::canonical_hero,
                )
            } else {
                hero_auth::resolve_hero(
                    career_hero.as_deref(),
                    portrait,
                    &parsed.hero,
                    &hero_auth,
                    parse::canonical_hero,
                )
            };
            hero_auth = next_auth;
            parsed.hero = hero;
            parsed.role = parse::guess_role_public(&parsed.hero);
            let source_label = source.label();
            tracing::info!(
                target: "scuffed_stat_tracker",
                hero = %parsed.hero,
                source = source_label,
                career_ever_ok = hero_auth.career_ever_ok,
                portrait_conf = portrait_hero.as_ref().map(|(_, c)| *c),
                "hero resolved (CG-4 C authority)"
            );

            // Auto-collect a portrait reference when the hero is identified and
            // collection is enabled, or, always, when the career panel (the
            // authoritative OCR read) names a hero whose reference is missing.
            // The historical Doctrine stand-in digest (PROVISIONAL_PORTRAITS)
            // still counts as missing, so a capture replaces that old file.
            // A file with any other bytes is a real crop or a user file and
            // is never overwritten. The player's row must be known: cropping
            // row 0 when the row was not identified would store someone
            // else's hero and then stick. The portrait matcher reads the
            // installed hero pack. A local crop is used only for a stem that
            // pack does not have, and only when a pack is installed. With no
            // pack the matcher names no hero. Only the career-panel source
            // may seed (a portrait/held/text guess must not template itself).
            // The heroes-v2 icon reads (`reader = "new"`) do not force a
            // seed either: a wrong seed sticks. Only the
            // `collect_portraits` opt-in seeds under the new reader.
            let portraits_path = detect::hero_portrait::portraits_dir(data_dir);
            let reference_path =
                detect::hero_portrait::portrait_reference_path(&portraits_path, &parsed.hero);
            let slot_replaceable = parsed.hero != "Unknown"
                && detect::hero_portrait::portrait_slot_is_replaceable(&reference_path);
            let career_panel = matches!(source, HeroSource::CareerPanel);
            let save_portrait = detect::hero_portrait::should_save_portrait_crop(
                career_panel,
                collect_portraits,
                slot_replaceable,
                player_row_idx.is_some(),
                detect::hero_portrait::portrait_seed_layout_known(team_size, name_match.as_ref()),
            );
            if save_portrait && career_panel {
                tracing::info!(target: "scuffed_stat_tracker", hero = %parsed.hero, "no real portrait reference for career-panel hero: seeding one from this capture");
            }
            if save_portrait && let Some(row) = player_row_idx {
                // Shared geometry (5v5/6v6 + team gap): an inlined 5v5-only copy
                // here used to mis-crop 6v6/team-2 references into the template
                // library.
                let dims = (scoreboard_img.width(), scoreboard_img.height());
                if let Some(r) = detect::hero_portrait::portrait_rect(dims, row, team_size) {
                    let crop = scoreboard_img.crop_imm(r.x, r.y, r.w, r.h);
                    if let Err(e) = detect::hero_portrait::save_captured_portrait(
                        &portraits_path,
                        &parsed.hero,
                        &crop,
                    ) {
                        tracing::warn!(target: "scuffed_stat_tracker", error = %e, hero = %parsed.hero, "portrait save failed");
                    }
                }
            }

            let captured_at = Utc::now();
            let now = SurrealDatetime::from(captured_at);

            // Edge-ink suspect mask (CG-3) for the player's row, read from the same
            // per-cell OCR the stats came from. Threaded into BOTH the split decision
            // (DUP-1: a suspect column must not vote as a regression) and the capture
            // gate (a suspect read never corroborates a jump or drives an un-latch).
            // Under the new reader the mask is its held cells: no OCR cell
            // was read, so there is no edge ink to look at.
            let suspect = if board_reader {
                held_cols
            } else {
                parse::player_row_suspect_mask(&rows, player_row_idx)
            };

            // Stat-regression boundary (detector-independent): scoreboard stats
            // are cumulative within a match, so if this capture's counters sit
            // below the session's previous accepted capture, the poller missed
            // the game boundary and this board belongs to a new game.
            // Same-map/hero unfinished session: a stat-looking regression after
            // the 120s gap is almost always OCR noise or a late scoreboard of
            // this match. Do not split it off. A fresh-match drop is a candidate
            // new game even inside that gap. The first is held.
            let raw_counters = Counters {
                elims: parsed.elims,
                assists: parsed.assists,
                deaths: parsed.deaths,
                damage: parsed.damage,
                healing: parsed.healing,
                mitigation: parsed.mitigation,
            };
            let mut facts = BoardFacts {
                counters: raw_counters,
                suspect,
                // `trusted_cells` is `stats_from_row` on the identified row,
                // which is the same predicate as `parse::row_counts`.
                trusted_cells,
                held: held_cols,
                suspect_row: own_row_suspect,
                row_id: identified_row_id(player_row_idx),
                hero: &parsed.hero,
                map_from_panel: map_from_panel.as_deref(),
                panel_from_banner: map_from_banner,
                parsed_map: &parsed.map_name,
                frame_outcome,
            };
            let rejoin = rejoin_board(&req, &facts);
            note_rejoin(&req, &rejoin, raw_counters);
            facts.counters = rejoin.counters;
            let plan = plan_from_board(&req, &facts);
            if plan.skip_store {
                if plan.defer {
                    tracing::info!(
                        target: "scuffed_stat_tracker",
                        session_id = %session_id,
                        elims = parsed.elims,
                        deaths = parsed.deaths,
                        damage = parsed.damage,
                        "fresh-match board held: not written onto the current game"
                    );
                }
                // A deferred board with a held cell is not carried
                // (`plan.deferred_counters` is none): no hero or time either.
                let carried = plan.defer && plan.deferred_counters.is_some();
                return Ok(CaptureReport {
                    finished_game_tab,
                    ..skipped_capture_report(
                        &plan,
                        session_id,
                        source.is_authority(),
                        hero_auth,
                        carried.then(|| parsed.hero.clone()),
                        carried.then_some(captured_at),
                    )
                });
            }
            let mut staged = stage_capture(&req, &plan, &facts, captured_at);
            // Shadow recognizer: hand the accepted board to the log-only
            // worker. try_send, never blocks; a busy worker drops it.
            if let (Some(worker), Some(input)) = (&ctx.shadow, &shadow_input) {
                worker.try_submit(shadow::worker::ShadowJob::new(
                    input,
                    &rows,
                    player_row_idx,
                    &staged.target_session,
                    captured_at,
                ));
            }
            let split = staged.split;
            outcome = staged.outcome;
            outcome_label = staged.outcome_label.clone();
            parsed.outcome = outcome_label.clone();
            let target_session = staged.target_session.clone();
            // This board's session, including a split's new game. The first
            // Tab that still has the map label wins; later Tabs no-op.
            archive_map_label_crop(data_dir, &target_session, &frame_img);
            if split {
                tracing::info!(
                    target: "scuffed_stat_tracker",
                    old_session = %session_id,
                    new_session = %target_session,
                    elims = parsed.elims,
                    deaths = parsed.deaths,
                    damage = parsed.damage,
                    after_end_screen = req.after_end_screen,
                    reset_streak = plan.reset_streak,
                    "player stats regressed: previous game never closed; splitting into a new session"
                );
            }
            let gate = &staged.gate;
            // CG-4 B3: always surface the per-column suspect mask on the accept
            // path so a latched inflation can be diagnosed as flagged vs clean
            // (the 07-22 HLG 22994 case was undiagnosable without this).
            if suspect.iter().any(|&s| s) {
                tracing::info!(
                    target: "scuffed_stat_tracker",
                    session_id = %target_session,
                    suspect = ?suspect,
                    raw_elims = raw_counters.elims,
                    raw_assists = raw_counters.assists,
                    raw_deaths = raw_counters.deaths,
                    raw_damage = raw_counters.damage,
                    raw_healing = raw_counters.healing,
                    raw_mitigation = raw_counters.mitigation,
                    accepted_healing = gate.accepted.healing,
                    accepted_damage = gate.accepted.damage,
                    accepted_mitigation = gate.accepted.mitigation,
                    "capture accepted with edge-ink suspect mask"
                );
            }
            for h in &gate.holds {
                if h.kind == capture_gate::HoldKind::Unread {
                    // Not a misread caught by the gate: the reader gave no
                    // number for the cell (or a first read was not taken).
                    tracing::info!(
                        target: "scuffed_stat_tracker",
                        session_id = %target_session,
                        col = h.col,
                        raw = h.raw,
                        held = h.held,
                        "no sure read of a stat cell: the last accepted value stays"
                    );
                    continue;
                }
                if board_reader {
                    // No OCR read the cell: the digit read does not fit
                    // the game so far.
                    tracing::warn!(
                        target: "scuffed_stat_tracker",
                        session_id = %target_session,
                        col = h.col,
                        kind = ?h.kind,
                        raw = h.raw,
                        held = h.held,
                        "capture gate held a cell (the read does not fit the game so far)"
                    );
                    continue;
                }
                tracing::warn!(
                    target: "scuffed_stat_tracker",
                    session_id = %target_session,
                    col = h.col,
                    kind = ?h.kind,
                    raw = h.raw,
                    held = h.held,
                    "capture gate held a cell (suspected OCR misread)"
                );
            }
            for u in &gate.unlatches {
                if u.replaced_unconfirmed {
                    tracing::warn!(
                        target: "scuffed_stat_tracker",
                        session_id = %target_session,
                        col = u.col,
                        raw = u.raw,
                        revised_from = u.revised_from,
                        "capture gate replaced an unconfirmed cell"
                    );
                } else {
                    tracing::warn!(
                        target: "scuffed_stat_tracker",
                        session_id = %target_session,
                        col = u.col,
                        raw = u.raw,
                        revised_from = u.revised_from,
                        "capture gate un-latched a cell (clean reads revised a latched value down)"
                    );
                }
            }
            if gate.state.low_trust {
                tracing::warn!(
                    target: "scuffed_stat_tracker",
                    session_id = %target_session,
                    elims = gate.accepted.elims,
                    assists = gate.accepted.assists,
                    deaths = gate.accepted.deaths,
                    damage = gate.accepted.damage,
                    healing = gate.accepted.healing,
                    mitigation = gate.accepted.mitigation,
                    "stored capture is low-trust"
                );
            }
            let held_stats = gate.accepted;
            let held_cells = gate.holds.clone();
            // The stats as read, before the gate's held values replace them:
            // the report kit's `ocr_v1` (spec section 2). ocr-v1: the
            // Tesseract read. `reader = "new"`: there is no Tesseract read,
            // so the field carries the digit read before the gate, and for a
            // cell with no sure read the stored value (set below).
            let tesseract_stats = (
                parsed.elims,
                parsed.assists,
                parsed.deaths,
                parsed.damage,
                parsed.healing,
                parsed.mitigation,
            );
            parsed.elims = held_stats.elims;
            parsed.assists = held_stats.assists;
            parsed.deaths = held_stats.deaths;
            parsed.damage = held_stats.damage;
            parsed.healing = held_stats.healing;
            parsed.mitigation = held_stats.mitigation;
            parsed.map_name = staged.map_name.clone();

            // Untrusted Deathmatch falls back to the open map. A split stays
            // Unknown. Adlersbrunn with stats on only one team is not stored
            // as Eichenwalde. The current row is inserted before the carried
            // board, so a failed insert leaves the hold for the next Tab.
            if prepare_capture_row(
                &mut parsed,
                &mut staged,
                req.session_map.as_deref(),
                map_panel_raw.as_deref(),
                &ocr_result.raw_text,
                match &new_board {
                    Some(board) => reader_apply::both_teams_have_stats(board, team_size),
                    None => parse::both_teams_have_stats(&rows, team_size),
                },
            ) == PreparedRow::Drop
            {
                return Ok(CaptureReport {
                    finished_game_tab,
                    ..blank_rejected_report(outcome, session_id, hero_auth, &req)
                });
            }

            // The session is owned by the active game (map-vote to accolade). The
            // first capture creates the session row; later captures (including hero
            // swaps and the post-match scoreboard) append to the same session.
            parsed.session_id = target_session.clone();
            // The read before the gate, kept for the report kit's `ocr_v1`
            // field: this capture's fields with the stats as they were read
            // (Tesseract under ocr-v1, the digit reader under the new one).
            let mut ocr_parsed = parsed.clone();
            (
                ocr_parsed.elims,
                ocr_parsed.assists,
                ocr_parsed.deaths,
                ocr_parsed.damage,
                ocr_parsed.healing,
                ocr_parsed.mitigation,
            ) = tesseract_stats;
            // The new reader's own read of each stat before the gate held
            // any: the best guess for a column that never gets a sure read.
            let mut raw_guess: Option<[u32; capture_gate::GATE_COLS]> = None;
            // The reader's confidences, for the report kit (empty under ocr-v1).
            let mut kit_confidences = std::collections::BTreeMap::new();
            if let Some(board) = new_board {
                let guesses = reader_apply::own_row_guesses(&board, board_row);
                offer_own_name_seed(
                    ctx.own_name.as_ref(),
                    own_name_candidate,
                    &target_session,
                    &parsed.map_name,
                )
                .await;
                kit_confidences = reader_apply::kit_confidences(&board, board_row);
                kit_confidences.insert(
                    "result".to_string(),
                    stat_tracker::report::kit::ReaderRead::from_field(&shadow::result_field(
                        &req.result_evidence,
                    )),
                );
                apply_new_reader(
                    &mut parsed,
                    &req.result_evidence,
                    board_row,
                    own_row_suspect,
                    board,
                    pack_hero.as_deref(),
                );
                // The reader's number for each cell, sure or not; a cell with
                // none keeps what the row holds.
                let held = [
                    parsed.elims,
                    parsed.assists,
                    parsed.deaths,
                    parsed.damage,
                    parsed.healing,
                    parsed.mitigation,
                ];
                raw_guess = Some(std::array::from_fn(|col| guesses[col].unwrap_or(held[col])));
                let prior = gate_prev_for_store(req.prev_gate, req.awaiting_first_board);
                let gate_suspect = if own_row_suspect {
                    [true; capture_gate::GATE_COLS]
                } else {
                    suspect
                };
                staged.gate = hold_new_reader_stats_with_guesses(
                    &mut parsed,
                    prior,
                    gate_suspect,
                    held_cols,
                    split,
                    trusted_cells && !own_row_suspect,
                    carried_board(staged.carried, staged.carried_at, captured_at),
                    guesses,
                );
                // A held cell has no read of its own: the kit's `ocr_v1`
                // repeats the stored value there, never a number carried in
                // from before a split.
                let stored = [
                    parsed.elims,
                    parsed.assists,
                    parsed.deaths,
                    parsed.damage,
                    parsed.healing,
                    parsed.mitigation,
                ];
                let read = [
                    &mut ocr_parsed.elims,
                    &mut ocr_parsed.assists,
                    &mut ocr_parsed.deaths,
                    &mut ocr_parsed.damage,
                    &mut ocr_parsed.healing,
                    &mut ocr_parsed.mitigation,
                ];
                for (col, cell) in read.into_iter().enumerate() {
                    if held_cols[col] {
                        *cell = stored[col];
                    }
                }
                // A member's own hero pick on this game is never overridden
                // by a later Tab's read (O8): this row carries the pick too.
                if !split && target_session == session_id {
                    carry_session_hero_edit(store, session_id, &mut parsed).await;
                }
            } else {
                note_gate_suspects(
                    &mut parsed,
                    held_stats,
                    &capture_gate::unsure_fields(&held_cells),
                );
            }
            let report = CaptureReport {
                best_guess: None,
                recorded: true,
                outcome,
                map: staged.recorded_map.clone(),
                map_source: staged.map_source,
                session_id: target_session.clone(),
                split,
                armed_reset: false,
                ignore_row: plan.ignore_row,
                reset_streak: plan.reset_streak,
                reset_baseline: plan.reset_baseline,
                baseline_row: plan.baseline_row,
                refresh_baseline: plan.refresh_baseline,
                clear_hint: plan.clear_hint,
                count_progress: plan.count_progress,
                career_panel: source.is_authority(),
                held_counters: None,
                held_hero: None,
                gate_state: Some(staged.gate.state),
                hero_auth,
                seal: plan.seal,
                close_reason: plan.close_reason,
                held_at: None,
                map_replaces_text_fallback: staged.replaced_text_fallback,
                rejoin_carry: rejoin.carry,
                finished_game_tab,
            };
            // A stat column this game has never read is a placeholder, not
            // a number: no row is written while one is left (F266-1). The
            // board still went through the gate and the boundary plan, and
            // the report carries both, so the game goes on from here and
            // its row appears with the first board that reads the column.
            let unread = staged.gate.state.unread_cols();
            if !unread.is_empty() {
                tracing::info!(
                    target: "scuffed_stat_tracker",
                    session_id = %target_session,
                    ?unread,
                    split,
                    "board read but not stored: a stat column of this game has no sure read yet"
                );
                // The best guess waits on the active game. A later Tab with
                // a sure read stores the real row; if none comes, the game
                // stores this guess when it closes (alpha.8).
                // An all-suspect read, including a suspect own row, must not
                // be saved later as the game's best guess either.
                let best_guess = if own_row_suspect || suspect.iter().all(|&cell| cell) {
                    None
                } else {
                    raw_guess.map(|guess| {
                        let mut row = parsed.clone();
                        apply_best_guess(&mut row, &unread, guess);
                        row
                    })
                };
                return Ok(CaptureReport {
                    recorded: false,
                    best_guess,
                    ..report
                });
            }
            let created_this_capture =
                commit_capture_rows(store, data_dir, &staged, &parsed, now).await?;

            tracing::info!(
                target: "scuffed_stat_tracker",
                hero = %parsed.hero,
                map = %parsed.map_name,
                elims = parsed.elims,
                deaths = parsed.deaths,
                "parsed scoreboard"
            );
            let recorded_map = staged.recorded_map.clone();

            // Dump the accepted scoreboard crop to a bounded ring so a corrupt
            // ACCEPTED board is diagnosable after the fact: tonight's corruption
            // was undiagnosable because only rejected frames were ever saved.
            // Only with debug_ocr on: the crop shows other players' names.
            // The own-row file is a separate crop of this player's row, widened
            // left of the name column so the hero portrait is included. A
            // header clipped at the scoreboard's left edge is cut from the
            // widened frame crop the hero reader uses.
            save_own_row_review(
                ctx.debug_ocr,
                data_dir,
                &frame_img,
                scoreboard_img,
                player_row_idx,
                team_size,
            );
            // The unpadded crop, not the 5v5 recognition image: offline
            // replay places rows from crop height, and these files are the
            // benchmark boards.
            save_accepted_frame(ctx.debug_ocr, data_dir, debug_accepted_board(&frame_img));
            record_report_kit(
                data_dir,
                &target_session,
                &frame_img,
                team_size,
                &ocr_parsed,
                &parsed,
                captured_at,
                req.tab_at,
                board_names(&rows),
                &kit_confidences,
            );

            // Re-derive the session's hero timeline from its RAW snapshots after
            // every capture (HS-1). A single capture can mislabel (career panel
            // shows the spectated hero while dead; portrait matching can misfire),
            // and a late hero swap must be recorded as its own segment rather than
            // stamped across the whole game (the old set_session_hero last-write
            // recorded a 97%-Ana game as Rein). set_session_hero is now a MANUAL
            // repair helper only. Per-snapshot reads stay raw; the derived primary
            // drives the displayed/uploaded hero.
            if !created_this_capture
                && let Err(e) = store.refresh_session_hero_timeline(&target_session).await
            {
                tracing::debug!(target: "scuffed_stat_tracker", error = %e, "failed to refresh session hero timeline");
            }
            // Diagnostics: after N consecutive scoreboard captures that parsed but
            // resolved no map, dump the map-label region so the next failure is
            // debuggable from raw pixels (the 07-17 Ilios game left none).
            use std::sync::atomic::Ordering;
            if recorded_map.is_none() {
                let n = ctx.empty_map_reads.fetch_add(1, Ordering::Relaxed) + 1;
                if n >= EMPTY_MAP_DUMP_THRESHOLD {
                    ctx.empty_map_reads.store(0, Ordering::Relaxed);
                    let region = ocr::preprocess::crop_map_name(&frame_img);
                    let dir = data_dir.join("debug").join("mapmiss");
                    tracing::warn!(
                        target: "scuffed_stat_tracker",
                        consecutive = n,
                        "N consecutive scoreboard captures resolved no map: dumping map region to debug/mapmiss"
                    );
                    tokio::task::spawn_blocking(move || {
                        save_frame_ring(&dir, "mapmiss", &region, MAPMISS_KEEP);
                    });
                }
            } else {
                ctx.empty_map_reads.store(0, Ordering::Relaxed);
            }

            Ok(report)
        }
        Err(miss) => {
            // Recording another row's stats would be worse than recording nothing.
            // A found row whose cells did not parse is not the same failure as a
            // frame where the row itself is missing. CellsUnreadable also fires
            // when the name shows up only in chat or the kill feed.
            let reason = match miss {
                parse::ScoreboardMiss::PlayerRowNotFound if ctx.use_new_reader => {
                    // The new reader finds the row from the own-name seeds,
                    // the highlight and the career hero; `player_name` only
                    // feeds the name fallback, so "set player_name" is not
                    // the fix here. With classic OCR off there was no name
                    // read to fall back on, so the line does not claim one.
                    let name_read = if ocr::gate::is_off() {
                        "classic OCR is off, so the name was not read"
                    } else {
                        "and the name read found none"
                    };
                    tracing::warn!(
                        target: "scuffed_stat_tracker",
                        source = ?own_row.map(|own| own.source.label()),
                        "capture held: own row not found. No own-name seed matched a row, no row is \
                         highlighted and the career panel's hero is on no single row of your team \
                         ({name_read}), or a seed and a clear highlight name \
                         different rows. Nothing is stored for this Tab (saved to debug/rejected \
                         when debug_ocr is on)"
                    );
                    "noplayerrow"
                }
                parse::ScoreboardMiss::PlayerRowNotFound => {
                    tracing::warn!(
                        target: "scuffed_stat_tracker",
                        "capture rejected: player row not found (saved to debug/rejected when debug_ocr is on; \
                     set player_name in config.toml if it is missing)"
                    );
                    "noplayerrow"
                }
                parse::ScoreboardMiss::CellsUnreadable => {
                    // Empty under the new reader: it reads no OCR cell.
                    let cells = player_row_idx.and_then(|i| rows.get(i)).map(|row| {
                        row.stats
                            .iter()
                            .take(6)
                            .map(|cell| cell.value.clone())
                            .collect::<Vec<_>>()
                    });
                    tracing::warn!(
                        target: "scuffed_stat_tracker",
                        ?player_row_idx,
                        ?cells,
                        "capture rejected: stat cells unreadable (saved to debug/rejected when debug_ocr is on)"
                    );
                    "unreadable"
                }
            };
            archive_rejected_map_label_crop(
                ctx.debug_ocr,
                data_dir,
                session_id,
                &frame_img,
                reason,
            );
            save_rejected_frame(ctx.debug_ocr, data_dir, frame_img, reason);
            Ok(CaptureReport {
                best_guess: None,
                recorded: false,
                outcome,
                map: None,
                map_source: None,
                session_id: session_id.to_string(),
                split: false,
                armed_reset: false,
                ignore_row: true,
                reset_streak: req.reset_streak,
                reset_baseline: req.reset_baseline,
                baseline_row: req.baseline_row,
                refresh_baseline: false,
                clear_hint: false,
                count_progress: false,
                career_panel: false,
                held_counters: None,
                held_hero: None,
                gate_state: None,
                hero_auth,
                seal: None,
                close_reason: None,
                held_at: None,
                map_replaces_text_fallback: false,
                rejoin_carry: None,
                finished_game_tab: None,
            })
        }
    }
}
