//! Starting the daemon: `run`, the pid guard, flags, the store, the sync
//! client and player name, the wait for the reader packs, and the keyboard.

use crate::*;

/// Bounded retries when startup finds zero keyboards. A miss is usually
/// "not in the `input` group" (only a new login's process credentials can
/// fix that) or `/dev/input` not enumerated yet (a same-process retry can).
/// Kept short so the pid file is not held while the GUI shows a daemon
/// that will never capture. After this, the process exits non-zero and
/// systemd `Restart=on-failure` tries again; a later graphical-session
/// start covers re-login.
pub(crate) const KEYBOARD_OPEN_ATTEMPTS: u32 = 3;

/// Gap between [`KEYBOARD_OPEN_ATTEMPTS`]. This wait also polls SIGTERM
/// and Ctrl+C, without that, `systemctl --user stop` sits until
/// TimeoutStopSec and SIGKILL while the handler is registered but never
/// received.
pub(crate) const KEYBOARD_OPEN_RETRY: std::time::Duration = std::time::Duration::from_secs(2);

/// What to do after a failed keyboard open, before the next attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeyboardWait {
    /// Interval elapsed; try `MultiKeyboardStream::open` again.
    Retry,
    /// SIGTERM or Ctrl+C. Exit 0 so an explicit stop is not a failure
    /// (`Restart=on-failure` must not bring the daemon back after stop).
    Shutdown,
}

/// Keyboard stream acquired, or a clean shutdown while waiting to retry.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum KeyboardAcquire<T> {
    Ready(T),
    Shutdown,
}

/// Polled between keyboard-open attempts. Production uses signals; tests
/// script the events.
pub(crate) trait KeyboardWaiter {
    fn wait(&mut self) -> impl std::future::Future<Output = anyhow::Result<KeyboardWait>> + '_;
}

pub(crate) struct SignalKeyboardWaiter<'a> {
    pub(crate) sigterm: &'a mut tokio::signal::unix::Signal,
}

impl KeyboardWaiter for SignalKeyboardWaiter<'_> {
    async fn wait(&mut self) -> anyhow::Result<KeyboardWait> {
        tokio::select! {
            biased;
            r = tokio::signal::ctrl_c() => {
                r?;
                tracing::info!(target: "scuffed_stat_tracker", "shutting down");
                Ok(KeyboardWait::Shutdown)
            }
            _ = self.sigterm.recv() => {
                tracing::info!(target: "scuffed_stat_tracker", "SIGTERM received: shutting down");
                Ok(KeyboardWait::Shutdown)
            }
            _ = tokio::time::sleep(KEYBOARD_OPEN_RETRY) => Ok(KeyboardWait::Retry),
        }
    }
}

/// Open a keyboard, retrying on failure. `Err` is "still no keyboard",
/// callers must propagate it so the process exits non-zero. `Ok(Shutdown)`
/// is a clean stop during the retry wait.
pub(crate) async fn acquire_keyboard<T, E, F, W>(
    mut open_keyboard: F,
    waiter: &mut W,
    max_attempts: u32,
) -> anyhow::Result<KeyboardAcquire<T>>
where
    F: FnMut() -> Result<T, E>,
    E: std::fmt::Display,
    W: KeyboardWaiter,
{
    let mut attempt = 0u32;
    loop {
        attempt += 1;
        match open_keyboard() {
            Ok(stream) => return Ok(KeyboardAcquire::Ready(stream)),
            Err(e) => {
                tracing::error!(
                    target: "scuffed_stat_tracker",
                    error = %e,
                    attempt,
                    max_attempts,
                    "evdev init failed: no keyboard detected"
                );
                if attempt >= max_attempts {
                    anyhow::bail!(
                        "no keyboard available after {attempt} attempts ({e}): \
                         exiting so systemd can restart once an input device is readable"
                    );
                }
                tracing::info!(target: "scuffed_stat_tracker", "retrying keyboard open; SIGTERM or Ctrl+C will quit");
                if waiter.wait().await? == KeyboardWait::Shutdown {
                    return Ok(KeyboardAcquire::Shutdown);
                }
            }
        }
    }
}

/// Startup keyboard open: retry while `select!`ing SIGTERM and Ctrl+C.
///
/// `Ok(None)` means the operator stopped us (exit 0, pid file dropped).
/// `Err` means no keyboard after the bounded retries (exit non-zero).
pub(crate) async fn open_keyboard_or_shutdown(
    sigterm: &mut tokio::signal::unix::Signal,
) -> anyhow::Result<Option<detect::MultiKeyboardStream>> {
    let mut waiter = SignalKeyboardWaiter { sigterm };
    match acquire_keyboard(
        detect::MultiKeyboardStream::open,
        &mut waiter,
        KEYBOARD_OPEN_ATTEMPTS,
    )
    .await?
    {
        KeyboardAcquire::Ready(stream) => Ok(Some(stream)),
        KeyboardAcquire::Shutdown => Ok(None),
    }
}

pub(crate) struct PidGuard {
    pub(crate) path: std::path::PathBuf,
    pub(crate) pid: u32,
}

/// Remove `path` only when it still contains `owner`.
///
/// The GUI used to unlink `daemon.pid` before this process finished shutting
/// down. A restart could write a new pid, and an unconditional unlink here
/// deleted that file. Returns whether the file was removed.
pub(crate) fn unlink_pid_file_if_owner(path: &std::path::Path, owner: u32) -> bool {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return false,
        Err(e) => {
            tracing::warn!(
                target: "scuffed_stat_tracker",
                error = %e,
                path = %path.display(),
                "could not read daemon pid file; not removing it"
            );
            return false;
        }
    };
    // The file is `pid\nstart_ticks\n` (see `proc_id::format_pid_record`).
    // Parsing the whole trimmed text as one u32 failed on that second line,
    // so every clean shutdown logged "not a pid" and left the file behind.
    match stat_tracker::proc_id::parse_pid_record(&text)
        .map(|rec| rec.pid)
        .ok_or(())
    {
        Ok(pid) if pid == owner => match std::fs::remove_file(path) {
            Ok(()) => true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => {
                tracing::warn!(
                    target: "scuffed_stat_tracker",
                    error = %e,
                    path = %path.display(),
                    "failed to remove daemon pid file"
                );
                false
            }
        },
        Ok(other) => {
            tracing::warn!(
                target: "scuffed_stat_tracker",
                owner,
                found = other,
                path = %path.display(),
                "daemon pid file belongs to another process; not removing it"
            );
            false
        }
        Err(_) => {
            tracing::warn!(
                target: "scuffed_stat_tracker",
                path = %path.display(),
                "daemon pid file is not a pid; not removing it"
            );
            false
        }
    }
}

impl Drop for PidGuard {
    fn drop(&mut self) {
        unlink_pid_file_if_owner(&self.path, self.pid);
    }
}

/// Package version shown by `--version` / `--help`.
///
/// Release CI sets `SST_RELEASE_VERSION` from the git tag so the binary
/// reports the tagged release version; local/dev builds fall back to
/// `CARGO_PKG_VERSION`.
pub(crate) fn package_version() -> &'static str {
    option_env!("SST_RELEASE_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))
}

pub(crate) async fn run(omp: stat_tracker::omp::Startup) -> anyhow::Result<()> {
    lock_down_umask();
    // No core file, also when the GUI starts the daemon outside systemd.
    let core_dumps = stat_tracker::fs_mode::disable_core_dumps();
    // Handle --version/--help before ANY init: smoke tests (CI clean-room,
    // installer) and humans probe these; unknown flags used to fall through
    // to full daemon startup, which blocks forever on headless machines.
    if handle_preinit_flags() {
        return Ok(());
    }

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            EnvFilter::new("scuffed_stat_tracker=info,stat_tracker=info,surrealdb=warn")
        }))
        .init();
    if let Err(e) = core_dumps {
        tracing::warn!(target: "scuffed_stat_tracker", error = %e, "could not turn off core dumps");
    }
    stat_tracker::omp::log_startup(&omp);
    super::memtune::log_startup();
    tokio::spawn(super::memtune::trim_loop());

    let collect_portraits = std::env::args().any(|a| a == "--collect-portraits");
    let dump_poll_frames = std::env::args().any(|a| a == "--dump-poll-frames");

    // Informational flags that need tracing initialized but exit before the
    // daemon starts (tessdata generation, output listing).
    if handle_info_flags().await {
        return Ok(());
    }

    let mut config = config::Config::load()
        .map_err(anyhow::Error::from_boxed)
        .context("failed to load config")?;
    tracing::info!(target: "scuffed_stat_tracker", "Scuffed Stat Tracker starting");
    stat_tracker::report::kit::set_retention(config.report_kit_retention());
    tracing::info!(target: "scuffed_stat_tracker", data_dir = %config.data_dir.display(), "using data directory");

    // ProtectSystem=strict (when the user manager can apply it) makes a
    // custom data_dir read-only. Fail here with the drop-in to add, instead
    // of dying later inside the store.
    stat_tracker::sandbox::ensure_data_dir_writable(&config.data_dir)?;
    // Existing installs may have been created with umask 022. Tighten the
    // tree before the store opens so the DB and command queue are owner-only.
    stat_tracker::fs_mode::tighten_private_tree(&config.data_dir);

    // Refuse to start alongside a live daemon. On drop the guard removes
    // daemon.pid only if the file still names this process. Held across
    // --vacuum / auto-vacuum so no second instance starts mid-compaction.
    // The store-directory flock (see LocalStore::open) is the single-writer
    // lock; this file is how the GUI finds the process.
    // Taken before anything talks to the site: a start that is going to
    // exit here must not sync reader packs first. It used to, and by
    // holding the pack lock it left the daemon that stayed without its
    // packs. The app also sees the daemon from here on, not only once the
    // startup sync is over.
    // A leftover tracker from an update (a different binary) is replaced
    // first. The service also replaces a same-binary stray. A second start
    // of this same binary, and --vacuum, do not signal the live daemon.
    // The pid guard below still refuses a second live daemon. A replace
    // waits out an open game so it does not cut it off.
    let replace_same = stat_tracker::replace::flag_enabled(
        std::env::var("STAT_TRACKER_REPLACE_STRAY").ok().as_deref(),
    );
    let vacuum = std::env::args().any(|arg| arg == "--vacuum");
    if let Some(policy) = stat_tracker::replace::policy_for_startup(vacuum, replace_same)
        && let Err(err) = stat_tracker::replace::stop_strays(&config.data_dir, true, policy).await
    {
        tracing::error!(target: "scuffed_stat_tracker", error = %err, "previous tracker was not replaced");
        anyhow::bail!("{err}");
    }
    let _pid_guard = acquire_pid_guard(&config.data_dir)?;

    // One client for the whole process. An unsafe URL logs once here and
    // leaves sync off, including the startup player-name fetch, so the
    // bearer token is never sent. The daemon keeps running.
    let sync_client = open_sync_client(config.sync.as_ref());
    // heroes-v1 syncs here; the other packs sync in the background once
    // capture is up. The new reader picks up heroes-v2 and heroes-v2-1080
    // on a later board when they land (`shadow::heroes::LiveHeroSet`).
    //
    // The name reader is made here, before the pack keeper, so the keeper
    // can reload it once names-v1 lands. Returns immediately: the names-v1
    // load, when the folder is already there, runs on one background
    // thread, and reads before it finishes reuse the Tesseract rows
    // already in hand.
    let name_switch = names::NameSwitch::load(config.name_reader, &config.data_dir);
    let mut extra_packs = None;
    if let Some(client) = &sync_client {
        let creds = client.credentials();
        let mut pack_startup = None;
        let pause = sync::auth_pause_reason(&config.data_dir, &creds.server_url, &creds.token);
        if matches!(
            pause,
            Some(sync::AuthPauseReason::Rejected | sync::AuthPauseReason::Expired)
        ) {
            let status = sync::auth_pause_status(&config.data_dir, &creds.server_url, &creds.token)
                .unwrap_or_else(|| sync::SYNC_TOKEN_REJECTED_STATUS.to_string());
            tracing::error!(target: "scuffed_stat_tracker", "{status}. Sync resumes when the URL or token changes in Settings.");
        } else {
            if pause == Some(sync::AuthPauseReason::DeletePending) {
                tracing::error!(
                    target: "scuffed_stat_tracker",
                    "{}. Sync resumes when a new token is saved in Settings.",
                    sync::SYNC_STATS_DELETE_PENDING_STATUS
                );
            }
            fetch_player_name_if_needed(&mut config, client).await;
            pack_startup = Some(
                stat_tracker::packs::sync_on_startup(
                    &creds.server_url,
                    &creds.token,
                    &config.data_dir,
                )
                .await,
            );
        }
        // The background job for every pack. It is made for every daemon
        // that has sync set up, whatever the startup sync came to (a busy
        // lock, a 429, packs switched off, no network, or no startup sync
        // at all while the token is paused), and it keeps trying while the
        // daemon runs. The client clone shares the URL and token with the
        // sync client, so when the daemon takes a new pair from Settings
        // (it does that while sync is paused on a rejected token) the job
        // uses it on its next try.
        let (data_dir, client) = (config.data_dir.clone(), client.clone());
        let mut keeper =
            stat_tracker::packs::PackKeeper::new(&config.data_dir, pack_startup, move || {
                pack_credentials(&data_dir, &client)
            });
        // A names-v1 install (a first start, or a newer pack) reloads the
        // name reader without a restart. 0.5.0-rc.1 loaded it once at
        // startup, a second before the keeper installed the pack.
        let names_on_install = name_switch.clone();
        keeper.on_installed(move |installed| {
            reload_names_after_install(&names_on_install, installed);
        });
        extra_packs = Some(keeper);
    }

    // Single wiring point for OCR debug dumps, the lib reads this switch
    // instead of re-loading config on first use; dumps land under data_dir.
    ocr::set_debug_ocr(config.debug_ocr_enabled());
    ocr::set_debug_dir(config.data_dir.join("debug"));
    // OCR pool size (Tesseract instances ≈ workers). Must run before first OCR.
    let ocr_threads = config.ocr_threads_resolved();
    ocr::set_ocr_threads(ocr_threads);

    // Maintenance mode: compact the store and exit (see LocalStore::vacuum).
    if maybe_vacuum(&config.data_dir).await? {
        return Ok(());
    }

    // perf.log belongs to the daemon that runs: started only once the pid
    // guard is held and no one-shot command (`--vacuum`) is left to exit,
    // so a refused second start writes no `setup` header into the live
    // daemon's log (S5-CAP-8).
    stat_tracker::perf_log::init(&config.data_dir);

    // Crash marker (report-bundle §7). Taken after the pid guard, so a live
    // daemon never reaches here. A stale run.marker becomes
    // last_crash.marker with one WARN. run.marker is removed only when the
    // main loop returns Ok (SIGTERM, Ctrl+C, a normal exit). A panic or an
    // error return (e.g. keyboard devices lost) leaves it, so the next start
    // offers a crash report. `last_seen` is rewritten once a minute until then.
    let run_marker = match stat_tracker::report::marker::start_run(&config.data_dir) {
        Ok((_, guard)) => {
            tokio::spawn(
                guard
                    .heartbeat()
                    .run(stat_tracker::report::marker::HEARTBEAT_INTERVAL),
            );
            Some(guard)
        }
        Err(e) => {
            tracing::warn!(target: "scuffed_stat_tracker", error = %e, "could not write run.marker; crash reports will not see this run");
            None
        }
    };

    // Log rotate, prune old pre-vacuum backups, auto-vacuum if store is bloated.
    // Must run before open_store (single-writer SurrealKV).
    if let Err(e) = storage::maintain::startup_maintenance(&config.data_dir).await {
        tracing::warn!(target: "scuffed_stat_tracker", error = %e, "startup maintenance failed: continuing with live store");
    }

    // The OCR on/off switch (`ocr` in config.toml), decided once for this
    // run. It sits below the pid guard and `--vacuum`, so a second start or
    // a maintenance run never rewrites the live daemon's status file. Off is
    // refused, with the reason in the log, while this build still needs
    // Tesseract for a read or a reader pack is missing. It sets the gate in
    // `ocr::gate`: with OCR off no Tesseract instance, OCR worker pool or
    // tessdata lookup is ever started.
    //
    // Startup synced heroes-v1 only; the other packs are still to come. When
    // off is asked for and they are what is missing, give them a bounded
    // moment first, or a first start would refuse off for the whole run.
    // Only when the pack job starts with downloads: after a refused or
    // failed startup sync its first move is a wait, and so would ours be.
    // The job goes on in the background either way.
    //
    // The run loop's SIGTERM handler does not exist yet. So the handler is
    // registered here, before the wait, and handed to the run loop: a stop
    // during the wait is a clean stop (run.marker removed, no crash report
    // offered on the next start), and a stop between the wait and the run
    // loop is still waiting in the handler when the loop first looks.
    let mut early_sigterm = None;
    let wait_for_packs = extra_packs
        .as_ref()
        .is_some_and(|job| job.starts_with_downloads())
        && stat_tracker::ocr_switch::needs_pack_wait(&config);
    if wait_for_packs {
        tracing::info!(
            target: "scuffed_stat_tracker",
            wait_secs = stat_tracker::ocr_switch::PACK_WAIT.as_secs(),
            "ocr = \"off\" needs reader packs that are still downloading; waiting for them before deciding"
        );
        if let Some(mut extras) = extra_packs.take() {
            let mut sigterm =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
            // The job's `run` returns only when every pack is settled, which
            // can be hours: the wait is on its first try, not on the task.
            let first = extras.first_try_done();
            tokio::spawn(extras.run());
            let stopped = stopped_while_waiting(
                stat_tracker::ocr_switch::PACK_WAIT,
                reader_packs_arrive(stat_tracker::ocr_switch::PACK_WAIT, first, &config),
                async {
                    tokio::select! {
                        _ = sigterm.recv() => {}
                        _ = tokio::signal::ctrl_c() => {}
                    }
                },
            )
            .await;
            if stopped {
                tracing::info!(target: "scuffed_stat_tracker", "stop asked while waiting for the reader packs, shutting down");
                if let Some(guard) = run_marker {
                    guard.finish();
                }
                return Ok(());
            }
            early_sigterm = Some(sigterm);
        }
    }
    let ocr_switch = stat_tracker::ocr_switch::Switch::start(&config);
    if !ocr_switch.is_off() {
        tracing::info!(
            target: "scuffed_stat_tracker",
            ocr_threads,
            configured = ?config.ocr_threads,
            "OCR workers (each holds ~23MB tessdata; set ocr_threads / STAT_TRACKER_OCR_THREADS / --ocr-threads)"
        );
    }

    // Tessdata generation is triggered manually via --generate-tessdata or the GUI button.
    // Don't run it at daemon startup, it can take minutes and blocks Tab capture.

    capture::set_prefs(capture::CapturePrefs {
        data_dir: config.data_dir.clone(),
        route: config.capture_route,
        debug_frames: config.debug_portal_frames_enabled(),
        max_fps: config.screencast_max_fps(),
    });
    // Pause the screen stream between grabs where the compositor handles it.
    // Decided next to the backend probe: on KDE the KWin version read can
    // take a few seconds (D-Bus, a 3 s retry, the binary). No stream is
    // opened before both are done.
    let (_, backend) = tokio::join!(
        capture::decide_stream_pause(config.screencast_pause_override()),
        capture::detect_backend(),
    );
    tracing::info!(target: "scuffed_stat_tracker", ?backend, "capture backend selected");
    // KDE direct asked for and not there, a saved Wayland-only route off
    // Wayland, or no X11 capture: the app says why under the capture
    // method. Written every start, so an old reason is cleared. A
    // screenshot portal that stops answering, and answers again, rewrites
    // it through the sink (S5-CAP-1).
    let notice_dir = config.data_dir.clone();
    capture::set_notice_sink(move |notice| {
        stat_tracker::ocr_switch::publish_capture_notice(&notice_dir, notice);
    });
    stat_tracker::ocr_switch::publish_capture_notice(&config.data_dir, &capture::capture_notice());

    log_selected_output(backend, &config).await;

    let store = open_store(&config.data_dir).await?;

    // One answer for "which reader": `ocr = "off"` implies the new one,
    // whatever `reader` says. The capture path, the reader packs, the
    // own-name seed store and the shadow log all use it.
    let reader = ocr_switch.decision.reader(&config);
    let use_new_reader = new_reader_in_effect(&config, &ocr_switch.decision);
    if use_new_reader {
        shadow::init(&shadow::ReaderConfig::from_data_dir(&config.data_dir));
        tracing::info!(target: "scuffed_stat_tracker", "new scoreboard reader on for saved stats");
    }

    let portrait_matcher = Arc::new(detect::hero_portrait::PortraitMatcher::load_installed(
        &config.data_dir,
    ));

    let data_dir = config.data_dir.clone();

    log_startup_readiness(&config, dump_poll_frames, collect_portraits);

    let ctx = Arc::new(DaemonCtx {
        backend,
        store,
        sync_client,
        player_name: config.player_name.clone(),
        capture_output: config.capture_output.clone(),
        auto_detect: config.auto_detect,
        finished_game_close: finished_game_close_after(config.finished_game_close_secs),
        game_process_names: config.game_process_names.clone(),
        portrait_matcher,
        collect_portraits,
        dump_poll_frames,
        debug_ocr: config.debug_ocr_enabled(),
        data_dir,
        empty_map_reads: std::sync::atomic::AtomicUsize::new(0),
        shadow: shadow::worker::ShadowWorker::start_for_reader(&config, reader),
        use_new_reader,
        own_name: open_own_name_seeds(use_new_reader, &config),
        // Made before the pack keeper, which reloads it on a names-v1
        // install.
        names: name_switch,
    });
    // Watches the reader packs while OCR is asked off: a pack that goes away
    // holds its reads and is named in the status file. It never turns OCR on.
    let ocr_switch = Arc::new(std::sync::Mutex::new(ocr_switch));
    tokio::spawn(stat_tracker::ocr_switch::watch(Arc::clone(&ocr_switch)));
    // Capture never waits on the extra packs: a stalled download only
    // delays this task.
    if let Some(extras) = extra_packs {
        tokio::spawn(extras.run());
    }
    let mut sync_task = None;
    let mut restart_to = None;
    let result = run_loop(
        Arc::clone(&ctx),
        early_sigterm,
        &mut sync_task,
        &mut restart_to,
    )
    .await;
    // The sync task holds a store clone: settle it first, leaving the
    // store's close its own share of the deadline.
    let within = stat_tracker::replace::shutdown_time_left().saturating_sub(STORE_CLOSE_WAIT);
    finish_or_abort_sync(sync_task.take(), within).await;
    // The status file's counters as they stand when the run ends.
    ocr_switch.lock().unwrap_or_else(|e| e.into_inner()).check();
    if let Some(path) = restart_to {
        let data_dir = ctx.data_dir.clone();
        close_store_before_exit(ctx).await;
        return restart_after_close(&data_dir, &path);
    }
    // Before the close's wait: a kill during it is a clean stop, not a
    // crash to report. The WAL replays what the close did not finish.
    if result.is_ok()
        && let Some(guard) = run_marker
    {
        guard.finish();
    }
    close_store_before_exit(ctx).await;
    result
}

/// The update restart, once the store is closed: exit for systemd or exec
/// the new binary. The run marker and pid file go first, so the new process
/// does not read this one as a crash or as a live daemon. A failed exec
/// puts both back and ends this process with an error: the store is closed,
/// so it cannot go on. [`stat_tracker::replace::restart_preflight`] already
/// passed, so that takes an exec error the file checks cannot see.
fn restart_after_close(data_dir: &std::path::Path, path: &std::path::Path) -> anyhow::Result<()> {
    let marker = stat_tracker::report::marker::release_run_marker(data_dir);
    let released_pid = stat_tracker::replace::release_own_pid_file(data_dir);
    let Err(err) = stat_tracker::replace::restart_for_replaced_binary(path) else {
        return Ok(());
    };
    if released_pid {
        stat_tracker::replace::restore_own_pid_file(data_dir);
    }
    let _ = stat_tracker::report::marker::restore_run_marker(data_dir, marker.as_ref());
    Err(err.context("could not start the new tracker after closing the store"))
}

/// Longest wait for SurrealKV's close at exit. A clean close took about
/// 130 ms in the KDE soak; the rest of the deadline stays with the runtime.
pub(crate) const STORE_CLOSE_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

/// Drop the last store handle and wait for SurrealKV's close while the
/// runtime still runs. Dropping the handle only starts the close (the SDK
/// router task runs it), so returning straight to `shutdown_timeout`
/// raced it: the runtime cancelled SurrealKV's background tasks mid-close
/// and it logged `Error shutting down task: JoinError::Cancelled`.
/// Two kinds of task can keep the store open. The sync task holds its own
/// store clone; it is settled before this ([`finish_or_abort_sync`]), and
/// one that still holds a clone makes the wait run out. A task that holds
/// the context (an abandoned capture) keeps it shared. Either way the
/// runtime drops the store at exit and the WAL replays it on the next
/// start, as in 0.5.0.
async fn close_store_before_exit(ctx: Arc<DaemonCtx>) -> StoreClose {
    let within = stat_tracker::replace::shutdown_time_left().min(STORE_CLOSE_WAIT);
    close_store_within(ctx, within).await
}

/// How [`close_store_before_exit`] ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StoreClose {
    /// SurrealKV finished its close: nothing to replay on the next start.
    Closed,
    /// Another task still holds the context; no close was started.
    Shared,
    /// The last context handle dropped, but the close did not finish in
    /// time (a store clone still alive, or a slow flush).
    NotClosed,
}

pub(crate) async fn close_store_within(
    ctx: Arc<DaemonCtx>,
    within: std::time::Duration,
) -> StoreClose {
    let data_dir = ctx.data_dir.clone();
    let Some(ctx) = Arc::into_inner(ctx) else {
        tracing::info!(
            target: "scuffed_stat_tracker",
            "store still in use by a stopped task at exit; its WAL is replayed on the next start"
        );
        return StoreClose::Shared;
    };
    drop(ctx);
    if storage::wait_for_engine_close(&data_dir, within).await {
        return StoreClose::Closed;
    }
    tracing::warn!(
        target: "scuffed_stat_tracker",
        "store not closed at exit; its WAL is replayed on the next start"
    );
    StoreClose::NotClosed
}

/// The pack keeper's install hook: a names-v1 install reloads the name
/// reader on its own thread. Other packs have readers that watch their
/// folders, so nothing is done for them here.
pub(crate) fn reload_names_after_install(
    switch: &names::NameSwitch,
    installed: &stat_tracker::packs::InstalledPack,
) {
    if installed.name == names::pack::PACK_NAME {
        switch.reload_installed(names::InstalledNames {
            version: installed.version.clone(),
            manifest_sha256: installed.manifest_sha256.clone(),
        });
    }
}

/// Wait for `work`, at most `limit`. True when `stop` (SIGTERM or Ctrl+C at
/// startup) came first: the caller then stops.
pub(crate) async fn stopped_while_waiting(
    limit: std::time::Duration,
    work: impl std::future::Future<Output = ()>,
    stop: impl std::future::Future<Output = ()>,
) -> bool {
    tokio::select! {
        _ = tokio::time::timeout(limit, work) => false,
        () = stop => true,
    }
}

/// How often startup looks at the packs again while it waits for them.
pub(crate) const PACK_WAIT_LOOK: std::time::Duration = std::time::Duration::from_secs(5);

/// Whether startup goes on waiting after the pack job's first try, with
/// `left` of its wait remaining.
///
/// * Nothing missing: no, the packs are there.
/// * The job's next try comes soon (a 429, a busy lock, the paced gap) and
///   inside what is left: yes, packs can still arrive in time.
/// * The next try is a minute or more away, or after the wait is over, or
///   the job has ended: no. Classic OCR stays on for this run. The tracker
///   says which packs are missing, and when they do arrive, that a restart
///   turns classic OCR off (`ocr_switch::RESTART_TO_APPLY`).
pub(crate) fn keep_waiting_for_packs(
    first: &stat_tracker::packs::FirstTry,
    left: std::time::Duration,
) -> bool {
    !first.missing.is_empty()
        && first
            .next_try_in
            .is_some_and(|next| next < std::time::Duration::from_secs(60) && next < left)
}

/// Ends when the reader packs `ocr = "off"` needs are on disk, or when
/// waiting longer cannot bring them inside `limit`
/// ([`keep_waiting_for_packs`]). The caller bounds it with the same `limit`.
///
/// After a 429 the packs come one per paced gap, which for a whole set is
/// longer than the wait: then this runs to the limit, off is refused for
/// the run, and the member is told to restart once the packs are there.
pub(crate) async fn reader_packs_arrive(
    limit: std::time::Duration,
    first: tokio::sync::oneshot::Receiver<stat_tracker::packs::FirstTry>,
    config: &config::Config,
) {
    let started = Instant::now();
    // The job was dropped without a first try: nothing to wait for.
    let Ok(first) = first.await else {
        return;
    };
    let left = limit.saturating_sub(started.elapsed());
    if !keep_waiting_for_packs(&first, left) {
        if !first.missing.is_empty() {
            tracing::info!(
                target: "scuffed_stat_tracker",
                missing = ?first.missing,
                next_try_secs = ?first.next_try_in.map(|d| d.as_secs()),
                "reader packs are still missing and will not arrive in time; deciding without them"
            );
        }
        return;
    }
    tracing::info!(
        target: "scuffed_stat_tracker",
        missing = ?first.missing,
        next_try_secs = ?first.next_try_in.map(|d| d.as_secs()),
        "reader packs are still arriving; waiting for them a little longer"
    );
    loop {
        tokio::time::sleep(PACK_WAIT_LOOK).await;
        // The look loads the pack folders: off the async workers.
        let config = config.clone();
        let still_missing =
            tokio::task::spawn_blocking(move || stat_tracker::ocr_switch::needs_pack_wait(&config))
                .await
                .unwrap_or(false);
        if !still_missing {
            return;
        }
    }
}

/// Handle flags that must run before ANY initialization (version/help probes
/// used by smoke tests and humans). Returns true if a flag was handled and
/// `main` should exit successfully.
pub(crate) fn handle_preinit_flags() -> bool {
    if std::env::args().any(|a| a == stat_tracker::omp::PRINT_FLAG) {
        print!("{}", stat_tracker::omp::print_report());
        return true;
    }
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("scuffed-stat-tracker {}", package_version());
        return true;
    }
    if std::env::args().any(|a| a == "--help" || a == "-h") {
        println!(
            "scuffed-stat-tracker {}: Overwatch 2 scoreboard OCR daemon\n\n\
             USAGE: scuffed-stat-tracker [FLAGS]\n\n\
             FLAGS:\n\
             \x20 --version, -V         print version and exit\n\
             \x20 --help, -h            this help\n\
             \x20 --list-outputs        list capture outputs and exit\n\
             \x20 --generate-tessdata   build the game-font tessdata model and exit\n\
             \x20 --vacuum              compact the local stats DB and exit\n\
             \x20                       (daemon also auto-vacuums at start if store is bloated)\n\
             \x20 --collect-portraits   only fills missing portraits and the Doctrine stand-in; it never overwrites an existing reference\n\
             \x20 --dump-poll-frames    dev: save every polled frame while running\n\
             \x20 --ocr-threads N       OCR workers 1..=8 (RAM vs speed; also config/env)\n\
             \x20 --print-omp-limit     print the OpenMP thread limit OCR runs with and exit\n\n\
             With no flags, runs the capture daemon (see README).",
            package_version()
        );
        return true;
    }
    false
}

/// Informational flags that need tracing initialized but exit before the daemon
/// starts (tessdata generation, output listing). Returns true if handled.
pub(crate) async fn handle_info_flags() -> bool {
    if std::env::args().any(|a| a == "--generate-tessdata") {
        match setup::ensure_koverwatch_tessdata() {
            Ok(()) => {
                println!("koverwatch.traineddata generated successfully.");
                return true;
            }
            Err(e) => {
                eprintln!("tessdata generation failed: {e}");
                std::process::exit(1);
            }
        }
    }

    if std::env::args().any(|a| a == "--list-outputs") {
        // Enumerate through the exact backend detect_backend chose so the CLI
        // and daemon agree on the capture source. An unavailable backend is a
        // hard error (non-zero exit), it must not read as "zero outputs".
        let backend = capture::detect_backend().await;
        match backend {
            capture::CaptureBackend::None => {
                eprintln!("no capture backend available: cannot list outputs");
                std::process::exit(1);
            }
            capture::CaptureBackend::Portal => {
                println!("portal backend does not support output selection");
            }
            _ => match capture::list_outputs(backend).await {
                Ok(outputs) => {
                    println!("Available outputs:");
                    for (i, name) in outputs.iter().enumerate() {
                        println!("  [{i}] {name}");
                    }
                    println!("\nSet capture_output in config.toml to select one.");
                }
                Err(e) => {
                    eprintln!("Failed to list outputs: {e}");
                    std::process::exit(1);
                }
            },
        }
        return true;
    }

    false
}

/// Build the sync client, or log once and return `None` when the server URL
/// must not carry the bearer token. Does not panic.
pub(crate) fn open_sync_client(sync_cfg: Option<&config::SyncConfig>) -> Option<sync::SyncClient> {
    let sync_cfg = sync_cfg?;
    match sync::SyncClient::try_new(sync_cfg.clone()) {
        Ok(client) => Some(client),
        Err(e) => {
            tracing::error!(
                target: "scuffed_stat_tracker",
                server_url = %sync_cfg.server_url,
                error = %e,
                "sync disabled: refusing to send the bearer token over an unsafe server URL"
            );
            None
        }
    }
}

/// The server URL and token for one try of the reader pack job: the sync
/// client's pair at that moment. The daemon swaps that pair only while sync
/// is paused on a rejected token (`maybe_resume_sync_after_settings_change`
/// then re-reads Settings); until then a pair saved in Settings waits for
/// a restart, for the job as for uploads. `None` while sync is paused on a
/// rejected or expired token: that token is not sent again. A stats-deletion
/// hold still sends it.
pub(crate) fn pack_credentials(
    data_dir: &std::path::Path,
    client: &sync::SyncClient,
) -> Option<stat_tracker::packs::PackCredentials> {
    if sync_credentials_rejected(data_dir, Some(client)) {
        return None;
    }
    let creds = client.credentials();
    Some(stat_tracker::packs::PackCredentials {
        server_url: creds.server_url,
        token: creds.token,
    })
}

/// Whether the new reader is in effect for this run.
///
/// The capture path (`DaemonCtx::use_new_reader`), the reader packs
/// (`shadow::init`) and the own-name seed store ([`open_own_name_seeds`])
/// all take this one answer, so the new reader can never run without its
/// packs or its seed store.
///
/// It is the OCR switch's decision, not the `reader` key alone: classic OCR
/// off implies the new reader whatever `reader` says, and a refused off
/// runs the file's reader ([`stat_tracker::ocr_switch::Decision::reader`]).
pub(crate) fn new_reader_in_effect(
    config: &config::Config,
    switch: &stat_tracker::ocr_switch::Decision,
) -> bool {
    switch.reader(config).is_new()
}

/// When `player_name` isn't set locally, fetch it from the server. This is the
/// "first run via GUI" path: the user set their name in the web UI and launched
/// the daemon with just a token, no manual config editing needed.
///
/// `client` is only present when [`open_sync_client`] accepted the URL.
pub(crate) async fn fetch_player_name_if_needed(
    config: &mut config::Config,
    client: &sync::SyncClient,
) {
    if config.player_name.is_none() {
        match client.fetch_daemon_config().await {
            Ok(remote) if remote.player_name.is_some() => {
                tracing::info!(
                    target: "scuffed_stat_tracker",
                    player_name = %remote.player_name.as_deref().unwrap_or(""),
                    "player_name fetched from server"
                );
                config.player_name = remote.player_name;
            }
            Ok(_) => {
                tracing::info!(
                    target: "scuffed_stat_tracker",
                    "server has no player_name configured: set it in the web UI under My Stats → Settings"
                );
            }
            Err(e)
                if matches!(
                    e.attempt(),
                    sync::SyncAttempt::AuthRejected | sync::SyncAttempt::TokenExpired
                ) =>
            {
                let creds = client.credentials();
                let reason = if e.is_token_expired() {
                    sync::AuthPauseReason::Expired
                } else {
                    sync::AuthPauseReason::Rejected
                };
                if let Err(err) = sync::write_auth_pause_reason(
                    &config.data_dir,
                    &creds.server_url,
                    &creds.token,
                    reason,
                ) {
                    tracing::warn!(target: "scuffed_stat_tracker", error = %err, "could not record sync auth pause");
                }
                if e.is_token_expired() {
                    tracing::error!(
                        target: "scuffed_stat_tracker",
                        error = %e,
                        "sync token expired on daemon-config: pausing until it is relinked in Settings"
                    );
                } else {
                    tracing::error!(
                        target: "scuffed_stat_tracker",
                        error = %e,
                        "sync token rejected by daemon-config: pausing sync until the URL or token changes in Settings"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(target: "scuffed_stat_tracker", error = %e, "could not fetch daemon config from server (continuing without player_name)");
            }
        }
    }
}

/// Write the pid file (refusing to start if another live daemon already holds
/// it) and return a guard that removes it on drop when it still names us.
pub(crate) fn acquire_pid_guard(data_dir: &std::path::Path) -> anyhow::Result<PidGuard> {
    let pid_path = data_dir.join("daemon.pid");
    if let Ok(text) = std::fs::read_to_string(&pid_path)
        && let Some(existing) = stat_tracker::proc_id::parse_pid_record(&text)
        && stat_tracker::proc_id::pid_is_live_tracker_started(existing.pid, existing.start_ticks)
    {
        tracing::error!(
            target: "scuffed_stat_tracker",
            pid = existing.pid,
            "another daemon is already running: stop it first"
        );
        anyhow::bail!("another daemon is already running (PID {})", existing.pid);
    }
    let _ = std::fs::remove_file(&pid_path);
    let pid = std::process::id();
    let body =
        stat_tracker::proc_id::format_pid_record(pid, stat_tracker::proc_id::proc_start_ticks(pid));
    std::fs::write(&pid_path, body)?;
    stat_tracker::fs_mode::tighten_private_file(&pid_path);
    Ok(PidGuard {
        path: pid_path,
        pid,
    })
}

/// Process umask 077 so the store, pid file, logs, and snapshots are created
/// owner-only. Linux `mode_t` is a 32-bit unsigned integer.
pub(crate) fn lock_down_umask() {
    #[cfg(unix)]
    // SAFETY: libc `umask` takes a `mode_t` and returns the previous mask.
    // The declaration matches the Linux signature (`mode_t` is `u32`).
    unsafe extern "C" {
        fn umask(mask: u32) -> u32;
    }
    #[cfg(unix)]
    // SAFETY: `umask` is async-signal-safe and only changes this process's
    // file-creation mask. Called once on the main thread before workers start.
    unsafe {
        umask(0o077);
    }
}

/// `--vacuum` maintenance mode: compact the store and report before/after
/// sizes. Returns true if the flag was present and `main` should exit. Runs
/// while the pid guard is held so no daemon starts mid-compaction.
pub(crate) async fn maybe_vacuum(data_dir: &std::path::Path) -> anyhow::Result<bool> {
    if !std::env::args().any(|a| a == "--vacuum") {
        return Ok(false);
    }
    let before = storage::maintain::store_size_bytes(data_dir);
    let (matches, sessions, tombstones) = storage::LocalStore::vacuum(data_dir)
        .await
        .map_err(anyhow::Error::from_boxed)
        .context("vacuum failed")?;
    let after = storage::maintain::store_size_bytes(data_dir);
    // vacuum() already prunes to keep-1; report for the operator.
    println!(
        "vacuum complete: {matches} matches, {sessions} sessions, {tombstones} tombstones; \
         store {:.1} MB -> {:.1} MB (newest pre-vacuum backup kept)",
        before as f64 / 1e6,
        after as f64 / 1e6,
    );
    Ok(true)
}

/// Log the available capture outputs and which one captures will use, using the
/// backend `detect_backend` already resolved (Portal/None yield an empty list).
pub(crate) async fn log_selected_output(backend: capture::CaptureBackend, config: &config::Config) {
    if let Ok(outputs) = capture::list_outputs(backend).await {
        // `.first()`, not `[0]`: zero outputs (headless / compositor hiccup)
        // must not panic the daemon at startup.
        let selected = config
            .capture_output
            .as_deref()
            .or_else(|| outputs.first().map(String::as_str))
            .unwrap_or("<none>");
        tracing::info!(
            target: "scuffed_stat_tracker",
            available = ?outputs,
            selected = %selected,
            "capture outputs"
        );
    }
}

/// Open the local store, log its match count, and write the initial live
/// snapshot so the GUI has current data the moment the daemon takes the lock.
pub(crate) async fn open_store(data_dir: &std::path::Path) -> anyhow::Result<storage::LocalStore> {
    let store = storage::LocalStore::open(data_dir)
        .await
        .map_err(anyhow::Error::from_boxed)
        .context("failed to open local store (is another daemon running?)")?;
    // SurrealKV's flush keeps running after the store handle drops. Hold the
    // directory flock until this process exits so a second writer cannot open
    // the same store during that window.
    store.hold_writer_lock_until_exit();
    let count = store
        .match_count()
        .await
        .map_err(anyhow::Error::from_boxed)?;
    tracing::info!(target: "scuffed_stat_tracker", stored_matches = count, "local store ready");

    // Initial snapshot so the GUI has current data from the moment the daemon
    // takes the store lock (refreshed after every mutation from here on).
    if let Err(e) = store.export_snapshot(data_dir).await {
        tracing::warn!(target: "scuffed_stat_tracker", error = %e, "failed to write initial live snapshot");
    }
    Ok(store)
}

/// One-time startup log lines: auto-detect config, the game-process gate,
/// dev-mode dumps, and the "ready" banner.
pub(crate) fn log_startup_readiness(
    config: &config::Config,
    dump_poll_frames: bool,
    collect_portraits: bool,
) {
    if config.auto_detect.enabled {
        tracing::info!(
            target: "scuffed_stat_tracker",
            poll_secs = ?config.auto_detect.poll_interval_secs,
            cooldown_secs = config.auto_detect.cooldown_secs,
            "auto-detect mode enabled: polling for match end screens"
        );
    }
    if config.game_process_names.is_empty() {
        tracing::info!(target: "scuffed_stat_tracker", "game-process gate disabled (game_process_names is empty)");
    } else {
        tracing::info!(
            target: "scuffed_stat_tracker",
            processes = ?config.game_process_names,
            "captures gated on game process: set game_process_names in config.toml if yours differs"
        );
    }
    if dump_poll_frames {
        tracing::info!(
            target: "scuffed_stat_tracker",
            dir = %config.data_dir.join("debug").join("poll").display(),
            "poll-frame dumping enabled (keeps the last {POLL_DUMP_KEEP} frames)"
        );
    } else if config.debug_ocr_enabled() {
        tracing::info!(
            target: "scuffed_stat_tracker",
            dir = %config.data_dir.join("debug").join("poll").display(),
            "debug_ocr: poll Victory/Defeat evidence frames on confirm and first streak (keeps the last {POLL_DUMP_KEEP} frames)"
        );
    }
    for dir in stale_board_debug_dirs(config.debug_ocr_enabled(), &config.data_dir) {
        tracing::info!(
            target: "scuffed_stat_tracker",
            dir = %dir.display(),
            "this debug folder is no longer written while debug_ocr is off (it shows other players' names); existing files are left as they are"
        );
    }
    tracing::info!(target: "scuffed_stat_tracker", "daemon ready: press Tab in-game to capture scoreboard");

    if collect_portraits {
        tracing::info!(
            target: "scuffed_stat_tracker",
            "portrait collection mode enabled: only fills missing portraits and the Doctrine stand-in; it never overwrites an existing reference"
        );
    }
}
