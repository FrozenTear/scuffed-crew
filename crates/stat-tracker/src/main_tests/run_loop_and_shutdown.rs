//! Daemon tests: run_loop_and_shutdown. See `mod.rs`.

use super::*;

/// P1c. A start that the pid guard is going to refuse must not reach
/// the site or the pack files first, and the reader pack job must be
/// started for every signed-in daemon, not only when the startup sync
/// got a list. The crash marker stays where it was: after the guard and
/// after the vacuum exit.
#[test]
fn startup_takes_the_pid_guard_before_the_site_and_always_starts_the_pack_keeper() {
    let src = include_str!("../main.rs");
    let begin = src
        .find("    lock_down_umask();\n")
        .expect("startup begins");
    let body = &src[begin..];
    let end = body
        .find("\nfn handle_preinit_flags()")
        .expect("startup ends before handle_preinit_flags");
    let body = &body[..end];
    let at = |needle: &str| {
        assert_eq!(body.matches(needle).count(), 1, "{needle} once in startup");
        body.find(needle).expect("counted above")
    };
    // The call of the run loop, without its arguments: another lane
    // adds one.
    const RUN_LOOP: &str = "let result = run_loop(";
    let guard = at("acquire_pid_guard(&config.data_dir)?");
    for later in [
        "open_sync_client(",
        "auth_pause_status(",
        "fetch_player_name_if_needed(",
        "packs::sync_on_startup(",
        "maybe_vacuum(",
        "marker::start_run(",
        "startup_maintenance(",
        "open_store(",
        "packs::PackKeeper::new(",
        RUN_LOOP,
    ] {
        assert!(guard < at(later), "{later} must come after the pid guard");
    }
    // The run loop is awaited in place, whatever arguments it takes.
    let run_loop = at(RUN_LOOP);
    let call = body[run_loop..].split(';').next().expect("a statement");
    assert!(call.ends_with(".await"), "{call}");
    assert!(at("maybe_vacuum(") < at("marker::start_run("));
    // S5-CAP-8: perf.log starts after the guard and the vacuum exit.
    assert!(guard < at("perf_log::init("));
    assert!(at("maybe_vacuum(") < at("perf_log::init("));
    assert!(at("packs::sync_on_startup(") < at("marker::start_run("));
    // The keeper is made after the paused / not paused branches have
    // both closed, so it does not depend on what the startup sync
    // returned or on whether one ran.
    let (sync, keeper) = (at("packs::sync_on_startup("), at("packs::PackKeeper::new("));
    assert!(sync < keeper);
    assert!(
        body[sync..keeper].contains("\n        }\n"),
        "the keeper must be made outside the startup sync's branch: {}",
        &body[sync..keeper]
    );
    let spawn =
        at("    if let Some(extras) = extra_packs {\n        tokio::spawn(extras.run());\n");
    assert!(keeper < spawn && spawn < run_loop);
}

/// Deleting the command-tick call, or the upload inside the timer, fails
/// this test even if a lookalike helper is left behind.
#[test]
fn daemon_loop_runs_the_quiet_close_timer_and_its_sync() {
    let src = include_str!("../main.rs");
    // Bound the slice to `run_loop` itself. The night harness and this
    // test both mention the same names later in the file; an unbounded
    // search would match those and stay green after the timer was deleted.
    let run_at = src.find("async fn run_loop").expect("run_loop");
    let after = &src[run_at..];
    let end = after
        .find("\nfn analyze_frame(")
        .expect("run_loop ends before analyze_frame");
    let run_loop = &after[..end];
    let call = run_loop
        .find("close_quiet_session_and_sync(")
        .expect("the command tick must call the quiet-close timer");
    let window = &run_loop[call..call + 800];
    assert!(
        window.contains("schedule_store_sync("),
        "the command tick must schedule the upload instead of awaiting it"
    );
    assert!(
        !window.contains("finish_sync_on_shutdown("),
        "the command tick must not await the shutdown upload"
    );
    let def = src
        .find("async fn close_quiet_session_and_sync")
        .expect("quiet-close timer");
    let fn_body = src[def..].split("\nasync fn ").next().unwrap();
    assert!(
        fn_body.contains("sync_now().await"),
        "deleting the sync call inside the quiet-close timer must fail"
    );
    assert!(
        fn_body.contains("persist_active_game(data_dir, None)"),
        "a restart must see the session already closed"
    );
    assert!(
        run_loop.contains("begin_daemon_session("),
        "startup must drop a stale skeleton and schedule its upload"
    );
    let startup = run_loop.find("begin_daemon_session(").expect("startup");
    let loop_at = run_loop.find("loop {").expect("daemon loop");
    assert!(
        startup < loop_at,
        "startup sync is scheduled before the loop"
    );
    let restore = src
        .find("async fn recover_or_sync_active_game")
        .expect("startup restore");
    let restore_body = src[restore..].split("\nasync fn ").next().unwrap();
    assert!(
        restore_body.contains("startup_session(data_dir)"),
        "startup sync must restore through startup_session"
    );
    let suspend = run_loop
        .find("suspend/clock-jump detected")
        .expect("suspend arm");
    let after_suspend = &run_loop[suspend..];
    let adopt_at = after_suspend
        .find("adopt_startup_session(")
        .expect("suspend must adopt the session begin_daemon_session restored");
    assert!(
        after_suspend[..adopt_at].contains("begin_daemon_session("),
        "a stale skeleton dropped on resume must be scheduled for sync"
    );
    assert!(
        !after_suspend[..adopt_at].contains("finish_sync_on_shutdown("),
        "resume must not await the upload"
    );
    let ctrl_c = run_loop
        .find("tracing::info!(\"shutting down\")")
        .expect("ctrl-c shutdown");
    let sigterm = run_loop
        .find("tracing::info!(\"SIGTERM received, shutting down\")")
        .expect("sigterm shutdown");
    assert!(ctrl_c < sigterm, "ctrl-c is registered before sigterm");
    assert!(
        run_loop[ctrl_c..sigterm].contains("shutdown_from_signal("),
        "Ctrl+C must close the session through shutdown_from_signal"
    );
    assert!(
        run_loop[sigterm..].contains("shutdown_from_signal("),
        "SIGTERM must close the session through shutdown_from_signal"
    );
    let shutdown_fn = src
        .find("async fn shutdown_from_signal(")
        .expect("shutdown helper");
    let shutdown_body = src[shutdown_fn..].split("\nasync fn ").next().unwrap();
    assert!(
        shutdown_body.contains("close_session_on_shutdown("),
        "each signal handler closes the session before the final upload"
    );
    let startup_fn = src
        .find("fn begin_daemon_session(")
        .expect("startup helper");
    let startup_body = src[startup_fn..].split("\nasync fn ").next().unwrap();
    assert!(
        startup_body.contains("dropped.clone()"),
        "a stale skeleton on the next start must settle that session's unknown rows"
    );
    assert!(
        run_loop.contains("schedule_settled_unknown("),
        "the command tick must settle the session a boundary just closed"
    );
    assert!(
        run_loop.contains("apply_queued_commands("),
        "the command tick must run the extracted command body"
    );
    let settle_fn = src
        .find("fn schedule_settled_unknown(")
        .expect("settle helper");
    let settle_body = src[settle_fn..]
        .split("\nasync fn apply_queued_commands(")
        .next()
        .unwrap();
    assert!(
        settle_body.contains("settle_closed_unknown.take()"),
        "the command tick must consume the closed session id"
    );
    assert!(
        settle_body.contains("schedule_store_sync("),
        "the command tick must schedule the settle"
    );
    let commands_fn = src
        .find("async fn apply_queued_commands(")
        .expect("command tick body");
    let commands_body = src[commands_fn..]
        .split("\nasync fn begin_daemon_session(")
        .next()
        .unwrap();
    let set_at = commands_body
        .find("StoreCommand::SetOutcome {")
        .expect("SetOutcome arm");
    assert!(
        commands_body[set_at..].contains("schedule_store_sync("),
        "a SetOutcome on the open session must schedule the next sync"
    );
    let biased = run_loop.find("biased;").expect("biased select");
    assert!(
        biased < ctrl_c,
        "shutdown is the first arm, so a ready signal beats a ready poll"
    );
    let poll_tick = run_loop.find("poll_timer.tick()").expect("poll arm");
    assert!(sigterm < poll_tick, "the signal arms stay above the poll");
    assert_eq!(
        run_loop.matches("tokio::task::spawn_blocking(").count(),
        1,
        "the poll analysis is the only blocking task in the loop"
    );
    let blocking = run_loop
        .find("tokio::task::spawn_blocking(")
        .expect("poll analysis");
    // rc.6: the poll's grab and read are also raced against the next
    // Tab key event (`stop_for_shutdown_or_key`).
    let wrap = run_loop[..blocking]
        .rfind("stop_for_shutdown_or_key(")
        .expect("analysis is raced against shutdown");
    assert!(
        run_loop[wrap + "stop_for_shutdown_or_key(".len()..blocking]
            .trim()
            .is_empty(),
        "spawn_blocking OCR is the future passed to stop_for_shutdown_or_key"
    );
    assert_eq!(
        run_loop.matches("stop_for_shutdown_or_key(").count(),
        2,
        "only the capture grab and the OCR are raced against shutdown"
    );
    let tick_at = run_loop.find("apply_poll_tick(").expect("poll write");
    let write_at = run_loop[..tick_at]
        .rfind("finish_within_shutdown(")
        .expect("the poll write keeps the shutdown deadline");
    let bound = tick_at
        + run_loop[tick_at..]
            .find("shutdown_time_left")
            .expect("the poll write is bounded by the shared deadline");
    assert!(
        !run_loop[write_at..bound].contains("stop_for_shutdown"),
        "a started poll write is not dropped when the signal wins"
    );
}

/// rc.6: a Tab used to wait behind the poll's grab and OCR (the poll ran
/// up to ~0.8 s inside the loop), so it was stamped and served late. A
/// press now drops the running poll, and the keyboard arm gets it next.
#[tokio::test]
async fn a_tab_press_preempts_a_running_poll_and_is_not_lost() {
    let t0 = Instant::now();
    let press = detect::TabKey { down: true, at: t0 };
    let mut keys = ScriptedKeys([press].into_iter().collect());
    let mut tabs = stat_tracker::tab_retry::TabKeys::default();
    let mut pending = None;
    let finished = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let started = std::time::Instant::now();
    let raced = stop_for_shutdown_or_key(
        slow_poll(std::sync::Arc::clone(&finished)),
        std::future::pending::<()>(),
        stat_tracker::tab_retry::next_press(&mut keys, &mut tabs),
        &mut pending,
    )
    .await;
    assert!(matches!(raced, PollRace::Preempted));
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
    assert!(!finished.load(std::sync::atomic::Ordering::SeqCst));

    // The press the poll kept is what the keyboard arm takes next,
    // before any new event.
    let next = pending_or_next(&mut pending, async { panic!("not read") }).await;
    assert_eq!(next.unwrap(), press);
    assert!(pending.is_none());
}

/// The run loop's two halves of the bridge: the poll arm keeps the
/// bridge a preempting press took (not dropped at once), and the
/// keyboard arm takes it at its top, before the match, so a keyboard
/// error drops it too and a press holds it until its own hold exists.
#[test]
fn the_run_loop_keeps_the_preempt_bridge_until_the_keyboard_arm() {
    let src = include_str!("../main.rs");
    let run_at = src
        .find(concat!("async ", "fn run_loop"))
        .expect("run_loop");
    let after = &src[run_at..];
    let end = after.find("\nfn analyze_frame(").expect("run_loop end");
    let run_loop = &after[..end];
    // Keyboard arm.
    let arm = run_loop
        .find(concat!("result = pending_or_next(", "&mut pending_key"))
        .expect("keyboard arm");
    let arm_text = &run_loop[arm..];
    let take = arm_text
        .find(concat!("let bridge = ", "preempt_stream_hold.take();"))
        .expect("the keyboard arm takes the bridge");
    let matched = arm_text.find(concat!("match ", "result {")).expect("match");
    assert!(
        take < matched,
        "taken before the match, so Err drops it too"
    );
    let ok_arm = &arm_text[matched..];
    let start = ok_arm
        .find(concat!("capture::", "stream_key_hold(press)"))
        .expect("the press's own hold");
    let dropped = ok_arm
        .find(concat!("drop(", "bridge);"))
        .expect("bridge dropped");
    assert!(
        start < dropped,
        "the bridge outlives the press's own hold start"
    );
    // Poll arm: the bridge a preempting press took is kept.
    let preempted = run_loop
        .find(concat!("PollRace::", "Preempted => {"))
        .expect("preempted branch");
    let branch = &run_loop[preempted..];
    let branch = &branch[..branch.find(concat!("conti", "nue;")).expect("continue")];
    assert!(
        branch.contains(concat!("preempt_stream_hold = ", "bridge.take();")),
        "the preempted poll hands its bridge to the keyboard arm"
    );
    // Poll arm: its key future is the bridge-taking one.
    let poll = run_loop
        .find(concat!("capture::poll_", "grab(backend"))
        .expect("the poll grab");
    let race = &run_loop[poll..];
    let race = &race[..race
        .find(concat!("&mut pending", "_key,"))
        .expect("race end")];
    let wrap = race
        .find(concat!("with_bridge_hold", "("))
        .expect("the poll's key future takes the bridge");
    let wrapped = &race[wrap..];
    assert!(wrapped.contains(concat!("tab_retry::next_press", "(&mut kbd, &mut tabs)")));
    assert!(wrapped.contains(concat!("&mut ", "bridge,")));
    assert!(wrapped.contains(concat!("capture::stream_", "bridge_hold,")));
}

/// The pause decision (which can wait seconds for KWin's version) runs
/// next to the backend probe, not before it.
#[test]
fn the_pause_decision_runs_alongside_the_backend_probe() {
    let src = include_str!("../main.rs");
    let join = src.find(concat!("tokio::", "join!(")).expect("join");
    let call = &src[join..];
    let call = &call[..call.find(");").expect("join end")];
    assert!(call.contains(concat!("capture::decide_", "stream_pause(")));
    assert!(call.contains(concat!("capture::detect_", "backend()")));
    assert_eq!(
        src.matches(concat!("capture::decide_", "stream_pause("))
            .count(),
        1,
        "decided once, inside the join"
    );
}

/// The bridge hold exists before the preempted poll's grab is dropped,
/// so the stream never pauses between the poll and the press.
#[tokio::test]
async fn a_preempting_press_takes_its_bridge_before_the_poll_is_dropped() {
    struct DropLog(std::sync::Arc<std::sync::Mutex<Vec<&'static str>>>);
    impl Drop for DropLog {
        fn drop(&mut self) {
            self.0.lock().unwrap().push("poll dropped");
        }
    }
    let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let press = detect::TabKey {
        down: true,
        at: Instant::now(),
    };
    let mut keys = ScriptedKeys([press].into_iter().collect());
    let mut tabs = stat_tracker::tab_retry::TabKeys::default();
    let mut pending = None;
    let mut bridge = None;
    let guard = DropLog(std::sync::Arc::clone(&log));
    let raced = stop_for_shutdown_or_key(
        async move {
            let _guard = guard;
            std::future::pending::<()>().await
        },
        std::future::pending::<()>(),
        with_bridge_hold(
            stat_tracker::tab_retry::next_press(&mut keys, &mut tabs),
            &mut bridge,
            || log.lock().unwrap().push("bridge"),
        ),
        &mut pending,
    )
    .await;
    assert!(matches!(raced, PollRace::Preempted));
    assert!(bridge.is_some());
    assert_eq!(*log.lock().unwrap(), vec!["bridge", "poll dropped"]);
    // A keyboard error takes no bridge.
    let mut none: Option<()> = None;
    let err: Result<(), &str> =
        with_bridge_hold(async { Err("keyboard lost") }, &mut none, || ()).await;
    assert!(err.is_err() && none.is_none());
}

/// The press's stream hold ends with the retry sequence, before the
/// result is analysed.
#[tokio::test]
async fn the_tab_stream_hold_ends_when_the_retries_end() {
    // No stream is registered in this process: the hold is inert, but
    // it still sits in the task until it is ended.
    let hold = capture::stream_key_hold(Instant::now());
    capture::with_tab_stream_hold(hold, async {
        assert_eq!(then_end_tab_hold(async { 7 }).await, 7);
        assert!(
            !capture::end_tab_stream_hold(),
            "then_end_tab_hold already ended it"
        );
    })
    .await;
    // Not before: the hold is still there while the retries run.
    let hold = capture::stream_key_hold(Instant::now());
    capture::with_tab_stream_hold(hold, async {
        assert!(
            then_end_tab_hold(async { capture::end_tab_stream_hold() }).await,
            "the retries still had the hold"
        );
    })
    .await;
}

/// A release must not drop the poll: during the end-reel wake that poll
/// is the outcome look in the ~3 s VICTORY window, right when a player
/// who held Tab lets go. The release still reaches the capture.
#[tokio::test]
async fn a_tab_release_does_not_preempt_the_poll() {
    let t0 = Instant::now();
    let mut keys = ScriptedKeys(
        [detect::TabKey {
            down: false,
            at: t0 + std::time::Duration::from_millis(400),
        }]
        .into_iter()
        .collect(),
    );
    let mut tabs = stat_tracker::tab_retry::TabKeys::default();
    let flight = stat_tracker::tab_retry::TabFlight::new(t0);
    tabs.started(Arc::clone(&flight));
    let mut pending = None;
    let raced = stop_for_shutdown_or_key(
        async {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            "read"
        },
        std::future::pending::<()>(),
        stat_tracker::tab_retry::next_press(&mut keys, &mut tabs),
        &mut pending,
    )
    .await;
    assert!(matches!(raced, PollRace::Done("read")));
    assert!(pending.is_none());
    assert_eq!(
        flight.released_at(),
        Some(t0 + std::time::Duration::from_millis(400))
    );
}

#[tokio::test]
async fn finished_poll_work_beats_a_ready_key_and_shutdown_beats_both() {
    let press = detect::TabKey {
        down: true,
        at: Instant::now(),
    };
    let mut keys = ScriptedKeys([press].into_iter().collect());
    let mut tabs = stat_tracker::tab_retry::TabKeys::default();
    let mut pending = None;
    let raced = stop_for_shutdown_or_key(
        async { "read" },
        std::future::pending::<()>(),
        stat_tracker::tab_retry::next_press(&mut keys, &mut tabs),
        &mut pending,
    )
    .await;
    assert!(matches!(raced, PollRace::Done("read")));
    assert!(pending.is_none());
    assert_eq!(keys.0.len(), 1, "the press waits in the channel");
    let mut pending = None;
    let raced =
        stop_for_shutdown_or_key(async { "read" }, async {}, async { "tab" }, &mut pending).await;
    assert!(matches!(raced, PollRace::Shutdown));
}

/// A poll analysis that would run for half a minute must lose to shutdown
/// well inside the shared 5 second deadline. The shutdown future is
/// injected so this test does not set the process-wide shutdown clock.
#[tokio::test]
async fn a_slow_poll_analysis_yields_to_shutdown_within_the_deadline() {
    let waiter = std::sync::Arc::new(tokio::sync::Notify::new());
    let flag = std::sync::Arc::clone(&waiter);
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(40)).await;
        flag.notify_one();
    });
    let started = std::time::Instant::now();
    let raced = stop_for_shutdown(
        async {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            "row"
        },
        waiter.notified(),
    )
    .await;
    assert!(raced.is_err(), "shutdown must discard the slow analysis");
    let elapsed = started.elapsed();
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "shutdown took {elapsed:?}, inside the 5s deadline"
    );
    assert_eq!(
        stop_for_shutdown(async { "row" }, std::future::pending::<()>())
            .await
            .expect("work finishes when shutdown never arrives"),
        "row"
    );
    assert!(
        stop_for_shutdown(async { "row" }, async {}).await.is_err(),
        "when both are ready, shutdown is polled first"
    );
}

/// Capture and OCR stay on the immediate race. Neither writes a row, so
/// dropping them when the signal wins is safe. The shutdown future is
/// injected so this test does not set the process-wide shutdown clock.
#[tokio::test]
async fn a_slow_capture_and_ocr_are_cut_off_promptly() {
    for label in ["capture", "ocr"] {
        let began = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let finished = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let began_run = std::sync::Arc::clone(&began);
        let finished_run = std::sync::Arc::clone(&finished);
        let raced = stop_for_shutdown(
            async move {
                began_run.store(true, std::sync::atomic::Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                finished_run.store(true, std::sync::atomic::Ordering::SeqCst);
                label
            },
            async {},
        )
        .await;
        assert!(raced.is_err(), "{label} must be cut off when shutdown wins");
        assert!(
            !began.load(std::sync::atomic::Ordering::SeqCst),
            "{label} must not start when shutdown is already waiting"
        );
        assert!(
            !finished.load(std::sync::atomic::Ordering::SeqCst),
            "{label} must not finish"
        );

        let finished_late = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = std::sync::Arc::clone(&finished_late);
        let waiter = std::sync::Arc::new(tokio::sync::Notify::new());
        let wake = std::sync::Arc::clone(&waiter);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(40)).await;
            wake.notify_one();
        });
        let started = std::time::Instant::now();
        let raced = stop_for_shutdown(
            async move {
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
            },
            waiter.notified(),
        )
        .await;
        assert!(raced.is_err(), "{label} in flight must be dropped");
        assert!(!finished_late.load(std::sync::atomic::Ordering::SeqCst));
        let elapsed = started.elapsed();
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "{label} took {elapsed:?}, inside the 5s deadline"
        );
    }
}

/// The signal lands after `retire_active_game` has taken the game out of
/// memory and before the sealed outcome is stored. The write is not
/// dropped, so the outcome is still in the store. The shutdown future is
/// injected so this test does not set the process-wide shutdown clock.
#[tokio::test]
async fn a_shutdown_during_retire_still_persists_the_sealed_outcome() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    store
        .insert_match(test_match("sess-shutdown-seal", "unknown"))
        .await
        .unwrap();
    let mut sealed = game(detect::MatchOutcome::Unknown, None, Instant::now());
    sealed.session_id = "sess-shutdown-seal".into();
    sealed.session_created = true;
    sealed.result_mark = Some(ResultMark {
        outcome: detect::MatchOutcome::Defeat,
        confirmed: false,
        seen_at: Instant::now() - Duration::from_secs(20),
    });
    let mut st = session(Some(sealed), Instant::now());

    let entered = std::sync::Arc::new(tokio::sync::Notify::new());
    let release = std::sync::Arc::new(tokio::sync::Notify::new());
    let shutdown = std::sync::Arc::new(tokio::sync::Notify::new());
    let entered_w = std::sync::Arc::clone(&entered);
    let release_w = std::sync::Arc::clone(&release);
    let shutdown_w = std::sync::Arc::clone(&shutdown);
    tokio::spawn(async move {
        entered_w.notified().await;
        shutdown_w.notify_one();
        tokio::task::yield_now().await;
        tokio::time::sleep(Duration::from_millis(30)).await;
        release_w.notify_one();
    });

    let started = Instant::now();
    let finished = finish_within_shutdown(
        retire_after_pause(
            &mut st,
            &store,
            dir.path(),
            Some(detect::MatchOutcome::Defeat),
            "test: shutdown mid-write",
            async move {
                entered.notify_one();
                release.notified().await;
            },
        ),
        shutdown.notified(),
        || Duration::from_secs(2),
    )
    .await;
    assert!(
        finished.is_ok(),
        "the seal write must finish inside the deadline"
    );
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(2),
        "the seal write took {elapsed:?}"
    );
    assert!(
        st.active_game.is_none(),
        "the game left memory before the store call"
    );
    let snaps = store
        .get_session_snapshots("sess-shutdown-seal")
        .await
        .unwrap();
    assert_eq!(snaps[0].outcome, "defeat");
}
