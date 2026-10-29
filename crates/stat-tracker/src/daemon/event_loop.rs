//! The daemon's event loop: `run_loop` with its poll, keyboard, Tab and
//! signal arms, the race helpers those arms share, and the command tick
//! that applies queued store commands.

use crate::*;

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_loop(
    ctx: Arc<DaemonCtx>,
    early_sigterm: Option<tokio::signal::unix::Signal>,
    // Left with the caller, which settles it before the store's close.
    sync_task: &mut Option<tokio::task::JoinHandle<()>>,
    // Set when the binary on disk was replaced: `run` restarts into it
    // after the store's close.
    restart_to: &mut Option<std::path::PathBuf>,
) -> anyhow::Result<()> {
    // Ergonomic locals over the shared context; spawned tasks clone the Arc.
    let backend = &ctx.backend;
    let store = &ctx.store;
    let sync_client = ctx.sync_client.as_ref();
    let capture_output = ctx.capture_output.as_deref();
    let auto_detect = &ctx.auto_detect;
    let dump_poll_frames = ctx.dump_poll_frames;
    let debug_ocr = ctx.debug_ocr;
    let data_dir: &std::path::Path = &ctx.data_dir;

    let mut game_gate = detect::game_running::GameProcessGate::new(&ctx.game_process_names);
    // Starts false so a boot with the game already closed does not close a
    // share that was never opened. The close runs on the running edge only.
    let mut game_was_running = false;
    // The GUI's Stop button (and systemd) send SIGTERM, shut down as
    // gracefully as Ctrl+C, with a final sync.
    // `early_sigterm` is the same handler, registered before startup's wait
    // for the reader packs (only when that wait ran): a SIGTERM that came
    // since is waiting in it.
    let mut sigterm = match early_sigterm {
        Some(sigterm) => sigterm,
        None => tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?,
    };
    // From here a SIGTERM is a clean shutdown (run.marker removed). Before
    // this line the default action still applies. Tests wait for it.
    tracing::info!(target: "scuffed_stat_tracker", "signal handlers ready");
    tokio::spawn(async {
        let mut term =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(term) => term,
                Err(_) => return,
            };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
        stat_tracker::replace::note_shutdown_signal();
    });
    // Failed open used to wait only on Ctrl+C. SIGTERM was registered above
    // and never polled, so `systemctl stop` hit TimeoutStopSec then SIGKILL,
    // and exit 0 kept Restart=on-failure from trying again. The pid file
    // (acquired in main) made the GUI show "running" the whole time.
    let mut kbd = match open_keyboard_or_shutdown(&mut sigterm).await? {
        Some(stream) => stream,
        None => return Ok(()),
    };

    let poll_interval = auto_detect.poll_interval(ctx.use_new_reader);
    tracing::info!(
        target: "scuffed_stat_tracker",
        poll_ms = poll_interval.as_millis() as u64,
        new_reader = ctx.use_new_reader,
        source = auto_detect.poll_interval_source(),
        "end-screen poll interval"
    );
    let new_game_debounce = std::time::Duration::from_secs(auto_detect.cooldown_secs);
    let finished_close = ctx.finished_game_close;
    let use_new_reader = ctx.use_new_reader;

    // Periodic sync runs as a spawned task so a slow or hung server can't
    // stall Tab capture, polling, or shutdown. Single-flight: while one sync
    // is in the air, the next trigger is skipped (the following one picks up
    // whatever it missed). Shutdown joins that task before the final upload
    // so the two never `mark_synced` the same ids. The client's HTTP timeout
    // bounds both the in-flight wait and the final upload.
    //
    // After a server/network failure the next *periodic* trigger waits on
    // `sync_backoff` (exponential, capped, reset on success). Shutdown does
    // not consult that clock, it still joins the in-flight task, then uploads
    // once. `sync_rev` compare-and-swap stays inside `try_sync_with`.
    let sync_backoff = Arc::new(std::sync::Mutex::new(sync::SyncBackoff::default()));
    if let Some(client) = sync_client {
        let creds = client.credentials();
        match sync::auth_pause_reason(data_dir, &creds.server_url, &creds.token) {
            Some(sync::AuthPauseReason::DeletePending) => {
                sync_backoff
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .record_delete_pending();
                tracing::error!(
                    target: "scuffed_stat_tracker",
                    "{}. Sync resumes when a new token is saved.",
                    sync::SYNC_STATS_DELETE_PENDING_STATUS
                );
            }
            Some(sync::AuthPauseReason::Rejected | sync::AuthPauseReason::Expired) => {
                sync_backoff
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .record_auth_rejected();
                if let Some(status) =
                    sync::auth_pause_status(data_dir, &creds.server_url, &creds.token)
                {
                    tracing::error!(target: "scuffed_stat_tracker", "{status}. Sync resumes when the URL or token changes.");
                }
            }
            None => {}
        }
    }

    // Startup drops a stale skeleton and schedules the upload on `sync_task`.
    // It does not await HTTP. `last_game_open` still comes from
    // `startup_session`, so the debounce survives the restart. SIGTERM waits
    // on `finish_sync_on_shutdown`.
    let mut st = begin_daemon_session(data_dir, sync_task, &sync_backoff, store, sync_client).await;
    if let Some(g) = &st.active_game {
        tracing::info!(
            target: "scuffed_stat_tracker",
            session_id = %g.session_id,
            outcome = %g.outcome,
            "recovered open game from previous run"
        );
    }

    // Tab OCR also runs as a spawned task (single-flight), reporting back on
    // this channel. Awaited inline, one capture starved the poller for a
    // measured 45-70s, long enough to miss the ~3s VICTORY/DEFEAT banner and
    // the whole accolade screen, i.e. the outcome. The 400ms "let the game
    // render the scoreboard" wait sleeps inside the task too.
    let (capture_tx, mut capture_rx) =
        tokio::sync::mpsc::unbounded_channel::<(String, Result<CaptureReport, String>)>();
    let mut capture_task: Option<tokio::task::JoinHandle<()>> = None;
    // The running Tab capture until its report is handled, a newer press
    // waiting for that report, and the last release.
    let mut tabs = stat_tracker::tab_retry::TabKeys::default();
    // A Tab press that preempted a poll, handled by the keyboard arm on the
    // next turn of the loop.
    let mut pending_key: Option<Result<detect::TabKey, Box<dyn std::error::Error>>> = None;
    // The stream hold of a press queued behind a running capture. Dropped
    // when that capture reports and the press does not run.
    let mut queued_stream_hold: Option<capture::KeyHold> = None;
    // Taken as a Tab press preempts a poll, before the poll's grab is
    // dropped, so the stream does not pause and resume in between.
    let mut preempt_stream_hold: Option<capture::KeyHold> = None;

    let mut poll_timer = tokio::time::interval(poll_interval);
    poll_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // GUI command queue (manual outcome edits, session deletion), checked on
    // its own timer so edits apply even while no game is running.
    let mut cmd_timer = tokio::time::interval(tokio::time::Duration::from_secs(3));
    cmd_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut idle_sync_checked = Instant::now();
    let mut binary_checked = tokio::time::Instant::now();
    let mut logged_binary_wait = false;
    let mut exec_failed_at: Option<std::time::Instant> = None;
    let mut seen_stream: Option<capture::SeenStream> = None;
    let mut poll_failures = PollFailureStreak::default();

    loop {
        // `biased` polls in source order. Shutdown is first, so a signal
        // that is already waiting beats a poll tick that is also ready.
        // A capture or OCR already running in this arm is raced with
        // `stop_for_shutdown`. A poll write that has started is not: it
        // finishes inside the shared deadline (`finish_within_shutdown`).
        tokio::select! {
            biased;
            _ = tokio::signal::ctrl_c() => {
                stat_tracker::replace::note_shutdown_signal();
                tracing::info!(target: "scuffed_stat_tracker", "shutting down");
                shutdown_from_signal(
                    &mut st,
                    &mut capture_task,
                    &mut capture_rx,
                    sync_task,
                    sync_client,
                    &sync_backoff,
                    store,
                    data_dir,
                )
                .await;
                return Ok(());
            }
            _ = sigterm.recv() => {
                stat_tracker::replace::note_shutdown_signal();
                tracing::info!(target: "scuffed_stat_tracker", "SIGTERM received, shutting down");
                shutdown_from_signal(
                    &mut st,
                    &mut capture_task,
                    &mut capture_rx,
                    sync_task,
                    sync_client,
                    &sync_backoff,
                    store,
                    data_dir,
                )
                .await;
                return Ok(());
            }
            // The background listener can see Ctrl+C before this loop's
            // own handler. `note_shutdown_signal` wakes this arm.
            _ = stat_tracker::replace::shutdown_notified() => {
                tracing::info!(target: "scuffed_stat_tracker", "shutting down");
                shutdown_from_signal(
                    &mut st,
                    &mut capture_task,
                    &mut capture_rx,
                    sync_task,
                    sync_client,
                    &sync_backoff,
                    store,
                    data_dir,
                )
                .await;
                return Ok(());
            }
            result = pending_or_next(&mut pending_key, kbd.wait_tab()) => {
                // Held from a poll this event preempted. Dropped at the end
                // of this arm, once a press has its own hold, and on a
                // keyboard error too.
                let bridge = preempt_stream_hold.take();
                match result {
                    // A release stops the running capture's retries: a
                    // frame after it shows the game, not the board. It is
                    // kept for a press queued behind that capture.
                    Ok(key) if !key.down => tabs.on_release(key.at),
                    Ok(key) => {
                        // The kernel's key-event time, not when this loop got
                        // round to it (a poll can hold the loop for most of a
                        // second). The stream deadline is measured from it.
                        let press = key.at;
                        let running = game_gate.is_running();
                        capture::note_game_seen(game_was_running, running, data_dir);
                        if capture::share_close_due(
                            capture::stream_route_active(),
                            game_was_running,
                            running,
                            || capture::share_restorable(data_dir),
                        ) {
                            capture::close_if_open().await;
                        }
                        game_was_running = running;
                        if !running {
                            tracing::debug!(target: "scuffed_stat_tracker", "Tab ignored: game process not running");
                            continue;
                        }

                        // A capture whose task is gone without a report (a
                        // panic) must not hold later presses forever. A task
                        // that finished with its report still queued is still
                        // running: that report lands next.
                        let capture_gone = capture_task.as_ref().is_none_or(|t| t.is_finished())
                            && capture_rx.is_empty();
                        if let Some(press) = tabs.on_press(press, st.last_tab_capture, capture_gone) {
                            // Resume a paused screen stream from the key
                            // event, before the session choice below.
                            let hold = capture::stream_key_hold(press);
                            queued_stream_hold = None;
                            let (task, flight) = start_tab_capture(
                                &mut st,
                                &ctx,
                                sync_task,
                                &sync_backoff,
                                &capture_tx,
                                press,
                                hold,
                            )
                            .await;
                            tabs.started(flight);
                            capture_task = Some(task);
                        } else if tabs.queued_press() == Some(press) {
                            // Queued by this press. An ignored or debounced
                            // press leaves an earlier queued hold alone.
                            queued_stream_hold = Some(capture::stream_key_hold(press));
                        }
                        drop(bridge);
                    }
                    Err(e) => {
                        tracing::error!(target: "scuffed_stat_tracker", error = %e, "keyboard devices lost: attempting to reopen");
                        match detect::MultiKeyboardStream::open() {
                            Ok(new_kbd) => {
                                kbd = new_kbd;
                                tracing::info!(target: "scuffed_stat_tracker", "keyboard monitoring reopened");
                            }
                            Err(e2) => {
                                tracing::error!(target: "scuffed_stat_tracker", error = %e2, "failed to reopen keyboard: exiting");
                                drain_capture(capture_task.take()).await;
                                if let Some(client) = sync_client {
                                    drain_sync_then(sync_task, || try_sync(store, client, data_dir, None)).await;
                                } else {
                                    drain_sync_then(sync_task, || std::future::ready(())).await;
                                }
                                // Exit non-zero so Restart=on-failure starts a
                                // new process once a device is readable again.
                                // Exit 0 here looked like a clean stop.
                                anyhow::bail!(
                                    "keyboard devices lost and could not be reopened: {e2}"
                                );
                            }
                        }
                    }
                }
            }
            Some((sid, result)) = capture_rx.recv() => {
                let board_taken = result.as_ref().is_ok_and(CaptureReport::board_taken);
                // A Tab past the grace window of a finished game: its board
                // picked the session (R9-2).
                let sid = settle_tab_probe(&mut st, store, data_dir, sid, &result, Instant::now(), || {
                    schedule_store_sync(sync_task, &sync_backoff, store, sync_client, data_dir, None);
                    std::future::ready(())
                })
                .await;
                // The report handler is the same function the night harness
                // calls. Sync stays here so a slow upload cannot hide inside it.
                if apply_capture_report(&mut st, store, data_dir, &sid, result, Instant::now()).await {
                    st.capture_count += 1;
                    maybe_spawn_periodic_sync(
                        sync_task,
                        &sync_backoff,
                        store,
                        sync_client,
                        data_dir,
                        st.capture_count,
                    );
                    // A board stored after the result (the final scoreboard)
                    // belongs to a finished game: that write is owed even with
                    // no GUI running.
                    if st.active_game.as_ref().is_some_and(|g| g.finished()) {
                        refresh_snapshot_game_finished(store, data_dir).await;
                    } else {
                        refresh_snapshot(store, data_dir).await;
                    }
                }
                // A Tab pressed while that capture ran runs now, unless the
                // capture took its board (it would read the same board). A
                // taken board starts the debounce; a refused one does not.
                let queued_hold = queued_stream_hold.take();
                if let Some(press) = tabs.on_report(&mut st.last_tab_capture, board_taken) {
                    let hold = queued_hold.unwrap_or_else(|| capture::stream_key_hold(press));
                    let (task, flight) = start_tab_capture(
                        &mut st,
                        &ctx,
                        sync_task,
                        &sync_backoff,
                        &capture_tx,
                        press,
                        hold,
                    )
                    .await;
                    tabs.started(flight);
                    capture_task = Some(task);
                }
            }
            _ = poll_timer.tick(), if auto_detect.enabled => {
                // Mid-match: a Tab capture saturates the OCR pool, skip the
                // full poll so we do not contend (H3). During end-reel wake
                // that skip starves the ~3s Victory/Defeat window (2026-09-06:
                // Tab held, wake set, no poll result word). Run a cheap
                // outcome-only path instead; do not adopt Tab noscoreboard
                // Victory here (explicit follow-up).
                let tab_in_flight = capture_task.as_ref().is_some_and(|t| !t.is_finished());
                let now = Instant::now();
                if skip_poll_for_tab_in_flight(tab_in_flight, &st, now) {
                    tracing::debug!(target: "scuffed_stat_tracker", "poll tick skipped: Tab capture in flight");
                    continue;
                }
                let outcome_only = poll_outcome_only_while_tab_busy(tab_in_flight, &st, now);
                if outcome_only {
                    tracing::debug!(
                        target: "scuffed_stat_tracker",
                        "cheap outcome poll: Tab capture in flight during end-reel wake"
                    );
                }
                let running = game_gate.is_running();
                capture::note_game_seen(game_was_running, running, data_dir);
                if capture::share_close_due(
                    capture::stream_route_active(),
                    game_was_running,
                    running,
                    || capture::share_restorable(data_dir),
                ) {
                    capture::close_if_open().await;
                }
                game_was_running = running;
                if !running {
                    clear_cadence_wakes(&mut st);
                    st.ocr_stability.reset();
                    poll_failures.game_stopped();
                    continue;
                }

                // PR-B: adaptive cadence, mid-match, pay the screencopy only
                // on every SLOW_POLL_DIVISORth tick. The skipped ticks cost
                // nothing (no capture, no scans); full cadence resumes the
                // moment any end/start evidence lands in session state.
                // On Older screenshot and KDE direct the same skip runs out
                // of a match too, down to IDLE_SHOT_POLL_FLOOR.
                let divisor = poll_divisor(&st, Instant::now(), capture::shot_route(), poll_interval);
                if divisor > 1 {
                    st.poll_ticks_skipped += 1;
                    if st.poll_ticks_skipped < divisor {
                        tracing::trace!(target: "scuffed_stat_tracker", divisor, "poll tick skipped: adaptive slow cadence");
                        continue;
                    }
                }

                // A Tab press preempts the poll's grab and analysis: the poll
                // is dropped (the next tick polls again) and the press is
                // kept for the keyboard arm, never lost. A release does not
                // preempt (`next_press` records it), so the outcome-only
                // poll in the ~3 s end-reel window survives letting go of Tab.
                // On a screenshot route the end-reel outcome-only poll waits
                // up to OUTCOME_POLL_LOCK_WAIT for a Tab grab's shot lock;
                // an ordinary poll that finds it held is skipped at once.
                let mut bridge = None;
                let grabbed = match stop_for_shutdown_or_key(
                    capture::poll_grab(backend, capture_output, outcome_only),
                    stat_tracker::replace::shutdown_notified(),
                    with_bridge_hold(
                        stat_tracker::tab_retry::next_press(&mut kbd, &mut tabs),
                        &mut bridge,
                        capture::stream_bridge_hold,
                    ),
                    &mut pending_key,
                )
                .await
                {
                    PollRace::Done(grabbed) => Ok(grabbed),
                    PollRace::Shutdown => Err(()),
                    PollRace::Preempted => {
                        preempt_stream_hold = bridge.take();
                        tracing::debug!(target: "scuffed_stat_tracker", "poll dropped during its grab: a Tab press comes first");
                        continue;
                    }
                };
                // Reset only for a poll that ran its grab: a dropped one, or
                // one turned away by a held screenshot lock, leaves the slow
                // cadence due on the next tick.
                if poll_resets_cadence(&grabbed) {
                    st.poll_ticks_skipped = 0;
                }
                match grabbed {
                    Err(()) => {
                        shutdown_from_signal(
                            &mut st,
                            &mut capture_task,
                            &mut capture_rx,
                            sync_task,
                            sync_client,
                            &sync_backoff,
                            store,
                            data_dir,
                        )
                        .await;
                        return Ok(());
                    }
                    Ok(Ok(captured)) => {
                        if let Some((failed, over)) = poll_failures.recovered(Instant::now()) {
                            tracing::info!(
                                target: "scuffed_stat_tracker",
                                failed,
                                secs = over.as_secs(),
                                "poll capture works again"
                            );
                        }
                        if capture::reused_frame_skips_the_read(
                            captured.is_new,
                            captured.seq,
                            captured.stream,
                            seen_stream,
                        ) {
                            log_reused_frame_skip();
                            continue;
                        }
                        // Recorded only once this frame is read: a poll that
                        // a Tab drops leaves the frame unread.
                        let kept_frame = match (captured.stream, captured.seq) {
                            (Some(id), Some(seq)) => Some(capture::SeenStream { id, seq }),
                            _ => None,
                        };
                        let img = captured.image;
                        let dump_dir = dump_poll_frames.then(|| data_dir.join("debug").join("poll"));
                        // On-hit evidence (debug_ocr): confirm + first streak only.
                        // `--dump-poll-frames` still writes every tick as `poll_*`.
                        let on_hit_dir = debug_ocr.then(|| data_dir.join("debug").join("poll"));
                        let prior_streak = st
                            .word_outcome_streak
                            .as_ref()
                            .map(|streak| (streak.outcome, streak.seen_at.elapsed()));
                        // A copy: a poll that a Tab drops leaves the
                        // session's stability history as it was.
                        let mut stability = st.ocr_stability.clone();
                        // Dropping this join detaches the blocking OCR. That
                        // task does not write a match row. The row is written
                        // below, and only when shutdown has not begun and no
                        // Tab key event preempted it.
                        let analyzed = stop_for_shutdown_or_key(
                            tokio::task::spawn_blocking(move || {
                            let analyze_started = Instant::now();
                            if let Some(dir) = &dump_dir {
                                save_frame_ring(dir, "poll", &img, POLL_DUMP_KEEP);
                            }
                            // One RGBA→RGB conversion shared by the outcome + phase
                            // detectors (P6); title OCR still uses the original frame.
                            let rgb = img.to_rgb8();
                            // PR-A: the polled variants gate their OCR on the
                            // crop holding still across consecutive ticks,
                            // combat frames stop paying a Tesseract call per
                            // tick; static post-match/phase screens still read
                            // (one tick later at most).
                            // `reader = "new"` reads the end title from the
                            // words-v1 pack, once per tick, for the outcome
                            // signal and for the wake, and no OCR reads it
                            // (alpha.7 O2). A first sure read only wakes;
                            // the same word on the next tick is the signal.
                            // With no end-title templates loaded the titles
                            // fall back to OCR, as before O2. Only that
                            // fallback still reads the FINAL SCORE line (its
                            // gate), with the words-v1 template (alpha.7 O3).
                            let titles = detect::match_end::Titles::for_rgb(use_new_reader, &rgb);
                            // And it reads the accolade and rank-screen
                            // result word from the words-v1 pack, behind the
                            // same lit-crop gate and stability wait as the
                            // OCR it replaces (alpha.7 O11).
                            let words = detect::match_end::ResultWords::for_reader(use_new_reader);
                            let signal = detect::match_end::detect_outcome_signal_polled(
                                &img,
                                &rgb,
                                &mut stability,
                                detect::match_end::FinalScoreSource::for_reader(use_new_reader),
                                &titles,
                                &words,
                            );
                            // Cheap wake path: while Tab cell OCR is in
                            // flight, skip the phase and accolade-map reads
                            // so it keeps the pool. Outcome signal and
                            // end-reel refresh still run (same confirm /
                            // on-hit dump machinery below). Such a tick
                            // reads the outcome word but not the map, so two
                            // of them confirm with no map and seal onto the
                            // open session. Otherwise only a result-word
                            // tick reads the map ([`choose_accolade_map`]).
                            let accolade_map = choose_accolade_map(
                                use_new_reader,
                                outcome_only,
                                signal.map(|(_, source)| source),
                                || shadow::read_accolade_map(&rgb),
                                || detect::match_end::read_accolade_map(&img),
                            );
                            // `reader = "new"` confirms the vote, ban and
                            // select screens with the words-v1 phrase masks,
                            // not OCR (alpha.7 O7).
                            let phase = if outcome_only {
                                detect::GamePhase::Unknown
                            } else {
                                detect::match_start::detect_phase_polled(
                                    &img,
                                    &rgb,
                                    &mut stability,
                                    detect::match_start::PhaseSource::for_reader(use_new_reader),
                                )
                            };
                            // Wake hint only, does not confirm an outcome.
                            // Ban Heroes is a distinct hook (`detect_ban_screen`)
                            // and is excluded inside detect_end_reel; do not
                            // treat GamePhase::HeroBan as end_reel_wake_until.
                            // `reader = "new"`: a sure end title wakes, and
                            // the title card's phrase OCR does not run.
                            let end_reel =
                                detect::match_end::detect_end_reel_with(&img, &rgb, &titles);
                            // Result templates are not on the Tab frame. A
                            // polled outcome frame feeds the new reader's
                            // result. The read the signal came from is that
                            // evidence: the frame is not read twice (O11).
                            let result_read = tick_result_read(
                                use_new_reader,
                                signal.is_some(),
                                words.into_taken(),
                                || shadow::read_result(&img),
                            );
                            if let Some(dir) = &on_hit_dir
                                && let Some((kind, outcome)) =
                                    poll_debug_hit(signal, prior_streak, OUTCOME_CONFIRM_WINDOW)
                            {
                                save_frame_ring(
                                    dir,
                                    &poll_debug_prefix(kind, outcome),
                                    &img,
                                    POLL_DUMP_KEEP,
                                );
                            }
                            // Report kit (spec section 3): the rank screen's
                            // word box, cut from this same frame, only when
                            // the rank screen detector fired on it.
                            let rank_outcome_png = stat_tracker::report::kit::rank_outcome_png(
                                signal.map(|(_, source)| source),
                                &img,
                            );
                            stat_tracker::perf::record(
                                stat_tracker::perf::Stage::PollAnalyze,
                                analyze_started.elapsed(),
                            );
                            (signal, phase, accolade_map, end_reel, stability, result_read, rank_outcome_png)
                        }),
                            stat_tracker::replace::shutdown_notified(),
                            stat_tracker::tab_retry::next_press(&mut kbd, &mut tabs),
                            &mut pending_key,
                        )
                        .await;
                        let analyzed = match analyzed {
                            PollRace::Done(joined) => Ok(joined),
                            PollRace::Shutdown => Err(()),
                            PollRace::Preempted => {
                                tracing::debug!(target: "scuffed_stat_tracker", "poll dropped during its read: a Tab press comes first");
                                continue;
                            }
                        };
                        if let Some(seen) = kept_frame {
                            seen_stream = Some(seen);
                        }
                        let (signal, phase, accolade_map, end_reel, stability, result_read, rank_outcome_png) = match analyzed {
                            Err(()) => {
                                shutdown_from_signal(
                                    &mut st,
                                    &mut capture_task,
                                    &mut capture_rx,
                                    sync_task,
                                    sync_client,
                                    &sync_backoff,
                                    store,
                                    data_dir,
                                )
                                .await;
                                return Ok(());
                            }
                            Ok(joined) => joined.unwrap_or_else(|_| {
                                (None, detect::GamePhase::Unknown, None, false, detect::stability::FrameStability::default(), None, None)
                            }),
                        };
                        if stat_tracker::replace::shutdown_requested() {
                            shutdown_from_signal(
                                &mut st,
                                &mut capture_task,
                                &mut capture_rx,
                                sync_task,
                                sync_client,
                                &sync_backoff,
                                store,
                                data_dir,
                            )
                            .await;
                            return Ok(());
                        }
                        if let Some(png) = rank_outcome_png
                            && let Some(game) = st.active_game.as_ref()
                            // the rank screen follows the game it belongs
                            // to: a game quiet for longer gets no outcome crop
                            && game.last_activity.elapsed() <= RANK_OUTCOME_MAX_IDLE
                            && let Err(e) = stat_tracker::report::kit::record_outcome_png(
                                data_dir,
                                &game.session_id,
                                &png,
                            )
                        {
                            tracing::debug!(target: "scuffed_stat_tracker", error = %e, "report kit: outcome crop not written");
                        }
                        st.ocr_stability = stability;
                        if let Some(read) = result_read
                            && let Some(game) = st.active_game.as_mut()
                        {
                            game.result_evidence.add(read);
                        }

                        if end_reel {
                            let already_awake = st
                                .end_reel_wake_until
                                .is_some_and(|t| Instant::now() < t);
                            st.end_reel_wake_until = Some(Instant::now() + END_REEL_WAKE);
                            if !already_awake {
                                tracing::info!(
                                    target: "scuffed_stat_tracker",
                                    hold_secs = END_REEL_WAKE.as_secs(),
                                    "end-reel / POTG: holding full poll cadence"
                                );
                            }
                        }

                        // Every outcome (end title, accolade or rank screen)
                        // needs a second agreeing read within the
                        // confirmation window; no source acts on one tick
                        // (0.5.0-rc.9). The streak is recorded before
                        // the decision and carries a map only inside that
                        // window. [`resolve_word_tick`] is the same rule the
                        // night harness runs.
                        let now = Instant::now();
                        let tick = resolve_word_tick(
                            &mut st.word_outcome_streak,
                            signal,
                            now,
                            accolade_map,
                        );

                        // Boundaries live in `boundary::decide_poll`. A map vote
                        // or a hero ban after a board-followed hint splits and
                        // seals it. A hero select only arms. A confirmed word
                        // on a trusted different map opens the next session.
                        // A start screen still inside the debounce does not
                        // open another session.
                        let start_screen = match &phase {
                            detect::GamePhase::MapVote { maps } => {
                                let candidates: Vec<String> = maps
                                    .iter()
                                    .filter_map(|m| parse::canonical_map(m))
                                    .collect();
                                Some(boundary::StartScreen::MapVote { candidates })
                            }
                            detect::GamePhase::HeroSelect => Some(boundary::StartScreen::HeroSelect),
                            detect::GamePhase::HeroBan => Some(boundary::StartScreen::HeroBan),
                            _ => None,
                        };
                        // `retire_active_game` takes the game out of memory
                        // before the store calls return. Dropping this future
                        // on the signal would cancel the best-guess or the
                        // sealed outcome, and shutdown would have nothing
                        // left to save. The write runs to completion inside
                        // whatever of the shared deadline is left, then the
                        // loop shuts down.
                        let wrote = finish_within_shutdown(
                            async {
                                apply_poll_tick(
                                    &mut st,
                                    store,
                                    data_dir,
                                    &tick,
                                    start_screen,
                                    now,
                                    new_game_debounce,
                                    use_new_reader,
                                )
                                .await;
                            },
                            stat_tracker::replace::shutdown_notified(),
                            stat_tracker::replace::shutdown_time_left,
                        )
                        .await;
                        if wrote.is_err() || stat_tracker::replace::shutdown_requested() {
                            shutdown_from_signal(
                                &mut st,
                                &mut capture_task,
                                &mut capture_rx,
                                sync_task,
                                sync_client,
                                &sync_backoff,
                                store,
                                data_dir,
                            )
                            .await;
                            return Ok(());
                        }
                    }
                    Ok(Err(e)) => {
                        // The game gate ran first: the game is running. A
                        // known state (a held lock, a share opening, declined
                        // or closed) is no failure and has its own log line.
                        match poll_failures.on_error(&e.to_string(), Instant::now()) {
                            PollErrorLog::Expected => {
                                tracing::trace!(target: "scuffed_stat_tracker", error = %e, "poll capture skipped");
                            }
                            PollErrorLog::Warn => {
                                tracing::warn!(
                                    target: "scuffed_stat_tracker",
                                    error = %e,
                                    failures = poll_failures.count,
                                    "poll capture failed while the game runs; repeats are logged at most every 5 minutes"
                                );
                            }
                            PollErrorLog::Trace => {
                                tracing::trace!(target: "scuffed_stat_tracker", error = %e, failures = poll_failures.count, "poll capture failed");
                            }
                        }
                    }
                }
            }
            _ = cmd_timer.tick() => {
                // Auto-detect off never ticks the poll arm, so the share
                // still closes here when the game stops. The same edge, and
                // only when a restore token can reopen it.
                let running = game_gate.is_running();
                capture::note_game_seen(game_was_running, running, data_dir);
                if capture::share_close_due(
                    capture::stream_route_active(),
                    game_was_running,
                    running,
                    || capture::share_restorable(data_dir),
                ) {
                    capture::close_if_open().await;
                }
                game_was_running = running;
                if binary_checked.elapsed() >= std::time::Duration::from_secs(60) {
                    binary_checked = tokio::time::Instant::now();
                    if let Some(path) = stat_tracker::replace::replaced_binary_path() {
                        if st.active_game.is_some() {
                            if !logged_binary_wait {
                                tracing::info!(
                                    target: "scuffed_stat_tracker",
                                    "tracker binary on disk was replaced; waiting until the current game is saved"
                                );
                                logged_binary_wait = true;
                            }
                        } else if !stat_tracker::replace::exec_retry_due(
                            exec_failed_at,
                            std::time::Instant::now(),
                            stat_tracker::replace::EXEC_RETRY_BACKOFF,
                        ) {
                            tracing::debug!(
                                target: "scuffed_stat_tracker",
                                "tracker binary on disk was replaced; waiting to recheck it after it failed the start check"
                            );
                        } else if let Err(err) =
                            stat_tracker::replace::restart_preflight(&path)
                        {
                            // Checked while the store is still open: this
                            // process keeps running and tries again later.
                            tracing::error!(
                                target: "scuffed_stat_tracker",
                                error = %err,
                                "cannot start the new tracker; keeping this process"
                            );
                            exec_failed_at = Some(std::time::Instant::now());
                        } else {
                            shutdown_from_signal(
                                &mut st,
                                &mut capture_task,
                                &mut capture_rx,
                                sync_task,
                                sync_client,
                                &sync_backoff,
                                store,
                                data_dir,
                            )
                            .await;
                            // `run` closes the store, then exits or execs.
                            *restart_to = Some(path);
                            return Ok(());
                        }
                    } else {
                        logged_binary_wait = false;
                        exec_failed_at = None;
                    }
                }
                // Quiet-session timer. Runs on this existing tick so it does
                // not take a screenshot or run OCR, and it does not wait for
                // Tab. The upload is scheduled; it does not block the loop.
                close_quiet_session_and_sync(
                    &mut st,
                    store,
                    data_dir,
                    Instant::now(),
                    finished_close,
                    || {
                        schedule_store_sync(
                            sync_task,
                            &sync_backoff,
                            store,
                            sync_client,
                            data_dir,
                            None,
                        );
                        std::future::ready(())
                    },
                )
                .await;
                schedule_settled_unknown(
                    &mut st,
                    sync_task,
                    &sync_backoff,
                    store,
                    sync_client,
                    data_dir,
                );
                maybe_resume_sync_after_settings_change(sync_client, &sync_backoff, data_dir);
                // Suspend detection (m4): after a sleep, every Instant-based
                // window believes no time passed. Treat resume like a daemon
                // restart, drop the volatile windows and re-admit the active
                // game only through the wall-clock recovery bound (the on-disk
                // skeleton was persisted with correct wall times pre-suspend).
                // A skeleton past that bound is dropped. Decided rows upload.
                // Unknown rows for that session are held locally.
                let mono = st.suspend_probe.0.elapsed();
                let wall = (Utc::now() - st.suspend_probe.1).to_std().unwrap_or(mono);
                if wall > mono + SUSPEND_RESET_GAP {
                    tracing::info!(
                        target: "scuffed_stat_tracker",
                        gap_secs = (wall - mono).as_secs(),
                        "suspend/clock-jump detected: resetting session windows"
                    );
                    // Schedule the upload, including a stale skeleton. Then
                    // `resume_after_suspend` re-admits through `startup_session`,
                    // so the debounce comes back from `opened_at`. HTTP does
                    // not run on this tick.
                    let restored = begin_daemon_session(
                        data_dir,
                        sync_task,
                        &sync_backoff,
                        store,
                        sync_client,
                    )
                    .await;
                    adopt_startup_session(&mut st, restored);
                }
                st.suspend_probe = (Instant::now(), Utc::now());

                apply_queued_commands(
                    &mut st,
                    store,
                    data_dir,
                    sync_task,
                    &sync_backoff,
                    sync_client,
                )
                .await;
                maybe_spawn_idle_sync(
                    sync_task,
                    &sync_backoff,
                    store,
                    sync_client,
                    data_dir,
                    &mut idle_sync_checked,
                    Instant::now(),
                )
                .await;
            }
        }
    }
}

/// One command tick of the GUI queue. A SetOutcome or a delete schedules
/// an upload on this same tick, so a deleted game leaves the site right
/// away instead of at the next Tab sync or daemon restart (O3). An edit
/// does not; the idle sync tick picks it up.
pub(crate) async fn apply_queued_commands(
    st: &mut SessionState,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    sync_task: &mut Option<tokio::task::JoinHandle<()>>,
    sync_backoff: &Arc<std::sync::Mutex<sync::SyncBackoff>>,
    sync_client: Option<&sync::SyncClient>,
) {
    let cmds = storage::read_commands(data_dir);
    if cmds.is_empty() {
        flush_snapshot_if_due(store, data_dir).await;
        return;
    }
    for (cmd_file, cmd) in &cmds {
        tracing::info!(target: "scuffed_stat_tracker", ?cmd, "applying GUI command");
        match cmd {
            storage::StoreCommand::SetOutcome {
                session_id,
                outcome,
            } => {
                if let Some(g) = st.active_game.as_mut()
                    && g.session_id == *session_id
                {
                    g.record_member_outcome(
                        outcome.parse().unwrap_or(detect::MatchOutcome::Unknown),
                    );
                    persist_active_game(data_dir, Some(g));
                }
            }
            storage::StoreCommand::DeleteSession { session_id } => {
                // A deleted game's report kit goes too (QC1-6).
                stat_tracker::report::kit::remove(data_dir, session_id);
                if st
                    .active_game
                    .as_ref()
                    .is_some_and(|g| g.session_id == *session_id)
                {
                    st.active_game = None;
                    persist_active_game(data_dir, None);
                }
            }
            storage::StoreCommand::EditMatch { session_id, edit } => {
                if let Some(g) = st.active_game.as_mut()
                    && g.session_id == *session_id
                    && let Some(outcome) = &edit.outcome
                {
                    g.record_member_outcome(
                        outcome.parse().unwrap_or(detect::MatchOutcome::Unknown),
                    );
                    persist_active_game(data_dir, Some(g));
                }
            }
            storage::StoreCommand::ResolveSegment { .. }
            | storage::StoreCommand::RetryUpload { .. }
            | storage::StoreCommand::RetryRejectedUploads => {}
        }
        match store.apply_command(cmd).await {
            Ok(()) => {
                storage::remove_command_file(cmd_file);
                if matches!(
                    cmd,
                    storage::StoreCommand::SetOutcome { .. }
                        | storage::StoreCommand::DeleteSession { .. }
                        | storage::StoreCommand::RetryUpload { .. }
                        | storage::StoreCommand::RetryRejectedUploads
                ) {
                    schedule_store_sync(
                        sync_task,
                        sync_backoff,
                        store,
                        sync_client,
                        data_dir,
                        None,
                    );
                }
            }
            Err(e) => {
                tracing::warn!(target: "scuffed_stat_tracker", error = %e, "GUI command failed; will retry");
            }
        }
    }
    refresh_snapshot(store, data_dir).await;
}

/// How a poll's grab or read ended when raced against shutdown and the
/// keyboard.
pub(crate) enum PollRace<T> {
    Done(T),
    Shutdown,
    /// A Tab press arrived first. The work was dropped and the press is in
    /// the caller's `pending` slot.
    Preempted,
}

/// Race `work` against shutdown and the next Tab press. Shutdown is polled
/// first, so a signal that is already waiting wins even when `work` is also
/// ready. Finished work beats a ready key (its result is applied and the
/// key waits in the channel). Otherwise the key wins, it is stored in
/// `pending` for the keyboard arm, and `work` is dropped, so a Tab never
/// waits behind a poll and is never lost.
///
/// This is for the poll's capture grab and OCR only. Dropping `work`
/// detaches a `spawn_blocking` task. That OCR keeps running and its result
/// is discarded. It does not write a match row. A future that writes game
/// state goes through [`finish_within_shutdown`] instead: dropping it would
/// cancel a store call that had already taken the game out of memory.
pub(crate) async fn stop_for_shutdown_or_key<T, K, S, KF>(
    work: impl std::future::Future<Output = T>,
    shutdown: S,
    key: KF,
    pending: &mut Option<K>,
) -> PollRace<T>
where
    S: std::future::Future<Output = ()>,
    KF: std::future::Future<Output = K>,
{
    tokio::pin!(work);
    tokio::pin!(shutdown);
    tokio::pin!(key);
    tokio::select! {
        biased;
        _ = &mut shutdown => PollRace::Shutdown,
        value = &mut work => PollRace::Done(value),
        key = &mut key => {
            *pending = Some(key);
            PollRace::Preempted
        }
    }
}

/// `key`, and a stream hold taken the moment a press comes in, before the
/// race that awaits this drops the poll's grab (and the grab's hold). A
/// keyboard error takes none.
pub(crate) async fn with_bridge_hold<K, E, H>(
    key: impl std::future::Future<Output = Result<K, E>>,
    slot: &mut Option<H>,
    hold: impl FnOnce() -> H,
) -> Result<K, E> {
    let key = key.await;
    if key.is_ok() {
        *slot = Some(hold());
    }
    key
}

/// Run a Tab's retry sequence, then end the press's stream hold
/// ([`capture::end_tab_stream_hold`]): no grab follows, so the stream may
/// pause while the result is analysed.
///
/// Not an async fn: an async wrapper keeps the retries future in its own
/// state as well as the awaited copy, which doubled that part of
/// `handle_capture`'s future (23,280 B in a debug build). `inspect` wraps
/// it in place.
pub(crate) fn then_end_tab_hold<F: std::future::Future>(
    retries: F,
) -> impl std::future::Future<Output = F::Output> {
    use futures_util::FutureExt as _;
    retries.inspect(|_| {
        capture::end_tab_stream_hold();
    })
}

/// The key event a dropped poll kept, else the next one from `next`. A kept
/// event returns on the first poll, so the keyboard arm takes it at once.
pub(crate) async fn pending_or_next<K>(
    pending: &mut Option<K>,
    next: impl std::future::Future<Output = K>,
) -> K {
    match pending.take() {
        Some(key) => key,
        None => next.await,
    }
}

/// [`stop_for_shutdown_or_key`] with no key: the shutdown race alone.
#[cfg(test)]
pub(crate) async fn stop_for_shutdown<T, S>(
    work: impl std::future::Future<Output = T>,
    shutdown: S,
) -> Result<T, ()>
where
    S: std::future::Future<Output = ()>,
{
    let mut never: Option<()> = None;
    match stop_for_shutdown_or_key(work, shutdown, std::future::pending::<()>(), &mut never).await {
        PollRace::Done(value) => Ok(value),
        PollRace::Shutdown => Err(()),
        PollRace::Preempted => unreachable!("a pending key never arrives"),
    }
}

/// Let a game-state write finish. If `shutdown` fires first, `work` keeps
/// running until `budget` elapses. The budget is read when the signal
/// fires, so it is the shared deadline's remainder (`shutdown_time_left`),
/// not a zero timeout captured before the signal.
///
/// `Err` means the deadline passed with the write still running. That drop
/// is the same bound as the rest of shutdown. A signal by itself does not
/// drop the write.
pub(crate) async fn finish_within_shutdown<T, S, B>(
    work: impl std::future::Future<Output = T>,
    shutdown: S,
    budget: B,
) -> Result<T, ()>
where
    S: std::future::Future<Output = ()>,
    B: FnOnce() -> std::time::Duration,
{
    tokio::pin!(work);
    tokio::pin!(shutdown);
    tokio::select! {
        biased;
        _ = &mut shutdown => match tokio::time::timeout(budget(), &mut work).await {
            Ok(value) => Ok(value),
            Err(_elapsed) => Err(()),
        },
        value = &mut work => Ok(value),
    }
}
