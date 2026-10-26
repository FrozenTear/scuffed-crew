use stat_tracker::boundary::{self, ResultMark};
use stat_tracker::capture_gate::{self, Counters, GateState};
use stat_tracker::hero_auth::{self, HeroAuthState, HeroSource};
use stat_tracker::{
    capture, config, detect, mapcrops, names, ocr, parse, reader_apply, setup, shadow, storage,
    sync,
};

use std::sync::Arc;
use std::time::Instant;

use anyhow::Context as _;
use chrono::Utc;
use surrealdb_types::Datetime as SurrealDatetime;
use tracing_subscriber::EnvFilter;

mod daemon;
use daemon::*;

/// Window for two word-OCR outcome reads to confirm each other. Sized to span
/// the accolade → rank-screen transition under a starved poller (measured 45s
/// between the last accolade tick and the first rank-screen tick) while still
/// bounding how long a single stray read stays actionable.
const OUTCOME_CONFIRM_WINDOW: std::time::Duration = std::time::Duration::from_secs(60);

/// Bounded retries when startup finds zero keyboards. A miss is usually
/// "not in the `input` group" (only a new login's process credentials can
/// fix that) or `/dev/input` not enumerated yet (a same-process retry can).
/// Kept short so the pid file is not held while the GUI shows a daemon
/// that will never capture. After this, the process exits non-zero and
/// systemd `Restart=on-failure` tries again; a later graphical-session
/// start covers re-login.
const KEYBOARD_OPEN_ATTEMPTS: u32 = 3;

/// Gap between [`KEYBOARD_OPEN_ATTEMPTS`]. This wait also polls SIGTERM
/// and Ctrl+C, without that, `systemctl --user stop` sits until
/// TimeoutStopSec and SIGKILL while the handler is registered but never
/// received.
const KEYBOARD_OPEN_RETRY: std::time::Duration = std::time::Duration::from_secs(2);

/// What to do after a failed keyboard open, before the next attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyboardWait {
    /// Interval elapsed; try `MultiKeyboardStream::open` again.
    Retry,
    /// SIGTERM or Ctrl+C. Exit 0 so an explicit stop is not a failure
    /// (`Restart=on-failure` must not bring the daemon back after stop).
    Shutdown,
}

/// Keyboard stream acquired, or a clean shutdown while waiting to retry.
#[derive(Debug, PartialEq, Eq)]
enum KeyboardAcquire<T> {
    Ready(T),
    Shutdown,
}

/// Polled between keyboard-open attempts. Production uses signals; tests
/// script the events.
trait KeyboardWaiter {
    fn wait(&mut self) -> impl std::future::Future<Output = anyhow::Result<KeyboardWait>> + '_;
}

struct SignalKeyboardWaiter<'a> {
    sigterm: &'a mut tokio::signal::unix::Signal,
}

impl KeyboardWaiter for SignalKeyboardWaiter<'_> {
    async fn wait(&mut self) -> anyhow::Result<KeyboardWait> {
        tokio::select! {
            biased;
            r = tokio::signal::ctrl_c() => {
                r?;
                tracing::info!("shutting down");
                Ok(KeyboardWait::Shutdown)
            }
            _ = self.sigterm.recv() => {
                tracing::info!("SIGTERM received: shutting down");
                Ok(KeyboardWait::Shutdown)
            }
            _ = tokio::time::sleep(KEYBOARD_OPEN_RETRY) => Ok(KeyboardWait::Retry),
        }
    }
}

/// Open a keyboard, retrying on failure. `Err` is "still no keyboard",
/// callers must propagate it so the process exits non-zero. `Ok(Shutdown)`
/// is a clean stop during the retry wait.
async fn acquire_keyboard<T, E, F, W>(
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
                tracing::info!("retrying keyboard open; SIGTERM or Ctrl+C will quit");
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
async fn open_keyboard_or_shutdown(
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

struct PidGuard {
    path: std::path::PathBuf,
    pid: u32,
}

/// Remove `path` only when it still contains `owner`.
///
/// The GUI used to unlink `daemon.pid` before this process finished shutting
/// down. A restart could write a new pid, and an unconditional unlink here
/// deleted that file. Returns whether the file was removed.
fn unlink_pid_file_if_owner(path: &std::path::Path, owner: u32) -> bool {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return false,
        Err(e) => {
            tracing::warn!(
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
                    error = %e,
                    path = %path.display(),
                    "failed to remove daemon pid file"
                );
                false
            }
        },
        Ok(other) => {
            tracing::warn!(
                owner,
                found = other,
                path = %path.display(),
                "daemon pid file belongs to another process; not removing it"
            );
            false
        }
        Err(_) => {
            tracing::warn!(
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

/// Long-lived daemon dependencies and configuration, built once in `main` and
/// threaded as one value instead of 11 positional parameters whose adjacent
/// same-typed members were swap-prone (A1).
struct DaemonCtx {
    backend: capture::CaptureBackend,
    store: storage::LocalStore,
    sync_client: Option<sync::SyncClient>,
    player_name: Option<String>,
    capture_output: Option<String>,
    auto_detect: config::AutoDetectConfig,
    /// Quiet period before a finished game is closed. Already clamped so it
    /// is never shorter than post-match grace.
    finished_game_close: std::time::Duration,
    game_process_names: Vec<String>,
    portrait_matcher: Arc<detect::hero_portrait::PortraitMatcher>,
    collect_portraits: bool,
    dump_poll_frames: bool,
    /// Same gate as Tab OCR dumps (`debug_ocr` / `STAT_TRACKER_DEBUG_OCR`).
    /// When set, the poller writes Victory/Defeat evidence frames on confirm
    /// and first word-OCR streak, not every mid-match tick. Also gates the
    /// `debug/accepted` and `debug/rejected` rings (unblanked board pixels).
    debug_ocr: bool,
    data_dir: std::path::PathBuf,
    /// Consecutive scoreboard captures that parsed but resolved no map. Drives
    /// the `debug/mapmiss/` region dump (see `EMPTY_MAP_DUMP_THRESHOLD`).
    empty_map_reads: std::sync::atomic::AtomicUsize,
    /// Shadow digit recognizer (log only), `Some` when `shadow_recognizer`
    /// is on. Never joined: dropping it on exit drops the sender, so a slow
    /// shadow frame can not delay the final session save.
    shadow: Option<shadow::worker::ShadowWorker>,
    /// `reader = "new"`, or `ocr = "off"`, which implies it
    /// ([`stat_tracker::ocr_switch`]). Saved rows use the new reader: the
    /// six stats come from the digit reader only, and the hero, the top-bar
    /// map, the result words and the game-start screens from the reader
    /// packs where they are loaded (see `config::Config::reader`). The
    /// accolade map the game boundaries are given is the map pack's too
    /// ([`choose_accolade_map`]).
    use_new_reader: bool,
    /// `reader = "new"`: the member's own-name seeds, which find the own row
    /// without a name read (`<data_dir>/private/own-name`, alpha.7 O9).
    /// `None` under ocr-v1, which never opens or creates that folder.
    own_name: Option<Arc<std::sync::Mutex<detect::own_row::SeedStore>>>,
    /// Letter-template names. On unless config says `name_reader = false`.
    /// Independent of [`Self::use_new_reader`].
    names: names::NameSwitch,
}

/// Per-capture parameters decided by the session state machine at Tab time.
struct CaptureRequest {
    /// The Tab press that asked for this capture, on the wall and the
    /// monotonic clock: the report kit ages its portrait references from it.
    tab_at: stat_tracker::report::blank::CaptureTime,
    session_id: String,
    create_session: bool,
    game_outcome: detect::MatchOutcome,
    session_map: Option<String>,
    /// Where [`Self::session_map`] was read. The board case of a different-map
    /// Tab trusts the top bar and the accolade only.
    session_map_source: Option<boundary::MapSource>,
    map_candidates: Vec<String>,
    allow_banner_recovery: bool,
    /// The session's per-cell capture-gate state (last accepted + last raw
    /// counters) and how long ago that capture was accepted. Feeds both the
    /// whole-row game-split signal ([`stats_regressed`]) and the per-cell
    /// monotonic-hold + rate-cap gate ([`capture_gate::apply_gate`]).
    prev_gate: Option<(GateState, std::time::Duration)>,
    /// CG-4 C: career/portrait authority carried across captures of this game.
    hero_auth: HeroAuthState,
    /// An end screen (confirmed outcome or result streak) was already seen
    /// for this session. Logged when a stat split opens the next game.
    after_end_screen: bool,
    /// Time since the reset baseline. Rate checks use this, not the age of
    /// the last stored row.
    baseline_age: Option<std::time::Duration>,
    /// Progressed boards already accepted since the current hint.
    progressed_boards: u8,
    /// The deferred board was carried onto this session from the one that
    /// closed. A board deferred here is dropped when the next capture stays.
    deferred_imported: bool,
    /// See [`ActiveGame::reset_streak`].
    reset_streak: u32,
    /// See [`ActiveGame::reset_baseline`].
    reset_baseline: Option<GateState>,
    /// Player row that owns the reset baseline.
    baseline_row: Option<u32>,
    /// Unconfirmed decided word already on the session.
    hint: Option<detect::MatchOutcome>,
    pending_boundary: bool,
    awaiting_first_board: bool,
    /// See [`ActiveGame::text_fallback_locked`]. A locked text-fallback map
    /// is not replaced by a banner read either.
    text_fallback_locked: bool,
    /// A row of this session carries a member's own map choice
    /// ([`session_has_member_map_edit`]). Filled by the capture path, false
    /// from [`build_capture_request`].
    member_map_edit: bool,
    /// Board held off the previous session. Written onto the session this
    /// capture actually stores, at `carried_deferred_at`.
    carried_deferred: Option<Counters>,
    carried_deferred_hero: Option<String>,
    carried_deferred_at: Option<chrono::DateTime<Utc>>,
    /// Result-word reads collected for this game while `reader = "new"`.
    /// Not persisted. A restart drops them, and the next capture keeps the
    /// ocr-v1 result for that field.
    result_evidence: shadow::result::ResultEvidence,
    /// [`REJOIN_CONTINUE_WINDOW`] is open on this game and no rejoin has
    /// continued it yet ([`rejoin_board`]).
    rejoin_window_open: bool,
    /// See [`ActiveGame::rejoin_carry`].
    rejoin_carry: Option<Counters>,
    /// R9-2: this request is for the fresh session a Tab past the grace
    /// window would open, and this is the finished game's own request. The
    /// board decides which one the capture uses ([`finished_game_tab_probe`]).
    /// `None` from [`build_capture_request`].
    finished_game_probe: Option<Box<CaptureRequest>>,
    /// Tab was pressed for the game's last accepted board after its result
    /// was recorded (not just reported after it: a result can be confirmed
    /// while a capture runs). Only such a board can show the result was
    /// false ([`boundary::finished_game_tab`]).
    last_board_after_result: bool,
}

/// Package version shown by `--version` / `--help`.
///
/// Release CI sets `SST_RELEASE_VERSION` from the git tag so the binary
/// reports the tagged release version; local/dev builds fall back to
/// `CARGO_PKG_VERSION`.
fn package_version() -> &'static str {
    option_env!("SST_RELEASE_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))
}

fn main() -> anyhow::Result<()> {
    // First, before any thread, log line or Tesseract use: libgomp reads
    // OMP_THREAD_LIMIT when it loads, so a start without it (a shell, the
    // app with no unit installed) is replaced here by the same binary with
    // it set.
    // Under the systemd unit the variable is already there and this returns.
    let omp = stat_tracker::omp::ensure_thread_limit();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to start the async runtime")?;
    let result = runtime.block_on(run(omp));
    // Whatever is left of the one deadline. No signal yet means the full
    // deadline, including a self-restart after an update. Tasks that have
    // already finished do not wait it out.
    runtime.shutdown_timeout(stat_tracker::replace::shutdown_time_left());
    result
}

async fn run(omp: stat_tracker::omp::Startup) -> anyhow::Result<()> {
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
        tracing::warn!(error = %e, "could not turn off core dumps");
    }
    stat_tracker::omp::log_startup(&omp);

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
    tracing::info!("Scuffed Stat Tracker starting");
    stat_tracker::report::kit::set_retention(config.report_kit_retention());
    tracing::info!(data_dir = %config.data_dir.display(), "using data directory");

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
        tracing::error!(error = %err, "previous tracker was not replaced");
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
            tracing::error!("{status}. Sync resumes when the URL or token changes in Settings.");
        } else {
            if pause == Some(sync::AuthPauseReason::DeletePending) {
                tracing::error!(
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
            tracing::warn!(error = %e, "could not write run.marker; crash reports will not see this run");
            None
        }
    };

    // Log rotate, prune old pre-vacuum backups, auto-vacuum if store is bloated.
    // Must run before open_store (single-writer SurrealKV).
    if let Err(e) = storage::maintain::startup_maintenance(&config.data_dir).await {
        tracing::warn!(error = %e, "startup maintenance failed: continuing with live store");
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
                tracing::info!("stop asked while waiting for the reader packs, shutting down");
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
    tracing::info!(?backend, "capture backend selected");
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
        tracing::info!("new scoreboard reader on for saved stats");
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
    let result = run_loop(ctx, early_sigterm).await;
    // The status file's counters as they stand when the run ends.
    ocr_switch.lock().unwrap_or_else(|e| e.into_inner()).check();
    if result.is_ok()
        && let Some(guard) = run_marker
    {
        guard.finish();
    }
    result
}

/// The pack keeper's install hook: a names-v1 install reloads the name
/// reader on its own thread. Other packs have readers that watch their
/// folders, so nothing is done for them here.
fn reload_names_after_install(
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
async fn stopped_while_waiting(
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
const PACK_WAIT_LOOK: std::time::Duration = std::time::Duration::from_secs(5);

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
fn keep_waiting_for_packs(
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
async fn reader_packs_arrive(
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
                missing = ?first.missing,
                next_try_secs = ?first.next_try_in.map(|d| d.as_secs()),
                "reader packs are still missing and will not arrive in time; deciding without them"
            );
        }
        return;
    }
    tracing::info!(
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
fn handle_preinit_flags() -> bool {
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
async fn handle_info_flags() -> bool {
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
fn open_sync_client(sync_cfg: Option<&config::SyncConfig>) -> Option<sync::SyncClient> {
    let sync_cfg = sync_cfg?;
    match sync::SyncClient::try_new(sync_cfg.clone()) {
        Ok(client) => Some(client),
        Err(e) => {
            tracing::error!(
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
fn pack_credentials(
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
fn new_reader_in_effect(
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
async fn fetch_player_name_if_needed(config: &mut config::Config, client: &sync::SyncClient) {
    if config.player_name.is_none() {
        match client.fetch_daemon_config().await {
            Ok(remote) if remote.player_name.is_some() => {
                tracing::info!(
                    player_name = %remote.player_name.as_deref().unwrap_or(""),
                    "player_name fetched from server"
                );
                config.player_name = remote.player_name;
            }
            Ok(_) => {
                tracing::info!(
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
                    tracing::warn!(error = %err, "could not record sync auth pause");
                }
                if e.is_token_expired() {
                    tracing::error!(
                        error = %e,
                        "sync token expired on daemon-config: pausing until it is relinked in Settings"
                    );
                } else {
                    tracing::error!(
                        error = %e,
                        "sync token rejected by daemon-config: pausing sync until the URL or token changes in Settings"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "could not fetch daemon config from server (continuing without player_name)");
            }
        }
    }
}

/// Write the pid file (refusing to start if another live daemon already holds
/// it) and return a guard that removes it on drop when it still names us.
fn acquire_pid_guard(data_dir: &std::path::Path) -> anyhow::Result<PidGuard> {
    let pid_path = data_dir.join("daemon.pid");
    if let Ok(text) = std::fs::read_to_string(&pid_path)
        && let Some(existing) = stat_tracker::proc_id::parse_pid_record(&text)
        && stat_tracker::proc_id::pid_is_live_tracker_started(existing.pid, existing.start_ticks)
    {
        tracing::error!(
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
fn lock_down_umask() {
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
async fn maybe_vacuum(data_dir: &std::path::Path) -> anyhow::Result<bool> {
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
async fn log_selected_output(backend: capture::CaptureBackend, config: &config::Config) {
    if let Ok(outputs) = capture::list_outputs(backend).await {
        // `.first()`, not `[0]`: zero outputs (headless / compositor hiccup)
        // must not panic the daemon at startup.
        let selected = config
            .capture_output
            .as_deref()
            .or_else(|| outputs.first().map(String::as_str))
            .unwrap_or("<none>");
        tracing::info!(
            available = ?outputs,
            selected = %selected,
            "capture outputs"
        );
    }
}

/// Open the local store, log its match count, and write the initial live
/// snapshot so the GUI has current data the moment the daemon takes the lock.
async fn open_store(data_dir: &std::path::Path) -> anyhow::Result<storage::LocalStore> {
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
    tracing::info!(stored_matches = count, "local store ready");

    // Initial snapshot so the GUI has current data from the moment the daemon
    // takes the store lock (refreshed after every mutation from here on).
    if let Err(e) = store.export_snapshot(data_dir).await {
        tracing::warn!(error = %e, "failed to write initial live snapshot");
    }
    Ok(store)
}

/// One-time startup log lines: auto-detect config, the game-process gate,
/// dev-mode dumps, and the "ready" banner.
fn log_startup_readiness(config: &config::Config, dump_poll_frames: bool, collect_portraits: bool) {
    if config.auto_detect.enabled {
        tracing::info!(
            poll_secs = ?config.auto_detect.poll_interval_secs,
            cooldown_secs = config.auto_detect.cooldown_secs,
            "auto-detect mode enabled: polling for match end screens"
        );
    }
    if config.game_process_names.is_empty() {
        tracing::info!("game-process gate disabled (game_process_names is empty)");
    } else {
        tracing::info!(
            processes = ?config.game_process_names,
            "captures gated on game process: set game_process_names in config.toml if yours differs"
        );
    }
    if dump_poll_frames {
        tracing::info!(
            dir = %config.data_dir.join("debug").join("poll").display(),
            "poll-frame dumping enabled (keeps the last {POLL_DUMP_KEEP} frames)"
        );
    } else if config.debug_ocr_enabled() {
        tracing::info!(
            dir = %config.data_dir.join("debug").join("poll").display(),
            "debug_ocr: poll Victory/Defeat evidence frames on confirm and first streak (keeps the last {POLL_DUMP_KEEP} frames)"
        );
    }
    for dir in stale_board_debug_dirs(config.debug_ocr_enabled(), &config.data_dir) {
        tracing::info!(
            dir = %dir.display(),
            "this debug folder is no longer written while debug_ocr is off (it shows other players' names); existing files are left as they are"
        );
    }
    tracing::info!("daemon ready: press Tab in-game to capture scoreboard");

    if collect_portraits {
        tracing::info!(
            "portrait collection mode enabled: only fills missing portraits and the Doctrine stand-in; it never overwrites an existing reference"
        );
    }
}

/// The game currently in progress. Opened when the poller sees a game-start
/// screen (map vote / hero select / ban) and reused for every Tab capture until
/// the next game starts, so captures taken across hero swaps all land in one
/// session. `outcome` is filled in when the poller reads the post-match screens
/// (or recovered from a captured frame), then back-filled onto the snapshots.
struct ActiveGame {
    /// Restored after a restart with a result screen already seen for it.
    /// Memory only. The first Tab after [`POST_MATCH_GRACE`] from that
    /// result starts a new game ([`should_start_fresh_session`]).
    restored_with_result: bool,
    /// When this daemon run restored the game from a previous run that did
    /// not finish it (a crash or restart mid-game). Memory only. Opens the
    /// [`REJOIN_CONTINUE_WINDOW`]: a same-map board whose stats dropped
    /// (the player rejoined and the game reset their scoreboard) continues
    /// this game instead of splitting it ([`rejoin_board`]).
    restart_recovered_at: Option<Instant>,
    /// Counters this game had before a rejoin reset its scoreboard. Added to
    /// every later board of this game, so stored rows stay cumulative for
    /// the whole match. Persisted in `active_game.json`: a second restart in
    /// the same game keeps adding it, and no new window opens on a game that
    /// already has one, so post-rejoin stats are never counted twice.
    rejoin_carry: Option<Counters>,
    session_id: String,
    /// alpha.8: the newest pending board's row with the reader's best guess
    /// in each unread stat column, those columns in `suspect_fields`. Stored
    /// when the game closes with no row ([`retire_active_game`]); a board
    /// that stores a row makes it moot.
    best_guess: Option<storage::PersonalMatch>,
    outcome: detect::MatchOutcome,
    /// The map actually being played, once a read names it. Never set from
    /// the map vote. [`Self::map_source`] says which read stored it. A
    /// full-board text fallback is recorded and is not a map for a poll
    /// split or a later different-map Tab until a top bar or an accolade
    /// agrees with it.
    map: Option<String>,
    /// Where [`Self::map`] was read. Absent until a read stores the map.
    /// A skeleton from before 0.4.19 has no source; recovery treats a map
    /// on that file as untrusted text.
    map_source: Option<boundary::MapSource>,
    /// Canonicalized names seen on the map-vote screen. The winner is
    /// unknowable at vote time, so these are CANDIDATES only, they constrain
    /// later OCR reads (a read that isn't one of them is a misread) but are
    /// never stored as the played map themselves.
    map_candidates: Vec<String>,
    /// Whether the `match_session` row has been created (on the first capture).
    session_created: bool,
    /// When `outcome` was recorded. Drives the post-match grace window: Tab
    /// presses shortly after the outcome (the post-match scoreboard) still
    /// belong to this game; later ones belong to the next.
    outcome_recorded_at: Option<Instant>,
    /// When the game was opened (start screen seen or first Tab).
    opened_at: Instant,
    /// Last recorded evidence the game is still this game (capture stored,
    /// outcome/map recorded). Bounds how long an unfinished session can
    /// absorb captures, see [`UNFINISHED_SESSION_IDLE`].
    last_activity: Instant,
    /// Per-cell capture-gate state (last accepted + last raw counters) from the
    /// most recent accepted capture. Scoreboard stats are cumulative within a
    /// match, so this drives both the detector-independent game-split signal
    /// (see [`stats_regressed`]) and the per-cell hold gate
    /// ([`capture_gate::apply_gate`]).
    gate: Option<GateState>,
    /// When `gate` was last updated (an accepted capture).
    last_stats_at: Option<Instant>,
    /// CG-4 C: portrait confirm-not-switch + career-ever-ok for this game.
    hero_auth: HeroAuthState,
    /// Result word seen for this session. An unconfirmed hint stays sealable
    /// until a second progressed board, or one progressed board after an arm.
    /// A confirmed result's grace starts when it is recorded, not when the
    /// word was first sighted.
    result_mark: Option<ResultMark>,
    /// Hero select after a board-followed hint. The first reset signal.
    /// The next fresh board splits and seals. A second progressed board, or
    /// one after this arm, drops the hint.
    pending_boundary: bool,
    /// Opened by a start screen; the first board has not been accepted.
    awaiting_first_board: bool,
    /// Consecutive fresh-match boards already seen. One is not a new game.
    /// A refresh, or counted progress, clears this. An unidentified or
    /// implausible row leaves it.
    reset_streak: u32,
    /// Counters the reset is measured from. Set when the first fresh-match
    /// board is held. A refresh replaces them with the accepted gate. An
    /// unidentified or implausible row leaves them.
    reset_baseline: Option<GateState>,
    /// Player row that owns [`Self::reset_baseline`]. A later board counts
    /// as a fresh reset only on this same row.
    baseline_row: Option<u32>,
    /// When [`Self::reset_baseline`] was taken. Rate checks measure from here.
    baseline_at: Option<Instant>,
    /// Progressed boards accepted since the current hint. The second drops it.
    progressed_boards: u8,
    /// First fresh-match board, held off the current session until a split
    /// stores it on the new one. `deferred_at` is that capture's own time.
    deferred: Option<Counters>,
    deferred_hero: Option<String>,
    deferred_at: Option<chrono::DateTime<Utc>>,
    /// Set when `deferred` was carried from the session that just closed.
    /// The first stored capture writes that board and then clears this,
    /// along with `deferred`. A deferral taken on this session stays false
    /// and is dropped if the next stored board does not split.
    deferred_imported: bool,
    /// A different result replaced the hint. The confirming accolade must
    /// not rewrite a text-fallback map after that. Cleared with the hint,
    /// except when recovery drops the hint because its timestamp cannot be
    /// mapped onto this boot: the lock stays.
    text_fallback_locked: bool,
    /// The text-fallback name a sure banner read replaced on this session
    /// ([`banner_replaces_text_fallback`]). The game's own accolade can still
    /// name it (the full-board text and the accolade agreed on it), so the
    /// poll path counts that accolade as this session's map, not as a
    /// different-map end screen that would split the game (F252-1).
    replaced_text_fallback_map: Option<String>,
    /// Result-word reads for `reader = "new"`. Memory only: a restart loses
    /// them and the next capture falls back the result field to ocr-v1.
    result_evidence: shadow::result::ResultEvidence,
    /// The recorded result may not be cleared by a Tab past the grace window
    /// (R9-2, [`finished_game_tab_probe`]): a member set it in the app, or it
    /// came back from a previous run whose source is not known. Memory only.
    outcome_locked: bool,
    /// When Tab was pressed for the board in [`Self::gate`]. A result
    /// confirmed while that capture ran lands before its report, so
    /// [`Self::last_stats_at`] (report time) can be after a result whose
    /// board is from before it. Memory only: a restored game has none, and
    /// none is never after a result ([`CaptureRequest::last_board_after_result`]).
    last_board_pressed_at: Option<Instant>,
}

impl ActiveGame {
    #[cfg(test)]
    fn open_now(
        session_id: String,
        outcome: detect::MatchOutcome,
        map_candidates: Vec<String>,
    ) -> Self {
        Self::open_at(session_id, outcome, map_candidates, Instant::now())
    }

    fn open_at(
        session_id: String,
        outcome: detect::MatchOutcome,
        map_candidates: Vec<String>,
        now: Instant,
    ) -> Self {
        ActiveGame {
            best_guess: None,
            session_id,
            outcome_recorded_at: (outcome != detect::MatchOutcome::Unknown).then_some(now),
            outcome,
            map: None,
            map_source: None,
            map_candidates,
            session_created: false,
            opened_at: now,
            last_activity: now,
            gate: None,
            last_stats_at: None,
            hero_auth: HeroAuthState::default(),
            result_mark: None,
            pending_boundary: false,
            awaiting_first_board: false,
            reset_streak: 0,
            reset_baseline: None,
            baseline_row: None,
            baseline_at: None,
            progressed_boards: 0,
            deferred: None,
            deferred_hero: None,
            deferred_at: None,
            deferred_imported: false,
            text_fallback_locked: false,
            replaced_text_fallback_map: None,
            result_evidence: shadow::result::ResultEvidence::default(),
            restored_with_result: false,
            restart_recovered_at: None,
            rejoin_carry: None,
            outcome_locked: false,
            last_board_pressed_at: None,
        }
    }

    fn boundary_state(&self) -> boundary::BoundaryState {
        boundary::BoundaryState {
            map: self.map.clone(),
            outcome: self.outcome,
            outcome_at: self.outcome_recorded_at,
            result: self.result_mark,
            reset_streak: self.reset_streak,
            reset_baseline: self.reset_baseline,
            baseline_row: self.baseline_row,
            last_board_at: self.last_stats_at,
            pending_boundary: self.pending_boundary,
            awaiting_first_board: self.awaiting_first_board,
            progressed_boards: self.progressed_boards,
            baseline_at: self.baseline_at,
            hero: self.hero_auth.accepted_hero.clone(),
            deferred: self.deferred,
            gate: self.gate,
            text_fallback_locked: self.text_fallback_locked,
        }
    }

    fn apply_boundary_state(&mut self, state: &boundary::BoundaryState) {
        self.map = state.map.clone();
        self.outcome = state.outcome;
        self.outcome_recorded_at = state.outcome_at;
        self.result_mark = state.result;
        self.reset_streak = state.reset_streak;
        self.reset_baseline = state.reset_baseline;
        self.baseline_row = state.baseline_row;
        self.pending_boundary = state.pending_boundary;
        self.awaiting_first_board = state.awaiting_first_board;
        self.progressed_boards = state.progressed_boards;
        self.baseline_at = state.baseline_at;
        self.deferred = state.deferred;
        if state.deferred.is_none() {
            self.deferred_hero = None;
            self.deferred_at = None;
            self.deferred_imported = false;
        }
        if state.last_board_at.is_some() {
            self.last_stats_at = state.last_board_at;
        }
        self.gate = state.gate;
        self.text_fallback_locked = state.text_fallback_locked;
    }

    /// Top bar and accolade. A text fallback, and a missing source, are not.
    fn map_is_trusted(&self) -> bool {
        self.map_source
            .is_some_and(boundary::MapSource::trusted_for_board_split)
    }

    fn has_post_result(&self) -> bool {
        boundary::has_post_result(self.outcome, self.result_mark)
    }

    fn finished(&self) -> bool {
        !matches!(self.outcome, detect::MatchOutcome::Unknown)
    }

    fn record_outcome(&mut self, outcome: detect::MatchOutcome) {
        self.record_outcome_at(outcome, Instant::now());
    }

    fn record_outcome_at(&mut self, outcome: detect::MatchOutcome, now: Instant) {
        self.outcome = outcome;
        self.outcome_recorded_at = Some(now);
        self.last_activity = now;
        self.outcome_locked = false;
    }

    /// A result the member set in the app (SetOutcome, EditMatch). A Tab
    /// never clears it (R9-2).
    fn record_member_outcome(&mut self, outcome: detect::MatchOutcome) {
        self.record_outcome(outcome);
        self.outcome_locked = outcome.is_decided();
    }

    fn touch_at(&mut self, now: Instant) {
        self.last_activity = now;
    }

    /// Remember a result word. A repeat of the same outcome keeps the first
    /// sighting. The post-match grace is [`Self::outcome_recorded_at`],
    /// stamped when the result is recorded. The hint stays sealable until a
    /// second progressed board, or one progressed board after an arm.
    fn note_result(&mut self, outcome: detect::MatchOutcome, confirmed: bool) {
        if !outcome.is_decided() {
            return;
        }
        let seen_at = self
            .result_mark
            .filter(|m| m.outcome == outcome)
            .map(|m| m.seen_at)
            .unwrap_or_else(Instant::now);
        let confirmed = confirmed
            || self
                .result_mark
                .is_some_and(|m| m.outcome == outcome && m.confirmed);
        self.result_mark = Some(ResultMark {
            outcome,
            confirmed,
            seen_at,
        });
    }
}

/// On-disk mirror of [`ActiveGame`] (wall-clock timestamps instead of
/// `Instant`s). The session state machine is otherwise memory-only, and daemon
/// restarts are routine, without this, a restart mid-game either split the
/// game (new session per Tab) or merged it into whatever came next.
#[derive(serde::Serialize, serde::Deserialize)]
struct PersistedGame {
    session_id: String,
    outcome: detect::MatchOutcome,
    #[serde(default)]
    map: Option<String>,
    /// Missing on skeletons written before 0.4.19. Recovery treats a map
    /// with no source as untrusted text ([`boundary::MapSource::TextFallback`]).
    /// A file with no map keeps `None`.
    #[serde(default)]
    map_source: Option<boundary::MapSource>,
    #[serde(default)]
    map_candidates: Vec<String>,
    session_created: bool,
    opened_at: chrono::DateTime<Utc>,
    last_activity: chrono::DateTime<Utc>,
    outcome_recorded_at: Option<chrono::DateTime<Utc>>,
    // Renamed from `last_stats: (u32,u32,u32)`; an old on-disk skeleton simply
    // defaults this to None (one game's cross-restart gate memory lost, the
    // file is best-effort recovery, never the capture itself).
    #[serde(default)]
    gate: Option<GateState>,
    #[serde(default)]
    last_stats_at: Option<chrono::DateTime<Utc>>,
    #[serde(default)]
    hero_auth: HeroAuthState,
    #[serde(default)]
    result_outcome: Option<detect::MatchOutcome>,
    #[serde(default)]
    result_confirmed: bool,
    #[serde(default)]
    result_seen_at: Option<chrono::DateTime<Utc>>,
    #[serde(default)]
    pending_boundary: bool,
    #[serde(default)]
    awaiting_first_board: bool,
    #[serde(default)]
    reset_streak: u32,
    #[serde(default)]
    reset_baseline: Option<GateState>,
    #[serde(default)]
    baseline_row: Option<u32>,
    #[serde(default)]
    deferred: Option<Counters>,
    #[serde(default)]
    deferred_hero: Option<String>,
    #[serde(default)]
    deferred_at: Option<chrono::DateTime<Utc>>,
    #[serde(default)]
    deferred_imported: bool,
    #[serde(default)]
    progressed_boards: u8,
    #[serde(default)]
    baseline_at: Option<chrono::DateTime<Utc>>,
    /// Missing on older skeletons. A file from before this field was not
    /// mid-replacement, so recovery leaves the window open.
    #[serde(default)]
    text_fallback_locked: bool,
    /// See [`ActiveGame::replaced_text_fallback_map`]. Missing on older
    /// skeletons, which had no banner rename.
    #[serde(default)]
    replaced_text_fallback_map: Option<String>,
    /// See [`ActiveGame::best_guess`]. Kept so a restart mid-game still
    /// stores the guess when the game closes. Missing on older skeletons.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    best_guess: Option<storage::PersonalMatch>,
    /// See [`ActiveGame::rejoin_carry`]. Missing on older skeletons, which
    /// had no rejoin carry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rejoin_carry: Option<Counters>,
}

fn active_game_path(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir.join("active_game.json")
}

/// Persist (or clear) the open-game skeleton. Fire-and-forget: losing this
/// file only degrades restart recovery, never the capture itself.
fn untracked_note_key(session_id: &str, reason: &str) -> String {
    format!("{session_id}\n{reason}")
}

fn noted_untracked() -> &'static std::sync::Mutex<Option<std::collections::HashSet<String>>> {
    static SEEN: std::sync::Mutex<Option<std::collections::HashSet<String>>> =
        std::sync::Mutex::new(None);
    &SEEN
}

/// Process-local record of jsonl notes. The file is read once, not on every
/// Deathmatch Tab.
fn untracked_already_noted(data_dir: &std::path::Path, session_id: &str, reason: &str) -> bool {
    let mut guard = noted_untracked()
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    if guard.is_none() {
        let mut set = std::collections::HashSet::new();
        let path = data_dir.join("debug").join("unrecorded_games.jsonl");
        if let Ok(text) = std::fs::read_to_string(path) {
            for line in text.lines() {
                let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
                    continue;
                };
                let Some(sid) = value.get("session_id").and_then(|v| v.as_str()) else {
                    continue;
                };
                let Some(why) = value.get("reason").and_then(|v| v.as_str()) else {
                    continue;
                };
                set.insert(untracked_note_key(sid, why));
            }
        }
        *guard = Some(set);
    }
    guard
        .as_ref()
        .is_some_and(|set| set.contains(&untracked_note_key(session_id, reason)))
}

fn remember_untracked_note(session_id: &str, reason: &str) {
    let mut guard = noted_untracked()
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    guard
        .get_or_insert_with(std::collections::HashSet::new)
        .insert(untracked_note_key(session_id, reason));
}

fn append_unrecorded_row(data_dir: &std::path::Path, row: &serde_json::Value) {
    let dir = data_dir.join("debug");
    if std::fs::create_dir_all(&dir).is_ok()
        && let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("unrecorded_games.jsonl"))
    {
        use std::io::Write;
        let _ = writeln!(file, "{row}");
    }
}

/// Deathmatch and practice are local only. Log once at info and append the
/// same jsonl the unrecorded path uses, so a later close does not warn again.
fn note_untracked_once(data_dir: &std::path::Path, g: &ActiveGame, reason: &str) {
    if untracked_already_noted(data_dir, &g.session_id, reason) {
        return;
    }
    tracing::info!(
        session_id = %g.session_id,
        map = g.map.as_deref().unwrap_or("?"),
        reason,
        "untracked map: not uploaded"
    );
    let row = serde_json::json!({
        "closed_at": Utc::now().to_rfc3339(),
        "session_id": g.session_id,
        "outcome": g.outcome.to_string(),
        "map": g.map,
        "age_secs": g.opened_at.elapsed().as_secs(),
        "reason": reason,
    });
    remember_untracked_note(&g.session_id, reason);
    append_unrecorded_row(data_dir, &row);
}

const QUIET_CLOSE_REASON: &str = "finished game closed after quiet period";
const SHUTDOWN_CLOSE_REASON: &str = "daemon shutdown";
const UNFINISHED_MAX_REASON: &str = "unfinished game closed after several hours";
const EMPTY_IDLE_REASON: &str = "empty game closed after 20 minutes idle";

/// Quiet close, shutdown, and the long unfinished bound each keep their own
/// message. Every other close is a new-game boundary.
fn session_close_log(reason: &str) -> &'static str {
    match reason {
        QUIET_CLOSE_REASON => QUIET_CLOSE_REASON,
        SHUTDOWN_CLOSE_REASON => SHUTDOWN_CLOSE_REASON,
        UNFINISHED_MAX_REASON => UNFINISHED_MAX_REASON,
        EMPTY_IDLE_REASON => EMPTY_IDLE_REASON,
        _ => "closed session at new-game boundary",
    }
}

#[cfg(test)]
thread_local! {
    static LAST_END_SCREEN_SPLIT: std::cell::Cell<Option<String>> =
        const { std::cell::Cell::new(None) };
}

fn remember_end_screen_split(line: &str) {
    #[cfg(test)]
    LAST_END_SCREEN_SPLIT.with(|slot| slot.set(Some(line.to_string())));
    #[cfg(not(test))]
    let _ = line;
}

/// End-screen split fields for the daemon log. A missing accolade map prints
/// `none` instead of panicking. The arm flag is the session's pending boundary.
fn end_screen_split_log(
    pending_boundary: bool,
    open: Option<&boundary::OpenNew>,
    seal: Option<detect::MatchOutcome>,
) -> String {
    let (armed, map) = open
        .and_then(|open| boundary::end_screen_log_fields(pending_boundary, open))
        .unwrap_or_else(|| (pending_boundary, "none".to_string()));
    boundary::format_end_screen_split(armed, seal, &map)
}

/// A game that closes without a single accepted Tab capture never reached the
/// store: `session_created` stays false, the outcome and map the poller saw
/// live only in `active_game.json` and are overwritten by the next game.
/// Say so at WARN, and keep a one-line record in `debug/unrecorded_games.jsonl`
/// so the evening can be reconstructed from disk after the journal rotates.
/// A board that was just written sets `session_created`, so this does not
/// warn for that session.
fn note_unrecorded_game(data_dir: &std::path::Path, g: &ActiveGame, reason: &str) {
    if parse::map_is_untracked(g.map.as_deref().unwrap_or("")) {
        note_untracked_once(
            data_dir,
            g,
            parse::untracked_close_reason(g.map.as_deref().unwrap_or("")),
        );
        return;
    }
    if g.session_created {
        return;
    }
    // `reader = "new"`: boards were read, but a stat column never had a
    // sure read, so the game has no complete row and nothing was stored: a
    // placeholder is never stored or uploaded as a number (F266-1).
    let unread = g.gate.map(|gate| gate.unread_cols()).unwrap_or_default();
    // "Never read" and "read, and not taken" are told apart (R3-2): a first
    // read is not taken on a Tab where another column dropped.
    let not_taken = g.gate.map(|gate| gate.refused_cols()).unwrap_or_default();
    let never_read: Vec<&str> = unread
        .iter()
        .copied()
        .filter(|col| !not_taken.contains(col))
        .collect();
    if unread.is_empty() {
        tracing::warn!(
            session_id = %g.session_id,
            outcome = %g.outcome,
            map = g.map.as_deref().unwrap_or("?"),
            age_secs = g.opened_at.elapsed().as_secs(),
            reason,
            "game closed with NO Tab captures: it was not recorded (no scoreboard seen: Tab not pressed, keyboard grabbed, or capture rejected)"
        );
    } else {
        tracing::warn!(
            session_id = %g.session_id,
            outcome = %g.outcome,
            map = g.map.as_deref().unwrap_or("?"),
            age_secs = g.opened_at.elapsed().as_secs(),
            reason,
            ?never_read,
            ?not_taken,
            "game closed and NOT stored: it has no number for every stat. `never_read`: no Tab gave a sure read of the column. `not_taken`: a sure read was seen and not taken, because other columns had dropped on that Tab (it may have been the next game's)"
        );
    }
    let mut row = serde_json::json!({
        "closed_at": Utc::now().to_rfc3339(),
        "session_id": g.session_id,
        "outcome": g.outcome.to_string(),
        "map": g.map,
        "age_secs": g.opened_at.elapsed().as_secs(),
        "reason": reason,
    });
    if !unread.is_empty() {
        row["unread"] = serde_json::json!(unread);
        row["never_read"] = serde_json::json!(never_read);
        row["read_not_taken"] = serde_json::json!(not_taken);
    }
    append_unrecorded_row(data_dir, &row);
}

/// Close the in-memory session because a new game is starting.
///
/// A confirmed outcome is already in the store. A provisional hint is written
/// when `seal` is set: a map vote, a hero ban, a hero select with no board
/// yet, or a stat reset or gap that closed a hinted session. A stat split
/// passes `report.seal` the same way. A session with no Tab captures is
/// logged to `debug/unrecorded_games.jsonl`.
async fn retire_active_game(
    st: &mut SessionState,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    seal: Option<detect::MatchOutcome>,
    reason: &str,
) {
    // The game leaves memory in `begin_retire`, before these store calls
    // return. Dropping this future cancels the best-guess and the sealed
    // outcome, and close_session_on_shutdown then has nothing left to save.
    let Some((g, had_outcome)) = begin_retire(st, data_dir, seal, reason) else {
        return;
    };
    commit_retired_game(st, store, data_dir, g, had_outcome, reason).await;
}

/// Test-only pause after the game has left memory and before the store
/// writes. Release builds do not have this hook.
#[cfg(test)]
async fn retire_after_pause<P>(
    st: &mut SessionState,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    seal: Option<detect::MatchOutcome>,
    reason: &str,
    pause: P,
) where
    P: std::future::Future<Output = ()>,
{
    let Some((g, had_outcome)) = begin_retire(st, data_dir, seal, reason) else {
        return;
    };
    pause.await;
    commit_retired_game(st, store, data_dir, g, had_outcome, reason).await;
}

fn begin_retire(
    st: &mut SessionState,
    data_dir: &std::path::Path,
    seal: Option<detect::MatchOutcome>,
    reason: &str,
) -> Option<(ActiveGame, bool)> {
    // The rank-screen log must not hide the next game's same result. Clear
    // it before the early return: a Tab-opened game can close while the
    // rank screen is up even when no game is active.
    stat_tracker::detect::match_end::clear_rank_result_log();
    let mut g = st.active_game.take()?;
    // The game is over: its unblanked portrait cells leave memory and a
    // below-floor column calibration must not stick into the next game.
    stat_tracker::ocr::clear_below_floor_column_cache();
    stat_tracker::report::kit::forget_portraits(data_dir, &g.session_id);
    let had_outcome = g.finished();
    if !had_outcome {
        // Only an explicit two-signal seal. A leftover hint is not an outcome.
        let outcome = boundary::outcome_sealed_on_close(g.outcome, seal);
        if let Some(outcome) = outcome {
            g.record_outcome(outcome);
            tracing::info!(
                outcome = %outcome,
                session_id = %g.session_id,
                reason,
                "sealing session with end-screen outcome at new-game boundary"
            );
        }
    }
    Some((g, had_outcome))
}

async fn commit_retired_game(
    st: &mut SessionState,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    mut g: ActiveGame,
    had_outcome: bool,
    reason: &str,
) {
    store_best_guess_on_close(store, data_dir, &mut g).await;
    if g.session_created && g.finished() && !had_outcome {
        if let Err(e) = store
            .set_session_outcome(&g.session_id, &g.outcome.to_string())
            .await
        {
            tracing::warn!(error = %e, "failed to seal session outcome");
        }
        refresh_snapshot(store, data_dir).await;
    }
    // No result ever arrived. The server cannot store that. The command
    // tick marks the rows synced locally and does not send them.
    if g.session_created && !g.finished() {
        st.settle_closed_unknown = Some(g.session_id.clone());
    }
    if g.session_created {
        tracing::info!(
            session_id = %g.session_id,
            outcome = %g.outcome,
            map = g.map.as_deref().unwrap_or("?"),
            reason,
            "{}",
            session_close_log(reason)
        );
    } else {
        note_unrecorded_game(data_dir, &g, reason);
    }
}

/// Close a quiet session the way a boundary does, then schedule an upload.
///
/// The session id is left on the stored rows. `active_game.json` is removed
/// before the upload so a restart cannot close the same session again.
/// `sync_now` schedules [`schedule_store_sync`]. It does not run the
/// shutdown upload. No screen capture and no OCR.
async fn close_quiet_session_and_sync<F, Fut>(
    st: &mut SessionState,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    now: Instant,
    close_after: std::time::Duration,
    sync_now: F,
) -> bool
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let Some(game) = st.active_game.as_ref() else {
        return false;
    };
    let Some(reason) = quiet_close_reason(game, now, close_after) else {
        return false;
    };
    let session_id = game.session_id.clone();
    let carried = game
        .deferred
        .map(|counters| (counters, game.deferred_hero.clone(), game.deferred_at));
    tracing::info!(
        session_id = %session_id,
        reason,
        "closing quiet session"
    );
    // No seal. A finished game already has its outcome. An unfinished game
    // closes here only after [`UNFINISHED_SESSION_MAX`], or after
    // [`UNFINISHED_SESSION_IDLE`] when it is empty ([`is_empty_tab_game`]).
    retire_active_game(st, store, data_dir, None, reason).await;
    if let Some((counters, hero, at)) = carried {
        let mut opened = ActiveGame::open_at(
            format!("{:016x}", rand_id()),
            detect::MatchOutcome::Unknown,
            Vec::new(),
            now,
        );
        opened.deferred = Some(counters);
        opened.deferred_hero = hero;
        opened.deferred_at = at;
        opened.deferred_imported = true;
        st.active_game = Some(opened);
        st.last_game_open = Some(now);
        persist_active_game(data_dir, st.active_game.as_ref());
    } else {
        persist_active_game(data_dir, None);
    }
    sync_now().await;
    true
}

/// Apply one [`boundary::decide_poll`] result: record on the open session, or
/// close it and start the next one.
async fn apply_poll_decision(
    st: &mut SessionState,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    decision: boundary::PollDecision,
    now: Instant,
    use_new_reader: bool,
) {
    if let boundary::PollDecision::IgnoreContradictory { kept, ignored } = &decision {
        tracing::info!(
            kept = %kept,
            ignored = %ignored,
            session_id = st.active_game.as_ref().map(|g| g.session_id.as_str()),
            "result word ignored: it does not override the open session"
        );
    }
    let Some(current) = st.active_game.as_ref() else {
        return;
    };
    let mut state = current.boundary_state();
    let before = state.clone();
    let carried_deferred = current
        .deferred
        .map(|counters| (counters, current.deferred_hero.clone(), current.deferred_at));
    let pending_boundary = current.pending_boundary;
    let (candidates, end_screen) = match &decision {
        boundary::PollDecision::Open(open)
            if open.reason == boundary::CloseReason::EndScreenMap =>
        {
            (open.candidates.clone(), Some(open.clone()))
        }
        boundary::PollDecision::Open(open) => (open.candidates.clone(), None),
        _ => (Vec::new(), None),
    };
    let commit = boundary::commit_poll(&mut state, decision, now);
    if let Some(closed) = commit.closed {
        let new_id = format!("{:016x}", rand_id());
        match closed.reason {
            boundary::CloseReason::MapVote => {
                tracing::info!(
                    ?candidates,
                    session_id = %new_id,
                    "auto-detect: map vote: new game"
                );
            }
            boundary::CloseReason::HeroSelectOrBan => {
                tracing::info!(
                    session_id = %new_id,
                    "auto-detect: hero select/ban: new game (map vote missed)"
                );
            }
            boundary::CloseReason::EndScreenMap => {
                let split =
                    end_screen_split_log(pending_boundary, end_screen.as_ref(), closed.seal);
                remember_end_screen_split(&split);
                tracing::info!(
                    reason = closed.reason.log(),
                    session_id = %new_id,
                    split = %split,
                    "auto-detect: new game boundary"
                );
            }
            other => {
                tracing::info!(reason = other.log(), session_id = %new_id, "auto-detect: new game boundary");
            }
        }
        retire_active_game(st, store, data_dir, closed.seal, closed.reason.log()).await;
        let mut g = ActiveGame::open_at(new_id, state.outcome, candidates, now);
        g.apply_boundary_state(&state);
        // A different-map end screen is an accolade read. That is the new
        // session's map, and it is trusted for a later board-case split.
        if g.map.is_some() {
            g.map_source = Some(boundary::MapSource::Accolade);
        }
        // The deferred board belongs to the session being opened. The fresh
        // state has none; put it back so the next stored capture writes it
        // once. That capture clears the hold.
        if let Some((counters, hero, at)) = carried_deferred {
            g.deferred = Some(counters);
            g.deferred_hero = hero;
            g.deferred_at = at;
            g.deferred_imported = true;
        }
        st.active_game = Some(g);
        st.last_game_open = Some(now);
        clear_cadence_wakes(st);
        st.pending_outcome = None;
        persist_active_game(data_dir, st.active_game.as_ref());
        return;
    }
    let Some(g) = st.active_game.as_mut() else {
        return;
    };
    g.apply_boundary_state(&state);
    if let Some(outcome) = commit.recorded_outcome {
        g.touch_at(now);
        tracing::info!(
            ?outcome,
            session_id = %g.session_id,
            "auto-detect: outcome confirmed from post-match screens"
        );
        if g.session_created {
            if let Err(e) = store
                .set_session_outcome(&g.session_id, &g.outcome.to_string())
                .await
            {
                tracing::warn!(error = %e, "failed to back-fill session outcome");
            }
            refresh_snapshot(store, data_dir).await;
        }
    }
    if let Some(map) = commit.adopted_map {
        if !g.map_candidates.is_empty() && !g.map_candidates.contains(&map) {
            tracing::warn!(
                map = %map,
                candidates = ?g.map_candidates,
                "accolade map is not a vote candidate"
            );
        }
        g.map_source = Some(boundary::MapSource::Accolade);
        tracing::info!(
            map = %map,
            session_id = %g.session_id,
            "map recovered from accolade screen"
        );
        g.touch_at(now);
        if g.session_created {
            // `reader = "new"`: a member's own map choice is never renamed,
            // and the mode stored with it is never rewritten (the top-bar
            // rule of F252-3, for the accolade). That is a rule for the
            // rows that carry the choice, not for the session (F265-1): a
            // choice is stamped on the rows that exist when it is made, and
            // the rows captured after it, the game's newest row among them,
            // take the accolade map like any other row. With ocr-v1 every
            // row is written as before.
            let written = if use_new_reader {
                store
                    .set_session_map_keeping_member_choices(&g.session_id, &map)
                    .await
            } else {
                store.set_session_map(&g.session_id, &map).await.map(|()| 0)
            };
            match written {
                Ok(0) => {}
                Ok(kept) => tracing::info!(
                    session_id = %g.session_id,
                    kept,
                    "accolade map not written to the rows whose map a member chose"
                ),
                Err(e) => tracing::warn!(error = %e, "failed to set session map"),
            }
            refresh_snapshot(store, data_dir).await;
        }
    }
    if state != before {
        persist_active_game(data_dir, Some(g));
    }
}

fn persist_active_game(data_dir: &std::path::Path, game: Option<&ActiveGame>) {
    let path = active_game_path(data_dir);
    let Some(g) = game else {
        let _ = std::fs::remove_file(&path);
        return;
    };
    let now_i = Instant::now();
    let now_w = Utc::now();
    let to_wall = |i: Instant| {
        now_w - chrono::Duration::from_std(now_i.duration_since(i)).unwrap_or_default()
    };
    let persisted = PersistedGame {
        session_id: g.session_id.clone(),
        outcome: g.outcome,
        map: g.map.clone(),
        map_source: g.map_source,
        map_candidates: g.map_candidates.clone(),
        session_created: g.session_created,
        opened_at: to_wall(g.opened_at),
        last_activity: to_wall(g.last_activity),
        outcome_recorded_at: g.outcome_recorded_at.map(to_wall),
        gate: g.gate,
        last_stats_at: g.last_stats_at.map(to_wall),
        hero_auth: g.hero_auth.clone(),
        result_outcome: g.result_mark.map(|m| m.outcome),
        result_confirmed: g.result_mark.is_some_and(|m| m.confirmed),
        result_seen_at: g.result_mark.map(|m| to_wall(m.seen_at)),
        pending_boundary: g.pending_boundary,
        awaiting_first_board: g.awaiting_first_board,
        reset_streak: g.reset_streak,
        reset_baseline: g.reset_baseline,
        baseline_row: g.baseline_row,
        deferred: g.deferred,
        deferred_hero: g.deferred_hero.clone(),
        deferred_at: g.deferred_at,
        deferred_imported: g.deferred_imported,
        progressed_boards: g.progressed_boards,
        baseline_at: g.baseline_at.map(to_wall),
        text_fallback_locked: g.text_fallback_locked,
        replaced_text_fallback_map: g.replaced_text_fallback_map.clone(),
        best_guess: g.best_guess.clone(),
        rejoin_carry: g.rejoin_carry,
    };
    let write = || -> std::io::Result<()> {
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(&persisted)?)?;
        std::fs::rename(&tmp, &path)
    };
    if let Err(e) = write() {
        tracing::debug!(error = %e, "failed to persist active game");
    }
}

/// A held board found on a skeleton that is too old to resume.
struct StaleHeldBoard {
    counters: Counters,
    hero: Option<String>,
    map: Option<String>,
    at: Option<chrono::DateTime<Utc>>,
    outcome: detect::MatchOutcome,
    imported: bool,
}

/// Where a stale hold is written. An imported hold stays on its session.
/// A fresh hold is the next game, so it gets its own unknown session.
struct ArchivedHold {
    session_id: String,
    counters: Counters,
    hero: Option<String>,
    map: String,
    outcome: String,
    at: chrono::DateTime<Utc>,
}

/// A skeleton [`retire_stale_skeleton`] removed, plus a hold to keep.
struct StaleDrop {
    session_id: String,
    archive: Option<ArchivedHold>,
}

/// What [`admit_persisted_game`] decided about `active_game.json`.
enum ActiveAdmission {
    Open(Box<ActiveGame>),
    /// The file parsed, and the game is past the idle bound (or its clock
    /// cannot be mapped onto this boot). The rows are still in the store.
    /// `hold` is a board that was never written.
    Stale {
        session_id: String,
        hold: Option<StaleHeldBoard>,
    },
    Absent,
}

/// Recover the open game from a previous daemon run, if it is still plausibly
/// the current game (last activity within [`UNFINISHED_SESSION_IDLE`]).
/// Timestamps that predate the current boot (recovery across a reboot) are
/// treated as stale rather than clamped. [`startup_session`] is the session
/// restore. [`recover_or_sync_active_game`] deletes a stale skeleton and
/// uploads before that restore.
fn recover_active_game(data_dir: &std::path::Path) -> Option<ActiveGame> {
    match admit_persisted_game(data_dir) {
        ActiveAdmission::Open(game) => Some(*game),
        ActiveAdmission::Stale { .. } | ActiveAdmission::Absent => None,
    }
}

fn admit_persisted_game(data_dir: &std::path::Path) -> ActiveAdmission {
    let bytes = match std::fs::read(active_game_path(data_dir)) {
        Ok(bytes) => bytes,
        Err(_) => return ActiveAdmission::Absent,
    };
    let Ok(persisted) = serde_json::from_slice::<PersistedGame>(&bytes) else {
        return ActiveAdmission::Absent;
    };
    let session_id = persisted.session_id.clone();
    let hold = persisted.deferred.map(|counters| StaleHeldBoard {
        counters,
        hero: persisted.deferred_hero.clone(),
        map: persisted.map.clone(),
        at: persisted.deferred_at,
        outcome: persisted.outcome,
        imported: persisted.deferred_imported,
    });
    let crash_seen = stat_tracker::report::marker::last_crash_seen(data_dir);
    match active_game_from_persisted(persisted, crash_seen) {
        Some(game) => ActiveAdmission::Open(Box::new(game)),
        None => ActiveAdmission::Stale { session_id, hold },
    }
}

fn active_game_from_persisted(
    p: PersistedGame,
    crash_seen: Option<chrono::DateTime<Utc>>,
) -> Option<ActiveGame> {
    let to_instant =
        |w: chrono::DateTime<Utc>| Instant::now().checked_sub((Utc::now() - w).to_std().ok()?);
    if let Some(reason) = restart_resume_refusal(&p, crash_seen, Utc::now()) {
        tracing::info!(
            session_id = %p.session_id,
            map = p.map.as_deref().unwrap_or(""),
            reason,
            "previous game not resumed after restart: the next Tab starts a new game"
        );
        return None;
    }
    let last_activity = to_instant(p.last_activity)?;
    if last_activity.elapsed() > UNFINISHED_SESSION_IDLE {
        return None;
    }
    let result_mark = recover_result_mark(
        p.result_outcome,
        p.result_confirmed,
        p.result_seen_at,
        &to_instant,
    );
    let hint_recovered = result_mark.is_some();
    Some(ActiveGame {
        best_guess: p.best_guess,
        session_id: p.session_id,
        outcome: p.outcome,
        map: p.map.clone(),
        map_source: p
            .map_source
            .or_else(|| p.map.is_some().then_some(boundary::MapSource::TextFallback)),
        map_candidates: p.map_candidates,
        session_created: p.session_created,
        opened_at: to_instant(p.opened_at)?,
        last_activity,
        // An unrecoverable timestamp behaves as "unstamped", which the grace
        // logic already treats as stale, the outcome can't leak forward.
        outcome_recorded_at: p.outcome_recorded_at.and_then(to_instant),
        last_stats_at: p.last_stats_at.and_then(to_instant),
        gate: p.gate,
        hero_auth: p.hero_auth,
        result_mark,
        // An arm with no hint would hold full poll cadence forever.
        pending_boundary: p.pending_boundary && hint_recovered,
        awaiting_first_board: p.awaiting_first_board,
        reset_streak: p.reset_streak,
        reset_baseline: p.reset_baseline,
        baseline_row: p.baseline_row,
        baseline_at: p.baseline_at.and_then(to_instant),
        progressed_boards: p.progressed_boards,
        deferred: p.deferred,
        deferred_hero: p.deferred_hero,
        deferred_at: p.deferred_at,
        deferred_imported: p.deferred_imported,
        // The hint is dropped when its timestamp cannot be mapped onto
        // this boot. The lock stays, so a text-fallback name does not
        // become open for an accolade just because the hint did not
        // survive the restart. The arm is the other way around: it is
        // dropped in that same case, above.
        text_fallback_locked: p.text_fallback_locked,
        replaced_text_fallback_map: p.replaced_text_fallback_map,
        result_evidence: shadow::result::ResultEvidence::default(),
        restored_with_result: hint_recovered,
        restart_recovered_at: None,
        rejoin_carry: p.rejoin_carry,
        // The source of a restored result is not saved: never un-latched.
        outcome_locked: p.outcome.is_decided(),
        last_board_pressed_at: None,
    })
}

/// Longest gap between the previous run stopping and this restart for
/// which the restart resumes that game. A crash and restart mid-game takes
/// well under a minute; the next game's first Tab usually comes after a
/// result screen, a queue and hero select. Past this, the next Tab starts a
/// new game. The gap is measured from when the previous run was last seen
/// alive ([`restart_stop_time`]), not from its last Tab: a player who did
/// not press Tab for a few minutes before a crash is still in that game.
const RESUME_AFTER_RESTART_MAX_GAP: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/// Why the game in `active_game.json` is not resumed after a restart, or
/// `None` when it is clearly the same game. An unfinished game with a
/// stored board resumes only when it has a known map and the previous run
/// stopped at most [`RESUME_AFTER_RESTART_MAX_GAP`] ago
/// ([`restart_stop_time`]). The 20 minute [`UNFINISHED_SESSION_IDLE`]
/// bound on its last capture still applies. A Tab after the restart
/// that names another map still splits on the boundary's different-map
/// rule.
///
/// A game with a result already seen is restored so its result and its
/// post-match scoreboard stay on it. It is marked
/// [`ActiveGame::restored_with_result`], so the first Tab after
/// [`POST_MATCH_GRACE`] from that result starts a new game. A game whose
/// outcome is recorded starts a new game on that Tab anyway.
fn restart_resume_refusal(
    p: &PersistedGame,
    crash_seen: Option<chrono::DateTime<Utc>>,
    now: chrono::DateTime<Utc>,
) -> Option<&'static str> {
    if p.outcome.is_decided() {
        return None;
    }
    // A session with no stored board (opened by a start screen, or holding
    // the next game's board after a shutdown) cannot become a duplicate
    // game, so it keeps the [`UNFINISHED_SESSION_IDLE`] bound alone.
    if !p.session_created {
        return None;
    }
    if known_label(p.map.as_deref()).is_none() {
        return Some("no map recorded, so it cannot be matched");
    }
    if p.last_activity > now {
        return Some("last capture is later than now (clock moved back)");
    }
    match (now - restart_stop_time(p, crash_seen, now)).to_std() {
        Ok(gap) if gap <= RESUME_AFTER_RESTART_MAX_GAP => None,
        Ok(_) => Some("previous run stopped too long ago"),
        Err(_) => Some("last capture is later than now (clock moved back)"),
    }
}

/// When the previous run stopped, as far as the tracker can tell. The crash
/// marker's `last_seen` heartbeat (at most a minute before the crash) when
/// it is later than the game's last capture, else that last capture. An
/// old crash marker from an earlier run is older than the game's last
/// capture and changes nothing; a clean shutdown leaves no marker, so it
/// falls back to the last capture as before. A heartbeat in the future
/// (clock moved back) is ignored.
fn restart_stop_time(
    p: &PersistedGame,
    crash_seen: Option<chrono::DateTime<Utc>>,
    now: chrono::DateTime<Utc>,
) -> chrono::DateTime<Utc> {
    match crash_seen {
        Some(seen) if seen > p.last_activity && seen <= now => seen,
        _ => p.last_activity,
    }
}

/// Log the board that continued a game after a rejoin.
fn note_rejoin(req: &CaptureRequest, rejoin: &RejoinRead, raw: Counters) {
    if rejoin.continued {
        tracing::info!(
            session_id = %req.session_id,
            raw_elims = raw.elims,
            raw_deaths = raw.deaths,
            raw_damage = raw.damage,
            elims = rejoin.counters.elims,
            deaths = rejoin.counters.deaths,
            damage = rejoin.counters.damage,
            "rejoin after restart: same map with reset stats continues the recovered game"
        );
    }
}

/// How long after a restart restored an unfinished game a rejoin can still
/// continue it ([`rejoin_board`]). Measured from the restart, which comes
/// right after the crash. Overwatch puts the player back in the match a
/// minute or two after the client comes back; past this, a stat drop is
/// handled by the normal stat-reset rule and splits as before.
const REJOIN_CONTINUE_WINDOW: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/// Open the rejoin window on a game this run restored unfinished from the
/// previous run: no result seen, a stored board and a known map. A game
/// that a rejoin already continued keeps its carry and gets no new window:
/// its next board counts from the rejoin, below the stored total, and must
/// not be read as a second rejoin.
fn mark_restart_recovered(st: &mut SessionState, now: Instant) {
    if let Some(g) = st.active_game.as_mut()
        && g.rejoin_carry.is_none()
        && !g.finished()
        && g.result_mark.is_none()
        && g.session_created
        && known_label(g.map.as_deref()).is_some()
    {
        g.restart_recovered_at = Some(now);
    }
}

/// What a rejoin does to one board's counters. See [`rejoin_board`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RejoinRead {
    counters: Counters,
    /// The carry to keep on the game after this board.
    carry: Option<Counters>,
    /// This board is the one that continued the game after a rejoin.
    continued: bool,
}

fn add_carry(carry: Counters, cur: Counters, held: &[bool; capture_gate::GATE_COLS]) -> Counters {
    // A held column already carries the game's last accepted value, which
    // includes the carry. Adding it again would double it.
    let add = |col: usize, c: u32, v: u32| if held[col] { v } else { c.saturating_add(v) };
    Counters {
        elims: add(0, carry.elims, cur.elims),
        assists: add(1, carry.assists, cur.assists),
        deaths: add(2, carry.deaths, cur.deaths),
        damage: add(3, carry.damage, cur.damage),
        healing: add(4, carry.healing, cur.healing),
        mitigation: add(5, carry.mitigation, cur.mitigation),
    }
}

/// Rejoin after a crash (2026-10-15 Rialto). Overwatch starts a rejoining
/// player's scoreboard again from zero, so the first boards after the
/// restart look like a fresh match and the stat-reset rule split the game.
///
/// Inside [`REJOIN_CONTINUE_WINDOW`] after [`mark_restart_recovered`], a
/// board continues the restored game when all of these hold:
/// * the board names the session's map (an actual read on this Tab, not
///   the session map filled in), so a different map still starts a new game;
/// * it is the identified own row with trusted cells and not a suspect row;
/// * at least two of elims, deaths and damage fell below the last accepted
///   board.
///
/// The pre-rejoin counters become the game's carry and are added to this
/// and every later board of the game, so the stored rows stay cumulative
/// (pre-crash plus post-rejoin) and never go down. Lower values never
/// overwrite the higher pre-crash ones, and the capture gate sees normal
/// growth. Only one continuation per window: once the carry is set, a later
/// drop goes through the normal reset rule.
fn rejoin_board(req: &CaptureRequest, facts: &BoardFacts<'_>) -> RejoinRead {
    let raw = facts.counters;
    if let Some(carry) = req.rejoin_carry {
        return RejoinRead {
            counters: add_carry(carry, raw, &facts.held),
            carry: Some(carry),
            continued: false,
        };
    }
    let unchanged = RejoinRead {
        counters: raw,
        carry: None,
        continued: false,
    };
    if !req.rejoin_window_open
        || !facts.trusted_cells
        || facts.suspect_row
        || facts.row_id.is_none()
    {
        return unchanged;
    }
    let read_map = facts
        .map_from_panel
        .filter(|m| !m.trim().is_empty())
        .or(Some(facts.parsed_map).filter(|m| !m.trim().is_empty()));
    let same_map = matches!(
        (known_label(req.session_map.as_deref()), known_label(read_map)),
        (Some(session), Some(read)) if session.eq_ignore_ascii_case(read)
    );
    let Some((prev, _)) = req.prev_gate else {
        return unchanged;
    };
    let before = prev.accepted;
    let fell = [
        (0, before.elims, raw.elims),
        (2, before.deaths, raw.deaths),
        (3, before.damage, raw.damage),
    ]
    .iter()
    .filter(|(col, b, c)| !facts.held[*col] && c < b)
    .count();
    if !same_map || fell < 2 {
        return unchanged;
    }
    RejoinRead {
        counters: add_carry(before, raw, &facts.held),
        carry: Some(before),
        continued: true,
    }
}

/// Test double for a stale start. Production uses [`begin_daemon_session`]
/// and does not await this.
///
/// The callback receives the dropped session id. `Some` settles that
/// session's unknown rows and still uploads decided rows. `None` is a
/// fresh skeleton or an empty start, and nothing is settled. A fresh
/// skeleton stays on disk. [`startup_session`] restores it afterwards,
/// including `last_game_open`.
#[cfg_attr(not(test), allow(dead_code))]
async fn recover_or_sync_active_game<F, Fut>(
    data_dir: &std::path::Path,
    store: &storage::LocalStore,
    sync_now: F,
) -> SessionState
where
    F: FnOnce(Option<String>) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let dropped = retire_stale_skeleton(data_dir);
    if let Some(drop) = dropped.as_ref()
        && let Some(archive) = &drop.archive
    {
        archive_stale_hold(store, data_dir, archive).await;
    }
    let dropped = dropped.map(|drop| drop.session_id);
    sync_now(dropped.clone()).await;
    if let Some(session_id) = &dropped {
        tracing::info!(
            session_id = %session_id,
            "synced unsynced rows after dropping a stale active game"
        );
    }
    startup_session(data_dir)
}

/// A result timestamp that cannot be mapped back onto this boot drops the
/// hint only. The rest of the recovered session still loads, including the
/// text-fallback lock: losing the hint must not reopen the accolade window.
fn recover_result_mark(
    outcome: Option<detect::MatchOutcome>,
    confirmed: bool,
    seen: Option<chrono::DateTime<Utc>>,
    to_instant: &impl Fn(chrono::DateTime<Utc>) -> Option<Instant>,
) -> Option<ResultMark> {
    let outcome = outcome.filter(|outcome| outcome.is_decided())?;
    let seen_at = to_instant(seen?)?;
    Some(ResultMark {
        outcome,
        confirmed,
        seen_at,
    })
}

/// How long after a game's outcome is recorded that Tab captures still belong
/// to it. The post-match scoreboard is typically inspected right after the
/// result screens; without this window each such Tab opened a duplicate
/// session for the same match and double-counted it. Past the window, the
/// finished result must not leak onto the next match's captures.
const POST_MATCH_GRACE: std::time::Duration = std::time::Duration::from_secs(75);

/// Quiet time before a finished game is closed when nothing further is
/// recorded. Counted from `last_activity` (a stored capture, a recorded
/// outcome, an accolade map, or the session open). Never shorter than
/// [`POST_MATCH_GRACE`].
fn finished_game_close_after(configured_secs: u64) -> std::time::Duration {
    std::time::Duration::from_secs(configured_secs.max(POST_MATCH_GRACE.as_secs()))
}

/// Why the command-tick timer is closing the open session. Not a new-game
/// boundary: the session id stays, and no outcome is invented. Unfinished
/// games stay open on this timer so a late result can still attach.
/// [`UNFINISHED_SESSION_IDLE`] still starts a fresh session when a later
/// Tab arrives. An unfinished game is closed at the next new-game
/// boundary, or on the next start once the on-disk session is older than
/// [`UNFINISHED_SESSION_IDLE`]. A clean shutdown keeps that skeleton so a
/// restart inside the bound resumes it. After [`UNFINISHED_SESSION_MAX`]
/// with no result, this timer closes it. Its `unknown` rows are marked
/// synced locally and are not sent. A GUI SetOutcome requeues them with
/// `edited` set.
///
/// An empty game ([`is_empty_tab_game`]) has nothing a late result could
/// attach to, so it closes after [`UNFINISHED_SESSION_IDLE`] instead. That
/// is the same bound after which the next Tab would start a fresh game
/// anyway. Nothing is stored or uploaded for it.
fn quiet_close_reason(
    game: &ActiveGame,
    now: Instant,
    close_after: std::time::Duration,
) -> Option<&'static str> {
    if !game.finished() {
        let idle = now.saturating_duration_since(game.last_activity);
        if idle >= UNFINISHED_SESSION_MAX {
            return Some(UNFINISHED_MAX_REASON);
        }
        if is_empty_tab_game(game) && idle >= UNFINISHED_SESSION_IDLE {
            return Some(EMPTY_IDLE_REASON);
        }
        return None;
    }
    let close_after = close_after.max(POST_MATCH_GRACE);
    let grace_expired = game
        .outcome_recorded_at
        .is_none_or(|recorded| now.saturating_duration_since(recorded) > POST_MATCH_GRACE);
    let quiet = now.saturating_duration_since(game.last_activity) >= close_after;
    if grace_expired && quiet {
        Some(QUIET_CLOSE_REASON)
    } else {
        None
    }
}

/// A game a Tab opened whose captures were all rejected: no row stored, no
/// board read, no held board, no result hint, and not opened by a start
/// screen. On 2026-10-17 a Tab on the rank screen 81s after a Victory opened
/// one, and it stayed open for [`UNFINISHED_SESSION_MAX`].
fn is_empty_tab_game(game: &ActiveGame) -> bool {
    !game.session_created
        && !game.awaiting_first_board
        && game.gate.is_none()
        && game.best_guess.is_none()
        && game.deferred.is_none()
        && game.result_mark.is_none()
        && game.rejoin_carry.is_none()
}

/// How long an unfinished session stays reusable with no recorded activity.
/// If the poller misses the outcome AND the next game's start screens (likely
/// under Tab starvation, or with auto-detect off), an unbounded session would
/// absorb every capture that follows, yesterday's unfinished game swallowing
/// today's first Tab. Sized comfortably above a long match plus queue time.
const UNFINISHED_SESSION_IDLE: std::time::Duration = std::time::Duration::from_secs(20 * 60);

/// Upper bound for a session that never got a result. A long match still
/// takes its result. Past this, the command tick closes the session and
/// holds the unknown rows locally.
const UNFINISHED_SESSION_MAX: std::time::Duration = std::time::Duration::from_secs(6 * 60 * 60);

/// Minimum age of a poller-opened game before a Tab frame's accolade and
/// rank-screen word reads are trusted ([`FrameFlags::allow_banner_recovery`]);
/// a real end can't appear this early into a match. The end-title text path
/// stays available regardless, and a session freshly opened by the Tab
/// itself (daemon joined mid/post match) is exempt, there the end screen is
/// exactly the evidence being recovered. Named for the colour-flood outcome
/// this gate was made for, which false-positived on heavy mid-fight red and
/// is gone since 0.5.0-rc.9.
const MIN_BANNER_SESSION_AGE: std::time::Duration = std::time::Duration::from_secs(300);

/// Wall-vs-monotonic divergence treated as a suspend (m4). `Instant` is
/// CLOCK_MONOTONIC, which freezes during suspend, after resume every
/// in-memory window (grace, pending TTL, idle bound) silently believes no
/// time passed, so yesterday's post-match state can swallow today's first
/// game. Well above tick jitter and NTP step corrections, far below any
/// meaningful sleep.
const SUSPEND_RESET_GAP: std::time::Duration = std::time::Duration::from_secs(60);

/// A game restored after a restart with a result screen already seen is a
/// finished game once [`POST_MATCH_GRACE`] has passed since that result:
/// the next Tab is the next game, even on the same map.
fn restored_result_is_past_grace(g: &ActiveGame, now: Instant) -> bool {
    g.restored_with_result
        && g.result_mark
            .is_some_and(|m| now.saturating_duration_since(m.seen_at) > POST_MATCH_GRACE)
}

/// Whether a Tab capture should open a fresh session instead of reusing the
/// active one.
fn should_start_fresh_session(game: Option<&ActiveGame>, now: Instant) -> bool {
    match game {
        // No game open, daemon started mid-match or the start screen was missed.
        None => true,
        // Mid-game capture, unless the session has been idle so long it
        // can't plausibly be the same game.
        Some(g) if !g.finished() => {
            now.duration_since(g.last_activity) > UNFINISHED_SESSION_IDLE
                || restored_result_is_past_grace(g, now)
        }
        // Finished: reuse within the grace window (post-match scoreboard of the
        // same match), start fresh after it. An unstamped outcome is treated as
        // stale so a finished result can never leak forward.
        Some(g) => g
            .outcome_recorded_at
            .is_none_or(|t| now.duration_since(t) > POST_MATCH_GRACE),
    }
}

/// How recent a finished game's last accepted board must be for a Tab past
/// the grace window to be compared with it ([`finished_game_tab_probe`]).
/// After a real end, the next game's board needs the end screens, a queue,
/// hero select and play before it can reach a mature board's numbers; well
/// over this. The same gap as [`STAT_SPLIT_MIN_GAP`], below which a stat
/// drop does not split an unfinished game either.
const FINISHED_GAME_TAB_MAX_GAP: std::time::Duration = std::time::Duration::from_secs(120);

/// Whether this Tab would open a fresh session past the grace window of a
/// finished game (as [`should_start_fresh_session`] says) but its board may
/// show that same game still being played (R9-2, 0.5.0-rc.9). Then the board
/// decides ([`boundary::finished_game_tab`]): the Tab's capture carries the
/// open game's request too ([`CaptureRequest::finished_game_probe`]), and
/// the fresh session opens only when its report lands and the board was not
/// that game ([`settle_tab_probe`]).
///
/// Only a game whose last accepted board is recent
/// ([`FINISHED_GAME_TAB_MAX_GAP`]), with a stored row, a trusted map, a
/// result the tracker read itself ([`ActiveGame::outcome_locked`]), and no
/// held fresh-match board, rejoin carry or restored result. Anything else
/// opens the fresh session at the press, as before.
fn finished_game_tab_probe(game: Option<&ActiveGame>, now: Instant) -> bool {
    let Some(g) = game else {
        return false;
    };
    g.finished()
        && !g.outcome_locked
        && g.session_created
        && !g.restored_with_result
        && !g.awaiting_first_board
        && g.deferred.is_none()
        && g.rejoin_carry.is_none()
        && known_label(g.map.as_deref()).is_some()
        && g.map_is_trusted()
        && g.gate.is_some()
        && g.last_stats_at
            .is_some_and(|at| now.saturating_duration_since(at) <= FINISHED_GAME_TAB_MAX_GAP)
        && should_start_fresh_session(game, now)
}

/// Identity of a would-be new session (map-vote, Tab, or stat-regression split).
/// Missing map/hero are wildcards so a vote screen (no hero) can still match
/// an unfinished game that already learned its map from a Tab.
struct IncomingIdentity<'a> {
    map: Option<&'a str>,
    hero: Option<&'a str>,
    vote_candidates: &'a [String],
}

fn known_label(s: Option<&str>) -> Option<&str> {
    s.map(str::trim)
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("unknown"))
}

fn maps_compatible(
    prev_map: Option<&str>,
    incoming_map: Option<&str>,
    vote_candidates: &[String],
) -> bool {
    match (known_label(prev_map), known_label(incoming_map)) {
        (Some(prev), Some(incoming)) => prev.eq_ignore_ascii_case(incoming),
        (Some(prev), None) if !vote_candidates.is_empty() => {
            vote_candidates.iter().any(|c| c.eq_ignore_ascii_case(prev))
        }
        // Unfinished game already has a map; the opener has no map yet
        // (lingering vote with empty OCR, or a split frame that hasn't
        // resolved). Same match until a contradictory map arrives.
        (Some(_), None) => true,
        // No confirmed previous map, identity reuse does not apply.
        // Map-vote debounce still covers a lingering vote with no Tab yet.
        (None, _) => false,
    }
}

fn heroes_compatible(prev: Option<&str>, incoming: Option<&str>) -> bool {
    match (known_label(prev), known_label(incoming)) {
        (Some(prev), Some(incoming)) => prev.eq_ignore_ascii_case(incoming),
        _ => true,
    }
}

/// Reuse the open session instead of splitting when an unfinished (no-outcome)
/// game with the same map+hero is still active or recently active.
///
/// The map-vote cooldown (`auto_detect.cooldown_secs`, default 120s) only
/// debounces a lingering vote opening a *second* new game. A Tab that opened
/// the first session sets `last_game_open`, so after 120s a lingering or
/// false-positive vote can open another, observed as two Games cards for one
/// match (same map/hero, empty first row + later WIN, ~6 min apart).
fn same_unfinished_match(
    unfinished: bool,
    activity_age: std::time::Duration,
    prev_map: Option<&str>,
    prev_hero: Option<&str>,
    incoming: IncomingIdentity<'_>,
) -> bool {
    unfinished
        && activity_age <= UNFINISHED_SESSION_IDLE
        && maps_compatible(prev_map, incoming.map, incoming.vote_candidates)
        && heroes_compatible(prev_hero, incoming.hero)
}

fn should_reuse_unfinished_same_match(
    previous: Option<&ActiveGame>,
    incoming: IncomingIdentity<'_>,
    now: Instant,
) -> bool {
    let Some(g) = previous else {
        return false;
    };
    same_unfinished_match(
        !g.finished(),
        now.saturating_duration_since(g.last_activity),
        g.map.as_deref(),
        g.hero_auth.accepted_hero.as_deref(),
        incoming,
    )
}

/// Map-vote opener: existing debounce, plus same-map/hero reuse so a lingering
/// vote after the 120s cooldown cannot split an unfinished match.
fn map_vote_should_open_new_game(
    active: Option<&ActiveGame>,
    last_game_open: Option<Instant>,
    now: Instant,
    debounce: std::time::Duration,
    vote_candidates: &[String],
) -> bool {
    let game_finished = active.is_some_and(ActiveGame::finished);
    let cooldown_elapsed =
        last_game_open.is_none_or(|t| now.saturating_duration_since(t) >= debounce);
    if !(active.is_none() || game_finished || cooldown_elapsed) {
        return false;
    }
    !should_reuse_unfinished_same_match(
        active,
        IncomingIdentity {
            map: None,
            hero: None,
            vote_candidates,
        },
        now,
    )
}

/// Map vote uses the debounce and the same-map guard. Hero select and ban
/// are blocked only while the session they just opened is still inside the
/// debounce, so a stable screen does not split itself on the next tick.
fn start_screen_blocked(
    active: Option<&ActiveGame>,
    last_game_open: Option<Instant>,
    now: Instant,
    debounce: std::time::Duration,
    screen: &boundary::StartScreen,
) -> bool {
    match screen {
        boundary::StartScreen::MapVote { candidates } => {
            !map_vote_should_open_new_game(active, last_game_open, now, debounce, candidates)
        }
        boundary::StartScreen::HeroSelect | boundary::StartScreen::HeroBan => {
            active.is_some_and(|g| g.awaiting_first_board)
                && last_game_open
                    .is_some_and(|opened| now.saturating_duration_since(opened) < debounce)
        }
    }
}

/// Start a session when the poller sees a start screen and nothing is open.
fn open_detected_game(
    st: &mut SessionState,
    data_dir: &std::path::Path,
    screen: &boundary::StartScreen,
    now: Instant,
    debounce: std::time::Duration,
) {
    let (candidates, log_vote) = match screen {
        boundary::StartScreen::MapVote { candidates } => {
            if !map_vote_should_open_new_game(None, st.last_game_open, now, debounce, candidates) {
                return;
            }
            (candidates.clone(), true)
        }
        boundary::StartScreen::HeroSelect | boundary::StartScreen::HeroBan => (Vec::new(), false),
    };
    let sid = format!("{:016x}", rand_id());
    if log_vote {
        tracing::info!(?candidates, session_id = %sid, "auto-detect: map vote: new game");
    } else {
        tracing::info!(
            session_id = %sid,
            "auto-detect: hero select/ban: new game (map vote missed)"
        );
    }
    let mut g = ActiveGame::open_at(sid, detect::MatchOutcome::Unknown, candidates, now);
    g.awaiting_first_board = true;
    st.active_game = Some(g);
    st.last_game_open = Some(now);
    clear_cadence_wakes(st);
    st.pending_outcome = None;
    persist_active_game(data_dir, st.active_game.as_ref());
}

/// Minimum time since the session's last accepted capture before a stat
/// regression is allowed to split off a new session. Real between-game gaps
/// (result screens + queue + load) measured ≥3 min; consecutive captures of
/// the same board are seconds apart. The gap guard prevents a garbage OCR row
/// followed by a correct one from faking a regression (observed 2026-07-14:
/// a misread E9/D11/DMG61029 row would otherwise split on the next capture).
const STAT_SPLIT_MIN_GAP: std::time::Duration = std::time::Duration::from_secs(120);

/// Whether a capture's player stats regressed versus the session's previous
/// accepted capture. Elims, deaths, and damage are cumulative within one
/// match (hero swaps included), they never decrease. Requiring at least two
/// of the three to drop keeps a single misread column (e.g. an inflated
/// elims read) from faking a boundary, while a real new game, all counters
/// restarting near zero, trips it reliably.
///
/// DUP-1 guard: a column whose CURRENT read is `cur_suspect` (edge-ink
/// clipped/bled, CG-3) must NOT vote as a drop.
///
/// F-CG2-1 raw-continuity vote: DUP-1 alone is not enough, because
/// latch-recovery reads are CLEAN. A CG-2-latched `accepted` sits high while the
/// gate's `last_raw` tracks reality, so after the ≥120s gap the first clean real
/// read (streak still 0, un-latch not yet fired) drops versus the latched
/// `accepted` and would split MID-game (Rialto 2026-07-20T22:23:41Z: "active
/// game replaced after stat-regression split"). So a column votes only if the
/// read also dropped versus `prev_raw` beyond the corroboration band, a
/// latch-recovery read is continuous with `last_raw` and does not, while a
/// genuine new-game reset drops versus BOTH and still splits. When `prev_raw`
/// for a column is itself suspect (untrustworthy for continuity) we fall back
/// conservatively to the DUP-1 behaviour: an accepted-drop alone may vote.
fn stats_regressed(
    prev_acc: (u32, u32, u32),
    prev_raw: (u32, u32, u32),
    cur: (u32, u32, u32),
    cur_suspect: (bool, bool, bool),
    prev_raw_suspect: (bool, bool, bool),
) -> bool {
    let votes = |cur: u32, acc: u32, raw: u32, cur_sus: bool, raw_sus: bool| -> bool {
        if cur_sus || cur >= acc {
            // Current read untrustworthy, or no drop versus accepted → no vote.
            return false;
        }
        if raw_sus {
            // last_raw is not a reliable continuity anchor → DUP-1 fallback.
            true
        } else {
            // Real new-game resets drop below last_raw too; latch-recovery
            // reads stay continuous with it (no vote).
            capture_gate::raw_dropped(raw, cur)
        }
    };
    let drops = [
        votes(
            cur.0,
            prev_acc.0,
            prev_raw.0,
            cur_suspect.0,
            prev_raw_suspect.0,
        ),
        votes(
            cur.1,
            prev_acc.1,
            prev_raw.1,
            cur_suspect.1,
            prev_raw_suspect.1,
        ),
        votes(
            cur.2,
            prev_acc.2,
            prev_raw.2,
            cur_suspect.2,
            prev_raw_suspect.2,
        ),
    ];
    drops.iter().filter(|&&d| d).count() >= 2
}

/// How long an outcome seen with no game open stays applicable to the next
/// session that opens. Covers "daemon started during the post-match screens";
/// without the bound, an outcome from hours ago could stamp a future game.
const PENDING_OUTCOME_TTL: std::time::Duration = std::time::Duration::from_secs(90);

/// Take the pending outcome if it is still fresh; stale ones are discarded.
fn take_fresh_pending(
    pending: &mut Option<(detect::MatchOutcome, Instant)>,
    now: Instant,
) -> Option<detect::MatchOutcome> {
    let (outcome, seen_at) = pending.take()?;
    if now.duration_since(seen_at) <= PENDING_OUTCOME_TTL {
        Some(outcome)
    } else {
        tracing::debug!(?outcome, "discarding stale pending outcome");
        None
    }
}

/// The map to store on a capture snapshot.
///
/// Priority: the session's confirmed map > top-bar label OCR > the fuzzy
/// scoreboard-text read. Map-vote names never appear here, the vote winner
/// is unknowable at vote time (recording candidates as the played map was
/// wrong ~2/3 of the time with 2+ candidates), but when candidates are known
/// they veto OCR reads that aren't among them: the played map must be one of
/// the voted maps, so a read outside the set is a misread.
#[cfg(test)]
fn resolve_map(
    session_map: Option<&str>,
    panel_read: Option<&str>,
    text_read: &str,
    candidates: &[String],
) -> String {
    resolved_map(session_map, panel_read, text_read, candidates).0
}

/// The map to store, and which read supplied it.
///
/// A session that already has a map keeps that name. A top-bar label that
/// agrees with it is [`MapSource::TopBar`], so a later Tab can upgrade a
/// text fallback. A top-bar label on a session with no map is
/// [`MapSource::TopBar`]. The full-board text is
/// [`MapSource::TextFallback`] and is not trusted for a poll split or a
/// later different-map split until a top bar or an accolade agrees.
///
/// Residual risk (PR 163): a Gibraltar vote card read only as `WATCHPOINT`
/// is not a candidate, because a bare watchpoint is not a map. The
/// candidate veto can then drop a real Gibraltar top-bar read.
fn resolved_map(
    session_map: Option<&str>,
    panel_read: Option<&str>,
    text_read: &str,
    candidates: &[String],
) -> (String, Option<boundary::MapSource>) {
    let plausible = |m: &&str| candidates.is_empty() || candidates.iter().any(|c| c == m);
    if let Some(map) = session_map.map(str::trim).filter(|name| !name.is_empty()) {
        let upgrade = panel_read
            .map(str::trim)
            .filter(|panel| {
                !panel.is_empty() && panel.eq_ignore_ascii_case(map) && plausible(panel)
            })
            .map(|_| boundary::MapSource::TopBar);
        return (map.to_string(), upgrade);
    }
    let dropped = |m: &&str| {
        if !plausible(m) {
            tracing::debug!(
                read = %m,
                ?candidates,
                "map read is not a vote candidate: dropping as misread"
            );
        }
    };
    if let Some(panel) = panel_read.inspect(dropped).filter(plausible) {
        return (panel.to_string(), Some(boundary::MapSource::TopBar));
    }
    if let Some(text) = Some(text_read)
        .filter(|m| !m.is_empty())
        .inspect(dropped)
        .filter(plausible)
    {
        return (text.to_string(), Some(boundary::MapSource::TextFallback));
    }
    (String::new(), None)
}

/// What [`stage_capture`] decided to write. Production and the night harness
/// both store from this, so the gate, the map source, and a carried board
/// cannot drift between them.
struct StagedCapture {
    target_session: String,
    create_session: bool,
    map_name: String,
    map_source: Option<boundary::MapSource>,
    outcome: detect::MatchOutcome,
    outcome_label: String,
    gate: capture_gate::GateOutcome,
    carried: Option<Counters>,
    carried_hero: Option<String>,
    carried_at: chrono::DateTime<Utc>,
    split: bool,
    /// Empty when no map was read. Callers store `None`, not `"Unknown"`.
    recorded_map: Option<String>,
    /// `map_name` is a sure banner read that replaces the session's
    /// untrusted text-fallback map ([`banner_replaces_text_fallback`]).
    replaced_text_fallback: bool,
}

/// Gate, map, and carried-board decisions for one planned capture.
///
/// `plan.skip_store` is the hold path and is not staged: nothing is written.
/// The trust bit rides on [`BoardFacts`] with the counters, so this does not
/// grow another argument.
fn stage_capture(
    req: &CaptureRequest,
    plan: &boundary::CapturePlan,
    facts: &BoardFacts<'_>,
    captured_at: chrono::DateTime<Utc>,
) -> StagedCapture {
    let split = plan.split;
    let outcome = plan.stored_outcome;
    let target_session = if split {
        format!("{:016x}", rand_id())
    } else {
        req.session_id.clone()
    };
    let carried = carried_counters_to_write(req.carried_deferred, split, req.deferred_imported);
    let carried_at = req
        .carried_deferred_at
        .unwrap_or(captured_at - chrono::Duration::seconds(1));
    let gate_prev = gate_prev_for_store(req.prev_gate, req.awaiting_first_board);
    let gate_suspect = if facts.suspect_row {
        [true; capture_gate::GATE_COLS]
    } else {
        facts.suspect
    };
    let gate = capture_gate::apply_gate_seeded(
        gate_prev,
        facts.counters,
        gate_suspect,
        facts.held,
        split,
        facts.trusted_cells && !facts.suspect_row,
        gate_seed(carried, carried_at, captured_at, &facts.held),
    );
    let (mut map_name, mut map_source) = if split {
        resolved_map(None, facts.map_from_panel, facts.parsed_map, &[])
    } else {
        resolved_map(
            req.session_map.as_deref(),
            facts.map_from_panel,
            facts.parsed_map,
            &req.map_candidates,
        )
    };
    let replacement = banner_replaces_text_fallback(req, facts, split);
    let replaced_text_fallback = replacement.is_some();
    if let Some(banner) = replacement {
        tracing::info!(
            text_fallback = %map_name,
            banner = %banner,
            "a sure top-bar banner read replaces the text-fallback session map"
        );
        map_name = banner;
        map_source = Some(boundary::MapSource::TopBar);
    }
    StagedCapture {
        target_session,
        create_session: split || req.create_session,
        recorded_map: (!map_name.is_empty()).then(|| map_name.clone()),
        map_name,
        map_source,
        outcome,
        outcome_label: outcome.to_string(),
        gate,
        carried,
        carried_hero: req.carried_deferred_hero.clone(),
        carried_at,
        split,
        replaced_text_fallback,
    }
}

/// The carried board as the capture gate's seed
/// ([`capture_gate::apply_gate_seeded`]), for a capture with held cells.
///
/// The carried board is written onto the same session just before this
/// capture, so a column this capture has no number for takes the carried
/// board's instead of a placeholder 0. `None` when nothing is held (every
/// ocr-v1 capture), so the gate is then the plain one.
fn gate_seed(
    carried: Option<Counters>,
    carried_at: chrono::DateTime<Utc>,
    captured_at: chrono::DateTime<Utc>,
    held: &[bool; capture_gate::GATE_COLS],
) -> Option<(Counters, std::time::Duration)> {
    carried_board(carried, carried_at, captured_at).filter(|_| held.iter().any(|&cell| cell))
}

/// The carried board with its age at this capture.
fn carried_board(
    carried: Option<Counters>,
    carried_at: chrono::DateTime<Utc>,
    captured_at: chrono::DateTime<Utc>,
) -> Option<(Counters, std::time::Duration)> {
    let age = (captured_at - carried_at).to_std().unwrap_or_default();
    carried.map(|board| (board, age))
}

/// A sure banner read that renames the session's untrusted map (F251-1).
///
/// The session map came from the full-board text (or has no source), and
/// this Tab's top-bar map is a sure banner read ([`choose_top_bar_map`])
/// naming another map. It replaces the text-fallback name only while no
/// boundary signal has been seen: no recorded outcome, no unconfirmed result
/// hint (any hint, not only a conflicting one), no armed boundary, no
/// text-fallback lock, no primed reset and no carried board. This is
/// deliberately stricter than the accolade rule
/// (`boundary::text_fallback_open_for_accolade`), which lets an agreeing hint
/// through; do not loosen it to match. After any of those signals the Tab
/// may already belong to the next game, so the old session keeps its name.
///
/// Two more bounds (F252-2, F252-3):
/// * the session's last accepted capture is at most [`STAT_SPLIT_MIN_GAP`]
///   (120 s) before this Tab, the same gap the stat-regression splitter uses
///   to tell one match's boards from the next game's. A longer gap, or no
///   earlier board, is where a missed game end hides, so game 2's first
///   banner read never renames game 1's rows;
/// * no row of the session carries a member's own map choice
///   (`member_map_edit`): a member's pick is never renamed, and its stored
///   mode is never rewritten.
///
/// The vote-candidate veto applies, and a split is a new session that takes
/// the read anyway. A trusted session map (top bar, accolade) is never
/// replaced: the first trusted map wins. The top-bar OCR never replaces it,
/// so `reader = "ocr-v1"` is unchanged.
fn banner_replaces_text_fallback(
    req: &CaptureRequest,
    facts: &BoardFacts<'_>,
    split: bool,
) -> Option<String> {
    if split || !facts.panel_from_banner {
        return None;
    }
    let banner = facts
        .map_from_panel
        .map(str::trim)
        .filter(|m| !m.is_empty())?;
    let session = req
        .session_map
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty())?;
    let trusted = req
        .session_map_source
        .is_some_and(boundary::MapSource::trusted_for_board_split);
    let window_open = !req.text_fallback_locked
        && !req.pending_boundary
        && !req.game_outcome.is_decided()
        && req.hint.is_none()
        && req.reset_streak == 0
        && req.carried_deferred.is_none()
        && !req.member_map_edit
        && req
            .prev_gate
            .is_some_and(|(_, age)| age <= STAT_SPLIT_MIN_GAP);
    let plausible = req.map_candidates.is_empty() || req.map_candidates.iter().any(|c| c == banner);
    (!trusted && window_open && plausible && !session.eq_ignore_ascii_case(banner))
        .then(|| banner.to_string())
}

/// A row of the open session carries a member's own map choice (a map edit
/// or pick: `corrected_map_name`, or `map_name` in `edited_fields`). Only
/// asked when the session map is untrusted, the one case a banner read could
/// rename it; a store error answers true, so nothing is renamed.
async fn session_has_member_map_edit(store: &storage::LocalStore, req: &CaptureRequest) -> bool {
    let untrusted_map = req
        .session_map
        .as_deref()
        .is_some_and(|m| !m.trim().is_empty())
        && !req
            .session_map_source
            .is_some_and(boundary::MapSource::trusted_for_board_split);
    if !untrusted_map {
        return false;
    }
    session_rows_have_member_map_edit(store, &req.session_id).await
}

/// A row of `session_id` carries a member's own map choice (a map edit or
/// pick: `corrected_map_name`, or `map_name` in `edited_fields`). A store
/// error answers true, so nothing of the member's is renamed.
async fn session_rows_have_member_map_edit(store: &storage::LocalStore, session_id: &str) -> bool {
    match store.get_session_snapshots(session_id).await {
        Ok(rows) => rows.iter().any(|m| m.has_member_map_edit()),
        Err(e) => {
            tracing::warn!(error = %e, "session rows unreadable; the map is not renamed");
            true
        }
    }
}

/// `reader = "new"`: copy the member's own hero pick on `session_id`
/// ([`storage::member_hero_edit`]) onto `parsed`, a row of that session about
/// to be written. The stored read stays as read; the pick is the overlay the
/// display and upload use, so the read never overrides it. A store error
/// carries nothing and is logged: the pick cannot be known then.
async fn carry_session_hero_edit(
    store: &storage::LocalStore,
    session_id: &str,
    parsed: &mut storage::PersonalMatch,
) {
    match store.get_session_snapshots(session_id).await {
        Ok(rows) => {
            if let Some(edited) = storage::member_hero_edit(&rows)
                && storage::carry_member_hero_edit(edited, parsed)
            {
                tracing::info!(read = %parsed.hero, "member hero pick kept on this capture");
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "session rows unreadable; member hero pick not checked")
        }
    }
}

/// Everything the blocking vision/OCR pass extracts from one Tab frame.
/// Replaces a 9-tuple whose adjacent same-typed fields were swap-prone.
struct FrameAnalysis {
    /// Result read off this frame only. A split stores this and never the
    /// inherited session outcome.
    frame_outcome: detect::MatchOutcome,
    /// Full-image OCR (hero/map name lookup); the per-cell rows carry stats.
    /// OCR site T7, run under `reader = "ocr-v1"` only: always the empty
    /// text under the new reader ([`full_board_text`]).
    ocr: Result<ocr::OcrResult, Box<dyn std::error::Error + Send + Sync>>,
    /// Column-calibrated per-cell OCR rows, one per scoreboard row. Under
    /// `reader = "new"` nothing is read into them: no name and no stat cell
    /// on any row (`ocr::unread_rows`, [`read_tab_rows`]).
    rows: Vec<ocr::RowOcrResult>,
    /// `reader = "new"`: the board was found empty and this Tab's rows
    /// were read with ocr-v1 instead ([`reader_apply::ocr_fallback_reason`]).
    /// The Tab is then read and stored as an ocr-v1 Tab.
    ocr_fallback: bool,
    /// `reader = "new"`: the digit, icon and banner reads of this Tab, which
    /// carry the trust gate and the stats. `None` under ocr-v1.
    board: Option<shadow::BoardRead>,
    /// Portrait template match: (hero file stem, confidence).
    portrait_hero: Option<(String, f64)>,
    /// Career-panel hero title (most reliable source when present). OCR site
    /// T5, read under `reader = "ocr-v1"` only: always `None` under the new
    /// reader, which uses `pack_hero` instead.
    career_hero: Option<String>,
    /// `reader = "new"`: the hero the heroes-v2 icon reads authenticate: the
    /// own row's and the career panel's portrait, each only when accepted,
    /// and never two different heroes ([`shadow::TabHeroes::authority`]). It
    /// has the career-panel title's authority
    /// ([`hero_auth::resolve_hero_new_reader`]). Always `None` under ocr-v1.
    pack_hero: Option<String>,
    /// Top-bar map label OCR, already canonicalized.
    map_from_panel: Option<String>,
    /// The top bar's raw label ([`top_bar_raw_label`]): the OCR text before
    /// canonicalization, or `ADLERSBRUNN` after a sure banner read of it.
    /// The Adlersbrunn alias check needs the raw label; a literal
    /// Eichenwalde read is not an alias.
    map_panel_raw: Option<String>,
    /// `map_from_panel` is a sure banner read (`reader = "new"`).
    map_from_banner: bool,
    /// Cropped scoreboard the portrait auto-collection reads from. On 5v5
    /// this includes the downward pad so the bottom portrait fits. The
    /// shadow worker gets the unpadded crop, and `debug/accepted` writes
    /// that same unpadded crop ([`debug_accepted_board`]).
    scoreboard: Arc<image::DynamicImage>,
    /// The player's row index. ocr-v1: by name match or brightness
    /// highlight. `reader = "new"`: by the own-name seeds, the highlight
    /// and the career hero's row ([`find_own_row`]); by a name read only
    /// when none of them names a row.
    player_row_idx: Option<usize>,
    /// The name match behind `player_row_idx`, with the name window that
    /// matched. `None` when the row came from the highlight, a seed, or
    /// the career hero, and no name was read.
    name_match: Option<parse::NameMatch>,
    /// Which reader produced `name_match`. `None` when this Tab did not
    /// read a name. `names-v1` is the pack. `ocr-v1` is Tesseract.
    name_recognizer: Option<names::NameRecognizer>,
    /// `reader = "new"`: how `player_row_idx` was found (seed, highlight,
    /// career hero, name fallback). `None` under ocr-v1.
    own_row: Option<detect::own_row::OwnRow>,
    /// `reader = "new"`: the own row's name line (the highlighted row's,
    /// or the career hero's), already through the seeding gate, for the
    /// own-name seed store. Pixels only; never logged, reported or uploaded.
    own_name_candidate: Option<detect::own_row::Candidate>,
    /// `reader = "new"`: what this board says about the trusted own-name
    /// seeds (confirmed, or naming another row than the career hero's).
    own_seed_verdict: detect::own_row::Verdict,
    /// Detected team size (5 or 6), portrait geometry depends on it.
    team_size: usize,
    /// The full frame, kept for the rejected-capture archive.
    frame: image::DynamicImage,
    /// Shadow recognizer input; `None` unless shadow mode is on.
    shadow_input: Option<shadow::worker::ShadowInput>,
}

/// Result of the blocking vision pass: a full analysis, or a cheap rejection
/// by the pre-OCR preflight before any Tesseract/portrait work ran.
enum FrameAnalysisOutcome {
    Analyzed(Box<FrameAnalysis>),
    /// The frame is not a scoreboard: neither row dips nor header labels, or
    /// the pitch failed and the table detector found no table. Carried back
    /// with the frame so it lands in debug/rejected. `scan` is logged with
    /// the rejection. A gameplay frame whose pitch does not settle stays on
    /// this path when it has no table.
    NotAScoreboard {
        outcome: detect::MatchOutcome,
        frame: image::DynamicImage,
        scan: detect::hero_portrait::RowScan,
    },
    /// Row structure is there, but the row pitch does not say 5v5 or 6v6
    /// without contradiction (`RowScan::checked_team_size`). Reading it with
    /// a guessed size cuts every row one slot off and stores neighbours'
    /// stats at high confidence, so the frame is rejected instead.
    TeamSizeUncertain {
        outcome: detect::MatchOutcome,
        frame: image::DynamicImage,
        scan: detect::hero_portrait::RowScan,
    },
    /// The stream had no new frame. Retried like a drawing board, and never
    /// written into debug/rejected.
    StreamMiss,
    /// The share dialog is still up. Retried, and not logged as a missed frame.
    ShareOpening,
    /// The screenshot before this one (a poll's) was still running after
    /// the Tab's lock wait. Retried while Tab is held, and ranked lowest so
    /// it never replaces a frame already grabbed.
    CaptureBusy,
    /// The share was declined or the permission dialog timed out. Not an
    /// error: the one warn already happened, and a later Tab stays quiet.
    Declined,
    /// The screenshot portal is not answering: its request was given up,
    /// or new requests are paused after one was. Not retried (a retry
    /// inside the pause gets the same answer), and not an error: the portal
    /// code logged its own WARN.
    PortalPaused,
}

/// What a Tab capture actually did, reported back to the session state machine.
struct CaptureReport {
    /// A pending board's row with the best guess in its unread columns
    /// ([`apply_best_guess`]). `None` for every other report.
    best_guess: Option<storage::PersonalMatch>,
    /// A snapshot row was written (and the session row created if this was the
    /// session's first capture). False = rejected by a trust gate.
    recorded: bool,
    /// Outcome stored on the snapshot, may have been recovered from the frame
    /// itself (a result word or the end title) when the game's outcome was still
    /// Unknown, in which case the caller back-fills it onto the session.
    outcome: detect::MatchOutcome,
    /// Map stored on the snapshot, if one was read, the caller adopts the
    /// first discovery onto the active game so the whole session shares it.
    map: Option<String>,
    /// Where `map` was read. [`resolved_map`] returns
    /// [`boundary::MapSource::TopBar`] whenever the top bar agrees with the
    /// stored name, trusted or not.
    /// `None` while `map` still holds the kept session map: the top bar was
    /// missing or disagreed, so this capture did not adopt a new source.
    map_source: Option<boundary::MapSource>,
    /// The session the snapshot was actually written to. Differs from the
    /// requested session when a stat regression split off a new game.
    session_id: String,
    /// The capture's stats regressed versus the session's previous accepted
    /// capture, a new game was detected and written to a fresh session; the
    /// caller must replace its active game to match.
    split: bool,
    /// First fresh-match board. Not written to the current session. The
    /// streak and baseline are what the next Tab uses. Also set for a live
    /// match, not only after a result.
    armed_reset: bool,
    /// The hold path. `plan_capture` does not return this for a stored row.
    ignore_row: bool,
    /// Fresh-match streak to keep when `split` is false. A refresh or counted
    /// progress sets this to zero. An unidentified or implausible row leaves it.
    reset_streak: u32,
    /// Baseline to keep when `split` is false and `refresh_baseline` is false.
    reset_baseline: Option<GateState>,
    /// Player row that owns `reset_baseline`.
    baseline_row: Option<u32>,
    /// This capture's accepted gate becomes the reset baseline.
    refresh_baseline: bool,
    /// A second progressed board, or one after an arm, arrived. The caller
    /// clears an unconfirmed hint. A confirmed mark is left alone.
    clear_hint: bool,
    /// First progressed board after a hint. The hint stays.
    count_progress: bool,
    /// This capture's hero came from the career panel.
    career_panel: bool,
    /// Counters of a deferred fresh-match board, stored on the new session
    /// when the reset commits.
    held_counters: Option<Counters>,
    held_hero: Option<String>,
    /// The per-cell capture-gate state after this capture (accepted + raw
    /// counters), carried into the next capture's monotonic-hold + rate-cap
    /// checks and the whole-row regression check. `None` when nothing was
    /// recorded.
    gate_state: Option<GateState>,
    /// Updated hero authority after this capture (carry into the next Tab).
    hero_auth: HeroAuthState,
    /// Hint sealed onto the session this capture closed, when `split` is set.
    seal: Option<detect::MatchOutcome>,
    close_reason: Option<boundary::CloseReason>,
    /// Wall time of a deferred board, taken when that capture was parsed.
    held_at: Option<chrono::DateTime<Utc>>,
    /// `map` is a sure banner read that replaces the session's text-fallback
    /// map ([`banner_replaces_text_fallback`]).
    map_replaces_text_fallback: bool,
    /// See [`ActiveGame::rejoin_carry`]. Set on a stored board of a game a
    /// rejoin continued ([`rejoin_board`]).
    rejoin_carry: Option<Counters>,
    /// R9-2: what the board said when this Tab was past the grace window of
    /// a finished game ([`TabProbe`]); `None` for every other capture, and
    /// for one whose frame was missed or rejected before its board was read.
    finished_game_tab: Option<boundary::FinishedGameTab>,
}

impl CaptureReport {
    /// A board the game has no complete row for yet (`reader = "new"`): it
    /// was read and went through the capture gate, so `gate_state` is set,
    /// but a stat column of this game has still never been read, and no
    /// row was written. The session keeps the gate, the boundary plan and
    /// the hero authority of the capture; the row appears with the first
    /// board that leaves no column unread. Every other report that is not
    /// recorded has no gate state.
    fn pending(&self) -> bool {
        !self.recorded && self.gate_state.is_some()
    }

    /// The capture took its board: a row was recorded, the board was held
    /// as a fresh-match board, or it is kept pending. This, not the retry
    /// loop's verdict on the frame, starts the Tab debounce and drops a
    /// press queued behind the capture (S5-CAP-2). A board a later gate
    /// refused (unreadable own row, the reader's trust gate) is not taken.
    fn board_taken(&self) -> bool {
        self.recorded || self.armed_reset || self.pending()
    }
}

/// Volatile session-tracking state owned by [`run_loop`]. Bundles the mutable
/// locals that the `select!` arms read and write so they travel as one value
/// instead of a fistful of parallel `let mut`s (QUAL-004). Purely
/// organizational, every field keeps the exact meaning it had as a standalone
/// local; no behavior depends on the grouping.
struct SessionState {
    /// Accepted-capture counter driving the periodic sync cadence
    /// (`SYNC_EVERY_N_CAPTURES`).
    capture_count: u32,
    /// When the current game was last (re)opened, debounces poller-driven
    /// new-game detection against `new_game_debounce`.
    last_game_open: Option<Instant>,
    /// Last accepted Tab capture, powers the Tab debounce.
    last_tab_capture: Option<Instant>,
    /// The game currently in progress, opened at the map-vote / hero-select
    /// screen, reused for every capture until the next game starts. Recovered
    /// from the previous run when the daemon restarted mid-game (crash, upgrade,
    /// systemd restart), so the restart neither splits the game into two
    /// sessions nor loses its outcome/map context.
    active_game: Option<ActiveGame>,
    /// Outcome detected by the poller while no game was open, applied to the
    /// next session that opens, if still fresh (`PENDING_OUTCOME_TTL`).
    pending_outcome: Option<(detect::MatchOutcome, Instant)>,
    /// Last result-word OCR read, for confirmation: a word outcome is only
    /// trusted once two reads agree within `OUTCOME_CONFIRM_WINDOW`, so a single
    /// hallucinated OCR read can't finish the open game with a wrong outcome.
    /// The reads may come from different screens (accolade → rank screen) and
    /// need not be consecutive ticks; garbage/transition frames don't reset it.
    /// `map` is this tick's accolade, or the map carried from the previous
    /// agreeing read when this tick has none.
    word_outcome_streak: Option<WordStreak>,
    /// Reference (monotonic, wall) pair for suspend detection
    /// (`SUSPEND_RESET_GAP`): refreshed each cmd tick; wall time advancing much
    /// further than the monotonic clock between ticks means the machine slept.
    suspend_probe: (Instant, chrono::DateTime<Utc>),
    /// Poll-tick OCR stability gate (PR-A): word/phase OCR only runs on crops
    /// that held still since the previous tick, so combat frames stop paying a
    /// Tesseract call every tick. Taken (`mem::take`) into the poll tick's
    /// `spawn_blocking` closure and moved back out through its return value.
    ocr_stability: detect::stability::FrameStability,
    /// Interval ticks skipped since the last performed poll capture (PR-B
    /// adaptive cadence): while [`poll_slow_mode`] holds, only every
    /// [`SLOW_POLL_DIVISOR`]th tick pays the screencopy.
    poll_ticks_skipped: u32,
    /// Deadline through which a POTG / end-reel sighting holds full poll
    /// cadence. None = no active wake. See [`END_REEL_WAKE`].
    end_reel_wake_until: Option<Instant>,
    /// Session id closed while its outcome was still unknown. The next
    /// command tick marks that session's rows synced locally and does not
    /// send them. Other unknown rows stay unsynced. Cleared when the
    /// settle is scheduled.
    settle_closed_unknown: Option<String>,
    /// The running Tab capture's board decides between two sessions
    /// ([`TabProbe`]).
    tab_probe: Option<TabProbe>,
    /// The press of the running Tab capture, taken by the report that lands
    /// for it ([`ActiveGame::last_board_pressed_at`]).
    tab_press: Option<Instant>,
}

/// The Tab request the capture task runs. Ages are measured from `now`,
/// which is [`Instant::now`] in the daemon and the injected clock in tests.
fn build_capture_request(g: &ActiveGame, opened_by_this_tab: bool, now: Instant) -> CaptureRequest {
    let banner_ok =
        opened_by_this_tab || now.saturating_duration_since(g.opened_at) >= MIN_BANNER_SESSION_AGE;
    // This session's own gate. A start screen's first board has none, and
    // the previous game is not an anchor for it.
    let prev_gate = match (g.gate, g.last_stats_at) {
        (Some(state), Some(at)) => Some((state, now.saturating_duration_since(at))),
        _ => None,
    };
    CaptureRequest {
        tab_at: stat_tracker::report::blank::CaptureTime {
            wall: Utc::now(),
            mono: now,
        },
        session_id: g.session_id.clone(),
        create_session: !g.session_created,
        game_outcome: g.outcome,
        session_map: g.map.clone(),
        session_map_source: g.map_source,
        map_candidates: g.map_candidates.clone(),
        allow_banner_recovery: banner_ok,
        prev_gate,
        hero_auth: g.hero_auth.clone(),
        after_end_screen: g.has_post_result(),
        baseline_age: g.baseline_at.map(|at| now.saturating_duration_since(at)),
        progressed_boards: g.progressed_boards,
        deferred_imported: g.deferred_imported,
        reset_streak: g.reset_streak,
        reset_baseline: g.reset_baseline,
        baseline_row: g.baseline_row,
        hint: g
            .result_mark
            .filter(|mark| !mark.confirmed && mark.outcome.is_decided())
            .map(|mark| mark.outcome),
        pending_boundary: g.pending_boundary,
        awaiting_first_board: g.awaiting_first_board,
        text_fallback_locked: g.text_fallback_locked,
        member_map_edit: false,
        carried_deferred: g.deferred,
        carried_deferred_hero: g.deferred_hero.clone(),
        carried_deferred_at: g.deferred_at,
        result_evidence: g.result_evidence.clone(),
        rejoin_window_open: g.rejoin_carry.is_none()
            && !g.finished()
            && g.result_mark.is_none()
            && g.restart_recovered_at
                .is_some_and(|at| now.saturating_duration_since(at) <= REJOIN_CONTINUE_WINDOW),
        rejoin_carry: g.rejoin_carry,
        finished_game_probe: None,
        last_board_after_result: g
            .last_board_pressed_at
            .zip(g.outcome_recorded_at)
            .is_some_and(|(pressed, result)| pressed > result),
    }
}

/// Whether this Tab closes a game that has a result and stored rows. That
/// game is then uploaded right away ([`open_fresh_for_tab_and_sync`]).
fn tab_closes_a_recorded_game(game: Option<&ActiveGame>, now: Instant) -> bool {
    game.is_some_and(|g| g.finished() && g.session_created) && should_start_fresh_session(game, now)
}

/// [`open_fresh_if_tab_starts_one`], then `sync_now` when the game this Tab
/// closed had a result and stored rows. Without this the closed game waited
/// for the next game's periodic sync, and the quiet-close timer only looks
/// at the open game, so a late Tab off the scoreboard left the last game of
/// the night unsent for hours (2026-10-17, Oasis). `sync_now` schedules
/// [`schedule_store_sync`]; it does not block the loop.
async fn open_fresh_for_tab_and_sync<F, Fut>(
    st: &mut SessionState,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    now: Instant,
    sync_now: F,
) -> bool
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let upload_closed = tab_closes_a_recorded_game(st.active_game.as_ref(), now);
    let opened = open_fresh_if_tab_starts_one(st, store, data_dir, now).await;
    if opened && upload_closed {
        sync_now().await;
    }
    opened
}

/// A Tab past the grace window of a finished game whose board decides
/// whether it is that game ([`finished_game_tab_probe`]). Memory only, and
/// only while that Tab's capture runs: [`settle_tab_probe`] takes it when
/// the report lands.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TabProbe {
    /// The finished game the board may continue.
    old_session: String,
    /// The session the Tab opens when it does not (the request's own).
    fresh_session: String,
    /// The Tab press: that session opens as of then, as it did before rc.9.
    press: Instant,
}

/// The capture request for the Tab pressed at `press`: the session choice of
/// [`open_fresh_for_tab_and_sync`], or past the grace window of a finished
/// game that its board may continue ([`finished_game_tab_probe`]), the fresh
/// session's request with the finished game's request inside. Nothing is
/// closed or opened then; [`settle_tab_probe`] does that when the report
/// lands.
async fn tab_capture_request<F, Fut>(
    st: &mut SessionState,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    press: Instant,
    sync_now: F,
) -> CaptureRequest
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    st.tab_press = Some(press);
    if finished_game_tab_probe(st.active_game.as_ref(), press) {
        let old = st.active_game.as_ref().expect("checked by the probe");
        let fresh_session = format!("{:016x}", rand_id());
        // Peeked, not taken: the pending outcome is taken when the fresh
        // session opens.
        let mut pending = st.pending_outcome;
        let fresh = tab_opened_game(
            fresh_session.clone(),
            take_fresh_pending(&mut pending, press),
            carried_deferred(Some(old)),
            press,
        );
        let mut req = build_capture_request(&fresh, true, press);
        req.finished_game_probe = Some(Box::new(build_capture_request(old, false, press)));
        tracing::info!(
            session_id = %old.session_id,
            outcome = %old.outcome,
            "Tab past the post-match grace window: the board decides whether this is the same game"
        );
        st.tab_probe = Some(TabProbe {
            old_session: old.session_id.clone(),
            fresh_session,
            press,
        });
        return req;
    }
    let opened_by_this_tab =
        open_fresh_for_tab_and_sync(st, store, data_dir, press, sync_now).await;
    build_capture_request(
        st.active_game.as_ref().expect("active_game set above"),
        opened_by_this_tab,
        press,
    )
}

/// The capture of a [`TabProbe`] Tab reported (`sid` is the request's
/// session, the fresh one). Returns the session the report is for.
///
/// * The board was the finished game's: that game stays open. The same
///   board ([`boundary::FinishedGameTab::SameBoard`]) is not stored and the
///   result stays. Still being played
///   ([`boundary::FinishedGameTab::StillPlaying`]), the capture stored the
///   board there, and the recorded result was false and is cleared
///   ([`unlatch_false_outcome`]).
/// * Anything else (another game's board, a board that cannot be compared,
///   a rejected or missed frame, a failed capture): the fresh session opens
///   as of the press and the finished game closes, as a Tab past the grace
///   window did before rc.9, with the upload of a closed game that had rows
///   ([`open_fresh_for_tab_and_sync`]). Not when another game took over
///   while the capture ran.
async fn settle_tab_probe<F, Fut>(
    st: &mut SessionState,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    sid: String,
    result: &Result<CaptureReport, String>,
    now: Instant,
    sync_now: F,
) -> String
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let Some(probe) = st.tab_probe.take_if(|probe| probe.fresh_session == sid) else {
        return sid;
    };
    let still_open = st
        .active_game
        .as_ref()
        .is_some_and(|g| g.session_id == probe.old_session);
    match result
        .as_ref()
        .ok()
        .and_then(|report| report.finished_game_tab)
    {
        Some(boundary::FinishedGameTab::SameBoard) => {
            tracing::info!(
                session_id = %probe.old_session,
                "Tab past the grace window shows the same board: same game, result kept"
            );
            probe.old_session
        }
        Some(boundary::FinishedGameTab::StillPlaying) => {
            // A result the member set while this capture ran stays (the
            // report re-seals it on the row).
            let unlocked = st.active_game.as_ref().is_some_and(|g| !g.outcome_locked);
            if still_open && unlocked {
                unlatch_false_outcome(st, store, data_dir, now).await;
            }
            probe.old_session
        }
        Some(boundary::FinishedGameTab::NewGame) | None => {
            if still_open || st.active_game.is_none() {
                let upload_closed = st
                    .active_game
                    .as_ref()
                    .is_some_and(|g| g.finished() && g.session_created);
                open_tab_game(st, store, data_dir, probe.fresh_session, probe.press).await;
                if upload_closed {
                    sync_now().await;
                }
            } else {
                tracing::warn!(
                    session_id = %sid,
                    "another game opened while this Tab was read: its report is not applied"
                );
            }
            sid
        }
    }
}

/// R9-2: the open game's board moved on past the grace window
/// ([`boundary::FinishedGameTab::StillPlaying`]), so its recorded result was
/// false (0.5.0-rc.8 report of 2026-10-22: DEFEAT taken off a red mid-match
/// frame). The result is cleared on the game and in the store, with the
/// result words read for this game (`reader = "new"` stamps them on every
/// later row otherwise), and the next confirmed result word sets it.
///
/// Result words read while the false result was held are not replayed:
/// those ticks were ignored (the game was past its end), and their reads
/// go with the cleared evidence. A real end screen already past is then not
/// read again, and the game ends without a result unless the member sets
/// one. A word streak from at or before the cleared result is dropped, so
/// a repeating misread needs two new reads again; a newer read (an end
/// screen seen while the false result was held) still pairs with the next
/// one inside the 60 s window.
///
/// Rows already uploaded with the false result keep it on the site:
/// `unknown` rows are not uploaded (`storage::LocalStore::get_unsynced`
/// selects `outcome != 'unknown'`, and the site skips them), so the site
/// is corrected only when a result is set on this game again.
async fn unlatch_false_outcome(
    st: &mut SessionState,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    now: Instant,
) {
    let Some(g) = st.active_game.as_mut() else {
        return;
    };
    if let Some(recorded) = g.outcome_recorded_at
        && st
            .word_outcome_streak
            .as_ref()
            .is_some_and(|streak| streak.seen_at <= recorded)
    {
        st.word_outcome_streak = None;
    }
    let cleared = g.outcome;
    g.outcome = detect::MatchOutcome::Unknown;
    g.outcome_recorded_at = None;
    g.result_mark = None;
    // Cleared with the hint ([`ActiveGame::text_fallback_locked`]).
    g.text_fallback_locked = false;
    g.result_evidence = Default::default();
    g.touch_at(now);
    tracing::warn!(
        session_id = %g.session_id,
        cleared = %cleared,
        "the board moved on after the result was recorded: the game is still being played, result cleared \
         (if its rows were already uploaded, the site keeps the old result until one is set again)"
    );
    if g.session_created
        && let Err(e) = store
            .set_session_outcome(&g.session_id, &detect::MatchOutcome::Unknown.to_string())
            .await
    {
        tracing::warn!(error = %e, "failed to clear the session outcome");
    }
    persist_active_game(data_dir, Some(g));
    refresh_snapshot(store, data_dir).await;
}

/// Open a session when this Tab is past grace or the unfinished idle bound.
/// Returns whether this Tab opened it. The deferred board of the session
/// being closed is carried onto the new one.
async fn open_fresh_if_tab_starts_one(
    st: &mut SessionState,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    now: Instant,
) -> bool {
    if !should_start_fresh_session(st.active_game.as_ref(), now) {
        return false;
    }
    open_tab_game(st, store, data_dir, format!("{:016x}", rand_id()), now).await;
    true
}

/// The deferred board a Tab-opened game carries from `previous`.
type CarriedDeferred = (Counters, Option<String>, Option<chrono::DateTime<Utc>>);

fn carried_deferred(previous: Option<&ActiveGame>) -> Option<CarriedDeferred> {
    previous.and_then(|g| {
        g.deferred
            .map(|counters| (counters, g.deferred_hero.clone(), g.deferred_at))
    })
}

/// The game a Tab opens as `session_id` at `now`: the pending outcome
/// `inherited`, and the deferred board of the game it replaces.
fn tab_opened_game(
    session_id: String,
    inherited: Option<detect::MatchOutcome>,
    carried: Option<CarriedDeferred>,
    now: Instant,
) -> ActiveGame {
    let mut opened = ActiveGame::open_at(
        session_id,
        inherited.unwrap_or(detect::MatchOutcome::Unknown),
        Vec::new(),
        now,
    );
    if let Some((counters, hero, at)) = carried {
        opened.deferred = Some(counters);
        opened.deferred_hero = hero;
        opened.deferred_at = at;
        opened.deferred_imported = true;
    }
    opened
}

/// Close the open game and open the Tab's game `session_id` at `now`.
async fn open_tab_game(
    st: &mut SessionState,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    session_id: String,
    now: Instant,
) {
    let carried = carried_deferred(st.active_game.as_ref());
    retire_active_game(st, store, data_dir, None, "superseded by Tab-opened game").await;
    let inherited = take_fresh_pending(&mut st.pending_outcome, now);
    st.active_game = Some(tab_opened_game(session_id, inherited, carried, now));
    st.last_game_open = Some(now);
    clear_cadence_wakes(st);
    persist_active_game(data_dir, st.active_game.as_ref());
}

/// The previous game is not a latch for a session that has no board yet.
///
/// Guard. [`build_capture_request`] already omits a gate this session does
/// not have, so production passes `None` while awaiting the first board.
/// The filter stays so a caller that still holds the previous game's gate
/// cannot latch this row to it. [`gate_prev_for_store`] is what both the
/// daemon and the night harness apply.
fn gate_prev_for_store(
    prev: Option<(GateState, std::time::Duration)>,
    awaiting_first_board: bool,
) -> Option<(GateState, std::time::Duration)> {
    prev.filter(|_| !awaiting_first_board)
}

/// A held board is written when this capture splits, or when it was carried
/// in from the session that just closed.
fn carried_counters_to_write(
    carried: Option<Counters>,
    split: bool,
    deferred_imported: bool,
) -> Option<Counters> {
    carried.filter(|_| split || deferred_imported)
}

/// What the vision pass (or a test stub) read off one Tab.
struct BoardFacts<'a> {
    counters: Counters,
    suspect: [bool; capture_gate::GATE_COLS],
    /// Per-cell parse succeeded. The capture gate treats false as low-trust.
    /// The boundary plan's `row_counts` is this bit: a raw-text fallback
    /// does not count as a stat-reset row. A row with held cells does
    /// (`boundary::plan_capture_held` leaves those columns out).
    trusted_cells: bool,
    /// `reader = "new"`: columns the digit reader did not vouch for. The
    /// capture has no number for them (`capture_gate::apply_gate_held`).
    /// All false under ocr-v1.
    held: [bool; capture_gate::GATE_COLS],
    /// `reader = "new"`: the own row is suspect (alpha.7 O9), so it may be
    /// another player's. The capture gate takes the read as untrusted, and
    /// the boundary plan lets it anchor the baseline but not split alone
    /// (`boundary::plan_capture_held`). False under ocr-v1.
    suspect_row: bool,
    row_id: Option<u32>,
    hero: &'a str,
    map_from_panel: Option<&'a str>,
    /// `map_from_panel` is a sure banner read, not the top-bar OCR.
    panel_from_banner: bool,
    parsed_map: &'a str,
    frame_outcome: detect::MatchOutcome,
}

/// The capture plan for one analyzed board. Production and the night harness
/// both call this, so the gap, the same-map guard, and the hinted-map split
/// cannot drift.
fn plan_from_board(req: &CaptureRequest, facts: &BoardFacts<'_>) -> boundary::CapturePlan {
    let incoming_map = facts
        .map_from_panel
        .filter(|s| !s.is_empty())
        .or(req.session_map.as_deref())
        .or(Some(facts.parsed_map).filter(|s| !s.is_empty()));
    let suppress_split = same_unfinished_match(
        matches!(req.game_outcome, detect::MatchOutcome::Unknown),
        std::time::Duration::ZERO,
        req.session_map.as_deref(),
        req.hero_auth.accepted_hero.as_deref(),
        IncomingIdentity {
            map: incoming_map,
            hero: Some(facts.hero),
            vote_candidates: &req.map_candidates,
        },
    );
    let (gate_age, classic_regressed) = match req.prev_gate {
        Some((state, age)) => {
            let classic = stats_regressed(
                state.accepted.edd(),
                state.last_raw.edd(),
                (
                    facts.counters.elims,
                    facts.counters.deaths,
                    facts.counters.damage,
                ),
                (facts.suspect[0], facts.suspect[2], facts.suspect[3]),
                (
                    state.last_raw_suspect[0],
                    state.last_raw_suspect[2],
                    state.last_raw_suspect[3],
                ),
            );
            (Some(age), classic)
        }
        None => (None, false),
    };
    let (prev_gate, baseline) = match req.prev_gate {
        Some((state, _)) => (Some(state), req.reset_baseline),
        None => (None, req.reset_baseline),
    };
    let input = boundary::CapturePlanInput {
        prev_gate: prev_gate.as_ref(),
        baseline: baseline.as_ref(),
        streak: req.reset_streak,
        cur: facts.counters,
        suspect: facts.suspect,
        // A game with boards read but no row stored yet (`reader = "new"`,
        // a column still unread) has a gate of its own to regress from. Its
        // session row does not exist, and that alone is not "no board yet".
        create_session: req.create_session && req.prev_gate.is_none(),
        suppress_same_unfinished: suppress_split,
        age: gate_age,
        min_gap: STAT_SPLIT_MIN_GAP,
        classic_regressed,
        row_counts: facts.trusted_cells,
        row_id: facts.row_id,
        baseline_row: req.baseline_row,
        confirmed_end: req.game_outcome.is_decided(),
        inherited_outcome: req.game_outcome,
        frame_outcome: facts.frame_outcome,
        session_hero: req.hero_auth.accepted_hero.as_deref(),
        hint: req.hint,
        pending_boundary: req.pending_boundary,
        awaiting_first_board: req.awaiting_first_board,
        baseline_age: req.baseline_age,
        progressed_boards: req.progressed_boards,
        session_map: req.session_map.as_deref(),
        session_map_source: req.session_map_source,
        incoming_map,
    };
    boundary::plan_capture_held(
        &input,
        boundary::BoardDoubt {
            held: facts.held,
            suspect_row: facts.suspect_row,
            // A board deferred on this session. One carried in from the
            // session that closed is this game's own row, not a signal.
            board_deferred: req.carried_deferred.is_some() && !req.deferred_imported,
        },
    )
}

/// Fold one capture report into the open session. Store writes already
/// happened. Returns whether this report counts toward the periodic sync.
async fn apply_capture_report(
    st: &mut SessionState,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    sid: &str,
    result: Result<CaptureReport, String>,
    now: Instant,
) -> bool {
    // The press of this report's Tab (one capture runs at a time). A caller
    // that made no request through `tab_capture_request` has none: then the
    // report time stands in.
    let pressed = st.tab_press.take().unwrap_or(now);
    match result {
        Err(e) => {
            tracing::error!(error = %e, "capture cycle failed");
            false
        }
        Ok(report) if report.armed_reset => {
            if let Some(g) = st.active_game.as_mut().filter(|g| g.session_id == sid) {
                let mut state = g.boundary_state();
                boundary::note_accepted_capture(
                    &mut state,
                    &boundary::CapturePlan {
                        split: false,
                        defer: true,
                        ignore_row: false,
                        skip_store: true,
                        clear_hint: false,
                        reset_streak: report.reset_streak,
                        reset_baseline: report.reset_baseline,
                        baseline_row: report.baseline_row,
                        refresh_baseline: false,
                        count_progress: false,
                        stored_outcome: g.outcome,
                        deferred_counters: report.held_counters,
                        seal: None,
                        close_reason: None,
                    },
                    GateState::default(),
                    now,
                );
                g.apply_boundary_state(&state);
                g.deferred_imported = false;
                g.deferred_hero = report.held_hero.clone();
                if g.deferred.is_some() {
                    g.deferred_at = report.held_at.or(Some(Utc::now()));
                }
                persist_active_game(data_dir, Some(g));
            }
            false
        }
        Ok(report) if !report.recorded && !report.pending() => {
            // A fuzzy Guillard or Practice Range read must not rename an open
            // Busan session. Only a trusted map (top bar or accolade) may
            // adopt an untracked map, and only onto an empty session or one
            // that is already untracked.
            if let Some(map) = report.map.clone()
                && parse::map_is_untracked(&map)
                && report
                    .map_source
                    .is_some_and(|source| source.trusted_for_board_split())
                && let Some(g) = st.active_game.as_mut().filter(|g| g.session_id == sid)
            {
                let current = g.map.as_deref().unwrap_or("");
                if current.is_empty() || parse::map_is_untracked(current) {
                    let reason = parse::untracked_close_reason(&map);
                    g.map = Some(map);
                    g.map_source = report.map_source;
                    note_untracked_once(data_dir, g, reason);
                    persist_active_game(data_dir, Some(g));
                }
                return false;
            }
            if !matches!(report.outcome, detect::MatchOutcome::Unknown)
                && let Some(g) = st.active_game.as_mut().filter(|g| g.session_id == sid)
                && !g.finished()
            {
                g.record_outcome_at(report.outcome, now);
                g.note_result(report.outcome, true);
                // Adopting the frame's outcome closes the armed boundary.
                // The hint is now the recorded result.
                g.pending_boundary = false;
                tracing::info!(
                    outcome = ?report.outcome,
                    session_id = %g.session_id,
                    "outcome recovered from rejected capture frame: adopting"
                );
                if g.session_created {
                    if let Err(e) = store
                        .set_session_outcome(&g.session_id, &g.outcome.to_string())
                        .await
                    {
                        tracing::warn!(error = %e, "failed to back-fill session outcome");
                    }
                    refresh_snapshot(store, data_dir).await;
                }
                persist_active_game(data_dir, Some(g));
            }
            false
        }
        Ok(report) if report.split => {
            if st.active_game.as_ref().is_some_and(|g| g.session_id == sid) {
                let reason = report
                    .close_reason
                    .map(|reason| reason.log())
                    .unwrap_or("superseded by stat reset");
                retire_active_game(st, store, data_dir, report.seal, reason).await;
                let mut g = session_opened_by_split(
                    report.session_id.clone(),
                    report.outcome,
                    report.hero_auth.clone(),
                    report.career_panel,
                    report.gate_state,
                    report.map.clone(),
                    now,
                );
                g.map_source = report.map_source;
                g.baseline_row = report.baseline_row;
                // A pending board (`reader = "new"`, a column not read yet)
                // opened the game without a row: the session row is made by
                // the first capture that stores one.
                g.session_created = report.recorded;
                if !report.recorded {
                    g.best_guess = report.best_guess.clone();
                }
                tracing::info!(
                    old_session = %sid,
                    session_id = %g.session_id,
                    reason,
                    "active game replaced after stat-regression split"
                );
                st.active_game = Some(g);
                st.last_game_open = Some(now);
                clear_cadence_wakes(st);
                persist_active_game(data_dir, st.active_game.as_ref());
            }
            true
        }
        Ok(report) => {
            // A stored row, or a pending board (`CaptureReport::pending`):
            // the gate, the boundary plan, the hero authority and the map
            // move on either way. Only a stored row creates the session row
            // and counts toward the sync cadence.
            let recorded = report.recorded;
            if let Some(g) = st.active_game.as_mut().filter(|g| g.session_id == sid) {
                if recorded {
                    g.session_created = true;
                    g.best_guess = None;
                } else if report.best_guess.is_some()
                    && report
                        .gate_state
                        .is_none_or(|state| state.refused_cols().is_empty())
                {
                    // A Tab whose first read of a column was not taken
                    // (other columns dropped: likely the next game's board)
                    // does not replace the guess.
                    g.best_guess = report.best_guess.clone();
                }
                g.touch_at(now);
                if report.rejoin_carry.is_some() {
                    g.rejoin_carry = report.rejoin_carry;
                }
                if let Some(accepted) = report.gate_state {
                    let mut state = g.boundary_state();
                    boundary::note_accepted_capture(
                        &mut state,
                        &boundary::CapturePlan {
                            split: false,
                            defer: false,
                            ignore_row: report.ignore_row,
                            skip_store: false,
                            clear_hint: report.clear_hint,
                            reset_streak: report.reset_streak,
                            reset_baseline: report.reset_baseline,
                            baseline_row: report.baseline_row,
                            refresh_baseline: report.refresh_baseline,
                            count_progress: report.count_progress,
                            stored_outcome: report.outcome,
                            deferred_counters: None,
                            seal: None,
                            close_reason: None,
                        },
                        accepted,
                        now,
                    );
                    g.apply_boundary_state(&state);
                    g.last_board_pressed_at = Some(pressed);
                }
                // The imported board was written with this stored capture.
                // Drop it so a later Tab that does not refresh the baseline
                // does not write it again, and so it no longer blocks this
                // session's own accolade. A deferral taken on this session
                // stays: `deferred_imported` is false there.
                if recorded && g.deferred_imported {
                    g.deferred = None;
                    g.deferred_hero = None;
                    g.deferred_at = None;
                    g.deferred_imported = false;
                }
                g.hero_auth = report.hero_auth.clone();
                if let Some(map) = report.map.clone() {
                    let agrees = g
                        .map
                        .as_ref()
                        .is_some_and(|stored| stored.eq_ignore_ascii_case(&map));
                    let untrusted = !g.map_is_trusted();
                    let incoming_trusted = report
                        .map_source
                        .is_some_and(boundary::MapSource::trusted_for_board_split);
                    if g.map.is_none() {
                        g.map = Some(map.clone());
                        g.map_source = report.map_source;
                        // No session row yet for a pending game: its first
                        // stored capture creates the row with this map.
                        if g.session_created
                            && let Err(e) = store.set_session_map(&g.session_id, &map).await
                        {
                            tracing::warn!(error = %e, "failed to set session map");
                        }
                    } else if agrees && untrusted && incoming_trusted {
                        // A later top bar that names the map already stored
                        // upgrades a text fallback. The name stays.
                        g.map_source = report.map_source;
                    } else if report.map_replaces_text_fallback && untrusted && incoming_trusted {
                        // A sure banner read inside the text-fallback window
                        // renames the session (banner_replaces_text_fallback).
                        g.replaced_text_fallback_map = g.map.replace(map.clone());
                        g.map_source = report.map_source;
                        if g.session_created
                            && let Err(e) = store.set_session_map(&g.session_id, &map).await
                        {
                            tracing::warn!(error = %e, "failed to replace session map");
                        }
                    }
                }
                if !g.finished() && !matches!(report.outcome, detect::MatchOutcome::Unknown) {
                    g.record_outcome_at(report.outcome, now);
                    g.note_result(report.outcome, true);
                    g.pending_boundary = false;
                    tracing::info!(
                        outcome = ?report.outcome,
                        session_id = %g.session_id,
                        "outcome recovered from captured frame: back-filling session"
                    );
                    if g.session_created
                        && let Err(e) = store
                            .set_session_outcome(&g.session_id, &g.outcome.to_string())
                            .await
                    {
                        tracing::warn!(error = %e, "failed to back-fill session outcome");
                    }
                } else if recorded
                    && g.finished()
                    && g.session_created
                    && report.outcome != g.outcome
                {
                    // MAIN-1: this capture took its outcome when Tab was
                    // pressed. The game was decided while it ran (an end-reel
                    // poll, a GUI edit), so its row and session write carry
                    // the old one. Seal the decided result over them again.
                    tracing::info!(
                        stored = %report.outcome,
                        outcome = %g.outcome,
                        session_id = %g.session_id,
                        "capture finished after the result was decided: re-sealing the session"
                    );
                    if let Err(e) = store
                        .set_session_outcome(&g.session_id, &g.outcome.to_string())
                        .await
                    {
                        tracing::warn!(error = %e, "failed to re-seal session outcome");
                    }
                }
                persist_active_game(data_dir, Some(g));
            }
            recorded
        }
    }
}

/// Upper bounds for [`tests::capture_futures_stay_small`] (debug build,
/// rc.7: 15,168 and 1,640 bytes). Unboxing the KDE direct or portal grab
/// puts `capture_screen_output_for` at 4.6 KB or 28 KB.
#[cfg(all(test, debug_assertions))]
const HANDLE_CAPTURE_FUTURE_MAX: usize = 20 * 1024;
#[cfg(all(test, debug_assertions))]
const CAPTURE_FUTURE_MAX: usize = 3 * 1024;

/// The most the shutdown waits for queued perf.log lines: 500 ms, or less
/// when less of the shutdown deadline is left.
fn shutdown_flush_wait(left: std::time::Duration) -> std::time::Duration {
    left.min(std::time::Duration::from_millis(500))
}

/// One result-word sighting. `map` survives onto the confirming tick when
/// that tick's screen has no map of its own.
struct WordStreak {
    outcome: detect::MatchOutcome,
    seen_at: Instant,
    map: Option<String>,
}

/// Record this result-word tick and return the map the decision should see.
///
/// A tick with no map keeps the map from the previous read of the same
/// outcome, and only while that read is still inside
/// [`OUTCOME_CONFIRM_WINDOW`]. A different outcome, or an older read, drops
/// the carried map so it cannot relabel the session.
fn note_word_streak(
    streak: &mut Option<WordStreak>,
    outcome: detect::MatchOutcome,
    seen_at: Instant,
    this_map: Option<String>,
) -> Option<String> {
    let carried = streak.as_ref().and_then(|prior| {
        let age = seen_at.saturating_duration_since(prior.seen_at);
        (prior.outcome == outcome && age <= OUTCOME_CONFIRM_WINDOW)
            .then(|| prior.map.clone())
            .flatten()
    });
    let map = this_map.or(carried);
    *streak = Some(WordStreak {
        outcome,
        seen_at,
        map: map.clone(),
    });
    map
}

/// What one poll tick decided about the result word, before the session
/// machine runs. Production and the night harness both call this, so the
/// carried map and the confirm rule cannot drift.
struct ResolvedWordTick {
    signal: Option<detect::MatchOutcome>,
    signal_confirmed: bool,
    accolade_map: Option<String>,
}

fn resolve_word_tick(
    streak: &mut Option<WordStreak>,
    signal: Option<(detect::MatchOutcome, detect::match_end::OutcomeSource)>,
    seen_at: Instant,
    this_map: Option<String>,
) -> ResolvedWordTick {
    let prior = streak.as_ref().map(|prior| {
        (
            prior.outcome,
            seen_at.saturating_duration_since(prior.seen_at),
        )
    });
    let hit = poll_debug_hit(signal, prior, OUTCOME_CONFIRM_WINDOW);
    match signal {
        Some((outcome, source)) => {
            let agreed = matches!(hit, Some((PollDebugHit::Confirm, _)));
            let accolade_map = note_word_streak(streak, outcome, seen_at, this_map);
            if !agreed {
                tracing::debug!(
                    ?outcome,
                    ?source,
                    "result word read: awaiting agreeing read"
                );
            }
            ResolvedWordTick {
                signal: Some(outcome),
                signal_confirmed: agreed,
                accolade_map,
            }
        }
        None => ResolvedWordTick {
            signal: None,
            signal_confirmed: false,
            accolade_map: this_map,
        },
    }
}

#[cfg(test)]
#[test]
fn reused_frame_is_not_a_confirm_and_a_new_pair_is() {
    let victory = Some((
        detect::MatchOutcome::Victory,
        detect::match_end::OutcomeSource::ResultWord,
    ));
    let seen = Some(capture::SeenStream { id: 1, seq: 4 });
    let held_at = std::time::Instant::now();
    let mut streak = Some(WordStreak {
        outcome: detect::MatchOutcome::Victory,
        seen_at: held_at,
        map: None,
    });
    // The poll loop continues before `resolve_word_tick` when this is
    // true. Applying the word moves `seen_at`, so an inverted predicate
    // fails here.
    let skips = capture::reused_frame_skips_the_read(false, Some(4), Some(1), seen);
    assert!(skips);
    if !skips {
        let _ = resolve_word_tick(&mut streak, victory, std::time::Instant::now(), None);
    }
    assert_eq!(
        streak.as_ref().map(|prior| prior.seen_at),
        Some(held_at),
        "a reused frame must not move the streak"
    );
    let mut if_applied = Some(WordStreak {
        outcome: detect::MatchOutcome::Victory,
        seen_at: held_at,
        map: None,
    });
    let skips_fresh = capture::reused_frame_skips_the_read(
        true,
        Some(5),
        Some(1),
        Some(capture::SeenStream { id: 1, seq: 4 }),
    );
    assert!(!skips_fresh);
    if !skips_fresh {
        let applied = resolve_word_tick(&mut if_applied, victory, std::time::Instant::now(), None);
        assert!(applied.signal.is_some());
    }
    assert_ne!(
        if_applied.as_ref().map(|prior| prior.seen_at),
        Some(held_at),
        "a new frame must move the streak"
    );
    // Bounded by name: `analyze_frame`, which used to end the slice, is in
    // src/daemon/tab/vision.rs since 0.6 S3.
    let run_loop = crate::main_tests::fn_source("run_loop");
    let call = run_loop
        .find("reused_frame_skips_the_read(")
        .expect("the poll must call the skip helper");
    let after_call = &run_loop[call..];
    let kept = after_call
        .find("seen_stream = Some")
        .expect("a kept frame is recorded after the skip");
    let window = &after_call[..kept];
    assert!(
        window.contains("continue"),
        "deleting the continue after a reused frame must fail: {window}"
    );
    assert!(
        run_loop.contains("_ = stat_tracker::replace::shutdown_notified() =>"),
        "the daemon select must wake on shutdown_notified"
    );

    assert!(capture::poll_uses_frame(
        true,
        Some(5),
        Some(1),
        Some(capture::SeenStream { id: 1, seq: 4 })
    ));
    assert!(!capture::poll_uses_frame(
        true,
        Some(5),
        Some(1),
        Some(capture::SeenStream { id: 1, seq: 5 })
    ));
    assert!(capture::poll_uses_frame(
        true,
        Some(1),
        Some(2),
        Some(capture::SeenStream { id: 1, seq: 40 })
    ));
    let mut streak = None;
    let now = std::time::Instant::now();
    let first = resolve_word_tick(
        &mut streak,
        Some((
            detect::MatchOutcome::Victory,
            detect::match_end::OutcomeSource::ResultWord,
        )),
        now,
        None,
    );
    assert!(!first.signal_confirmed);
    let second = resolve_word_tick(
        &mut streak,
        Some((
            detect::MatchOutcome::Victory,
            detect::match_end::OutcomeSource::ResultWord,
        )),
        now,
        None,
    );
    assert!(second.signal_confirmed);
    assert_eq!(second.signal, Some(detect::MatchOutcome::Victory));
}

/// The words-pack result read one poll tick adds to the open game's result
/// evidence (`reader = "new"`, #253). Only a tick with an outcome signal
/// adds one, as before.
///
/// `taken` is the read the tick's signal detection already made of this
/// frame's result words (the accolade or rank screen, alpha.7 O11). It is
/// `shadow::read_result`'s read of the same frame, so it is the evidence
/// and the frame is not read again. Without one (an end title, or a word
/// OCR read because the pack lacks that screen) `read`
/// reads the frame here.
fn tick_result_read(
    use_new_reader: bool,
    has_signal: bool,
    taken: Option<shadow::result::ResultRead>,
    read: impl FnOnce() -> shadow::result::ResultRead,
) -> Option<shadow::result::ResultRead> {
    if !(use_new_reader && has_signal) {
        return None;
    }
    Some(taken.unwrap_or_else(read))
}

/// The [`boundary::PollInput`] for one open session. `map_trusted` is the
/// stored source, so a text fallback stays absent on the poll path.
fn poll_input_from_game<'a>(
    game: &'a ActiveGame,
    signal: Option<detect::MatchOutcome>,
    signal_confirmed: bool,
    accolade_map: Option<&'a str>,
    start_screen: Option<boundary::StartScreen>,
    block_map_vote: bool,
    now: Instant,
) -> boundary::PollInput<'a> {
    boundary::PollInput {
        outcome: game.outcome,
        result: game.result_mark,
        pending_boundary: game.pending_boundary,
        awaiting_first_board: game.awaiting_first_board,
        has_board: game.gate.is_some(),
        reset_streak: game.reset_streak,
        map: game.map.as_deref(),
        map_trusted: game.map_is_trusted(),
        hero: game.hero_auth.accepted_hero.as_deref(),
        signal,
        signal_confirmed,
        accolade_map: accolade_map.map(|read| {
            // The replaced text-fallback name is this session's own map
            // (F252-1).
            match (
                game.replaced_text_fallback_map.as_deref(),
                game.map.as_deref(),
            ) {
                (Some(old), Some(map)) if read.trim().eq_ignore_ascii_case(old.trim()) => map,
                _ => read,
            }
        }),
        start_screen,
        block_map_vote,
        deferred: game.deferred.is_some(),
        text_fallback_locked: game.text_fallback_locked,
        now,
    }
}

/// One poll tick after the word is resolved. [`run_loop`] and the night
/// harness both call this, so the post-tick glue cannot drift: the
/// start-screen block, [`poll_input_from_game`] (including the carried
/// accolade map), [`apply_poll_decision`], a confirmed word with no game
/// open, and [`open_detected_game`].
#[allow(clippy::too_many_arguments)]
async fn apply_poll_tick(
    st: &mut SessionState,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    tick: &ResolvedWordTick,
    start_screen: Option<boundary::StartScreen>,
    now: Instant,
    debounce: std::time::Duration,
    use_new_reader: bool,
) {
    let block_map_vote = start_screen.as_ref().is_some_and(|screen| {
        start_screen_blocked(
            st.active_game.as_ref(),
            st.last_game_open,
            now,
            debounce,
            screen,
        )
    });
    let decision = st.active_game.as_ref().map(|g| {
        boundary::decide_poll(&poll_input_from_game(
            g,
            tick.signal,
            tick.signal_confirmed,
            tick.accolade_map.as_deref(),
            start_screen.clone(),
            block_map_vote,
            now,
        ))
    });
    let opened = matches!(decision, Some(boundary::PollDecision::Open(_)));
    if let Some(decision) = decision {
        apply_poll_decision(st, store, data_dir, decision, now, use_new_reader).await;
    } else if tick.signal_confirmed
        && let Some(outcome) = tick.signal
    {
        // No game open yet, applies to the next session if one opens
        // within the TTL.
        st.pending_outcome = Some((outcome, now));
    }
    // An open session is only closed by `decide_poll`. This arm starts a
    // session when nothing is active yet.
    if !opened
        && st.active_game.is_none()
        && let Some(screen) = &start_screen
    {
        open_detected_game(st, data_dir, screen, now, debounce);
    }
}

/// Session state at process start, and again after a suspend.
///
/// `opened_at` is the instant the session opened, which is also when
/// `last_game_open` was set. Restoring it keeps the debounce across a
/// restart so the first ban or select does not split a session that is
/// still inside it.
fn startup_session(data_dir: &std::path::Path) -> SessionState {
    let active_game = recover_active_game(data_dir);
    let last_game_open = active_game.as_ref().map(|g| g.opened_at);
    SessionState {
        capture_count: 0,
        last_game_open,
        last_tab_capture: None,
        active_game,
        pending_outcome: None,
        word_outcome_streak: None,
        suspend_probe: (Instant::now(), Utc::now()),
        ocr_stability: detect::stability::FrameStability::default(),
        poll_ticks_skipped: 0,
        end_reel_wake_until: None,
        settle_closed_unknown: None,
        tab_probe: None,
        tab_press: None,
    }
}

/// Resume after a suspend. Drops the volatile windows and re-admits the
/// active game through the same recovery startup uses, so `last_game_open`
/// is `opened_at` again and a ban inside the debounce does not split.
/// Take the session [`begin_daemon_session`] already restored. Do not read
/// the skeleton a second time.
fn adopt_startup_session(st: &mut SessionState, restored: SessionState) {
    st.pending_outcome = None;
    clear_cadence_wakes(st);
    st.last_tab_capture = None;
    st.last_game_open = restored.last_game_open;
    st.active_game = restored.active_game;
    st.settle_closed_unknown = restored.settle_closed_unknown;
}

#[cfg_attr(not(test), allow(dead_code))]
fn resume_after_suspend(st: &mut SessionState, data_dir: &std::path::Path) {
    st.pending_outcome = None;
    clear_cadence_wakes(st);
    st.last_tab_capture = None;
    let recovered = startup_session(data_dir);
    st.last_game_open = recovered.last_game_open;
    st.active_game = recovered.active_game;
    if st.active_game.is_none() {
        persist_active_game(data_dir, None);
    }
}

/// How long direct capture and the screenshot portal wait after the Tab
/// press for the game to draw the scoreboard. The screen stream waits for a
/// buffer instead.
const DIRECT_TAB_DRAW_WAIT: std::time::Duration = std::time::Duration::from_millis(400);

/// Start the Tab capture for `press` (the key-event instant): pick or open
/// the session, then spawn the single-flight capture task, which reports on
/// `capture_tx`. Returns the task and its shared flight state.
async fn start_tab_capture(
    st: &mut SessionState,
    ctx: &Arc<DaemonCtx>,
    sync_task: &mut Option<tokio::task::JoinHandle<()>>,
    sync_backoff: &Arc<std::sync::Mutex<sync::SyncBackoff>>,
    capture_tx: &tokio::sync::mpsc::UnboundedSender<(String, Result<CaptureReport, String>)>,
    press: Instant,
    stream_hold: capture::KeyHold,
) -> (
    tokio::task::JoinHandle<()>,
    Arc<stat_tracker::tab_retry::TabFlight>,
) {
    let store = &ctx.store;
    let data_dir: &std::path::Path = &ctx.data_dir;
    let sync_client = ctx.sync_client.as_ref();
    // Session choice: reuse the active game (mid-game, or post-match
    // scoreboard within the grace window), or open a fresh one, inheriting
    // a still-fresh outcome the poller saw before any game was open. Past
    // the grace window of a finished game, the board may decide (R9-2).
    let req = tab_capture_request(st, store, data_dir, press, || {
        schedule_store_sync(sync_task, sync_backoff, store, sync_client, data_dir, None);
        std::future::ready(())
    })
    .await;
    let on_stream = capture::stream_route_active();
    let sid = req.session_id.clone();
    let tx = capture_tx.clone();
    let ctx_task = Arc::clone(ctx);
    let flight = stat_tracker::tab_retry::TabFlight::new(press);
    let task = tokio::spawn(stat_tracker::tab_retry::with_flight(
        Arc::clone(&flight),
        capture::with_tab_press(press, async move {
            // Direct capture and the screenshot portal wait for the overlay
            // to finish drawing, counted from the key event. The screen
            // stream waits for a buffer instead.
            if !on_stream {
                tokio::time::sleep_until(tokio::time::Instant::from_std(
                    press + DIRECT_TAB_DRAW_WAIT,
                ))
                .await;
            }
            // Keeps a paused screen stream running from the key event until
            // the Tab retry sequence ends (`capture::end_tab_stream_hold`).
            let result = capture::with_tab_stream_hold(stream_hold, handle_capture(&ctx_task, req))
                .await
                // `{:#}` keeps anyhow's context chain in the string.
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send((sid, result));
        }),
    ));
    (task, flight)
}

#[allow(clippy::too_many_arguments)]
async fn run_loop(
    ctx: Arc<DaemonCtx>,
    early_sigterm: Option<tokio::signal::unix::Signal>,
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
    tracing::info!("signal handlers ready");
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
    let mut sync_task: Option<tokio::task::JoinHandle<()>> = None;
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
                    tracing::error!("{status}. Sync resumes when the URL or token changes.");
                }
            }
            None => {}
        }
    }

    // Startup drops a stale skeleton and schedules the upload on `sync_task`.
    // It does not await HTTP. `last_game_open` still comes from
    // `startup_session`, so the debounce survives the restart. SIGTERM waits
    // on `finish_sync_on_shutdown`.
    let mut st =
        begin_daemon_session(data_dir, &mut sync_task, &sync_backoff, store, sync_client).await;
    if let Some(g) = &st.active_game {
        tracing::info!(
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
                tracing::info!("shutting down");
                shutdown_from_signal(
                    &mut st,
                    &mut capture_task,
                    &mut capture_rx,
                    &mut sync_task,
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
                tracing::info!("SIGTERM received, shutting down");
                shutdown_from_signal(
                    &mut st,
                    &mut capture_task,
                    &mut capture_rx,
                    &mut sync_task,
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
                tracing::info!("shutting down");
                shutdown_from_signal(
                    &mut st,
                    &mut capture_task,
                    &mut capture_rx,
                    &mut sync_task,
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
                            tracing::debug!("Tab ignored: game process not running");
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
                                &mut sync_task,
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
                        tracing::error!(error = %e, "keyboard devices lost: attempting to reopen");
                        match detect::MultiKeyboardStream::open() {
                            Ok(new_kbd) => {
                                kbd = new_kbd;
                                tracing::info!("keyboard monitoring reopened");
                            }
                            Err(e2) => {
                                tracing::error!(error = %e2, "failed to reopen keyboard: exiting");
                                drain_capture(capture_task.take()).await;
                                let task = sync_task.take();
                                if let Some(client) = sync_client {
                                    drain_sync_then(task, || try_sync(store, client, data_dir, None)).await;
                                } else {
                                    drain_sync_then(task, || std::future::ready(())).await;
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
                    schedule_store_sync(&mut sync_task, &sync_backoff, store, sync_client, data_dir, None);
                    std::future::ready(())
                })
                .await;
                // The report handler is the same function the night harness
                // calls. Sync stays here so a slow upload cannot hide inside it.
                if apply_capture_report(&mut st, store, data_dir, &sid, result, Instant::now()).await {
                    st.capture_count += 1;
                    maybe_spawn_periodic_sync(
                        &mut sync_task,
                        &sync_backoff,
                        store,
                        sync_client,
                        data_dir,
                        st.capture_count,
                    );
                    refresh_snapshot(store, data_dir).await;
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
                        &mut sync_task,
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
                    tracing::debug!("poll tick skipped: Tab capture in flight");
                    continue;
                }
                let outcome_only = poll_outcome_only_while_tab_busy(tab_in_flight, &st, now);
                if outcome_only {
                    tracing::debug!(
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
                        tracing::trace!(divisor, "poll tick skipped: adaptive slow cadence");
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
                        tracing::debug!("poll dropped during its grab: a Tab press comes first");
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
                            &mut sync_task,
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
                                tracing::debug!("poll dropped during its read: a Tab press comes first");
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
                                    &mut sync_task,
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
                                &mut sync_task,
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
                            tracing::debug!(error = %e, "report kit: outcome crop not written");
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
                                &mut sync_task,
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
                                tracing::trace!(error = %e, "poll capture skipped");
                            }
                            PollErrorLog::Warn => {
                                tracing::warn!(
                                    error = %e,
                                    failures = poll_failures.count,
                                    "poll capture failed while the game runs; repeats are logged at most every 5 minutes"
                                );
                            }
                            PollErrorLog::Trace => {
                                tracing::trace!(error = %e, failures = poll_failures.count, "poll capture failed");
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
                                "tracker binary on disk was replaced; waiting after a failed re-exec"
                            );
                        } else {
                            shutdown_from_signal(
                                &mut st,
                                &mut capture_task,
                                &mut capture_rx,
                                &mut sync_task,
                                sync_client,
                                &sync_backoff,
                                store,
                                data_dir,
                            )
                            .await;
                            let marker =
                                stat_tracker::report::marker::release_run_marker(data_dir);
                            let released_pid =
                                stat_tracker::replace::release_own_pid_file(data_dir);
                            if let Err(err) =
                                stat_tracker::replace::restart_for_replaced_binary(&path)
                            {
                                // A failed re-exec keeps this process. The
                                // systemd path exits and does not return.
                                tracing::error!(
                                    error = %err,
                                    "could not re-exec the new tracker; keeping this process"
                                );
                                exec_failed_at = Some(std::time::Instant::now());
                                if released_pid {
                                    stat_tracker::replace::restore_own_pid_file(data_dir);
                                }
                                let _ = stat_tracker::report::marker::restore_run_marker(
                                    data_dir,
                                    marker.as_ref(),
                                );
                            } else {
                                return Ok(());
                            }
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
                            &mut sync_task,
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
                    &mut sync_task,
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
                        gap_secs = (wall - mono).as_secs(),
                        "suspend/clock-jump detected: resetting session windows"
                    );
                    // Schedule the upload, including a stale skeleton. Then
                    // `resume_after_suspend` re-admits through `startup_session`,
                    // so the debounce comes back from `opened_at`. HTTP does
                    // not run on this tick.
                    let restored = begin_daemon_session(
                        data_dir,
                        &mut sync_task,
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
                    &mut sync_task,
                    &sync_backoff,
                    sync_client,
                )
                .await;
                maybe_spawn_idle_sync(
                    &mut sync_task,
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

/// Session a stat split just opened. A decided `outcome` is the current
/// frame's own outcome (gap split) and stamps grace. Unknown is a reset split.
///
/// `career_ever_ok` is per game. It is set when this capture read the career
/// panel. A flag carried from the previous game does not. Portrait switch
/// state does not carry.
fn session_opened_by_split(
    session_id: String,
    outcome: detect::MatchOutcome,
    hero_auth: HeroAuthState,
    career_panel: bool,
    gate: Option<GateState>,
    map: Option<String>,
    now: Instant,
) -> ActiveGame {
    let mut g = ActiveGame::open_at(session_id, outcome, Vec::new(), now);
    g.session_created = true;
    g.map = map;
    g.gate = gate;
    g.last_stats_at = Some(now);
    g.baseline_at = gate.map(|_| now);
    g.reset_baseline = gate;
    g.hero_auth = HeroAuthState {
        career_ever_ok: career_panel,
        accepted_hero: hero_auth.accepted_hero,
        portrait_pending: None,
    };
    g
}

/// Under `reader = "new"`, the accolade map was read with OCR because no
/// map pack is loaded (none synced yet, or none that loads). Logged once
/// per process (F265-3), like the FINAL SCORE line's fallback. With OCR off
/// there is no such fallback and no such line ([`choose_accolade_map`]).
/// Returns whether this call logged (tests).
fn warn_accolade_map_ocr_fallback() -> bool {
    static WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let first = !WARNED.swap(true, std::sync::atomic::Ordering::Relaxed);
    if first {
        tracing::warn!(
            "reader = \"new\" but no map pack is loaded; \
             the accolade map is read with OCR"
        );
    }
    first
}

/// The map on this poll tick's accolade screen, for the session machine.
///
/// Only a result-word tick reads the map. The rank screen does not, and a
/// banner tick is the gameplay HUD under a result flash, so that crop is not
/// a map. A tick taken while Tab cell OCR is in flight (`outcome_only`) reads
/// no map either.
///
/// With `reader = "ocr-v1"` the map is the title OCR of the strip, as
/// before. With `reader = "new"` it is the map pack's read of the accolade
/// map line ([`shadow::read_accolade_map`]): sure and storable, or nothing.
/// When the pack read the frame and abstains there is no map; the OCR is
/// not run in its place, and a panic in the pack reader is an abstain too,
/// so it never costs the tick its result word. When no map pack is loaded
/// nothing read the frame, and the OCR reads it as under ocr-v1, with one
/// warning ([`warn_accolade_map_ocr_fallback`]): the rule of the top bar,
/// the FINAL SCORE line and the phase titles without their packs. With
/// classic OCR off there is no OCR to read it: no map pack means no map,
/// and the OCR read is not asked for (the switch does not grant off
/// without the map pack, so this is a pack that went away). What the
/// session does with the map is the same for both readers
/// (`boundary::decide_poll`).
fn choose_accolade_map(
    use_new_reader: bool,
    outcome_only: bool,
    signal: Option<detect::match_end::OutcomeSource>,
    pack: impl FnOnce() -> shadow::AccoladeRead,
    ocr: impl FnOnce() -> Option<String>,
) -> Option<String> {
    if outcome_only || signal != Some(detect::match_end::OutcomeSource::ResultWord) {
        return None;
    }
    if !use_new_reader {
        return ocr();
    }
    let read = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(pack)) {
        Ok(shadow::AccoladeRead::Map(read)) => read,
        Ok(shadow::AccoladeRead::Abstain) => return None,
        // No map pack, and classic OCR is off: held. `ocr` is not called.
        Ok(shadow::AccoladeRead::NoPack) if ocr::gate::is_off() => return None,
        Ok(shadow::AccoladeRead::NoPack) => {
            warn_accolade_map_ocr_fallback();
            return ocr();
        }
        Err(_) => {
            tracing::warn!("accolade map reader panicked; no accolade map for this tick");
            return None;
        }
    };
    tracing::info!(
        map = %read.name,
        score = read.score,
        margin = read.margin,
        recognizer = shadow::banner::ACCOLADE_MAP_RECOGNIZER_ID,
        "map read from accolade screen (map pack)"
    );
    Some(read.name)
}

/// A prepared analysis for tests of [`handle_capture`]. Production never sets it.
#[cfg(test)]
struct InjectedCapture {
    analysis: FrameAnalysis,
    board: Option<shadow::BoardRead>,
}

#[cfg(test)]
static INJECTED_CAPTURE: std::sync::Mutex<Option<InjectedCapture>> = std::sync::Mutex::new(None);

/// Held by each test that injects a capture, so two such tests never take
/// each other's injection.
#[cfg(test)]
static INJECTED_CAPTURE_SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn take_injected_capture() -> Option<(FrameAnalysis, Option<shadow::BoardRead>)> {
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
static INJECTED_GRAB_ERROR: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

fn take_injected_grab_error() -> Option<String> {
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
fn reader_refusal(analysis: &FrameAnalysis, use_new_reader: bool) -> Option<&'static str> {
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
fn tab_result(
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
fn tab_retry_reason(outcome: &FrameAnalysisOutcome, use_new_reader: bool) -> &'static str {
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
fn tab_outcome_rank(outcome: &FrameAnalysisOutcome, use_new_reader: bool) -> u8 {
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

fn log_reused_frame_skip() {
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
        tracing::debug!("poll skipped: stream frame already seen");
    } else {
        tracing::trace!("poll skipped: stream frame already seen");
    }
}

async fn grab_tab_attempt(
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

async fn handle_capture(ctx: &DaemonCtx, mut req: CaptureRequest) -> anyhow::Result<CaptureReport> {
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
        tracing::info!("Tab detected: capturing screen (hold Tab to keep scoreboard visible)");
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
                        tracing::info!(board_refusal, "Tab frame refused by the scoreboard reader");
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
                    tracing::info!("Tab capture missed: no fresh stream frame");
                }
                if matches!(analysis, FrameAnalysisOutcome::CaptureBusy) {
                    tracing::info!("Tab capture missed: the previous screenshot was still running");
                }
                if matches!(analysis, FrameAnalysisOutcome::PortalPaused) {
                    tracing::info!("Tab capture missed: the screenshot portal is not answering");
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
                tracing::info!(outcome = ?o, "outcome read from scoreboard OCR header line");
                frame_outcome = o;
            }
        }
    }
    let mut outcome = if game_outcome.is_decided() {
        game_outcome
    } else {
        frame_outcome
    };

    tracing::info!(?outcome, "frame analysis");
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
            confidence = ocr_result.confidence,
            text_preview = &ocr_result.raw_text[..preview_end],
            "OCR result"
        );
    }

    let player_row_conf = player_row_idx
        .and_then(|i| rows.get(i))
        .map(|r| r.mean_confidence);
    tracing::info!(
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
                    held = ?held_cols,
                    "new reader held stat cells it does not vouch for"
                );
            }
            if own_row_suspect {
                tracing::info!(
                    ?player_row_idx,
                    "own row is suspect: none of its stats are stored or used to raise the totals"
                );
            }
            if !trusted_cells {
                tracing::warn!(
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
                tracing::info!(hero = %parsed.hero, "no real portrait reference for career-panel hero: seeding one from this capture");
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
                        tracing::warn!(error = %e, hero = %parsed.hero, "portrait save failed");
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
                        session_id = %target_session,
                        col = u.col,
                        raw = u.raw,
                        revised_from = u.revised_from,
                        "capture gate replaced an unconfirmed cell"
                    );
                } else {
                    tracing::warn!(
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
                tracing::debug!(error = %e, "failed to refresh session hero timeline");
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

fn rand_id() -> u64 {
    use std::time::SystemTime;
    let seed = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    seed ^ (std::process::id() as u64).wrapping_mul(0x517cc1b727220a95)
}

/// Let an in-flight capture task finish before shutdown's final sync, so its
/// snapshot is uploaded and no store write is torn by the runtime dropping it.
async fn drain_capture(task: Option<tokio::task::JoinHandle<()>>) {
    if let Some(t) = task
        && !t.is_finished()
    {
        tracing::info!("waiting for in-flight capture to finish");
        let _ = t.await;
    }
}

/// Minimum gap between full snapshot rewrites (P11). Capture/poll can mark the
/// export dirty faster than this; the next due flush (or a forced one after
/// sync / shutdown) actually rewrites the file.
const SNAPSHOT_DEBOUNCE: std::time::Duration = std::time::Duration::from_secs(2);

/// (dirty, last successful export). Module-level so capture + sync tasks share it.
static SNAPSHOT_STATE: std::sync::Mutex<(bool, Option<std::time::Instant>)> =
    std::sync::Mutex::new((false, None));

/// Mark the live snapshot dirty and export if the debounce window has elapsed.
async fn refresh_snapshot(store: &storage::LocalStore, data_dir: &std::path::Path) {
    refresh_snapshot_inner(store, data_dir, false).await;
}

/// Export immediately (sync complete, shutdown). Resets dirty.
async fn refresh_snapshot_force(store: &storage::LocalStore, data_dir: &std::path::Path) {
    refresh_snapshot_inner(store, data_dir, true).await;
}

/// Flush a debounce-deferred snapshot once its window has passed. Called from
/// the cmd-timer tick so a dirty snapshot never waits on the next mutation.
async fn flush_snapshot_if_due(store: &storage::LocalStore, data_dir: &std::path::Path) {
    let due = {
        let state = SNAPSHOT_STATE.lock().unwrap_or_else(|e| e.into_inner());
        state.0
            && state
                .1
                .is_none_or(|last| last.elapsed() >= SNAPSHOT_DEBOUNCE)
    };
    if due {
        refresh_snapshot_inner(store, data_dir, true).await;
    }
}

/// Shutdown path: export a trailing dirty snapshot regardless of the window,
/// the GUI must see the final state (try_sync only force-exports on success).
async fn flush_snapshot_if_dirty(store: &storage::LocalStore, data_dir: &std::path::Path) {
    let dirty = SNAPSHOT_STATE.lock().unwrap_or_else(|e| e.into_inner()).0;
    if dirty {
        refresh_snapshot_inner(store, data_dir, true).await;
    }
}

async fn refresh_snapshot_inner(
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    force: bool,
) {
    {
        let mut state = SNAPSHOT_STATE.lock().unwrap_or_else(|e| e.into_inner());
        state.0 = true;
        if !force
            && let Some(last) = state.1
            && last.elapsed() < SNAPSHOT_DEBOUNCE
        {
            return;
        }
    }
    match store.export_snapshot(data_dir).await {
        Ok(()) => {
            if let Ok(mut state) = SNAPSHOT_STATE.lock() {
                state.0 = false;
                state.1 = Some(std::time::Instant::now());
            }
        }
        Err(e) => tracing::debug!(error = %e, "live snapshot refresh failed"),
    }
}

/// Delete a stale skeleton without uploading. The daemon loop schedules the
/// upload on `sync_task` so startup does not wait on HTTP. A held board is
/// returned so the caller can write it before that upload.
fn retire_stale_skeleton(data_dir: &std::path::Path) -> Option<StaleDrop> {
    match admit_persisted_game(data_dir) {
        ActiveAdmission::Stale { session_id, hold } => {
            let archive = hold.map(|hold| {
                // A fresh hold belongs to the next game. The skeleton map is
                // the previous game, so keeping it would archive that board
                // as Busan and a later GUI result would upload it as Busan.
                let (session_id, map, outcome) = if hold.imported {
                    (
                        session_id.clone(),
                        hold.map.unwrap_or_default(),
                        hold.outcome.to_string(),
                    )
                } else {
                    (
                        format!("{:016x}", rand_id()),
                        String::new(),
                        "unknown".into(),
                    )
                };
                tracing::warn!(
                    session_id = %session_id,
                    outcome = %outcome,
                    "stale restart kept a held board in local history"
                );
                ArchivedHold {
                    session_id,
                    counters: hold.counters,
                    hero: hold.hero,
                    map,
                    outcome,
                    at: hold.at.unwrap_or_else(Utc::now),
                }
            });
            persist_active_game(data_dir, None);
            tracing::info!(
                session_id = %session_id,
                "dropped stale active game; holding its unknown rows and uploading decided ones"
            );
            Some(StaleDrop {
                session_id,
                archive,
            })
        }
        ActiveAdmission::Open(_) | ActiveAdmission::Absent => None,
    }
}

/// Write a stale hold into local history. An imported hold keeps the
/// persisted map and outcome. A fresh hold is an unknown session with an
/// empty map. Unknown rows are not uploaded. A Deathmatch map stays local
/// even if a later edit sets a result.
async fn archive_stale_hold(
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    archive: &ArchivedHold,
) {
    let hero = archive.hero.clone().unwrap_or_default();
    if let Err(e) = store_held_board(
        store,
        data_dir,
        &archive.session_id,
        &hero,
        &archive.map,
        &archive.outcome,
        archive.counters,
        archive.at,
    )
    .await
    {
        tracing::warn!(
            error = %e,
            session_id = %archive.session_id,
            "failed to keep a stale held board in local history"
        );
    }
}

/// One command tick of the GUI queue. A SetOutcome or a delete schedules
/// an upload on this same tick, so a deleted game leaves the site right
/// away instead of at the next Tab sync or daemon restart (O3). An edit
/// does not; the idle sync tick picks it up.
async fn apply_queued_commands(
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
        tracing::info!(?cmd, "applying GUI command");
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
                tracing::warn!(error = %e, "GUI command failed; will retry");
            }
        }
    }
    refresh_snapshot(store, data_dir).await;
}

/// Startup and resume. Drop a stale skeleton, schedule its upload, then
/// restore the session. Does not wait for HTTP.
async fn begin_daemon_session(
    data_dir: &std::path::Path,
    sync_task: &mut Option<tokio::task::JoinHandle<()>>,
    backoff: &Arc<std::sync::Mutex<sync::SyncBackoff>>,
    store: &storage::LocalStore,
    client: Option<&sync::SyncClient>,
) -> SessionState {
    let dropped = retire_stale_skeleton(data_dir);
    if let Some(drop) = dropped.as_ref()
        && let Some(archive) = &drop.archive
    {
        archive_stale_hold(store, data_dir, archive).await;
    }
    let dropped = dropped.map(|drop| drop.session_id);
    // A stale skeleton is no longer the open game. Mark that session's
    // `unknown` rows synced locally and do not send them. A fresh skeleton
    // stays, and those rows stay unsynced until a result, a new-game
    // boundary, or the 6-hour bound. A clean shutdown keeps it.
    schedule_store_sync(sync_task, backoff, store, client, data_dir, dropped.clone());
    // A resume within 20 minutes must not write a pending imported hold.
    // The skeleton map is often empty, and writing it here would store the
    // board under that empty map. A stale start already archived the hold
    // above, with the persisted map and outcome.
    let mut st = startup_session(data_dir);
    // A game restored unfinished opens the rejoin window: a crash usually
    // drops the player out of the match, and the rejoin resets their stats.
    mark_restart_recovered(&mut st, Instant::now());
    st
}

/// Hero for a held board. Prefer the hero captured with the hold. Otherwise
/// use the newest stored row, the same fallback a later Tab uses for the
/// current hero.
async fn held_board_hero(
    store: &storage::LocalStore,
    session_id: &str,
    deferred_hero: Option<String>,
) -> String {
    if let Some(hero) = deferred_hero.filter(|hero| !hero.is_empty()) {
        return hero;
    }
    store
        .get_session_snapshots(session_id)
        .await
        .ok()
        .and_then(|rows| rows.last().map(|row| row.hero.clone()))
        .unwrap_or_default()
}

/// Write the imported hold onto its session. The caller clears the hold
/// only after this returns Ok.
async fn write_imported_hold(
    game: &ActiveGame,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
) -> anyhow::Result<()> {
    let Some(counters) = game.deferred else {
        return Ok(());
    };
    let hero = held_board_hero(store, &game.session_id, game.deferred_hero.clone()).await;
    let map_name = game.map.clone().unwrap_or_default();
    let outcome = game.outcome.to_string();
    let played_at = game.deferred_at.unwrap_or_else(Utc::now);
    store_held_board(
        store,
        data_dir,
        &game.session_id,
        &hero,
        &map_name,
        &outcome,
        counters,
        played_at,
    )
    .await
}

fn clear_imported_hold(game: &mut ActiveGame) {
    game.session_created = true;
    game.deferred = None;
    game.deferred_hero = None;
    game.deferred_at = None;
    game.deferred_imported = false;
}

/// Close or keep the open session before the final sync.
///
/// A held board is written onto the current session only when
/// `deferred_imported` is set (it already belongs to this session). A
/// fresh-match hold belongs to the next game: it is not written onto a
/// finished session. That session's decided rows stay the ones that upload.
/// An unfinished game keeps its skeleton so a clean restart resumes it.
/// If the imported write fails, the hold stays on the skeleton. A resume
/// within 20 minutes leaves it pending. A stale start (more than 20
/// minutes, or a clock that moved backwards) archives it with the
/// persisted map and outcome.
async fn close_session_on_shutdown(
    st: &mut SessionState,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
) {
    let imported_write = if let Some(game) = st.active_game.as_ref()
        && game.deferred_imported
        && game.deferred.is_some()
    {
        Some(write_imported_hold(game, store, data_dir).await)
    } else {
        None
    };
    match imported_write {
        Some(Ok(())) => {
            if let Some(game) = st.active_game.as_mut() {
                clear_imported_hold(game);
            }
        }
        Some(Err(e)) => {
            tracing::warn!(error = %e, "failed to store a held board on shutdown");
            persist_active_game(data_dir, st.active_game.as_ref());
            return;
        }
        None => {}
    }
    let fresh_hold = st.active_game.as_ref().and_then(|game| {
        if game.deferred_imported {
            None
        } else {
            game.deferred
                .map(|counters| (counters, game.deferred_hero.clone(), game.deferred_at))
        }
    });
    let finished = st.active_game.as_ref().is_some_and(|game| game.finished());
    if !finished {
        if let Some(game) = st.active_game.as_mut() {
            game.deferred = fresh_hold.as_ref().map(|(counters, _, _)| *counters);
        }
        persist_active_game(data_dir, st.active_game.as_ref());
        return;
    }
    if let Some(game) = st.active_game.as_mut()
        && fresh_hold.is_some()
    {
        game.deferred = None;
        game.deferred_hero = None;
        game.deferred_at = None;
        game.deferred_imported = false;
    }
    if st.active_game.is_some() {
        retire_active_game(st, store, data_dir, None, SHUTDOWN_CLOSE_REASON).await;
    }
    if let Some((counters, hero, at)) = fresh_hold {
        let mut opened = ActiveGame::open_at(
            format!("{:016x}", rand_id()),
            detect::MatchOutcome::Unknown,
            Vec::new(),
            Instant::now(),
        );
        opened.deferred = Some(counters);
        opened.deferred_hero = hero;
        opened.deferred_at = at;
        opened.deferred_imported = true;
        st.active_game = Some(opened);
        persist_active_game(data_dir, st.active_game.as_ref());
    } else {
        persist_active_game(data_dir, None);
    }
}

/// How a poll's grab or read ended when raced against shutdown and the
/// keyboard.
enum PollRace<T> {
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
async fn stop_for_shutdown_or_key<T, K, S, KF>(
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
async fn with_bridge_hold<K, E, H>(
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
fn then_end_tab_hold<F: std::future::Future>(
    retries: F,
) -> impl std::future::Future<Output = F::Output> {
    use futures_util::FutureExt as _;
    retries.inspect(|_| {
        capture::end_tab_stream_hold();
    })
}

/// The key event a dropped poll kept, else the next one from `next`. A kept
/// event returns on the first poll, so the keyboard arm takes it at once.
async fn pending_or_next<K>(
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
async fn stop_for_shutdown<T, S>(
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
async fn finish_within_shutdown<T, S, B>(
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

/// What the Tab capture task sends back to the run loop.
type CaptureReports = tokio::sync::mpsc::UnboundedReceiver<(String, Result<CaptureReport, String>)>;

/// One step of [`shutdown_from_signal`]. The steps run in
/// [`SHUTDOWN_STAGES`] order, and the deadline log names the one running.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShutdownStage {
    /// Wait for the in-flight Tab capture and apply its report.
    CaptureDrain,
    /// Close or keep the game, and write the skeleton.
    SessionSave,
    /// Close the screen share, bounded by [`SCREENCAST_CLOSE_TIMEOUT`].
    ScreencastClose,
    /// Join the in-flight sync, then the last upload.
    FinalUpload,
    /// Write the GUI snapshot.
    SnapshotExport,
}

impl ShutdownStage {
    fn label(self) -> &'static str {
        match self {
            Self::CaptureDrain => "in-flight capture",
            Self::SessionSave => "session save",
            Self::ScreencastClose => "screencast session",
            Self::FinalUpload => "final upload",
            Self::SnapshotExport => "snapshot export",
        }
    }
}

/// Shutdown order. The game is saved right after the in-flight capture is
/// applied, before the portal close and the upload: those wait on other
/// processes and the network, and must not cost the save its share of the
/// deadline (MAIN-3). The snapshot export reads the store after the upload.
const SHUTDOWN_STAGES: [ShutdownStage; 5] = [
    ShutdownStage::CaptureDrain,
    ShutdownStage::SessionSave,
    ShutdownStage::ScreencastClose,
    ShutdownStage::FinalUpload,
    ShutdownStage::SnapshotExport,
];

/// A portal that does not answer gets this long. The upload keeps the rest.
const SCREENCAST_CLOSE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);

/// Deadline time the capture drain leaves for the session save. A capture
/// stuck in OCR or a screenshot fallback is abandoned at that point, and the
/// game is saved without its report.
const SESSION_SAVE_RESERVE: std::time::Duration = std::time::Duration::from_millis(1500);

/// Wait for the in-flight capture until only [`SESSION_SAVE_RESERVE`] of
/// `left` remains. Returns false when the capture was abandoned; its task is
/// detached, not aborted, so a store write it has started is not torn.
async fn drain_capture_within(
    task: Option<tokio::task::JoinHandle<()>>,
    left: std::time::Duration,
) -> bool {
    let budget = left.saturating_sub(SESSION_SAVE_RESERVE);
    if tokio::time::timeout(budget, drain_capture(task))
        .await
        .is_ok()
    {
        return true;
    }
    tracing::warn!(
        budget_ms = budget.as_millis() as u64,
        "in-flight capture abandoned at shutdown; saving the game without it"
    );
    false
}

/// The deadline log for a shutdown stopped at `SHUTDOWN_STAGES[at]`.
fn shutdown_overrun_at(at: usize) -> String {
    let at = at.min(SHUTDOWN_STAGES.len() - 1);
    let not_reached: Vec<&str> = SHUTDOWN_STAGES[at + 1..]
        .iter()
        .map(|stage| stage.label())
        .collect();
    stat_tracker::replace::shutdown_overrun_line(SHUTDOWN_STAGES[at].label(), &not_reached)
}

/// Apply every capture report already sent. [`drain_capture`] waits for the
/// task; its report is in the channel, and would be lost with it: session
/// splits, the previous game's seal, the gate, a held board, the map and the
/// hero authority (MAIN-2).
async fn apply_landed_capture_reports(
    st: &mut SessionState,
    capture_rx: &mut CaptureReports,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
) {
    while let Ok((sid, result)) = capture_rx.try_recv() {
        // The shutdown sync that follows uploads a game this closes.
        let sid = settle_tab_probe(st, store, data_dir, sid, &result, Instant::now(), || {
            std::future::ready(())
        })
        .await;
        apply_capture_report(st, store, data_dir, &sid, result, Instant::now()).await;
    }
}

async fn close_screencast_on_shutdown() {
    if tokio::time::timeout(SCREENCAST_CLOSE_TIMEOUT, capture::close_if_open())
        .await
        .is_err()
    {
        tracing::warn!(
            timeout_ms = SCREENCAST_CLOSE_TIMEOUT.as_millis() as u64,
            "screencast session did not close in time; shutdown goes on"
        );
    }
}

/// Save the game and exit within the shared deadline. Returns the step the
/// deadline stopped, or `None` when every step finished.
#[allow(clippy::too_many_arguments)]
async fn shutdown_from_signal(
    st: &mut SessionState,
    capture_task: &mut Option<tokio::task::JoinHandle<()>>,
    capture_rx: &mut CaptureReports,
    sync_task: &mut Option<tokio::task::JoinHandle<()>>,
    sync_client: Option<&sync::SyncClient>,
    sync_backoff: &std::sync::Mutex<sync::SyncBackoff>,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
) -> Option<ShutdownStage> {
    let at = std::sync::atomic::AtomicUsize::new(0);
    tokio::select! {
        _ = async {
            for (i, stage) in SHUTDOWN_STAGES.iter().enumerate() {
                at.store(i, std::sync::atomic::Ordering::Relaxed);
                match stage {
                    ShutdownStage::CaptureDrain => {
                        drain_capture_within(
                            capture_task.take(),
                            stat_tracker::replace::shutdown_time_left(),
                        )
                        .await;
                        apply_landed_capture_reports(st, capture_rx, store, data_dir).await;
                    }
                    ShutdownStage::SessionSave => {
                        close_session_on_shutdown(st, store, data_dir).await;
                    }
                    ShutdownStage::ScreencastClose => close_screencast_on_shutdown().await,
                    ShutdownStage::FinalUpload => {
                        let task = sync_task.take();
                        finish_sync_on_shutdown(sync_client, sync_backoff, task, store, data_dir)
                            .await;
                    }
                    ShutdownStage::SnapshotExport => {
                        flush_snapshot_if_dirty(store, data_dir).await;
                        // perf.log lines already queued go to disk before the
                        // process ends (the open window is not closed here).
                        // Bounded by what is left of the shutdown deadline,
                        // off the async workers.
                        let wait = shutdown_flush_wait(stat_tracker::replace::shutdown_time_left());
                        let flushed = tokio::task::spawn_blocking(move || {
                            stat_tracker::perf_log::flush(wait)
                        })
                        .await
                        .unwrap_or(false);
                        if !flushed {
                            tracing::debug!(
                                wait_ms = wait.as_millis() as u64,
                                "perf.log lines may be lost: the log thread did not answer in time"
                            );
                        }
                    }
                }
            }
        } => None,
        _ = tokio::time::sleep(stat_tracker::replace::shutdown_time_left()) => {
            // Returning here used to leave the runtime drop waiting on
            // blocking tasks. `main` calls `Runtime::shutdown_timeout` with
            // this same deadline, so those tasks cannot hold the exit.
            // The step is read now, not when shutdown began.
            let stopped = at
                .load(std::sync::atomic::Ordering::Relaxed)
                .min(SHUTDOWN_STAGES.len() - 1);
            tracing::error!("{}", shutdown_overrun_at(stopped));
            Some(SHUTDOWN_STAGES[stopped])
        }
    }
}

async fn finish_sync_on_shutdown(
    client: Option<&sync::SyncClient>,
    backoff: &std::sync::Mutex<sync::SyncBackoff>,
    task: Option<tokio::task::JoinHandle<()>>,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
) {
    drain_sync_then(task, || async {
        let outcome = upload_once(client, backoff, store, data_dir, None).await;
        apply_sync_backoff(backoff, outcome);
    })
    .await;
}

#[cfg(test)]
mod ocr_bench;

#[cfg(test)]
mod main_tests;
