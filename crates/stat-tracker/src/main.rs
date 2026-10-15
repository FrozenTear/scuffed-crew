use stat_tracker::boundary::{self, ResultMark};
use stat_tracker::capture_gate::{self, Counters, GateState};
use stat_tracker::hero_auth::{self, HeroAuthState, HeroSource};
use stat_tracker::{
    capture, config, detect, mapcrops, ocr, parse, reader_apply, setup, shadow, storage, sync,
};

use std::sync::Arc;
use std::time::Instant;

use anyhow::Context as _;
use chrono::Utc;
use surrealdb_types::Datetime as SurrealDatetime;
use tracing_subscriber::EnvFilter;

const SYNC_EVERY_N_CAPTURES: u32 = 5;

/// Window for two word-OCR outcome reads to confirm each other. Sized to span
/// the accolade → rank-screen transition under a starved poller (measured 45s
/// between the last accolade tick and the first rank-screen tick) while still
/// bounding how long a single stray read stays actionable.
const OUTCOME_CONFIRM_WINDOW: std::time::Duration = std::time::Duration::from_secs(60);

/// PR-B (fleet::tracker-fps): mid-match the poller performs its screencopy
/// only on every Nth interval tick. Each compositor screencopy is a GPU
/// readback on the gaming output — the frametime hitch behind the "fuzzy"
/// reports — and mid-match ticks carry no signal worth that cost at full
/// cadence. 2 (not 3): phase/end screens must still land two poll ticks
/// inside their lifetime — the OCR stability gate defers the first sighting,
/// and the shortest screen (map vote, ~15s) only fits two ticks at the 8s
/// effective cadence this yields under the default 4s interval.
const SLOW_POLL_DIVISOR: u32 = 2;

/// How recently a game must have opened for the poller to stay at full
/// cadence: start screens (map vote / ban / hero select) and early corrective
/// evidence cluster in the first stretch of a session, and matches don't end
/// this early — after it, mid-match slow cadence applies until end evidence
/// shows up (outcome recorded, fresh word-OCR streak, POTG / end-reel wake,
/// or a new game opening).
const SLOW_AFTER_GAME_OPEN: std::time::Duration = std::time::Duration::from_secs(120);

/// How long a Play of the Game / end-reel sighting holds the poller at full
/// cadence. POTG is ~15–20s; the Victory/Defeat banner is ~3s and the
/// accolade screen follows immediately (see `read_result_word`). 45s from
/// first sighting covers remaining reel + the short V/D window with margin,
/// then slow mode resumes if no outcome/streak has landed. Mid-point of the
/// 30–60s range — long enough to not miss Victory, short enough that a
/// false letterbox does not pin 4s screencopy for a full minute.
const END_REEL_WAKE: std::time::Duration = std::time::Duration::from_secs(45);

/// How many poll-tick frames `--dump-poll-frames` keeps (ring buffer on disk).
/// At a 4s poll interval this is ~10 minutes — enough that a defeat's
/// post-match sequence survives even if the next game is already underway
/// before the frames are copied out. Frames are a few MB each.
const POLL_DUMP_KEEP: usize = 150;

/// How many rejected-capture frames are kept in `<data_dir>/debug/rejected/`.
/// A rejected capture records nothing, so the frame is the only evidence for
/// diagnosing why ("it didn't record my game" is undebuggable otherwise).
/// Written only with `debug_ocr` on: the frame is unblanked and shows other
/// players' names.
const REJECTED_KEEP: usize = 30;

/// How many ACCEPTED scoreboard crops are kept in `<data_dir>/debug/accepted/`.
/// A silently-corrupt accepted board (OCR drift that still passed every trust
/// gate) is otherwise unrecoverable for calibration retuning. Written only
/// with `debug_ocr` on: the crop is unblanked and shows other players' names.
const ACCEPTED_KEEP: usize = 20;

/// After this many consecutive scoreboard captures that parsed but resolved no
/// map, dump the map-label region to `debug/mapmiss/` so the miss is
/// diagnosable from raw pixels. The 07-17 Ilios game read no map on any frame
/// and left no pixel evidence of why (mangled OCR text is not enough).
const EMPTY_MAP_DUMP_THRESHOLD: usize = 5;

/// Ring size for the `debug/mapmiss/` map-region dumps.
const MAPMISS_KEEP: usize = 10;

/// Bounded retries when startup finds zero keyboards. A miss is usually
/// "not in the `input` group" (only a new login's process credentials can
/// fix that) or `/dev/input` not enumerated yet (a same-process retry can).
/// Kept short so the pid file is not held while the GUI shows a daemon
/// that will never capture. After this, the process exits non-zero and
/// systemd `Restart=on-failure` tries again; a later graphical-session
/// start covers re-login.
const KEYBOARD_OPEN_ATTEMPTS: u32 = 3;

/// Gap between [`KEYBOARD_OPEN_ATTEMPTS`]. This wait also polls SIGTERM
/// and Ctrl+C — without that, `systemctl --user stop` sits until
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

/// Open a keyboard, retrying on failure. `Err` is "still no keyboard" —
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
    /// and first word-OCR streak — not every mid-match tick. Also gates the
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
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to start the async runtime")?
        .block_on(run(omp))
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
    let _pid_guard = acquire_pid_guard(&config.data_dir)?;

    // One client for the whole process. An unsafe URL logs once here and
    // leaves sync off — including the startup player-name fetch — so the
    // bearer token is never sent. The daemon keeps running.
    let sync_client = open_sync_client(config.sync.as_ref());
    // heroes-v1 syncs here; the other packs sync in the background once
    // capture is up. The new reader picks up heroes-v2 and heroes-v2-1080
    // on a later board when they land (`shadow::heroes::LiveHeroSet`).
    let mut extra_packs = None;
    if let Some(client) = &sync_client {
        let creds = client.credentials();
        let mut pack_startup = None;
        if sync::auth_pause_matches(&config.data_dir, &creds.server_url, &creds.token) {
            tracing::error!(
                "sync paused: the server rejected this token. Update the token in Settings, then restart the tracker or save the new URL or token."
            );
        } else {
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
        extra_packs = Some(stat_tracker::packs::PackKeeper::new(
            &config.data_dir,
            pack_startup,
            move || pack_credentials(&data_dir, &client),
        ));
    }

    // Single wiring point for OCR debug dumps — the lib reads this switch
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
    // Don't run it at daemon startup — it can take minutes and blocks Tab capture.

    let backend = capture::detect_backend().await;
    tracing::info!(?backend, "capture backend selected");

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

    let portraits_path = detect::hero_portrait::portraits_dir(&config.data_dir);
    let portrait_matcher = Arc::new(detect::hero_portrait::PortraitMatcher::load(
        &portraits_path,
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
        // hard error (non-zero exit) — it must not read as "zero outputs".
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
/// a restart, for the job as for uploads. `None` while sync is paused: the
/// rejected token is not sent again.
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

/// The own-name seed store, opened only when the new reader is in effect
/// (`new_reader`, from [`new_reader_in_effect`]). A store kept for another
/// configured in-game name is cleared on open; a name that is not known in
/// this run (not set, or the fetch failed) keeps it
/// ([`detect::own_row::SeedStore::open`]).
fn open_own_name_seeds(
    new_reader: bool,
    config: &config::Config,
) -> Option<Arc<std::sync::Mutex<detect::own_row::SeedStore>>> {
    new_reader.then(|| {
        Arc::new(std::sync::Mutex::new(detect::own_row::SeedStore::open(
            &config.data_dir,
            config.player_name.as_deref(),
        )))
    })
}

/// The trusted own-name seeds for one vision pass. Empty under ocr-v1.
fn own_name_seeds(
    store: Option<&Arc<std::sync::Mutex<detect::own_row::SeedStore>>>,
) -> Vec<detect::own_row::Seed> {
    store
        .map(|store| {
            store
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .trusted()
                .to_vec()
        })
        .unwrap_or_default()
}

/// Tell the own-name seed store what one Tab's board says about the trusted
/// seeds, held Tab or stored (session `session_id`). Seeds that keep naming
/// another row than the one the career hero is read on are dropped, so a
/// wrong seed cannot hide the member's real row for good; and the log says
/// once when games go by without any trusted seed.
async fn note_own_seed_verdict(
    store: Option<&Arc<std::sync::Mutex<detect::own_row::SeedStore>>>,
    verdict: detect::own_row::Verdict,
    session_id: &str,
) {
    let Some(store) = store else {
        return;
    };
    let session = detect::own_row::game_key(session_id);
    // No disk write unless seeds are dropped, and that is rare: no
    // blocking task for the common case.
    let noted = store
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .note(verdict, &session);
    if noted.dropped > 0 {
        tracing::warn!(
            dropped = noted.dropped,
            boards = detect::own_row::DISAGREE_DROPS,
            "own-row seeds named another row than the career hero's on this many boards in a row: dropped, the name line is learned again"
        );
    }
    if noted.unseeded_notice {
        tracing::info!(
            games = detect::own_row::UNSEEDED_NOTICE_GAMES,
            "no own-name seed is trusted yet after this many games: the own row comes from the highlight or the career hero alone"
        );
    }
}

/// Hand a gated crop of the own row's name line to the seed store, for the
/// game `session_id` on `map`. The store trusts it only once enough crops
/// match each other: three from highlighted rows over two games, or four
/// over three games on two career heroes
/// ([`detect::own_row::SeedStore::offer`]). Runs off the async threads
/// because trusted seeds are written to disk.
async fn offer_own_name_seed(
    store: Option<&Arc<std::sync::Mutex<detect::own_row::SeedStore>>>,
    candidate: Option<detect::own_row::Candidate>,
    session_id: &str,
    map: &str,
) {
    let (Some(store), Some(candidate)) = (store, candidate) else {
        return;
    };
    let store = Arc::clone(store);
    let game = detect::own_row::GameTag {
        session: detect::own_row::game_key(session_id),
        map: map.to_string(),
        at: Utc::now().timestamp().max(0) as u64,
    };
    let proof = candidate.proof().label();
    let offered = tokio::task::spawn_blocking(move || {
        let mut store = store.lock().unwrap_or_else(|e| e.into_inner());
        let offer = store.offer(candidate, &game);
        (offer, store.trusted().len(), store.pending_len())
    })
    .await;
    if let Ok((offer, trusted, pending)) = offered {
        tracing::info!(?offer, proof, trusted, pending, "own-row seed crop offered");
    }
}

/// When `player_name` isn't set locally, fetch it from the server. This is the
/// "first run via GUI" path: the user set their name in the web UI and launched
/// the daemon with just a token — no manual config editing needed.
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
            Err(e) if matches!(e.attempt(), sync::SyncAttempt::AuthRejected) => {
                let creds = client.credentials();
                if let Err(err) =
                    sync::write_auth_pause(&config.data_dir, &creds.server_url, &creds.token)
                {
                    tracing::warn!(error = %err, "could not record sync auth pause");
                }
                tracing::error!(
                    error = %e,
                    "sync token rejected by daemon-config: pausing sync until the URL or token changes in Settings"
                );
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
/// the next game starts — so captures taken across hero swaps all land in one
/// session. `outcome` is filled in when the poller reads the post-match screens
/// (or recovered from a captured frame), then back-filled onto the snapshots.
struct ActiveGame {
    /// Restored after a restart with a result screen already seen for it.
    /// Memory only. The first Tab after [`POST_MATCH_GRACE`] from that
    /// result starts a new game ([`should_start_fresh_session`]).
    restored_with_result: bool,
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
    /// unknowable at vote time, so these are CANDIDATES only — they constrain
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
    /// absorb captures — see [`UNFINISHED_SESSION_IDLE`].
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
/// restarts are routine — without this, a restart mid-game either split the
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
    // defaults this to None (one game's cross-restart gate memory lost — the
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

/// Quiet close, shutdown, and the long unfinished bound each keep their own
/// message. Every other close is a new-game boundary.
fn session_close_log(reason: &str) -> &'static str {
    match reason {
        QUIET_CLOSE_REASON => QUIET_CLOSE_REASON,
        SHUTDOWN_CLOSE_REASON => SHUTDOWN_CLOSE_REASON,
        UNFINISHED_MAX_REASON => UNFINISHED_MAX_REASON,
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
    let Some(mut g) = st.active_game.take() else {
        return;
    };
    // The game is over: its unblanked portrait cells leave memory.
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
    // closes here only after [`UNFINISHED_SESSION_MAX`].
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
    match active_game_from_persisted(persisted) {
        Some(game) => ActiveAdmission::Open(Box::new(game)),
        None => ActiveAdmission::Stale { session_id, hold },
    }
}

fn active_game_from_persisted(p: PersistedGame) -> Option<ActiveGame> {
    let to_instant =
        |w: chrono::DateTime<Utc>| Instant::now().checked_sub((Utc::now() - w).to_std().ok()?);
    if let Some(reason) = restart_resume_refusal(&p, Utc::now()) {
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
        // logic already treats as stale — the outcome can't leak forward.
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
    })
}

/// Longest gap since the previous run's last capture for which a restart
/// resumes that game. A crash and restart mid-game takes well under a
/// minute; the next game's first Tab usually comes after a result screen,
/// a queue and hero select. Past this, the next Tab starts a new game.
const RESUME_AFTER_RESTART_MAX_GAP: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/// Why the game in `active_game.json` is not resumed after a restart, or
/// `None` when it is clearly the same game. An unfinished game with a
/// stored board resumes only when it has a known map and its last capture
/// is at most [`RESUME_AFTER_RESTART_MAX_GAP`] old. A Tab after the restart
/// that names another map still splits on the boundary's different-map
/// rule.
///
/// A game with a result already seen is restored so its result and its
/// post-match scoreboard stay on it. It is marked
/// [`ActiveGame::restored_with_result`], so the first Tab after
/// [`POST_MATCH_GRACE`] from that result starts a new game. A game whose
/// outcome is recorded starts a new game on that Tab anyway.
fn restart_resume_refusal(p: &PersistedGame, now: chrono::DateTime<Utc>) -> Option<&'static str> {
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
    match (now - p.last_activity).to_std() {
        Ok(gap) if gap <= RESUME_AFTER_RESTART_MAX_GAP => None,
        Ok(_) => Some("last capture too long ago"),
        Err(_) => Some("last capture is later than now (clock moved back)"),
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
fn quiet_close_reason(
    game: &ActiveGame,
    now: Instant,
    close_after: std::time::Duration,
) -> Option<&'static str> {
    if !game.finished() {
        if now.saturating_duration_since(game.last_activity) >= UNFINISHED_SESSION_MAX {
            return Some(UNFINISHED_MAX_REASON);
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

/// How long an unfinished session stays reusable with no recorded activity.
/// If the poller misses the outcome AND the next game's start screens (likely
/// under Tab starvation, or with auto-detect off), an unbounded session would
/// absorb every capture that follows — yesterday's unfinished game swallowing
/// today's first Tab. Sized comfortably above a long match plus queue time.
const UNFINISHED_SESSION_IDLE: std::time::Duration = std::time::Duration::from_secs(20 * 60);

/// Upper bound for a session that never got a result. A long match still
/// takes its result. Past this, the command tick closes the session and
/// holds the unknown rows locally.
const UNFINISHED_SESSION_MAX: std::time::Duration = std::time::Duration::from_secs(6 * 60 * 60);

/// Minimum age of a poller-opened game before a banner-color outcome read off
/// a Tab frame is trusted. The color-flood detector can false-positive on
/// heavy mid-fight red vignettes; a real banner can't appear this early into
/// a match. The end-title text path stays available regardless, and a
/// session freshly opened by the Tab itself (daemon joined mid/post match) is
/// exempt — there the banner is exactly the evidence being recovered.
const MIN_BANNER_SESSION_AGE: std::time::Duration = std::time::Duration::from_secs(300);

/// Wall-vs-monotonic divergence treated as a suspend (m4). `Instant` is
/// CLOCK_MONOTONIC, which freezes during suspend — after resume every
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
        // No game open — daemon started mid-match or the start screen was missed.
        None => true,
        // Mid-game capture — unless the session has been idle so long it
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
        // No confirmed previous map — identity reuse does not apply.
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
/// false-positive vote can open another — observed as two Games cards for one
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
/// match (hero swaps included) — they never decrease. Requiring at least two
/// of the three to drop keeps a single misread column (e.g. an inflated
/// elims read) from faking a boundary, while a real new game — all counters
/// restarting near zero — trips it reliably.
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
/// read also dropped versus `prev_raw` beyond the corroboration band — a
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
/// scoreboard-text read. Map-vote names never appear here — the vote winner
/// is unknowable at vote time (recording candidates as the played map was
/// wrong ~2/3 of the time with 2+ candidates) — but when candidates are known
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
    let gate = capture_gate::apply_gate_seeded(
        gate_prev,
        facts.counters,
        facts.suspect,
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
    /// Cropped scoreboard (portrait auto-collection reads from it). Shared
    /// with the shadow worker when it is on, never copied.
    scoreboard: Arc<image::DynamicImage>,
    /// The player's row index. ocr-v1: by name match or brightness
    /// highlight. `reader = "new"`: by the own-name seeds, the highlight
    /// and the career hero's row ([`find_own_row`]); by a name read only
    /// when none of them names a row.
    player_row_idx: Option<usize>,
    /// The name match behind `player_row_idx`, with the name window that
    /// matched. `None` when the row came from the highlight alone, and
    /// always under the new reader.
    name_match: Option<parse::NameMatch>,
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
    /// Detected team size (5 or 6) — portrait geometry depends on it.
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
}

/// What a Tab capture actually did, reported back to the session state machine.
struct CaptureReport {
    /// A pending board's row with the best guess in its unread columns
    /// ([`apply_best_guess`]). `None` for every other report.
    best_guess: Option<storage::PersonalMatch>,
    /// A snapshot row was written (and the session row created if this was the
    /// session's first capture). False = rejected by a trust gate.
    recorded: bool,
    /// Outcome stored on the snapshot — may have been recovered from the frame
    /// itself (banner colors / end-title text) when the game's outcome was still
    /// Unknown, in which case the caller back-fills it onto the session.
    outcome: detect::MatchOutcome,
    /// Map stored on the snapshot, if one was read — the caller adopts the
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
    /// capture — a new game was detected and written to a fresh session; the
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
}

/// Volatile session-tracking state owned by [`run_loop`]. Bundles the mutable
/// locals that the `select!` arms read and write so they travel as one value
/// instead of a fistful of parallel `let mut`s (QUAL-004). Purely
/// organizational — every field keeps the exact meaning it had as a standalone
/// local; no behavior depends on the grouping.
struct SessionState {
    /// Accepted-capture counter driving the periodic sync cadence
    /// (`SYNC_EVERY_N_CAPTURES`).
    capture_count: u32,
    /// When the current game was last (re)opened — debounces poller-driven
    /// new-game detection against `new_game_debounce`.
    last_game_open: Option<Instant>,
    /// Last accepted Tab capture — powers the Tab debounce.
    last_tab_capture: Option<Instant>,
    /// The game currently in progress — opened at the map-vote / hero-select
    /// screen, reused for every capture until the next game starts. Recovered
    /// from the previous run when the daemon restarted mid-game (crash, upgrade,
    /// systemd restart), so the restart neither splits the game into two
    /// sessions nor loses its outcome/map context.
    active_game: Option<ActiveGame>,
    /// Outcome detected by the poller while no game was open — applied to the
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
}

/// Unexpired POTG / end-reel deadline. Exclusive at the instant (`now < until`)
/// so expiry drops back to mid-match skip / slow cadence on the same tick.
fn end_reel_wake_active(st: &SessionState, now: Instant) -> bool {
    st.end_reel_wake_until.is_some_and(|until| now < until)
}

/// Fresh word-OCR streak or an unexpired end-reel wake — both force full
/// cadence. One helper so the two wake sources cannot drift.
fn cadence_wake_active(st: &SessionState, now: Instant) -> bool {
    st.word_outcome_streak
        .as_ref()
        .is_some_and(|streak| now.duration_since(streak.seen_at) <= OUTCOME_CONFIRM_WINDOW)
        || end_reel_wake_active(st, now)
}

/// Mid-match, skip the auto-detect poll while Tab scoreboard OCR saturates
/// the pool. During an active `end_reel_wake_until` the short Victory/Defeat
/// banner (~3s) can land in that same window — skipping starves the only
/// outcome path (2026-09-06: Tab held through end-reel, no poll
/// `result word: VICTORY`, no `poll_confirm_victory_*` dump). Outcome-signal
/// OCR is small-crop, not the Tab cell pool; see
/// [`poll_outcome_only_while_tab_busy`].
fn skip_poll_for_tab_in_flight(tab_in_flight: bool, st: &SessionState, now: Instant) -> bool {
    tab_in_flight && !end_reel_wake_active(st, now)
}

/// Cheap outcome-only poll: Tab OCR is in flight **and** end-reel wake is
/// still hot. Screenshot + `detect_outcome_signal_*` + `detect_end_reel` +
/// existing confirm/streak / `poll_debug_hit` dumps. Phase and accolade-map
/// OCR stay off so we do not compete with the Tab cell pool. Mid-match
/// (no wake) never takes this path — those ticks still skip.
fn poll_outcome_only_while_tab_busy(tab_in_flight: bool, st: &SessionState, now: Instant) -> bool {
    tab_in_flight && end_reel_wake_active(st, now)
}

fn clear_cadence_wakes(st: &mut SessionState) {
    st.word_outcome_streak = None;
    st.end_reel_wake_until = None;
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
    }
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
    let carried_deferred = st.active_game.as_ref().and_then(|g| {
        g.deferred
            .map(|counters| (counters, g.deferred_hero.clone(), g.deferred_at))
    });
    retire_active_game(st, store, data_dir, None, "superseded by Tab-opened game").await;
    let inherited = take_fresh_pending(&mut st.pending_outcome, now);
    let mut opened = ActiveGame::open_at(
        format!("{:016x}", rand_id()),
        inherited.unwrap_or(detect::MatchOutcome::Unknown),
        Vec::new(),
        now,
    );
    if let Some((counters, hero, at)) = carried_deferred {
        opened.deferred = Some(counters);
        opened.deferred_hero = hero;
        opened.deferred_at = at;
        opened.deferred_imported = true;
    }
    st.active_game = Some(opened);
    st.last_game_open = Some(now);
    clear_cadence_wakes(st);
    persist_active_game(data_dir, st.active_game.as_ref());
    true
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
                }
                persist_active_game(data_dir, Some(g));
            }
            recorded
        }
    }
}

/// PR-B: whether the poller sits mid-match with nothing imminent — a mature
/// open game, outcome still unknown, and no cadence wake (fresh word-OCR
/// streak or POTG / end-reel deadline). Every end-of-match signal path
/// already drops this back to full cadence through existing state: a banner
/// records the outcome (`finished()`), a word read sets `word_outcome_streak`,
/// an end-reel / POTG hit sets `end_reel_wake_until`, and a detected start
/// phase opens a new game (resetting `last_game_open`).
fn poll_slow_mode(st: &SessionState, now: Instant) -> bool {
    let hint_open = st.active_game.as_ref().is_some_and(|g| {
        g.pending_boundary
            || g.result_mark
                .is_some_and(|mark| !mark.confirmed && mark.outcome.is_decided())
    });
    st.active_game.as_ref().is_some_and(|g| !g.finished())
        && st
            .last_game_open
            .is_none_or(|t| now.duration_since(t) >= SLOW_AFTER_GAME_OPEN)
        && !cadence_wake_active(st, now)
        && !hint_open
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
        Some((outcome, detect::match_end::OutcomeSource::Banner)) => ResolvedWordTick {
            signal: Some(outcome),
            signal_confirmed: true,
            accolade_map: None,
        },
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

/// The words-pack result read one poll tick adds to the open game's result
/// evidence (`reader = "new"`, #253). Only a tick with an outcome signal
/// adds one, as before.
///
/// `taken` is the read the tick's signal detection already made of this
/// frame's result words (the accolade or rank screen, alpha.7 O11). It is
/// `shadow::read_result`'s read of the same frame, so it is the evidence
/// and the frame is not read again. Without one (a colour flood, an end
/// title, or a word OCR read because the pack lacks that screen) `read`
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
        // No game open yet — applies to the next session if one opens
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
    // The GUI's Stop button (and systemd) send SIGTERM — shut down as
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
        "end-screen poll interval"
    );
    let new_game_debounce = std::time::Duration::from_secs(auto_detect.cooldown_secs);
    let tab_debounce = std::time::Duration::from_secs(3);
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
    // not consult that clock — it still joins the in-flight task, then uploads
    // once. `sync_rev` compare-and-swap stays inside `try_sync_with`.
    let mut sync_task: Option<tokio::task::JoinHandle<()>> = None;
    let sync_backoff = Arc::new(std::sync::Mutex::new(sync::SyncBackoff::default()));
    if sync_credentials_rejected(data_dir, sync_client) {
        sync_backoff
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .record_auth_rejected();
        tracing::error!(
            "sync paused: token rejected. Update the token in Settings; sync resumes when the URL or token changes."
        );
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
    // measured 45-70s — long enough to miss the ~3s VICTORY/DEFEAT banner and
    // the whole accolade screen, i.e. the outcome. The 400ms "let the game
    // render the scoreboard" wait sleeps inside the task too.
    let (capture_tx, mut capture_rx) =
        tokio::sync::mpsc::unbounded_channel::<(String, Result<CaptureReport, String>)>();
    let mut capture_task: Option<tokio::task::JoinHandle<()>> = None;

    let mut poll_timer = tokio::time::interval(poll_interval);
    poll_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // GUI command queue (manual outcome edits, session deletion) — checked on
    // its own timer so edits apply even while no game is running.
    let mut cmd_timer = tokio::time::interval(tokio::time::Duration::from_secs(3));
    cmd_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            result = kbd.wait_tab() => {
                match result {
                    Ok(()) => {
                        if !game_gate.is_running() {
                            tracing::debug!("Tab ignored: game process not running");
                            continue;
                        }

                        if let Some(last) = st.last_tab_capture
                            && last.elapsed() < tab_debounce {
                                tracing::debug!("Tab debounced: ignoring rapid press");
                                continue;
                            }

                        // Single-flight: OCR of the previous Tab may still be
                        // running (it takes seconds) — don't stack captures.
                        if capture_task.as_ref().is_some_and(|t| !t.is_finished()) {
                            tracing::debug!("Tab ignored: a capture is already in progress");
                            continue;
                        }
                        st.last_tab_capture = Some(Instant::now());

                        // Session choice: reuse the active game (mid-game, or
                        // post-match scoreboard within the grace window), or
                        // open a fresh one, inheriting a still-fresh outcome
                        // the poller saw before any game was open.
                        let now = Instant::now();
                        let opened_by_this_tab =
                            open_fresh_if_tab_starts_one(&mut st, store, data_dir, now).await;
                        let req = build_capture_request(
                            st.active_game.as_ref().expect("active_game set above"),
                            opened_by_this_tab,
                            now,
                        );
                        let sid = req.session_id.clone();
                        let tx = capture_tx.clone();
                        let ctx_task = Arc::clone(&ctx);
                        capture_task = Some(tokio::spawn(async move {
                            // Wait for the game to render the scoreboard
                            // overlay after the Tab press.
                            tokio::time::sleep(tokio::time::Duration::from_millis(400)).await;
                            let result = handle_capture(&ctx_task, req)
                                .await
                                // `{:#}` keeps anyhow's context chain in the string.
                                .map_err(|e| format!("{e:#}"));
                            let _ = tx.send((sid, result));
                        }));
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
            }
            _ = poll_timer.tick(), if auto_detect.enabled => {
                // Mid-match: a Tab capture saturates the OCR pool — skip the
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
                if !game_gate.is_running() {
                    clear_cadence_wakes(&mut st);
                    st.ocr_stability.reset();
                    continue;
                }

                // PR-B: adaptive cadence — mid-match, pay the screencopy only
                // on every SLOW_POLL_DIVISORth tick. The skipped ticks cost
                // nothing (no capture, no scans); full cadence resumes the
                // moment any end/start evidence lands in session state.
                if poll_slow_mode(&st, Instant::now()) {
                    st.poll_ticks_skipped += 1;
                    if st.poll_ticks_skipped < SLOW_POLL_DIVISOR {
                        tracing::trace!("poll tick skipped: adaptive slow cadence mid-match");
                        continue;
                    }
                }
                st.poll_ticks_skipped = 0;

                match capture::capture_screen_output(backend, capture_output).await {
                    Ok(img) => {
                        let dump_dir = dump_poll_frames.then(|| data_dir.join("debug").join("poll"));
                        // On-hit evidence (debug_ocr): confirm + first streak only.
                        // `--dump-poll-frames` still writes every tick as `poll_*`.
                        let on_hit_dir = debug_ocr.then(|| data_dir.join("debug").join("poll"));
                        let prior_streak = st
                            .word_outcome_streak
                            .as_ref()
                            .map(|streak| (streak.outcome, streak.seen_at.elapsed()));
                        let mut stability = std::mem::take(&mut st.ocr_stability);
                        let (signal, phase, accolade_map, end_reel, stability, result_read, rank_outcome_png) = tokio::task::spawn_blocking(move || {
                            let analyze_started = Instant::now();
                            if let Some(dir) = &dump_dir {
                                save_frame_ring(dir, "poll", &img, POLL_DUMP_KEEP);
                            }
                            // One RGBA→RGB conversion shared by banner + phase
                            // detectors (P6); title OCR still uses the original frame.
                            let rgb = img.to_rgb8();
                            // PR-A: the polled variants gate their OCR on the
                            // crop holding still across consecutive ticks —
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
                            // Wake hint only — does not confirm an outcome.
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
                        }).await.unwrap_or_else(|_| {
                            (None, detect::GamePhase::Unknown, None, false, detect::stability::FrameStability::default(), None, None)
                        });
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

                        // The banner color-flood is specific enough to act on
                        // immediately (and only lasts ~3s — a second tick may
                        // never come). A word-OCR outcome (accolade or rank
                        // screen) needs a second agreeing read within the
                        // confirmation window. The streak is recorded before
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
                    }
                    Err(e) => {
                        tracing::trace!(error = %e, "poll capture failed (game may not be running)");
                    }
                }
            }
            _ = cmd_timer.tick() => {
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
                // restart — drop the volatile windows and re-admit the active
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
            }
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("shutting down");
                shutdown_from_signal(
                    &mut st,
                    &mut capture_task,
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
                tracing::info!("SIGTERM received, shutting down");
                shutdown_from_signal(
                    &mut st,
                    &mut capture_task,
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
    }
}

/// Switches for one vision pass, by name, so two of them cannot be swapped at
/// the call site (F251-4).
#[derive(Debug, Clone, Copy)]
struct FrameFlags {
    /// Colour-flood outcome detection is plausible for this session's age.
    allow_banner_recovery: bool,
    /// The open session already has a map (the lazy full-board OCR may skip).
    session_map_known: bool,
    /// The shadow log worker is on.
    shadow_on: bool,
    /// `reader = "new"`: the banner read may supply the top-bar map.
    use_new_reader: bool,
}

/// The blocking vision/OCR pass over one Tab frame, extracted from
/// `handle_capture` so the async orchestration reads as a straight line
/// (QUAL-001). Pure function of its inputs: decides the outcome, runs the
/// pre-OCR preflight, and (when the frame is a scoreboard) the portrait match,
/// per-cell OCR, career/map reads, and lazy full-board OCR. Returns everything
/// the caller needs, or a cheap rejection when the frame has no scoreboard rows.
fn analyze_frame(
    img: image::DynamicImage,
    matcher: Arc<detect::hero_portrait::PortraitMatcher>,
    player_name_owned: Option<String>,
    game_outcome: detect::MatchOutcome,
    flags: FrameFlags,
    own_seeds: Vec<detect::own_row::Seed>,
) -> FrameAnalysisOutcome {
    let FrameFlags {
        allow_banner_recovery,
        session_map_known,
        shadow_on,
        use_new_reader,
    } = flags;
    // Outcome: prefer the open game's result (read off the accolade
    // screen by the poller); else color-flood detection plus the word reads
    // (only when the caller deems a banner plausible — see
    // MIN_BANNER_SESSION_AGE), which end in the end-title read; else the
    // end-title read alone. T1, an OCR of the top-centre band for a result
    // header, is gone (alpha.7 O4; what that was measured on is in
    // `detect_outcome_text`). The full-board OCR's header line (ET-2, in
    // `handle_capture`) still runs under ocr-v1; the new reader never runs
    // the full-board OCR (O10). The end title is read once (F257-3).
    // `reader = "new"` takes the end title from the words-v1 pack, not OCR
    // (O2). With no end-title templates loaded it falls back to the OCR
    // read, and only that fallback still reads the FINAL SCORE line (its
    // gate), with the words-v1 template (O3).
    let p4 = detect::match_end::FinalScoreSource::for_reader(use_new_reader);
    let titles = detect::match_end::Titles::for_frame(use_new_reader, &img);
    // And the accolade and rank-screen words, when this frame is one of
    // those screens (O11). A Tab frame's read is not result evidence.
    let words = detect::match_end::ResultWords::for_reader(use_new_reader);
    let frame_outcome = if allow_banner_recovery {
        detect::match_end::detect_outcome_with(&img, p4, &titles, &words)
    } else {
        detect::match_end::detect_outcome_text_with(&img, p4, &titles)
    };
    // The open session's outcome wins on a capture that stays in that session.
    // A gap split keeps this frame's own outcome. A reset split stores Unknown.
    let outcome = if game_outcome.is_decided() {
        game_outcome
    } else {
        frame_outcome
    };

    let scoreboard = Arc::new(ocr::preprocess::crop_scoreboard(&img));
    // Pre-OCR preflight. Row dips or header stat labels accept a board, then
    // the pitch has to settle 5 or 6. The table check (layout and brightness,
    // no team colour) does not block that. It runs only when the pitch fails
    // or the two pitches disagree: no table is "not a scoreboard", a table is
    // "team size unsure". The OCR gate downstream stays the final check for
    // frames that pass.
    let (preflight, row_scan) = detect::hero_portrait::preflight_scoreboard(&scoreboard);
    let team_size = match preflight {
        detect::hero_portrait::ScoreboardPreflight::NotAScoreboard => {
            return FrameAnalysisOutcome::NotAScoreboard {
                outcome,
                frame: img,
                scan: row_scan,
            };
        }
        detect::hero_portrait::ScoreboardPreflight::TeamSizeUncertain => {
            return FrameAnalysisOutcome::TeamSizeUncertain {
                outcome,
                frame: img,
                scan: row_scan,
            };
        }
        detect::hero_portrait::ScoreboardPreflight::Ready(team_size) => team_size,
    };
    // Pass team_size into portrait match + cell OCR so neither re-detects
    // size or re-crops the full scoreboard (P7).
    let player_match = matcher.match_player_hero_with_team_size(&scoreboard, team_size);
    let portrait_match = player_match
        .as_ref()
        .map(|(name, conf, _)| (name.clone(), *conf));
    // The highlighted row stands on its own. A portrait below the match
    // threshold (2026-10-11: the ult-charge ring over Wrecking Ball's portrait
    // dropped it to 0.77) used to return no row at all, so every later Tab of
    // that game was rejected as "player row not found" and the slow full-board
    // OCR ran each time. The hero still comes from the career panel.
    let brightness_row_idx = detect::hero_portrait::resolve_player_row(
        player_match.map(|(_, _, idx)| idx),
        &scoreboard,
        team_size,
    );

    // ocr-v1 calibrates the stat columns and reads every name and stat cell
    // (OCR sites T2, T3, T4). The new reader reads none of them here: its
    // stats are the digit reader's (alpha.7 O10) and its own row comes from
    // the member's own-name seeds, the highlight and the career hero's row
    // (alpha.7 O9, O9b), so the rows are the board's rows with nothing read
    // on them.
    let rows = read_tab_rows(
        use_new_reader,
        || ocr::recognize_scoreboard_cells_pre_cropped(&scoreboard, team_size),
        || ocr::unread_rows(team_size),
    );

    // Player row. ocr-v1: if a player name is configured, scan ALL rows (both
    // teams) for a name match — this handles replays and post-match screens
    // where the player may be on team 2. Fall back to brightness-detected row
    // otherwise. `reader = "new"`: the seed match combined with the
    // highlight, and where both are silent the row the career panel's hero
    // is read on (alpha.7 O9b). Only when none of them names a row are this
    // board's name windows read, to match the configured name as main does.
    let highlight = brightness_row_idx.map(|row| detect::own_row::Highlight {
        row,
        lead: detect::hero_portrait::player_row_highlight(&scoreboard, team_size)
            .filter(|(lit, _)| *lit == row)
            .map_or(0.0, |(_, lead)| lead),
    });
    // The Tab's hero icons (heroes-v2), read once, before the own row: the
    // career hero's row is one of the own row's voices, and the same read
    // is the hero authority below. Never read under ocr-v1.
    let tab_heroes = read_tab_heroes(use_new_reader, || {
        shadow::read_heroes(&img, &scoreboard, team_size)
    });
    let OwnRowChoice {
        row_idx,
        name_match,
        own_row,
        candidate,
        verdict: own_seed_verdict,
    } = find_own_row(
        use_new_reader,
        || {
            player_name_owned
                .as_deref()
                .and_then(|name| parse::match_player_row_by_name(&rows, name))
        },
        || {
            detect::own_row::read_own_row(
                Some(&img),
                &scoreboard,
                team_size,
                highlight,
                tab_heroes.as_ref(),
                &own_seeds,
            )
        },
        || own_row_name_fallback(player_name_owned.as_deref(), &scoreboard, team_size),
        highlight,
        tab_heroes
            .as_ref()
            .and_then(detect::own_row::career_hero_row),
        !own_seeds.is_empty(),
    );

    // Hero authority (OCR site T5). ocr-v1 reads the career-panel hero
    // title. Guard against garbage OCR (happens when there is no career
    // panel — replay, post-match — by requiring the result to actually
    // match a known hero name, which match_hero_in_text already does). The
    // new reader never reads it: the heroes-v2 reads of the career panel's
    // portrait and of the own row take its place (alpha.7 O8).
    let OwnHeroRead {
        career_hero,
        pack_hero,
        tab_heroes,
    } = read_own_hero(
        use_new_reader,
        || {
            ocr::recognize_region(&ocr::preprocess::crop_career_hero(&img))
                .ok()
                .and_then(|t| parse::match_hero_in_text(&t))
        },
        tab_heroes,
        row_idx,
    );
    // A crop of the own row's name line (the highlighted row, or the career
    // hero's) may seed the own-name set only from a live board of the
    // member's own: no chat over the row, and the career panel showing the
    // hero on that row.
    let own_name_candidate = candidate.filter(|c| {
        let chat = match img.as_rgb8() {
            Some(rgb) => stat_tracker::report::blank::chat_over_board(rgb),
            None => stat_tracker::report::blank::chat_over_board(&img.to_rgb8()),
        };
        detect::own_row::seed_gate(
            c.row,
            c.row_bottom,
            scoreboard.height(),
            chat,
            tab_heroes.as_ref(),
        )
    });
    // The top-bar OCR label is the banner read's fallback (O6). With OCR
    // off there is no fallback: the banner read is the top-bar map or
    // nothing is, and the label is not asked for.
    let map_text = if ocr::gate::is_off() {
        String::new()
    } else {
        ocr::recognize_map_label(&img, preflight)
    };
    let map_panel_raw = if map_text.trim().is_empty() {
        None
    } else {
        Some(map_text)
    };
    let top_bar = choose_top_bar_map(
        use_new_reader,
        || shadow::read_banner(&img),
        map_panel_raw.as_deref().and_then(parse::match_map_in_text),
    );
    let map_from_panel = top_bar.map;
    let map_panel_raw = top_bar_raw_label(top_bar.banner.as_ref(), map_panel_raw);

    // Full-board OCR exists only to supply raw text for hero/map name
    // lookup and the name-in-raw-text stats fallback. On the happy path —
    // player row found and parseable, hero identified (career panel,
    // heroes-v2 icons or portrait match), map already known — it is pure
    // redundancy (adaptive preprocessing plus up to three threshold sweeps), so run
    // it lazily. A portrait match alone satisfies the hero requirement:
    // replay/post-match layouts have no career panel, and the raw-text
    // hero guess loses to the portrait in the priority order anyway (H6).
    //
    // `reader = "new"` never runs it (OCR site T7): its stats, hero and map
    // come from the digit, icon and banner readers, and a Tab those cannot
    // read is refused or held, not rebuilt from text.
    let hero_identified = career_hero.is_some() || pack_hero.is_some() || portrait_match.is_some();
    let ocr = full_board_text(
        use_new_reader,
        || {
            let cells_parse =
                parse::parse_scoreboard_cells(&rows, row_idx, "", "unknown", None).is_some();
            !cells_parse || !hero_identified || (!session_map_known && map_from_panel.is_none())
        },
        || ocr::recognize(&img),
    );
    // The board the trust gate and the stats come from under the new reader,
    // with the banner and the hero icons this pass already read, so the
    // frame is read once per Tab (F251-3, O8).
    let board = read_tab_board(use_new_reader, || {
        shadow::read_board_with(&img, top_bar.banner, tab_heroes)
    });

    FrameAnalysisOutcome::Analyzed(Box::new(FrameAnalysis {
        frame_outcome,
        ocr,
        rows,
        board,
        portrait_hero: portrait_match,
        career_hero,
        pack_hero,
        map_from_panel,
        map_panel_raw,
        map_from_banner: top_bar.from_banner,
        shadow_input: shadow::worker::ShadowInput::when(
            shadow_on,
            &scoreboard,
            team_size,
            img.height(),
        ),
        scoreboard,
        player_row_idx: row_idx,
        name_match,
        own_row,
        own_name_candidate: own_name_candidate.map(|c| c.candidate),
        own_seed_verdict,
        team_size,
        frame: img,
    }))
}

/// The row reads of one Tab with the daemon's reader switch (OCR sites T2,
/// T3 and T4).
///
/// * `reader = "ocr-v1"`: `cells` runs, exactly as before: the stat columns
///   are calibrated (T2) and every name and stat cell is read (T4, T3).
/// * `reader = "new"`: `cells` never runs, so there is no calibration, no
///   stat cell OCR and no name OCR. `unread` supplies the board's rows with
///   nothing read on them (`ocr::unread_rows`). The one name read left
///   under this reader is [`own_row_name_fallback`], for a board on which
///   neither a seed nor the highlight names a row.
fn read_tab_rows(
    use_new_reader: bool,
    cells: impl FnOnce() -> Vec<ocr::RowOcrResult>,
    unread: impl FnOnce() -> Vec<ocr::RowOcrResult>,
) -> Vec<ocr::RowOcrResult> {
    if use_new_reader { unread() } else { cells() }
}

/// The full-board OCR text of one Tab with the daemon's reader switch (OCR
/// site T7).
///
/// * `reader = "ocr-v1"`: `ocr` runs when `needed` says the cell path did
///   not supply everything, exactly as before.
/// * `reader = "new"`: neither closure runs. The text is empty, which every
///   reader of it already takes as "no text": no stats, no hero, no map, no
///   result header. An OCR failure can no longer fail the Tab.
fn full_board_text(
    use_new_reader: bool,
    needed: impl FnOnce() -> bool,
    ocr: impl FnOnce() -> Result<ocr::OcrResult, Box<dyn std::error::Error + Send + Sync>>,
) -> Result<ocr::OcrResult, Box<dyn std::error::Error + Send + Sync>> {
    if !use_new_reader && needed() {
        return ocr();
    }
    if !use_new_reader {
        tracing::debug!("skipping full-board OCR: cell path supplied everything");
    }
    Ok(ocr::OcrResult {
        raw_text: String::new(),
        confidence: 0,
    })
}

/// The new reader's board of one Tab. `None` under `reader = "ocr-v1"`,
/// where `read` never runs. A panic in the reader is "no board": the Tab is
/// refused, it does not fail and nothing else reads it.
fn read_tab_board(
    use_new_reader: bool,
    read: impl FnOnce() -> shadow::BoardRead,
) -> Option<shadow::BoardRead> {
    if !use_new_reader {
        return None;
    }
    Some(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(read)).unwrap_or_else(|_| {
            tracing::warn!("board reader panicked; no board for this Tab");
            shadow::reader::not_found(shadow::BoardStatus::NotFound)
        }),
    )
}

/// The own row of one Tab (OCR site T4).
struct OwnRowChoice {
    row_idx: Option<usize>,
    /// The name match behind `row_idx`: ocr-v1's, or the new reader's name
    /// fallback. `None` when the row came from seeds or the highlight.
    name_match: Option<parse::NameMatch>,
    /// New reader: how the row was found. `None` under ocr-v1.
    own_row: Option<detect::own_row::OwnRow>,
    /// New reader: the highlighted row's name line, before the seeding gate.
    candidate: Option<detect::own_row::RowCandidate>,
    /// New reader: what the board says about the trusted seeds.
    verdict: detect::own_row::Verdict,
}

/// The new reader's name fallback (OCR site T4, kept for one case): when
/// neither an own-name seed nor the highlight names a row, read this
/// board's name windows with Tesseract and match the configured name, as
/// main does for every board. This is the only name read under
/// `reader = "new"`.
///
/// With classic OCR off there is no such read: it returns `None` before
/// anything of OCR is touched (no crop, no OCR pool, no call at the gate).
/// A board with no seed match, no highlight and no row for the career
/// panel's hero is then held, with a WARN. That is the limit of
/// [`stat_tracker::ocr_switch::Read::OwnRow`].
fn own_row_name_fallback(
    player_name: Option<&str>,
    scoreboard: &image::DynamicImage,
    team_size: usize,
) -> Option<parse::NameMatch> {
    if ocr::gate::is_off() {
        return None;
    }
    let name = player_name.filter(|name| !name.trim().is_empty())?;
    let rows = ocr::recognize_scoreboard_names_pre_cropped(scoreboard, team_size);
    name_on_a_full_board(&rows, name, team_size)
}

/// The configured name's row among the name fallback's rows, when those are
/// the whole board.
///
/// A defensive check, not a known hazard. The index found here picks the
/// row on the digit reader's board (alpha.7 O10), so the names read has to
/// cover the same rows, in the same order. It does today:
/// `ocr::recognize_scoreboard_names_pre_cropped` leaves out a row only when
/// its crop falls outside the board, the preflight only settles on 5 or 6
/// rows a team, and for both the last row still fits (R2-6). If that ever
/// changes, a short set names no row and the capture is held, instead of an
/// index that may no longer mean the same row (R262-E).
fn name_on_a_full_board(
    rows: &[ocr::RowOcrResult],
    name: &str,
    team_size: usize,
) -> Option<parse::NameMatch> {
    if rows.len() != team_size * 2 {
        return None;
    }
    parse::match_player_row_by_name(rows, name)
}

/// Find the member's own row with the daemon's reader switch.
///
/// * `reader = "ocr-v1"`: `name_match` (the name OCR over every row) runs,
///   and the row is that match, else the highlighted row, exactly as before.
///   The seeds are not read.
/// * `reader = "new"`: `seed_read` (the own-name seeds matched on every row,
///   combined with the highlight in [`detect::own_row::combine`]) gives the
///   row. A seed against a strong highlight is no row: the capture is held.
///   Where seed and highlight are both silent, the row the career panel's
///   hero is read on is the row, marked suspect (O9b; no name is read for
///   it, so it works with Tesseract off).
///   When none of them names a row, `name_fallback` is asked: the one name
///   read left under the new reader, main's own (the name windows OCRed and
///   matched against the configured name), for this board only. Its row is
///   stored as main stores a name-matched row, and marked suspect when a
///   seed for this board exists and matched no row (F262-18). A lane that
///   turns Tesseract off passes a fallback that returns `None`; such a
///   board is then held.
///   A panic in the seed read does not fail the Tab: the highlight is used,
///   marked suspect when seeds exist (`seeded`), else the career hero's row
///   (`hero_row`).
fn find_own_row(
    use_new_reader: bool,
    name_match: impl FnOnce() -> Option<parse::NameMatch>,
    seed_read: impl FnOnce() -> detect::own_row::OwnRowRead,
    name_fallback: impl FnOnce() -> Option<parse::NameMatch>,
    highlight: Option<detect::own_row::Highlight>,
    hero_row: Option<usize>,
    seeded: bool,
) -> OwnRowChoice {
    if !use_new_reader {
        let name_match = name_match();
        return OwnRowChoice {
            row_idx: name_match.map(|m| m.row).or(highlight.map(|h| h.row)),
            name_match,
            own_row: None,
            candidate: None,
            verdict: detect::own_row::Verdict::Silent,
        };
    }
    let read = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(seed_read)) {
        Ok(read) => read,
        Err(_) => {
            tracing::warn!("own-row seed read panicked; the highlight row is used");
            detect::own_row::OwnRowRead::unread(highlight, hero_row, seeded)
        }
    };
    let mut own = read.own;
    let mut name_match = None;
    if own.source == detect::own_row::Source::Neither
        && let Some(found) = name_fallback()
    {
        own = detect::own_row::OwnRow::from_name(found.row, read.seeded);
        name_match = Some(found);
    }
    tracing::info!(
        row = ?own.row,
        source = own.source.label(),
        trust = ?own.trust,
        best = read.best,
        next = read.next,
        hero_row = ?read.hero_row,
        "own row from seeds, highlight and career hero"
    );
    OwnRowChoice {
        row_idx: own.row,
        name_match,
        own_row: Some(own),
        candidate: read.candidate,
        verdict: read.verdict,
    }
}

/// The hero authority inputs of one Tab (OCR site T5).
#[derive(Debug, Clone, PartialEq)]
struct OwnHeroRead {
    /// ocr-v1: the career-panel title. `None` under the new reader.
    career_hero: Option<String>,
    /// New reader: the hero the heroes-v2 icon reads authenticate
    /// ([`shadow::TabHeroes::authority`]). `None` under ocr-v1, and on any
    /// abstain.
    pack_hero: Option<String>,
    /// New reader: the whole icon read, for the board read. `None` under
    /// ocr-v1, and after a caught panic.
    tab_heroes: Option<shadow::TabHeroes>,
}

/// The hero icons of one Tab (the heroes-v2 read of the whole board and of
/// the career panel's portrait) with the daemon's reader switch.
///
/// * `reader = "ocr-v1"`: `heroes` is never called.
/// * `reader = "new"`: `heroes` runs once per Tab. Its read names the own
///   row where seed and highlight are silent
///   ([`detect::own_row::career_hero_row`]) and is the hero authority
///   ([`read_own_hero`]). A panic in the icon reader is `None`: no hero
///   authority and no career-hero row, and never a failed Tab, the same
///   isolation [`choose_top_bar_map`] gives the banner.
fn read_tab_heroes(
    use_new_reader: bool,
    heroes: impl FnOnce() -> shadow::TabHeroes,
) -> Option<shadow::TabHeroes> {
    if !use_new_reader {
        return None;
    }
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(heroes)) {
        Ok(read) => Some(read),
        Err(_) => {
            tracing::warn!("hero icon reader panicked; no hero authority for this Tab");
            None
        }
    }
}

/// Read the hero authority of one Tab with the daemon's reader switch.
///
/// * `reader = "ocr-v1"`: `career` (the career-panel OCR) runs, exactly as
///   before. The icons are not read.
/// * `reader = "new"`: the career-panel title is never read. `tab_heroes`
///   is this Tab's icon read ([`read_tab_heroes`]), and `pack_hero` is the
///   hero those reads authenticate ([`shadow::TabHeroes::authority`]).
///   Suspect or special reads, two reads naming different heroes, or no icon
///   read (a panic in the icon reader) are an abstain: no hero, and never an
///   OCR read in its place.
fn read_own_hero(
    use_new_reader: bool,
    career: impl FnOnce() -> Option<String>,
    tab_heroes: Option<shadow::TabHeroes>,
    own_row: Option<usize>,
) -> OwnHeroRead {
    if !use_new_reader {
        return OwnHeroRead {
            career_hero: career(),
            pack_hero: None,
            tab_heroes: None,
        };
    }
    let pack_hero = tab_heroes
        .as_ref()
        .and_then(|read| read.authority(own_row))
        .map(str::to_string);
    OwnHeroRead {
        career_hero: None,
        pack_hero,
        tab_heroes,
    }
}

/// The name the raw-text stats fallback (OCR site T7) may search for.
///
/// ocr-v1: the configured name, as before. `reader = "new"`: the configured
/// name only when the own row was found. With no own row (seed and highlight
/// disagree, or neither fired) there is no name, so the full-board text
/// cannot name a row in its place and the capture is held.
fn text_fallback_name(
    use_new_reader: bool,
    player_row_idx: Option<usize>,
    player_name: Option<&str>,
) -> Option<&str> {
    if use_new_reader && player_row_idx.is_none() {
        return None;
    }
    player_name
}

/// Row id passed into [`boundary::plan_capture`]. `None` when the player
/// row was not identified.
fn identified_row_id(player_row_idx: Option<usize>) -> Option<u32> {
    player_row_idx.map(|idx| idx as u32)
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

/// The next `store_held_board` for this session id fails once, then clears.
/// Other sessions are left alone so parallel tests do not trip it.
#[cfg(test)]
static FAIL_HELD_BOARD_SESSION: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Write the board that was held off the finished game onto the new session
/// at that capture's own timestamp, and record it the same way as a Tab.
#[allow(clippy::too_many_arguments)]
async fn store_held_board(
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    session_id: &str,
    hero: &str,
    map_name: &str,
    outcome: &str,
    counters: Counters,
    played_at: chrono::DateTime<Utc>,
) -> anyhow::Result<()> {
    #[cfg(test)]
    {
        let mut fail = FAIL_HELD_BOARD_SESSION
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if fail.as_deref() == Some(session_id) {
            *fail = None;
            anyhow::bail!("held board write failed");
        }
    }
    let played_at = SurrealDatetime::from(played_at);
    let hero = parse::canonical_hero(hero);
    let row = storage::PersonalMatch {
        id: None,
        hero: hero.clone(),
        map_name: map_name.to_string(),
        game_mode: parse::stored_game_mode(map_name),
        role: parse::guess_role_public(&hero),
        outcome: outcome.to_string(),
        elims: counters.elims,
        deaths: counters.deaths,
        assists: counters.assists,
        damage: counters.damage,
        healing: counters.healing,
        mitigation: counters.mitigation,
        played_at,
        synced: false,
        sync_rev: 0,
        upload_reject: None,
        session_id: session_id.to_string(),
        corrected_hero: None,
        corrected_role: None,
        corrected_map_name: None,
        corrected_outcome: None,
        corrected_elims: None,
        corrected_deaths: None,
        corrected_assists: None,
        corrected_damage: None,
        corrected_healing: None,
        corrected_mitigation: None,
        edited_fields: Vec::new(),
        edited_at: None,
        heroes_played: Vec::new(),
        segment_resolutions: Vec::new(),
        recognizer: scuffed_types::RECOGNIZER_OCR_V1.to_string(),
        suspect_fields: Vec::new(),
    };
    storage::append_match_log(data_dir, &row);
    store
        .insert_match(row)
        .await
        .map_err(anyhow::Error::from_boxed)
        .context("deferred board insert failed")?;
    if let Err(e) = store.append_capture(session_id, played_at, outcome).await {
        tracing::warn!(error = %e, "failed to append the deferred board to the session");
    }
    if let Err(e) = store.refresh_session_hero_timeline(session_id).await {
        tracing::debug!(error = %e, "failed to refresh hero timeline for the deferred board");
    }
    Ok(())
}

/// The hold-path report. Production and the night harness both build it
/// here, so the stored outcome and the career-panel flag cannot drift.
fn skipped_capture_report(
    plan: &boundary::CapturePlan,
    session_id: &str,
    career_panel: bool,
    hero_auth: HeroAuthState,
    held_hero: Option<String>,
    held_at: Option<chrono::DateTime<Utc>>,
) -> CaptureReport {
    CaptureReport {
        best_guess: None,
        recorded: false,
        outcome: plan.stored_outcome,
        map: None,
        map_source: None,
        session_id: session_id.to_string(),
        split: false,
        armed_reset: plan.defer,
        ignore_row: plan.ignore_row,
        reset_streak: plan.reset_streak,
        reset_baseline: plan.reset_baseline,
        baseline_row: plan.baseline_row,
        refresh_baseline: false,
        clear_hint: false,
        count_progress: false,
        career_panel,
        held_counters: plan.deferred_counters,
        held_hero,
        gate_state: None,
        hero_auth,
        seal: None,
        close_reason: None,
        held_at,
        map_replaces_text_fallback: false,
    }
}

/// The carried board is written only after the current row insert succeeds.
/// A failed current insert must leave the hold for the next stored Tab.
fn should_write_carried(current_inserted: bool, has_carried: bool) -> bool {
    current_inserted && has_carried
}

/// Session row for one staged capture. The carried board is not written here.
/// Production and the night harness both call [`write_carried_after_current`]
/// only after the current snapshot insert returns ok.
async fn write_staged_rows(
    store: &storage::LocalStore,
    staged: &StagedCapture,
    hero: &str,
    role: &str,
    now: SurrealDatetime,
) -> anyhow::Result<bool> {
    let mut create = staged.create_session;
    if staged.carried.is_some() && create {
        let played_at = staged.carried_at;
        let carried_hero = staged
            .carried_hero
            .clone()
            .unwrap_or_else(|| hero.to_string());
        let session = storage::MatchSession {
            session_id: staged.target_session.clone(),
            hero: carried_hero,
            map_name: staged.map_name.clone(),
            role: role.to_string(),
            started_at: SurrealDatetime::from(played_at),
            last_capture_at: SurrealDatetime::from(played_at),
            capture_count: 0,
            final_outcome: staged.outcome_label.clone(),
        };
        store
            .create_session(&session)
            .await
            .map_err(anyhow::Error::from_boxed)
            .context("session create failed")?;
        create = false;
    }
    if create {
        let session = storage::MatchSession {
            session_id: staged.target_session.clone(),
            hero: hero.to_string(),
            map_name: staged.map_name.clone(),
            role: role.to_string(),
            started_at: now,
            last_capture_at: now,
            capture_count: 1,
            final_outcome: staged.outcome_label.clone(),
        };
        // A failed create must abort the capture: pressing on would write
        // a snapshot the caller then marks "session created", and every
        // later outcome/map back-fill would update a session row that
        // doesn't exist. The Tab can simply be pressed again.
        store
            .create_session(&session)
            .await
            .map_err(anyhow::Error::from_boxed)
            .context("session create failed")?;
        tracing::info!(session_id = %staged.target_session, "started new match session");
    } else if let Err(e) = store
        .append_capture(&staged.target_session, now, &staged.outcome_label)
        .await
    {
        tracing::warn!(error = %e, "failed to append capture to session");
    }
    Ok(create)
}

/// Write the held board after the current snapshot insert has succeeded.
async fn write_carried_after_current(
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    staged: &StagedCapture,
    hero: &str,
    current_inserted: bool,
) -> anyhow::Result<()> {
    if !should_write_carried(current_inserted, staged.carried.is_some()) {
        return Ok(());
    }
    let Some(counters) = staged.carried else {
        return Ok(());
    };
    let carried_hero = staged
        .carried_hero
        .clone()
        .unwrap_or_else(|| hero.to_string());
    store_held_board(
        store,
        data_dir,
        &staged.target_session,
        &carried_hero,
        &staged.map_name,
        &staged.outcome_label,
        counters,
        staged.carried_at,
    )
    .await
}

/// Whether the staged row is stored or dropped before insert.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreparedRow {
    Store,
    Drop,
}

/// Map, mode, and the PvE gate for one staged capture.
///
/// An untrusted Château Guillard read is dropped back to the session map
/// so a fuzzy board cannot rename an open game. A stat split still carries
/// the previous game on the request, so that fallback is empty and the new
/// session stays Unknown. A trusted Deathmatch map stays, and the caller
/// stores that row locally. Adlersbrunn with stats on only one team is not
/// stored as Eichenwalde.
///
/// An untrusted read of a local-only map with no open map is dropped,
/// except when this capture is the stat-reset split. That split stores
/// the new session with an empty map. The review hold keeps it unsynced
/// until a pick, and it is not uploaded as the game that just closed.
fn prepare_capture_row(
    parsed: &mut storage::PersonalMatch,
    staged: &mut StagedCapture,
    session_map: Option<&str>,
    panel_raw: Option<&str>,
    board_text: &str,
    both_teams: bool,
) -> PreparedRow {
    if parse::map_is_untracked(&staged.map_name)
        && !staged
            .map_source
            .is_some_and(|source| source.trusted_for_board_split())
    {
        // `session_map` is the open game. On a split that game is the one
        // being closed, and an untrusted read must not name the new session
        // after it. With no open map the row would be blank, and a blank
        // map uploads, so the capture is dropped.
        let fallback = if staged.split {
            None
        } else {
            session_map.filter(|name| !name.is_empty())
        };
        staged.map_name = fallback.unwrap_or("").to_string();
        staged.recorded_map = (!staged.map_name.is_empty()).then(|| staged.map_name.clone());
        staged.map_source = None;
        if staged.map_name.is_empty() && !staged.split {
            tracing::info!("untracked map with no open game is not stored");
            return PreparedRow::Drop;
        }
    }
    parsed.map_name = staged.map_name.clone();
    parsed.game_mode = parse::stored_game_mode(&parsed.map_name);
    let alias = panel_raw.is_some_and(parse::is_adlersbrunn_alias)
        || parse::is_adlersbrunn_alias(board_text);
    if parse::reject_pve_adlersbrunn(alias, &parsed.map_name, both_teams) {
        tracing::info!(
            "adlersbrunn scoreboard has stats on only one team, not stored as Eichenwalde"
        );
        return PreparedRow::Drop;
    }
    PreparedRow::Store
}

fn blank_rejected_report(
    outcome: detect::MatchOutcome,
    session_id: &str,
    hero_auth: HeroAuthState,
    req: &CaptureRequest,
) -> CaptureReport {
    CaptureReport {
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
        hero_auth,
        seal: None,
        close_reason: None,
        held_at: None,
        map_replaces_text_fallback: false,
    }
}

/// Insert the current row, then the carried board. The carried write runs
/// only after the insert succeeds.
/// Put the reader's best guess in each unread column of `parsed` and list
/// those columns in `suspect_fields` (upload names: `e`, `a`, `d`, `dmg`,
/// `h`, `mit`), so the app marks them for a one-click edit.
fn apply_best_guess(
    parsed: &mut storage::PersonalMatch,
    unread: &[&'static str],
    guess: [u32; capture_gate::GATE_COLS],
) {
    let cells = [
        &mut parsed.elims,
        &mut parsed.assists,
        &mut parsed.deaths,
        &mut parsed.damage,
        &mut parsed.healing,
        &mut parsed.mitigation,
    ];
    for (col, cell) in cells.into_iter().enumerate() {
        if unread.contains(&capture_gate::COL_FIELD[col]) {
            *cell = guess[col];
        }
    }
    for name in unread {
        if !parsed.suspect_fields.iter().any(|f| f == name) {
            parsed.suspect_fields.push((*name).to_string());
        }
    }
}

/// alpha.8: a game that closes with no stored row but a best guess stores
/// the guess, with the game's outcome and map. Returns whether it stored.
async fn store_best_guess_on_close(
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    g: &mut ActiveGame,
) -> bool {
    if g.session_created {
        return false;
    }
    let Some(mut row) = g.best_guess.take() else {
        return false;
    };
    row.session_id = g.session_id.clone();
    if g.finished() {
        row.outcome = g.outcome.to_string();
    }
    storage::append_match_log(data_dir, &row);
    match store.insert_match(row.clone()).await {
        Ok(_) => {
            tracing::info!(
                session_id = %g.session_id,
                suspect = ?row.suspect_fields,
                "game closed with a stat never read for sure: stored the best guess, marked unsure"
            );
            g.session_created = true;
            refresh_snapshot(store, data_dir).await;
            true
        }
        Err(e) => {
            tracing::warn!(error = %e, session_id = %g.session_id, "best guess not stored");
            false
        }
    }
}

async fn commit_capture_rows(
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    staged: &StagedCapture,
    parsed: &storage::PersonalMatch,
    now: SurrealDatetime,
) -> anyhow::Result<bool> {
    let created = write_staged_rows(store, staged, &parsed.hero, &parsed.role, now).await?;
    storage::append_match_log(data_dir, parsed);
    store
        .insert_match(parsed.clone())
        .await
        .map_err(anyhow::Error::from_boxed)
        .context("store insert failed")?;
    write_carried_after_current(
        store,
        data_dir,
        staged,
        &parsed.hero,
        should_write_carried(true, staged.carried.is_some()),
    )
    .await?;
    Ok(created)
}

/// Put the game's last accepted value in each held stat column of `parsed`
/// (0 when the game has none yet).
///
/// A belt, not a rule: nothing that runs before the gate reads a held
/// column. The boundary plan leaves it out of every comparison
/// (`boundary::plan_capture_held`; `the_plan_does_not_read_a_held_cell`
/// pins that), the split vote skips a suspect column, both gates ignore
/// the number, and the report kit's field is overwritten after the gate.
/// It stays so that a later reader of these counters finds the game's own
/// value and not a 0 that looks like a read.
fn fill_held_cells(
    parsed: &mut storage::PersonalMatch,
    held: [bool; capture_gate::GATE_COLS],
    prior: Option<capture_gate::Counters>,
) {
    let prior = prior.unwrap_or_default();
    let prior = [
        prior.elims,
        prior.assists,
        prior.deaths,
        prior.damage,
        prior.healing,
        prior.mitigation,
    ];
    let cells = [
        &mut parsed.elims,
        &mut parsed.assists,
        &mut parsed.deaths,
        &mut parsed.damage,
        &mut parsed.healing,
        &mut parsed.mitigation,
    ];
    for (col, cell) in cells.into_iter().enumerate() {
        if held[col] {
            *cell = prior[col];
        }
    }
}

/// Run the capture gate on the new reader's stats.
///
/// `apply_new_reader` puts the sure digit reads back on the row. A drop is
/// clamped back and flagged. A single-field jump past the capture gate's
/// limit waits until the next read matches. A `held` column (the digit
/// reader did not vouch for the cell) keeps the game's last accepted value
/// and is flagged; whatever number sits in it on `parsed` is ignored.
/// `suspect` and `trusted` are this capture's marks. `carried` is the board
/// written onto the same session just before this row ([`carried_board`]);
/// it seeds the held columns of a capture that opens the game. The returned
/// gate is what the next capture compares against.
///
/// Every cell whose stored value is not this capture's read is flagged,
/// whatever kept it out (`capture_gate::held_fields`): under this reader a
/// rate-cap or trailing-digit hold after a run of held captures keeps a
/// number that can be minutes old (F266-3). So is every stat that is lower
/// than on the row before it (`capture_gate::revised_fields`,
/// `capture_gate::lowered_fields`).
fn hold_new_reader_stats(
    parsed: &mut storage::PersonalMatch,
    prior: Option<(capture_gate::GateState, std::time::Duration)>,
    suspect: [bool; capture_gate::GATE_COLS],
    held_cells: [bool; capture_gate::GATE_COLS],
    split: bool,
    trusted: bool,
    carried: Option<(Counters, std::time::Duration)>,
) -> capture_gate::GateOutcome {
    let seed = carried.filter(|_| held_cells.iter().any(|&cell| cell));
    // The row stored just before this one in the same game: the game's last
    // accepted counters once it has a row, or the carried board on the
    // capture that opens the game with one.
    let opens_game = split || prior.is_none();
    let before = if opens_game {
        carried.map(|(board, _)| board)
    } else {
        prior
            .map(|(state, _)| state)
            .filter(|state| state.unread_cols().is_empty())
            .map(|state| state.accepted)
    };
    let read = capture_gate::Counters {
        elims: parsed.elims,
        assists: parsed.assists,
        deaths: parsed.deaths,
        damage: parsed.damage,
        healing: parsed.healing,
        mitigation: parsed.mitigation,
    };
    let held =
        capture_gate::apply_gate_seeded(prior, read, suspect, held_cells, split, trusted, seed);
    parsed.elims = held.accepted.elims;
    parsed.assists = held.accepted.assists;
    parsed.deaths = held.accepted.deaths;
    parsed.damage = held.accepted.damage;
    parsed.healing = held.accepted.healing;
    parsed.mitigation = held.accepted.mitigation;
    // Flagged: every cell the gate kept at another value than the read,
    // every cell an un-latch revised down, and any stat lower than on the
    // row before, whatever let it down (R3-1: a stat never goes down
    // unflagged in a stored row).
    let mut unsure = capture_gate::held_fields(&held.holds);
    let lowered = before
        .map(|before| capture_gate::lowered_fields(before, held.accepted))
        .unwrap_or_default();
    for name in capture_gate::revised_fields(&held.unlatches)
        .into_iter()
        .chain(lowered)
    {
        if !unsure.contains(&name) {
            unsure.push(name);
        }
    }
    note_gate_suspects(parsed, held.accepted, &unsure);
    held
}

/// A gate hold keeps the old number and names the field.
///
/// `unsure` is the fields to name: ocr-v1 passes a drop and a single-field
/// jump (`capture_gate::unsure_fields`), the new reader every hold
/// (`capture_gate::held_fields`). The flag stays when the stored number is
/// the one the gate kept.
fn note_gate_suspects(
    parsed: &mut storage::PersonalMatch,
    held: capture_gate::Counters,
    unsure: &[&'static str],
) {
    let stored = [
        parsed.elims,
        parsed.assists,
        parsed.deaths,
        parsed.damage,
        parsed.healing,
        parsed.mitigation,
    ];
    let kept = [
        held.elims,
        held.assists,
        held.deaths,
        held.damage,
        held.healing,
        held.mitigation,
    ];
    for &name in unsure {
        let Some(col) = capture_gate::COL_FIELD
            .iter()
            .position(|field| *field == name)
        else {
            continue;
        };
        if stored[col] != kept[col] {
            continue;
        }
        if !parsed.suspect_fields.iter().any(|field| field == name) {
            parsed.suspect_fields.push(name.to_string());
        }
    }
    parsed.suspect_fields = reader_apply::upload_suspect_fields(&parsed.suspect_fields);
}

/// Replace stored fields from the new reader's board.
///
/// `board` is the board the capture path already accepted
/// (`reader_apply::board_refusal`, `reader_apply::read_own_row_stats`), and
/// `parsed` is the policy row built from it: its stats are the capture
/// gate's (a held cell carries the game's last accepted value), so a field
/// the board does not supply never takes an OCR number. The map and mode
/// already chosen by capture policy stay even when the board names another
/// map; under this reader that policy map already took the banner read as
/// its top-bar map ([`choose_top_bar_map`]). This does not touch the capture
/// gate, the session map, or the report.
///
/// The row is the one [`find_own_row`] found from the own-name seeds and the
/// highlight (alpha.7 O9); no name is matched here. `own_row_suspect` says
/// only one of the two named it where both could have: the row is used, and
/// every field read off it is marked suspect.
fn apply_new_reader(
    parsed: &mut storage::PersonalMatch,
    evidence: &shadow::result::ResultEvidence,
    own: reader_apply::OwnRow,
    own_row_suspect: bool,
    mut board: shadow::BoardRead,
    authority: Option<&str>,
) {
    board.set_result(shadow::result_field(evidence));
    let policy = reader_apply::PolicyRow {
        map: parsed.map_name.clone(),
        mode: parsed.game_mode.clone(),
        result: parsed.outcome.clone(),
        hero: parsed.hero.clone(),
        elims: parsed.elims,
        assists: parsed.assists,
        deaths: parsed.deaths,
        damage: parsed.damage,
        healing: parsed.healing,
        mitigation: parsed.mitigation,
    };
    // The own row's hero is stored only when the Tab's hero authority (own
    // row and career panel portrait) names it too.
    let saved =
        reader_apply::merge_saved_with_authority(&policy, own, Some(&board), Some(authority));
    parsed.map_name = saved.map;
    parsed.game_mode = saved.mode;
    parsed.outcome = saved.result;
    parsed.hero = saved.hero;
    parsed.role = parse::guess_role_public(&parsed.hero);
    parsed.elims = saved.elims;
    parsed.assists = saved.assists;
    parsed.deaths = saved.deaths;
    parsed.damage = saved.damage;
    parsed.healing = saved.healing;
    parsed.mitigation = saved.mitigation;
    parsed.recognizer = if saved.recognizer == shadow::digits::RECOGNIZER_ID {
        shadow::digits::live_recognizer_id().to_string()
    } else {
        saved.recognizer.to_string()
    };
    parsed.suspect_fields = saved.suspect_fields;
    if own_row_suspect {
        reader_apply::mark_own_row_suspect(&mut parsed.suspect_fields);
    }
    tracing::info!(
        recognizer = parsed.recognizer,
        suspects = ?parsed.suspect_fields,
        "saved scoreboard reader"
    );
}

/// The top-bar map of one Tab, and the banner read behind it.
#[derive(Debug, Clone, PartialEq)]
struct TopBarChoice {
    map: Option<String>,
    /// `map` is a sure banner read, not the top-bar OCR.
    from_banner: bool,
    /// The banner read, for the board's map field (see
    /// [`FrameAnalysis::banner`]).
    banner: Option<Option<shadow::banner::MapRead>>,
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

/// The top-bar map of one Tab: the read capture policy stores as
/// [`boundary::MapSource::TopBar`].
///
/// With `reader = "new"` a sure banner read (`shadow::top_bar_map` over
/// `banner()`: the map pack's glyph match, no OCR) is the top-bar map. It
/// then goes through the same policy as the top-bar OCR: the session map, the
/// vote-candidate veto, a hinted different-map split and the untracked-map
/// rules. When the banner reader abstains (no pack, no banner, a flagged
/// read, or a name the tracker would not store) the top-bar OCR is used.
/// A panic in the banner reader is an abstain too: it never fails the Tab,
/// the same isolation [`read_tab_board`] gives the board read (F251-2). With
/// `reader = "ocr-v1"` the banner is never read and the OCR label is
/// returned unchanged.
fn choose_top_bar_map(
    use_new_reader: bool,
    banner: impl FnOnce() -> Option<shadow::banner::MapRead>,
    ocr_map: Option<String>,
) -> TopBarChoice {
    let ocr = |banner| TopBarChoice {
        map: ocr_map.clone(),
        from_banner: false,
        banner,
    };
    if !use_new_reader {
        return ocr(None);
    }
    let read = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(banner)) {
        Ok(read) => read,
        Err(_) => {
            tracing::warn!("banner reader panicked; using the top-bar OCR for this Tab");
            return ocr(None);
        }
    };
    match read.as_ref().and_then(shadow::top_bar_map) {
        Some(top) => {
            tracing::info!(
                map = %top.name,
                score = top.score,
                margin = top.margin,
                ocr_agrees = ocr_map.as_deref() == Some(top.name.as_str()),
                "top-bar map from the map pack"
            );
            TopBarChoice {
                map: Some(top.name),
                from_banner: true,
                banner: Some(read),
            }
        }
        None => ocr(Some(read)),
    }
}

/// The top bar's raw label of one Tab, for the Adlersbrunn check in
/// [`prepare_capture_row`] (alpha.7 M1).
///
/// Adlersbrunn is the Halloween label of Eichenwalde, and a Junkenstein
/// board is told from a real game by that label plus stats on one team
/// only. ocr-v1 finds the label in the top-bar OCR text
/// ([`parse::is_adlersbrunn_alias`]). With `reader = "new"` a sure banner
/// read of `ADLERSBRUNN` ([`shadow::banner::MapRead::is_adlersbrunn`]: the
/// map pack's glyph match, no OCR) says the same thing, so the label is that
/// spelling whatever the OCR text was, the way [`choose_top_bar_map`] takes
/// the sure banner read over the OCR label for the map. With OCR off it is
/// the only thing that says it.
///
/// Every other banner read (another map, a flagged read, no banner, no map
/// pack) and `reader = "ocr-v1"` (`banner` is `None`: the banner is never
/// read) leave the OCR label exactly as read.
fn top_bar_raw_label(
    banner: Option<&Option<shadow::banner::MapRead>>,
    ocr_label: Option<String>,
) -> Option<String> {
    match banner {
        Some(Some(read)) if read.is_adlersbrunn() => Some(shadow::banner::ADLERSBRUNN.to_string()),
        _ => ocr_label,
    }
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

/// The Tab retry predicate: retry only when the scoreboard preflight
/// rejected the frame as not a scoreboard (a board still drawing). A team
/// size the pitch does not settle and an analysed frame are not retried.
fn tab_capture_should_retry(outcome: &FrameAnalysisOutcome) -> bool {
    matches!(outcome, FrameAnalysisOutcome::NotAScoreboard { .. })
}

/// How useful a Tab outcome is, so a later rejected grab never replaces a
/// better one: analysed over team size unsure over not a scoreboard.
fn tab_outcome_rank(outcome: &FrameAnalysisOutcome) -> u8 {
    match outcome {
        FrameAnalysisOutcome::NotAScoreboard { .. } => 0,
        FrameAnalysisOutcome::TeamSizeUncertain { .. } => 1,
        FrameAnalysisOutcome::Analyzed(_) => 2,
    }
}

async fn handle_capture(ctx: &DaemonCtx, mut req: CaptureRequest) -> anyhow::Result<CaptureReport> {
    if ctx.use_new_reader {
        req.member_map_edit = session_has_member_map_edit(&ctx.store, &req).await;
    }
    // QUAL-002 follow-up (multi-file seam split — parked for USER per DR-1 A4)
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
        // A board still drawing fails the preflight, so a rejected Tab frame
        // is captured again after a short wait (`stat_tracker::tab_retry`).
        let own_seeds = own_name_seeds(ctx.own_name.as_ref());
        let (analysis, _retries) = stat_tracker::tab_retry::with_retries(
            &stat_tracker::tab_retry::TAB_RETRY_WAITS,
            tab_capture_should_retry,
            tab_outcome_rank,
            |_| {
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
                async move {
                    let img = capture::capture_screen_output(backend, capture_output)
                        .await
                        .map_err(anyhow::Error::from_boxed)
                        .context("screen capture failed")?;
                    let analysis = tokio::task::spawn_blocking(move || {
                        let started = Instant::now();
                        let analysis = analyze_frame(
                            img,
                            matcher,
                            player_name_owned,
                            game_outcome,
                            flags,
                            own_seeds,
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
                    anyhow::Ok(analysis)
                }
            },
            tokio::time::sleep,
        )
        .await?;
        let analysis = match analysis {
            FrameAnalysisOutcome::Analyzed(a) => Ok(*a),
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
        own_name_candidate,
        own_seed_verdict,
        team_size,
        frame: frame_img,
        shadow_input,
    } = analysis;
    if ctx.use_new_reader {
        note_own_seed_verdict(ctx.own_name.as_ref(), own_seed_verdict, session_id).await;
    }
    let ocr_result = ocr
        .map_err(anyhow::Error::from_boxed)
        .context("full-board OCR failed")?;
    // `reader = "new"`: the board the trust gate and the stats come from.
    // No board (a test injection without one) is a board that was not found.
    let new_board = ctx.use_new_reader.then(|| {
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
    // is a low-trust read: the capture gate does not latch its counters,
    // and the boundary plan lets it anchor the reset baseline but not split
    // a game by itself (`BoardFacts::suspect_row`).
    let own_row_suspect = own_row.is_some_and(|own| own.suspect());
    let scoreboard_read = match &new_board {
        Some(board) => reader_apply::read_own_row_stats(board, board_row, &outcome_label)
            .map(|read| (read.matched, true, read.held)),
        None => parse::read_scoreboard(
            &rows,
            player_row_idx,
            &ocr_result.raw_text,
            &outcome_label,
            text_fallback_name(ctx.use_new_reader, player_row_idx, player_name),
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
                    "own row is suspect: its stats are a low-trust read, every field off it is flagged"
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
            // A bundled stand-in (PROVISIONAL_PORTRAITS: Blizzard Entertainment
            // artwork sourced via the Overwatch wiki, for Doctrine) counts as
            // missing, so the first career-panel crop replaces it. A file with
            // any other bytes is a real crop or a user file and is never
            // overwritten. The player's row must be known: cropping row 0 when
            // the row was not identified would store someone else's hero and
            // then stick. New heroes ship faster than bundled portraits
            // (D.Mon, WL-5): the first game on one seeds its reference here, and
            // the matcher picks it up on the next daemon start. Only the
            // career-panel source may seed (a portrait/held/text guess must not
            // template itself). The heroes-v2 icon reads (`reader = "new"`)
            // do not force a seed either: the reference library is the
            // ocr-v1 portrait matcher's, and a wrong seed sticks. Only the
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
            let suspect = if ctx.use_new_reader {
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
            let facts = BoardFacts {
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
                return Ok(skipped_capture_report(
                    &plan,
                    session_id,
                    source.is_authority(),
                    hero_auth,
                    carried.then(|| parsed.hero.clone()),
                    carried.then_some(captured_at),
                ));
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
                if ctx.use_new_reader {
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
                return Ok(blank_rejected_report(outcome, session_id, hero_auth, &req));
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
            if let Some(board) = new_board {
                let guesses = reader_apply::own_row_guesses(&board, board_row);
                offer_own_name_seed(
                    ctx.own_name.as_ref(),
                    own_name_candidate,
                    &target_session,
                    &parsed.map_name,
                )
                .await;
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
                staged.gate = hold_new_reader_stats(
                    &mut parsed,
                    prior,
                    suspect,
                    held_cols,
                    split,
                    trusted_cells && !own_row_suspect,
                    carried_board(staged.carried, staged.carried_at, captured_at),
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
                let best_guess = raw_guess.map(|guess| {
                    let mut row = parsed.clone();
                    apply_best_guess(&mut row, &unread, guess);
                    row
                });
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
            save_accepted_frame(ctx.debug_ocr, data_dir, scoreboard_img);
            record_report_kit(
                data_dir,
                &target_session,
                &frame_img,
                team_size,
                &ocr_parsed,
                &parsed,
                captured_at,
                req.tab_at,
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
            })
        }
    }
}

/// Why a poll tick earned a `debug_ocr` evidence PNG. Mid-match ticks with
/// no outcome signal are not saved; `--dump-poll-frames` is the every-tick
/// hammer and stays independent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PollDebugHit {
    /// Banner one-shot, or the second agreeing word-OCR tick.
    Confirm,
    /// First word-OCR streak sighting (pre-confirm). A lone VICTORY/SVACTORY
    /// that never gets a second tick still leaves a frame.
    Streak,
}

/// Decide whether this poll tick should write a `debug/poll/` evidence PNG
/// when `debug_ocr` is on. Mirrors the poller's confirm/streak match so a
/// test can pin the triggers without driving the `select!` loop.
fn poll_debug_hit(
    signal: Option<(detect::MatchOutcome, detect::match_end::OutcomeSource)>,
    prior_streak: Option<(detect::MatchOutcome, std::time::Duration)>,
    confirm_window: std::time::Duration,
) -> Option<(PollDebugHit, detect::MatchOutcome)> {
    match signal {
        Some((outcome, detect::match_end::OutcomeSource::Banner)) => {
            Some((PollDebugHit::Confirm, outcome))
        }
        Some((outcome, _)) => {
            let agreed =
                prior_streak.is_some_and(|(prev, age)| prev == outcome && age <= confirm_window);
            if agreed {
                Some((PollDebugHit::Confirm, outcome))
            } else {
                Some((PollDebugHit::Streak, outcome))
            }
        }
        None => None,
    }
}

fn poll_debug_prefix(hit: PollDebugHit, outcome: detect::MatchOutcome) -> String {
    let kind = match hit {
        PollDebugHit::Confirm => "confirm",
        PollDebugHit::Streak => "streak",
    };
    format!("poll_{kind}_{outcome}")
}

/// Save a debug frame into `dir` as `<prefix>_<timestamp>.png`, keeping at
/// most `keep` PNGs in the directory (oldest by mtime evicted). Each ring gets
/// a dedicated directory (`debug/poll`, `debug/rejected`), so every PNG there
/// participates in the same ring regardless of prefix. On-hit names are
/// `poll_confirm_{outcome}_…` / `poll_streak_{outcome}_…`.
fn save_frame_ring(dir: &std::path::Path, prefix: &str, img: &image::DynamicImage, keep: usize) {
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let name = format!(
        "{prefix}_{}.png",
        chrono::Local::now().format("%Y%m%d_%H%M%S")
    );
    if let Err(e) = img.save(dir.join(&name)) {
        tracing::debug!(error = %e, "failed to save debug frame");
        return;
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        let mut frames: Vec<(std::time::SystemTime, std::path::PathBuf)> = entries
            .flatten()
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("png"))
            .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
            .collect();
        if frames.len() > keep {
            frames.sort_by_key(|(t, _)| *t);
            for (_, old) in &frames[..frames.len() - keep] {
                let _ = std::fs::remove_file(old);
            }
        }
    }
}

/// Debug rings that hold unblanked board pixels (other players' names).
/// Written only while `debug_ocr` is on.
const BOARD_PIXEL_DEBUG_DIRS: [&str; 2] = ["accepted", "rejected"];

/// Archive a frame whose capture was rejected by a trust gate, for diagnosis.
/// Only with `debug_ocr` on (`debug`): the frame is unblanked. Runs the PNG
/// encode off the async runtime; fire-and-forget. The handle is for tests.
fn save_rejected_frame(
    debug: bool,
    data_dir: &std::path::Path,
    img: image::DynamicImage,
    reason: &'static str,
) -> Option<tokio::task::JoinHandle<()>> {
    if !debug {
        return None;
    }
    let dir = data_dir.join("debug").join("rejected");
    Some(tokio::task::spawn_blocking(move || {
        save_frame_ring(&dir, &format!("rejected_{reason}"), &img, REJECTED_KEEP);
    }))
}

/// A rank screen's `outcome.png` goes to the active game only when that game
/// recorded evidence (a capture, an outcome) this recently.
const RANK_OUTCOME_MAX_IDLE: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// Report kit (`report::kit`): the blanked scoreboard and the reads of this
/// accepted capture. Written off the capture path; a failure is logged and
/// never touches the stored match.
#[allow(clippy::too_many_arguments)]
fn record_report_kit(
    data_dir: &std::path::Path,
    session_id: &str,
    frame: &image::DynamicImage,
    team_size: usize,
    ocr_parsed: &storage::PersonalMatch,
    stored: &storage::PersonalMatch,
    captured_at: chrono::DateTime<Utc>,
    tab_at: stat_tracker::report::blank::CaptureTime,
) {
    let Some(layout) = stat_tracker::report::blank::TeamLayout::from_team_size(Some(team_size))
    else {
        return;
    };
    let reads = stat_tracker::report::kit::KitReads::from_capture(
        ocr_parsed,
        stored,
        frame.width(),
        frame.height(),
        layout,
        captured_at,
    );
    let (data_dir, session_id, frame) = (
        data_dir.to_path_buf(),
        session_id.to_string(),
        frame.clone(),
    );
    tokio::task::spawn_blocking(move || {
        if let Err(e) = stat_tracker::report::kit::record_scoreboard_at(
            &data_dir,
            &session_id,
            &frame,
            Some(team_size),
            &reads,
            tab_at,
        ) {
            tracing::debug!(error = %e, "report kit: scoreboard not written");
        }
    });
}

/// Archive the scoreboard crop of an ACCEPTED capture into a bounded ring, so a
/// board that OCR'd wrong but still passed every trust gate can be inspected
/// after the fact (the capture gate holds bad cells, but the underlying crop is
/// the only way to retune calibration). Only with `debug_ocr` on (`debug`):
/// the crop is unblanked and shows other players' names. Fire-and-forget;
/// encode off-runtime. The handle is for tests.
fn save_accepted_frame(
    debug: bool,
    data_dir: &std::path::Path,
    board: Arc<image::DynamicImage>,
) -> Option<tokio::task::JoinHandle<()>> {
    if !debug {
        return None;
    }
    let dir = data_dir.join("debug").join("accepted");
    Some(tokio::task::spawn_blocking(move || {
        save_frame_ring(&dir, "accepted", &board, ACCEPTED_KEEP);
    }))
}

/// Board-pixel debug folders left over on disk while `debug_ocr` is off.
/// They are never deleted (they are the user's files); startup only says
/// they are no longer written.
fn stale_board_debug_dirs(debug: bool, data_dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    if debug {
        return Vec::new();
    }
    BOARD_PIXEL_DEBUG_DIRS
        .iter()
        .map(|name| data_dir.join("debug").join(name))
        .filter(|dir| dir.is_dir())
        .collect()
}

/// Keep the full top-bar map-label box from a Tab whose label is not a flat bar.
/// The first such Tab writes `<session_id>-<width>x<height>-1.png`. A later
/// Tab at least 60 seconds after that writes the matching `-2.png`. Width and
/// height are the capture size. The PNG is the raw box from this frame, with
/// no timer trim and no brightness stretch. The map-label OCR retry does not
/// supply these pixels. A flat bar does not take a slot. Local files under
/// `debug/mapcrops` only.
/// Not part of the rolling debug rings and not included in report bundles.
/// From an accepted capture it is not gated on `debug_ocr`; a rejected
/// capture goes through [`archive_rejected_map_label_crop`]. The handle is
/// for tests.
fn archive_map_label_crop(
    data_dir: &std::path::Path,
    session_id: &str,
    frame: &image::DynamicImage,
) -> Option<tokio::task::JoinHandle<()>> {
    let job = mapcrops::prepare_mapcrop(data_dir, session_id, frame)?;
    Some(tokio::task::spawn_blocking(move || {
        mapcrops::commit_mapcrop(job)
    }))
}

/// [`archive_map_label_crop`] for a capture a trust gate rejected: only with
/// `debug_ocr` on, and never for `preflight` (a menu has no top bar). A
/// rejected frame may be gameplay rather than a Tab board, and there the
/// map-label rectangle covers the kill feed, which shows player names.
fn archive_rejected_map_label_crop(
    debug: bool,
    data_dir: &std::path::Path,
    session_id: &str,
    frame: &image::DynamicImage,
    reason: &'static str,
) -> Option<tokio::task::JoinHandle<()>> {
    if !debug || reason == "preflight" {
        return None;
    }
    archive_map_label_crop(data_dir, session_id, frame)
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

/// How many shutdown drains are blocked inside the in-flight sync join.
/// Tests wait on this so "final upload started early" cannot pass by winning
/// a race against a task that has not been scheduled yet.
#[cfg(test)]
static SYNC_DRAIN_WAITING: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Finish `task` (including its `mark_synced`) before `then`. Shutdown must
/// not start a second upload while the first still holds ids from its read:
/// both would mark those ids, and the older HTTP body can land on the server
/// after the newer one. Aborting the task would drop it between the response
/// and `mark_synced`, so the join waits instead.
async fn drain_sync_then<F, Fut>(task: Option<tokio::task::JoinHandle<()>>, then: F)
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future,
{
    if let Some(t) = task {
        if !t.is_finished() {
            tracing::info!("waiting for in-flight sync to finish before final upload");
        }
        #[cfg(test)]
        SYNC_DRAIN_WAITING.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let _ = t.await;
        #[cfg(test)]
        SYNC_DRAIN_WAITING.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
    then().await;
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

/// Shutdown path: export a trailing dirty snapshot regardless of the window —
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

async fn try_sync(
    store: &storage::LocalStore,
    client: &sync::SyncClient,
    data_dir: &std::path::Path,
    settle_session: Option<String>,
) -> sync::SyncAttempt {
    let client = client.clone();
    let outcome = try_sync_with(store, data_dir, settle_session, {
        let client = client.clone();
        move |matches, tombstones| {
            let client = client.clone();
            async move { client.upload_matches(&matches, &tombstones).await }
        }
    })
    .await;
    if matches!(outcome, sync::SyncAttempt::AuthRejected) {
        let creds = client.credentials();
        if let Err(e) = sync::write_auth_pause(data_dir, &creds.server_url, &creds.token) {
            tracing::warn!(error = %e, "could not record sync auth pause");
        }
    }
    outcome
}

/// True when this capture should start a periodic sync task.
///
/// Single-flight (`task_in_flight`) and the backoff window are both gates.
/// Shutdown does not use this predicate — it joins the in-flight task and
/// uploads once regardless of the backoff clock.
fn periodic_sync_due(capture_count: u32, task_in_flight: bool, backoff_allows: bool) -> bool {
    capture_count.is_multiple_of(SYNC_EVERY_N_CAPTURES) && !task_in_flight && backoff_allows
}

fn maybe_spawn_periodic_sync(
    sync_task: &mut Option<tokio::task::JoinHandle<()>>,
    backoff: &Arc<std::sync::Mutex<sync::SyncBackoff>>,
    store: &storage::LocalStore,
    client: Option<&sync::SyncClient>,
    data_dir: &std::path::Path,
    capture_count: u32,
) {
    let Some(client) = client else {
        return;
    };
    let in_flight = sync_task.as_ref().is_some_and(|t| !t.is_finished());
    let allow = backoff
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .should_attempt(Instant::now());
    if !periodic_sync_due(capture_count, in_flight, allow) {
        if capture_count.is_multiple_of(SYNC_EVERY_N_CAPTURES) && !in_flight && !allow {
            tracing::debug!("sync skipped: backing off before the next attempt");
        }
        return;
    }
    let store = store.clone();
    let client = client.clone();
    let data_dir = data_dir.to_path_buf();
    let backoff = Arc::clone(backoff);
    *sync_task = Some(tokio::spawn(async move {
        let outcome = try_sync(&store, &client, &data_dir, None).await;
        apply_sync_backoff(&backoff, outcome);
    }));
}

fn apply_sync_backoff(backoff: &std::sync::Mutex<sync::SyncBackoff>, outcome: sync::SyncAttempt) {
    let now = Instant::now();
    let mut clock = backoff.lock().unwrap_or_else(|e| e.into_inner());
    clock.observe(outcome, now);
    match outcome {
        sync::SyncAttempt::ServerError { .. } => {
            tracing::warn!(
                failures = clock.failures(),
                retry_in_secs = clock.retry_after(now).as_secs(),
                "sync upload failed: backing off so a down server is not hammered"
            );
        }
        sync::SyncAttempt::RateLimited { .. } => {
            tracing::warn!(
                retry_in_secs = clock.retry_after(now).as_secs(),
                "sync rate-limited: backing off before the next upload"
            );
        }
        sync::SyncAttempt::AuthRejected => {
            tracing::error!(
                "sync token rejected: pausing until the server URL or token changes in Settings"
            );
        }
        sync::SyncAttempt::Uploaded | sync::SyncAttempt::NoServerCall => {}
    }
}

fn sync_credentials_rejected(
    data_dir: &std::path::Path,
    client: Option<&sync::SyncClient>,
) -> bool {
    let Some(client) = client else {
        return false;
    };
    let creds = client.credentials();
    sync::auth_pause_matches(data_dir, &creds.server_url, &creds.token)
}

/// While sync is paused on a rejected token, re-read Settings. A different
/// URL or token clears the pause and updates the client. The rest of the
/// config is left alone.
fn maybe_resume_sync_after_settings_change(
    client: Option<&sync::SyncClient>,
    backoff: &std::sync::Mutex<sync::SyncBackoff>,
    data_dir: &std::path::Path,
) {
    let paused = backoff
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .auth_rejected();
    if !paused {
        return;
    }
    let Ok(cfg) = config::Config::load() else {
        return;
    };
    let Some(sync_cfg) = cfg.sync else {
        return;
    };
    if sync::auth_pause_matches(data_dir, &sync_cfg.server_url, &sync_cfg.token) {
        return;
    }
    let Some(client) = client else {
        return;
    };
    match sync::SyncClient::try_new(sync_cfg) {
        Ok(fresh) => {
            client.replace_credentials(fresh.credentials());
            sync::clear_auth_pause(data_dir);
            backoff
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clear_auth_rejected();
            tracing::info!("sync resumed: server URL or token changed");
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                "sync stays paused: the saved server URL is not safe for the token"
            );
        }
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

/// One upload. Auth rejection skips the HTTP call. The caller feeds the
/// result to [`apply_sync_backoff`].
async fn upload_once(
    client: Option<&sync::SyncClient>,
    backoff: &std::sync::Mutex<sync::SyncBackoff>,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    settle_session: Option<String>,
) -> sync::SyncAttempt {
    let rejected = backoff
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .auth_rejected();
    if rejected {
        tracing::warn!("sync paused, token rejected; skipping this upload");
        return sync::SyncAttempt::NoServerCall;
    }
    match client {
        Some(client) => try_sync(store, client, data_dir, settle_session).await,
        None => sync::SyncAttempt::NoServerCall,
    }
}

/// Run `task` after `previous` has finished. The join happens inside the
/// spawned task, so the caller returns immediately.
fn spawn_after_previous<F, Fut>(
    previous: Option<tokio::task::JoinHandle<()>>,
    task: F,
) -> tokio::task::JoinHandle<()>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    tokio::spawn(async move {
        if let Some(previous) = previous {
            let _ = previous.await;
        }
        task().await;
    })
}

/// Queue an upload on the single-flight sync task. The previous handle is
/// joined inside the new task. The result updates the backoff clock.
fn schedule_store_sync(
    sync_task: &mut Option<tokio::task::JoinHandle<()>>,
    backoff: &Arc<std::sync::Mutex<sync::SyncBackoff>>,
    store: &storage::LocalStore,
    client: Option<&sync::SyncClient>,
    data_dir: &std::path::Path,
    settle_session: Option<String>,
) {
    let previous = sync_task.take();
    let client = client.cloned();
    let store = store.clone();
    let data_dir = data_dir.to_path_buf();
    let backoff = Arc::clone(backoff);
    *sync_task = Some(spawn_after_previous(previous, move || async move {
        let outcome =
            upload_once(client.as_ref(), &backoff, &store, &data_dir, settle_session).await;
        apply_sync_backoff(&backoff, outcome);
    }));
}

/// Mark one closed session's unknown rows synced locally. Other unknown
/// rows stay unsynced. Nothing is sent for the settled session.
fn schedule_settled_unknown(
    st: &mut SessionState,
    sync_task: &mut Option<tokio::task::JoinHandle<()>>,
    backoff: &Arc<std::sync::Mutex<sync::SyncBackoff>>,
    store: &storage::LocalStore,
    client: Option<&sync::SyncClient>,
    data_dir: &std::path::Path,
) {
    if let Some(session_id) = st.settle_closed_unknown.take() {
        schedule_store_sync(
            sync_task,
            backoff,
            store,
            client,
            data_dir,
            Some(session_id),
        );
    }
}

/// One command tick of the GUI queue. A SetOutcome schedules an upload
/// on this same tick. A delete or an edit does not.
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
                    g.record_outcome(outcome.parse().unwrap_or(detect::MatchOutcome::Unknown));
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
                    g.record_outcome(outcome.parse().unwrap_or(detect::MatchOutcome::Unknown));
                    persist_active_game(data_dir, Some(g));
                }
            }
            storage::StoreCommand::ResolveSegment { .. }
            | storage::StoreCommand::RetryUpload { .. } => {}
        }
        match store.apply_command(cmd).await {
            Ok(()) => {
                storage::remove_command_file(cmd_file);
                if matches!(
                    cmd,
                    storage::StoreCommand::SetOutcome { .. }
                        | storage::StoreCommand::RetryUpload { .. }
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
    startup_session(data_dir)
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

async fn shutdown_from_signal(
    st: &mut SessionState,
    capture_task: &mut Option<tokio::task::JoinHandle<()>>,
    sync_task: &mut Option<tokio::task::JoinHandle<()>>,
    sync_client: Option<&sync::SyncClient>,
    sync_backoff: &std::sync::Mutex<sync::SyncBackoff>,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
) {
    drain_capture(capture_task.take()).await;
    close_session_on_shutdown(st, store, data_dir).await;
    let task = sync_task.take();
    finish_sync_on_shutdown(sync_client, sync_backoff, task, store, data_dir).await;
    flush_snapshot_if_dirty(store, data_dir).await;
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

fn sync_attempt_from_error(err: &sync::SyncUploadError) -> sync::SyncAttempt {
    let attempt = err.attempt();
    match attempt {
        sync::SyncAttempt::RateLimited { .. } => {
            tracing::warn!(error = %err, "sync rate-limited, will retry after backoff");
        }
        sync::SyncAttempt::AuthRejected => {
            tracing::error!(
                error = %err,
                "sync token rejected, will not retry until the URL or token changes"
            );
        }
        _ if err.message.starts_with(sync::SERVER_REFUSING_UPLOADS) => {
            tracing::error!("{}", err.message);
        }
        _ => tracing::error!(error = %err, "sync upload failed"),
    }
    attempt
}

/// Read unsynced rows, upload, then mark synced only where `sync_rev` is
/// still the revision captured here. `upload` is the HTTP call (or a test
/// double). It runs after the read and before the mark, which is the window
/// a local outcome/map/hero/GUI write can revise a row.
fn claims_covering(
    claims: &[storage::SyncClaim],
    rows: &[storage::PersonalMatch],
    indexes: impl IntoIterator<Item = usize>,
) -> Vec<storage::SyncClaim> {
    let mut picked = vec![false; claims.len()];
    for index in indexes {
        let Some(row) = rows.get(index) else {
            continue;
        };
        for (i, claim) in claims.iter().enumerate() {
            if claim.covers_row(row) {
                picked[i] = true;
            }
        }
    }
    claims
        .iter()
        .enumerate()
        .filter(|(i, _)| picked[*i])
        .map(|(_, claim)| claim.clone())
        .collect()
}

async fn try_sync_with<F, Fut>(
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    settle_session: Option<String>,
    mut upload: F,
) -> sync::SyncAttempt
where
    F: FnMut(Vec<storage::PersonalMatch>, Vec<String>) -> Fut,
    Fut: std::future::Future<
            Output = Result<scuffed_types::api::StatsUploadResponse, sync::SyncUploadError>,
        >,
{
    // Errors are stringified immediately: `Box<dyn Error>` isn't `Send`, and
    // this future runs on a spawned task.
    //
    // The server cannot store an outcome-less game. On a close, mark those
    // rows synced locally and do not put them in the request. A GUI
    // SetOutcome writes a decided outcome and `synced = false`, which the
    // decided query below then uploads with `edited` set.
    if let Some(session_id) = settle_session.as_deref() {
        match store
            .get_unsynced_unknown()
            .await
            .map_err(|e| e.to_string())
        {
            Ok(unknown) => {
                let unknown: Vec<_> = unknown
                    .into_iter()
                    .filter(|row| row.session_id == session_id)
                    .collect();
                if !unknown.is_empty() {
                    let claims = storage::SyncClaim::capture(&unknown);
                    match store.mark_synced(&claims).await.map_err(|e| e.to_string()) {
                        Ok(()) => tracing::info!(
                            rows = unknown.len(),
                            session_id,
                            "held unknown-outcome rows locally, not sent"
                        ),
                        Err(e) => {
                            tracing::error!(
                                error = %e,
                                "failed to hold an unknown-outcome row locally"
                            )
                        }
                    }
                }
            }
            Err(e) => tracing::error!(error = %e, "failed to query unknown-outcome rows"),
        }
    }
    let pending = store.get_unsynced().await;
    let unsynced = match pending.map_err(|e| e.to_string()) {
        Ok(u) => u,
        Err(e) => {
            tracing::error!(error = %e, "failed to query unsynced matches");
            return sync::SyncAttempt::NoServerCall;
        }
    };
    let (local_only, unsynced): (Vec<_>, Vec<_>) = unsynced.into_iter().partition(|row| {
        let sent = parse::uploaded_game_mode(row.display_map_name(), &row.game_mode);
        !parse::stats_row_is_tracked(row.display_map_name(), &row.game_mode)
            || !parse::stats_row_is_tracked(row.display_map_name(), &sent)
    });
    if !local_only.is_empty() {
        let claims = storage::SyncClaim::capture(&local_only);
        if let Err(e) = store.mark_synced(&claims).await.map_err(|e| e.to_string()) {
            tracing::error!(error = %e, "failed to keep a deathmatch row local");
        }
    }
    // Locally-deleted sessions whose server rows must go too.
    let tombstones = match store
        .get_pending_tombstones()
        .await
        .map_err(|e| e.to_string())
    {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(error = %e, "failed to query pending tombstones");
            Vec::new()
        }
    };
    if unsynced.is_empty() && tombstones.is_empty() {
        return sync::SyncAttempt::NoServerCall;
    }

    // The server keeps one row per session, so only the final snapshot of
    // each session is worth sending. Every fetched row is *claimed* at its
    // current `sync_rev`; `mark_synced` commits a claim only when that
    // revision is unchanged (a write during `upload` bumps it and stays
    // queued). Collapsed snapshots are represented by the uploaded one, but
    // a revision miss on any of them leaves the whole session queued so the
    // next sync cannot upload an older snapshot by itself.
    let claims = storage::SyncClaim::capture(&unsynced);
    let mut newest_first = unsynced;
    newest_first.reverse(); // get_unsynced is played_at ASC
    let staged = storage::latest_per_game(newest_first);
    let (review_held, to_upload): (Vec<_>, Vec<_>) =
        staged.into_iter().partition(sync::row_needs_review);
    if !review_held.is_empty() {
        tracing::info!(games = review_held.len(), "held games for review, not sent");
    }
    if to_upload.is_empty() && tombstones.is_empty() {
        return sync::SyncAttempt::NoServerCall;
    }
    tracing::info!(
        rows = claims.len(),
        games = to_upload.len(),
        tombstones = tombstones.len(),
        "syncing unsynced matches"
    );
    // Quarantine can leave deletes with no match rows. Those still need one
    // request. There is nothing to split. A deletes-only 400 still retries
    // forever (backoff, same as a 5xx). That matters if the server ever
    // validates deleted_sessions: one bad id keeps the delete queue retrying,
    // because there are no match rows to isolate.
    if to_upload.is_empty() {
        return match upload(Vec::new(), tombstones.clone()).await {
            Ok(resp) => {
                tracing::info!(deleted = resp.deleted, "sync complete");
                if let Err(e) = store
                    .clear_tombstones(tombstones)
                    .await
                    .map_err(|e| e.to_string())
                {
                    tracing::error!(error = %e, "failed to clear acknowledged tombstones");
                }
                refresh_snapshot_force(store, data_dir).await;
                sync::SyncAttempt::Uploaded
            }
            Err(err) => sync_attempt_from_error(&err),
        };
    }
    // 400 and 422 split the batch so one refused row does not retry the
    // whole queue forever. A pass that accepts nothing is a server-wide
    // refusal: quarantine nothing and back off. 5xx, network errors, 401,
    // and 403 stop the pass without splitting.
    let isolation = sync::isolate_rejected(&to_upload, |batch| {
        upload(batch.to_vec(), tombstones.clone())
    })
    .await;

    for rejected in &isolation.rejected {
        let covered = claims_covering(&claims, &to_upload, [rejected.index]);
        let session_id = to_upload
            .get(rejected.index)
            .map(|row| row.session_id.as_str())
            .unwrap_or("");
        tracing::warn!(
            session_id,
            error = %rejected.message,
            "upload rejected this row"
        );
        if let Err(e) = store
            .quarantine_upload(&covered, &rejected.message)
            .await
            .map_err(|e| e.to_string())
        {
            tracing::error!(error = %e, session_id, "failed to quarantine a rejected row");
        }
    }

    let accepted = claims_covering(&claims, &to_upload, isolation.accepted.iter().copied());
    if !accepted.is_empty() {
        if let Err(e) = store
            .mark_synced(&accepted)
            .await
            .map_err(|e| e.to_string())
        {
            tracing::error!(error = %e, "failed to mark matches as synced");
        }
        if let Err(e) = store
            .clear_tombstones(tombstones)
            .await
            .map_err(|e| e.to_string())
        {
            tracing::error!(error = %e, "failed to clear acknowledged tombstones");
        }
    }

    if !isolation.accepted.is_empty() || !isolation.rejected.is_empty() {
        refresh_snapshot_force(store, data_dir).await;
    }

    if let Some(err) = isolation.stopped {
        return sync_attempt_from_error(&err);
    }

    if !isolation.deferred.is_empty() {
        tracing::warn!(
            deferred = isolation.deferred.len(),
            requests = isolation.requests,
            "sync left rows queued after the request cap"
        );
    }

    if isolation.accepted.is_empty() && isolation.rejected.is_empty() {
        return sync::SyncAttempt::NoServerCall;
    }

    tracing::info!(
        uploaded = isolation.accepted.len(),
        quarantined = isolation.rejected.len(),
        deferred = isolation.deferred.len(),
        requests = isolation.requests,
        "sync pass finished"
    );
    sync::SyncAttempt::Uploaded
}

#[cfg(test)]
mod ocr_bench;

#[cfg(test)]
mod tests {
    use super::*;

    use std::time::Duration;

    #[test]
    fn tab_retry_predicate_retries_only_not_a_scoreboard() {
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
        assert!(tab_capture_should_retry(&rejected));
        assert!(!tab_capture_should_retry(&unsure));
        assert!(tab_outcome_rank(&unsure) > tab_outcome_rank(&rejected));
    }

    struct ScriptedWaiter {
        events: Vec<KeyboardWait>,
    }

    impl KeyboardWaiter for ScriptedWaiter {
        async fn wait(&mut self) -> anyhow::Result<KeyboardWait> {
            if self.events.is_empty() {
                anyhow::bail!("keyboard wait called more times than the test scripted");
            }
            Ok(self.events.remove(0))
        }
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

    // Tests construct `now` in the future so subtracting ages can't underflow
    // the monotonic clock on a freshly-booted machine.
    fn test_now() -> Instant {
        Instant::now() + Duration::from_secs(600)
    }

    fn word_streak(outcome: detect::MatchOutcome, seen_at: Instant) -> WordStreak {
        WordStreak {
            outcome,
            seen_at,
            map: None,
        }
    }

    fn game(
        outcome: detect::MatchOutcome,
        recorded_secs_ago: Option<u64>,
        now: Instant,
    ) -> ActiveGame {
        ActiveGame {
            best_guess: None,
            session_id: "test".into(),
            outcome,
            map: None,
            map_source: None,
            map_candidates: Vec::new(),
            session_created: true,
            outcome_recorded_at: recorded_secs_ago.map(|s| now - Duration::from_secs(s)),
            opened_at: now - Duration::from_secs(300),
            last_activity: now - Duration::from_secs(30),
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
        }
    }

    /// Session state around an optional game, everything else quiescent —
    /// the baseline the poll_slow_mode tests perturb.
    fn session(active_game: Option<ActiveGame>, now: Instant) -> SessionState {
        SessionState {
            capture_count: 0,
            last_game_open: None,
            last_tab_capture: None,
            active_game,
            pending_outcome: None,
            word_outcome_streak: None,
            suspend_probe: (now, Utc::now()),
            ocr_stability: detect::stability::FrameStability::default(),
            poll_ticks_skipped: 0,
            end_reel_wake_until: None,
            settle_closed_unknown: None,
        }
    }

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

        // Word streak alone is not the Tab-busy exception — mid-match skip
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

    #[test]
    fn tab_with_no_game_starts_fresh() {
        assert!(should_start_fresh_session(None, test_now()));
    }

    #[test]
    fn tab_mid_game_reuses_session() {
        let now = test_now();
        let g = game(detect::MatchOutcome::Unknown, None, now);
        assert!(!should_start_fresh_session(Some(&g), now));
    }

    #[test]
    fn tab_on_post_match_scoreboard_reuses_finished_session() {
        // Within the grace window the Tab capture is the post-match scoreboard
        // of the SAME match — a fresh session would double-count the game.
        let now = test_now();
        let g = game(detect::MatchOutcome::Defeat, Some(30), now);
        assert!(!should_start_fresh_session(Some(&g), now));
    }

    #[test]
    fn tab_long_after_finish_starts_fresh() {
        let now = test_now();
        let g = game(detect::MatchOutcome::Victory, Some(120), now);
        assert!(should_start_fresh_session(Some(&g), now));
        // An unstamped outcome is treated as stale: a finished result must
        // never leak onto the next match's captures.
        let g = game(detect::MatchOutcome::Victory, None, now);
        assert!(should_start_fresh_session(Some(&g), now));
    }

    #[test]
    fn idle_unfinished_session_goes_stale() {
        // An unfinished session with recent activity is reusable...
        let now = test_now() + UNFINISHED_SESSION_IDLE * 2;
        let g = game(detect::MatchOutcome::Unknown, None, now);
        assert!(!should_start_fresh_session(Some(&g), now));
        // ...but one idle past the bound can't plausibly be the same game:
        // yesterday's session must not absorb today's first Tab.
        let mut stale = game(detect::MatchOutcome::Unknown, None, now);
        stale.last_activity = now - UNFINISHED_SESSION_IDLE - Duration::from_secs(1);
        assert!(should_start_fresh_session(Some(&stale), now));
    }

    /// Career profile (Competitive Open Queue): **one** Neon Junction,
    /// ~10:02, 3–0 Victory. Oasis defeat and Ilios victory are different maps.
    /// The Games pair (21:49 empty `—`, 21:55 WIN, same Tank / Neon Junction /
    /// Wrecking Ball) is a double capture of that one match — not two queues.
    /// Do not invent a second Neon Junction session.
    #[test]
    fn unfinished_neon_junction_ball_2149_empty_2155_win_reuses() {
        let first_tab = test_now();
        let second = first_tab + Duration::from_secs(6 * 60);
        let mut g = game(detect::MatchOutcome::Unknown, None, first_tab);
        g.map = Some("Neon Junction".into());
        g.hero_auth.accepted_hero = Some("Wrecking Ball".into());
        g.last_activity = first_tab;
        g.opened_at = first_tab;

        let incoming = IncomingIdentity {
            map: Some("Neon Junction"),
            hero: Some("Wrecking Ball"),
            vote_candidates: &[],
        };
        assert!(
            should_reuse_unfinished_same_match(Some(&g), incoming, second),
            "double capture: merge 21:55 WIN into the 21:49 empty row; no second match"
        );

        // Map-vote after cooldown (default 120s): lingering vote includes the
        // played map. Pre-fix this opened a second session.
        let vote = [String::from("Neon Junction"), String::from("Dorado")];
        assert!(
            !map_vote_should_open_new_game(
                Some(&g),
                Some(first_tab),
                second,
                Duration::from_secs(120),
                &vote,
            ),
            "lingering map-vote 6 min after Tab must not open a second game"
        );

        // Stat-regression split path: unfinished + same identity, age > 120s.
        assert!(
            same_unfinished_match(
                true,
                Duration::from_secs(6 * 60),
                Some("Neon Junction"),
                Some("Wrecking Ball"),
                IncomingIdentity {
                    map: Some("Neon Junction"),
                    hero: Some("Wrecking Ball"),
                    vote_candidates: &[],
                },
            ),
            "stat-looking regression 6 min later must not split this match"
        );

        // Ilios on the career profile is a later, different match — a vote
        // that does not include Neon Junction may still open. Same-map must not.
        let other_vote = [String::from("Ilios"), String::from("Busan")];
        assert!(map_vote_should_open_new_game(
            Some(&g),
            Some(first_tab),
            second,
            Duration::from_secs(120),
            &other_vote,
        ));

        // Outcome already recorded — this is a finished game, not the pair.
        let mut won = game(detect::MatchOutcome::Victory, Some(6 * 60 - 30), second);
        won.map = Some("Neon Junction".into());
        won.hero_auth.accepted_hero = Some("Wrecking Ball".into());
        won.last_activity = first_tab + Duration::from_secs(30);
        assert!(map_vote_should_open_new_game(
            Some(&won),
            Some(first_tab),
            second,
            Duration::from_secs(120),
            &vote,
        ));
    }

    #[test]
    fn map_vote_cooldown_still_blocks_without_identity() {
        // No map/hero on the unfinished game: identity reuse does not apply,
        // so the 120s debounce is still the only guard (lingering vote at
        // start, before any Tab). Tab itself is not delayed.
        let opened = test_now();
        let g = game(detect::MatchOutcome::Unknown, None, opened);
        let vote = [String::from("Oasis")];
        assert!(!map_vote_should_open_new_game(
            Some(&g),
            Some(opened),
            opened + Duration::from_secs(30),
            Duration::from_secs(120),
            &vote,
        ));
        assert!(map_vote_should_open_new_game(
            Some(&g),
            Some(opened),
            opened + Duration::from_secs(121),
            Duration::from_secs(120),
            &vote,
        ));
    }

    #[test]
    fn live_map_vote_uses_the_open_game() {
        let now = test_now();
        let opened = now - Duration::from_secs(400);
        let mut g = game(detect::MatchOutcome::Unknown, None, now);
        g.map = Some("Busan".into());
        g.opened_at = opened;
        g.last_activity = now - Duration::from_secs(30);
        g.gate = Some(GateState {
            accepted: Counters {
                elims: 8,
                assists: 3,
                deaths: 2,
                damage: 1800,
                healing: 400,
                mitigation: 100,
            },
            ..GateState::default()
        });
        let debounce = Duration::from_secs(120);
        let other = vec!["Junkertown".into(), "Ilios".into()];
        let same = vec!["Busan".into(), "Dorado".into()];
        assert!(
            map_vote_should_open_new_game(Some(&g), Some(opened), now, debounce, &other),
            "an early leave plus a different vote closes the unfinished session"
        );
        assert!(!map_vote_should_open_new_game(
            Some(&g),
            Some(opened),
            now,
            debounce,
            &same,
        ));
        let open = boundary::decide_poll(&boundary::PollInput {
            outcome: g.outcome,
            result: g.result_mark,
            pending_boundary: g.pending_boundary,
            awaiting_first_board: g.awaiting_first_board,
            has_board: g.gate.is_some(),
            reset_streak: g.reset_streak,
            map: g.map.as_deref(),
            map_trusted: g.map_is_trusted(),
            hero: g.hero_auth.accepted_hero.as_deref(),
            signal: None,
            signal_confirmed: false,
            accolade_map: None,
            start_screen: Some(boundary::StartScreen::MapVote {
                candidates: other.clone(),
            }),
            block_map_vote: !map_vote_should_open_new_game(
                Some(&g),
                Some(opened),
                now,
                debounce,
                &other,
            ),
            now,

            deferred: false,
            text_fallback_locked: false,
        });
        match open {
            boundary::PollDecision::Open(opened_game) => {
                assert_eq!(opened_game.reason, boundary::CloseReason::MapVote);
                assert!(opened_game.seal_outcome.is_none());
                assert_eq!(opened_game.candidates, other);
            }
            other => panic!("expected a split, got {other:?}"),
        }
        let blocked = boundary::decide_poll(&boundary::PollInput {
            outcome: g.outcome,
            result: g.result_mark,
            pending_boundary: g.pending_boundary,
            awaiting_first_board: g.awaiting_first_board,
            has_board: g.gate.is_some(),
            reset_streak: g.reset_streak,
            map: g.map.as_deref(),
            map_trusted: g.map_is_trusted(),
            hero: g.hero_auth.accepted_hero.as_deref(),
            signal: None,
            signal_confirmed: false,
            accolade_map: None,
            start_screen: Some(boundary::StartScreen::MapVote {
                candidates: same.clone(),
            }),
            block_map_vote: !map_vote_should_open_new_game(
                Some(&g),
                Some(opened),
                now,
                debounce,
                &same,
            ),
            now,

            deferred: false,
            text_fallback_locked: false,
        });
        assert!(
            !matches!(blocked, boundary::PollDecision::Open(_)),
            "the same-map vote does not open a second session"
        );
    }

    #[test]
    fn active_game_persists_and_recovers() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut g = ActiveGame::open_now(
            "abc123".into(),
            detect::MatchOutcome::Unknown,
            vec!["Oasis".into(), "Busan".into()],
        );
        g.session_created = true;
        g.map = Some("Oasis".into());
        g.map_source = Some(boundary::MapSource::TopBar);
        let gate_state = GateState {
            accepted: Counters {
                elims: 12,
                assists: 4,
                deaths: 3,
                damage: 5400,
                healing: 900,
                mitigation: 1200,
            },
            last_raw: Counters {
                elims: 12,
                assists: 4,
                deaths: 3,
                damage: 5400,
                healing: 900,
                mitigation: 1200,
            },
            ..Default::default()
        };
        g.gate = Some(gate_state);
        g.last_stats_at = Some(Instant::now());
        g.note_result(detect::MatchOutcome::Defeat, false);
        g.pending_boundary = true;
        g.awaiting_first_board = true;
        g.deferred = Some(Counters {
            elims: 1,
            assists: 0,
            deaths: 0,
            damage: 100,
            healing: 0,
            mitigation: 50,
        });
        g.deferred_hero = Some("Wrecking Ball".into());
        g.deferred_at = Some(Utc::now() - chrono::Duration::seconds(30));
        g.deferred_imported = true;
        g.progressed_boards = 2;
        g.baseline_at = Some(Instant::now());
        persist_active_game(dir.path(), Some(&g));

        let r = recover_active_game(dir.path()).expect("recent game recovers");
        assert_eq!(r.session_id, "abc123");
        assert_eq!(r.outcome, detect::MatchOutcome::Unknown);
        assert_eq!(r.map.as_deref(), Some("Oasis"));
        assert_eq!(r.map_source, Some(boundary::MapSource::TopBar));
        assert_eq!(
            r.map_candidates,
            vec!["Oasis".to_string(), "Busan".to_string()]
        );
        assert!(r.session_created);
        assert_eq!(
            r.gate.map(|s| s.accepted.edd()),
            Some((12, 3, 5400)),
            "regression baseline must survive a daemon restart"
        );
        assert_eq!(
            r.gate.map(|s| s.accepted.elims),
            Some(12),
            "gate state (all six counters) survives a daemon restart"
        );
        assert!(r.last_stats_at.is_some());
        assert_eq!(
            r.result_mark.map(|m| (m.outcome, m.confirmed)),
            Some((detect::MatchOutcome::Defeat, false)),
            "a defeat streak must survive a restart so the next queue can close this session"
        );
        assert!(r.pending_boundary);
        assert!(r.awaiting_first_board);
        assert_eq!(r.deferred.map(|c| c.elims), Some(1));
        assert_eq!(r.deferred_hero.as_deref(), Some("Wrecking Ball"));
        assert!(r.deferred_at.is_some());
        assert!(r.deferred_imported);
        assert_eq!(r.progressed_boards, 2);
        assert!(r.baseline_at.is_some());
        assert!(
            start_screen_blocked(
                Some(&r),
                Some(r.opened_at),
                r.opened_at + Duration::from_secs(30),
                Duration::from_secs(120),
                &boundary::StartScreen::HeroSelect,
            ),
            "a recovered session still inside the debounce does not split on select"
        );

        // Clearing removes the file — nothing to recover.
        persist_active_game(dir.path(), None);
        assert!(recover_active_game(dir.path()).is_none());
    }

    #[test]
    fn future_result_timestamp_does_not_drop_the_recovered_game() {
        let dir = tempfile::tempdir().expect("tempdir");
        let g = ActiveGame::open_now("abc123".into(), detect::MatchOutcome::Unknown, Vec::new());
        persist_active_game(dir.path(), Some(&g));
        let path = active_game_path(dir.path());
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        value["result_outcome"] = serde_json::json!("defeat");
        value["result_confirmed"] = serde_json::json!(false);
        value["result_seen_at"] = serde_json::json!("2999-01-01T00:00:00Z");
        value["pending_boundary"] = serde_json::json!(true);
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let recovered = recover_active_game(dir.path()).expect("a bad hint timestamp is not fatal");
        assert_eq!(recovered.session_id, "abc123");
        assert!(recovered.result_mark.is_none());
        assert!(
            !recovered.pending_boundary,
            "an arm without a recoverable hint must not hold full cadence"
        );
    }

    #[test]
    fn split_session_does_not_inherit_post_match_grace() {
        let now = test_now();
        let opened = session_opened_by_split(
            "fresh".into(),
            detect::MatchOutcome::Unknown,
            HeroAuthState::default(),
            false,
            None,
            None,
            now,
        );
        assert!(!opened.finished());
        assert!(opened.outcome_recorded_at.is_none());
        assert!(
            !should_start_fresh_session(Some(&opened), now + Duration::from_secs(10)),
            "the new session is in progress, not a finished defeat inside the 75s grace"
        );
        let inherited = game(detect::MatchOutcome::Defeat, Some(10), now);
        assert!(inherited.finished());
        assert!(
            !should_start_fresh_session(Some(&inherited), now),
            "a defeat stamped now would swallow the next tab"
        );
    }

    #[test]
    fn identified_row_id_is_the_player_row() {
        assert_eq!(identified_row_id(Some(3)), Some(3));
        assert_eq!(identified_row_id(None), None);
    }

    #[test]
    fn gap_split_keeps_the_frame_outcome_on_the_new_session() {
        let defeat = session_opened_by_split(
            "gap".into(),
            detect::MatchOutcome::Defeat,
            HeroAuthState::default(),
            false,
            None,
            None,
            Instant::now(),
        );
        assert_eq!(defeat.outcome, detect::MatchOutcome::Defeat);
        assert!(defeat.outcome_recorded_at.is_some());
        let reset = session_opened_by_split(
            "reset".into(),
            detect::MatchOutcome::Unknown,
            HeroAuthState {
                career_ever_ok: true,
                accepted_hero: Some("Zenyatta".into()),
                portrait_pending: Some(("Illari".into(), 2)),
            },
            false,
            None,
            None,
            Instant::now(),
        );
        assert!(
            !reset.hero_auth.career_ever_ok,
            "a carried career flag is not this capture's career panel"
        );
        assert!(reset.hero_auth.portrait_pending.is_none());
        assert_eq!(reset.hero_auth.accepted_hero.as_deref(), Some("Zenyatta"));
        let seeded = session_opened_by_split(
            "seeded".into(),
            detect::MatchOutcome::Unknown,
            HeroAuthState {
                career_ever_ok: true,
                accepted_hero: Some("Wrecking Ball".into()),
                portrait_pending: None,
            },
            true,
            None,
            None,
            Instant::now(),
        );
        assert!(seeded.hero_auth.career_ever_ok);
        assert_eq!(
            seeded.hero_auth.accepted_hero.as_deref(),
            Some("Wrecking Ball")
        );
        assert!(!reset.finished());
        assert!(reset.outcome_recorded_at.is_none());
    }

    #[tokio::test]
    async fn held_board_is_inserted_on_the_new_session() {
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        let held = Counters {
            elims: 2,
            assists: 1,
            deaths: 0,
            damage: 400,
            healing: 80,
            mitigation: 900,
        };
        let played_at = Utc::now() - chrono::Duration::seconds(90);
        store
            .create_session(&storage::MatchSession {
                session_id: "junkertown".into(),
                hero: "Wrecking Ball".into(),
                map_name: "Junkertown".into(),
                role: "Tank".into(),
                started_at: SurrealDatetime::from(played_at),
                last_capture_at: SurrealDatetime::from(played_at),
                capture_count: 0,
                final_outcome: "unknown".into(),
            })
            .await
            .unwrap();
        store_held_board(
            &store,
            dir.path(),
            "junkertown",
            "Wrecking Ball",
            "Junkertown",
            "unknown",
            held,
            played_at,
        )
        .await
        .unwrap();
        let snaps = store.get_session_snapshots("junkertown").await.unwrap();
        assert_eq!(snaps.len(), 1);
        assert_eq!(snaps[0].hero, "Wrecking Ball");
        assert_eq!(snaps[0].elims, 2);
        assert_eq!(snaps[0].damage, 400);
        assert_eq!(snaps[0].outcome, "unknown");
        let stored: chrono::DateTime<Utc> = snaps[0].played_at.into();
        assert!((stored - played_at).num_seconds().abs() < 2);
    }

    #[tokio::test]
    async fn apply_poll_decision_stamps_grace_at_confirm_and_writes_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        store
            .insert_match(test_match("sess-grace", "unknown"))
            .await
            .unwrap();
        let sighting = Instant::now() - Duration::from_secs(200);
        let mut g = game(detect::MatchOutcome::Unknown, None, Instant::now());
        g.session_id = "sess-grace".into();
        g.session_created = true;
        g.result_mark = Some(ResultMark {
            outcome: detect::MatchOutcome::Defeat,
            confirmed: false,
            seen_at: sighting,
        });
        g.pending_boundary = false;
        let mut st = session(Some(g), Instant::now());
        let decision = {
            let g = st.active_game.as_ref().unwrap();
            boundary::decide_poll(&boundary::PollInput {
                outcome: g.outcome,
                result: g.result_mark,
                pending_boundary: g.pending_boundary,
                awaiting_first_board: g.awaiting_first_board,
                has_board: g.gate.is_some(),
                reset_streak: g.reset_streak,
                map: g.map.as_deref(),
                map_trusted: g.map_is_trusted(),
                hero: g.hero_auth.accepted_hero.as_deref(),
                signal: Some(detect::MatchOutcome::Defeat),
                signal_confirmed: true,
                accolade_map: None,
                start_screen: None,
                block_map_vote: false,
                deferred: g.deferred.is_some(),
                text_fallback_locked: g.text_fallback_locked,
                now: Instant::now(),
            })
        };
        apply_poll_decision(&mut st, &store, dir.path(), decision, Instant::now(), false).await;
        let g = st.active_game.expect("the confirm stays on this session");
        assert_eq!(g.outcome, detect::MatchOutcome::Defeat);
        let stamped = g.outcome_recorded_at.expect("grace is stamped");
        assert!(
            stamped.saturating_duration_since(sighting) > Duration::from_secs(150),
            "grace is not the word's first sighting"
        );
        assert!(
            stamped.elapsed() < Duration::from_secs(5),
            "grace starts when apply_poll_decision records the result"
        );
        assert!(!should_start_fresh_session(
            Some(&g),
            stamped + Duration::from_secs(10)
        ));
        assert!(should_start_fresh_session(
            Some(&g),
            stamped + POST_MATCH_GRACE + Duration::from_secs(1)
        ));
        let snaps = store.get_session_snapshots("sess-grace").await.unwrap();
        assert_eq!(snaps.len(), 1);
        assert_eq!(snaps[0].outcome, "defeat");
    }

    #[tokio::test]
    async fn retire_active_game_writes_a_seal_and_leaves_an_unsealed_hint() {
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        store
            .insert_match(test_match("sess-seal", "unknown"))
            .await
            .unwrap();
        store
            .insert_match(test_match("sess-hint", "unknown"))
            .await
            .unwrap();

        let mut sealed = game(detect::MatchOutcome::Unknown, None, Instant::now());
        sealed.session_id = "sess-seal".into();
        sealed.session_created = true;
        sealed.result_mark = Some(ResultMark {
            outcome: detect::MatchOutcome::Defeat,
            confirmed: false,
            seen_at: Instant::now() - Duration::from_secs(20),
        });
        let mut st = session(Some(sealed), Instant::now());
        retire_active_game(
            &mut st,
            &store,
            dir.path(),
            Some(detect::MatchOutcome::Defeat),
            "superseded by hero select/ban",
        )
        .await;
        assert!(st.active_game.is_none());
        let snaps = store.get_session_snapshots("sess-seal").await.unwrap();
        assert_eq!(snaps[0].outcome, "defeat");

        let mut hinted = game(detect::MatchOutcome::Unknown, None, Instant::now());
        hinted.session_id = "sess-hint".into();
        hinted.session_created = true;
        hinted.result_mark = Some(ResultMark {
            outcome: detect::MatchOutcome::Victory,
            confirmed: false,
            seen_at: Instant::now() - Duration::from_secs(10),
        });
        let mut st = session(Some(hinted), Instant::now());
        retire_active_game(
            &mut st,
            &store,
            dir.path(),
            None,
            "superseded by stat reset",
        )
        .await;
        assert!(st.active_game.is_none());
        let snaps = store.get_session_snapshots("sess-hint").await.unwrap();
        assert_eq!(
            snaps[0].outcome, "unknown",
            "retire with no seal leaves the hint unstored"
        );
    }

    #[test]
    fn stale_persisted_game_is_not_recovered() {
        let dir = tempfile::tempdir().expect("tempdir");
        let stale = PersistedGame {
            replaced_text_fallback_map: None,
            best_guess: None,
            session_id: "old".into(),
            outcome: detect::MatchOutcome::Unknown,
            map: None,
            map_source: None,
            map_candidates: Vec::new(),
            session_created: true,
            opened_at: Utc::now() - chrono::Duration::hours(9),
            last_activity: Utc::now() - chrono::Duration::hours(8),
            outcome_recorded_at: None,
            gate: None,
            last_stats_at: None,
            hero_auth: HeroAuthState::default(),
            result_outcome: None,
            result_confirmed: false,
            result_seen_at: None,
            pending_boundary: false,
            awaiting_first_board: false,
            reset_streak: 0,
            reset_baseline: None,
            baseline_row: None,
            deferred: None,
            deferred_hero: None,
            deferred_at: None,
            deferred_imported: false,
            progressed_boards: 0,
            baseline_at: None,
            text_fallback_locked: false,
        };
        std::fs::write(
            active_game_path(dir.path()),
            serde_json::to_vec(&stale).unwrap(),
        )
        .unwrap();
        assert!(
            recover_active_game(dir.path()).is_none(),
            "yesterday's unfinished game must not swallow today's captures"
        );
    }

    #[test]
    fn awaiting_first_board_does_not_latch_a_carried_gate() {
        let carried = (GateState::default(), Duration::from_secs(40));
        assert!(
            gate_prev_for_store(Some(carried), true).is_none(),
            "a session with no board of its own does not latch the previous game"
        );
        let own = (GateState::default(), Duration::from_secs(40));
        assert!(gate_prev_for_store(Some(own), false).is_some());
    }

    #[test]
    fn a_recovered_awaiting_session_keeps_the_debounce() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut g = ActiveGame::open_now(
            "await".into(),
            detect::MatchOutcome::Unknown,
            vec!["Busan".into()],
        );
        g.awaiting_first_board = true;
        persist_active_game(dir.path(), Some(&g));
        let st = startup_session(dir.path());
        let opened = st
            .last_game_open
            .expect("startup restores the debounce from opened_at");
        assert_eq!(opened, st.active_game.as_ref().unwrap().opened_at);
        let screen = boundary::StartScreen::HeroBan;
        let debounce = Duration::from_secs(120);
        let now = opened + Duration::from_secs(30);
        assert!(
            start_screen_blocked(
                st.active_game.as_ref(),
                Some(opened),
                now,
                debounce,
                &screen
            ),
            "a ban inside the restored debounce does not split the awaiting session"
        );
        let mut resumed = startup_session(dir.path());
        resumed.last_game_open = None;
        let recovered = startup_session(dir.path());
        resumed.last_game_open = recovered.last_game_open;
        resumed.active_game = recovered.active_game;
        assert!(start_screen_blocked(
            resumed.active_game.as_ref(),
            resumed.last_game_open,
            now,
            debounce,
            &screen,
        ));
        let game = resumed.active_game.as_ref().unwrap();
        let decision = boundary::decide_poll(&boundary::PollInput {
            outcome: game.outcome,
            result: game.result_mark,
            pending_boundary: game.pending_boundary,
            awaiting_first_board: game.awaiting_first_board,
            has_board: game.gate.is_some(),
            reset_streak: game.reset_streak,
            map: game.map.as_deref(),
            map_trusted: game.map_is_trusted(),
            hero: game.hero_auth.accepted_hero.as_deref(),
            signal: None,
            signal_confirmed: false,
            accolade_map: None,
            start_screen: Some(screen),
            block_map_vote: true,
            deferred: game.deferred.is_some(),
            text_fallback_locked: game.text_fallback_locked,
            now,
        });
        assert!(
            !matches!(decision, boundary::PollDecision::Open(_)),
            "the recovered session stays through the ban"
        );
    }

    #[test]
    fn a_suspend_resume_keeps_the_debounce() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut g = ActiveGame::open_now(
            "await".into(),
            detect::MatchOutcome::Unknown,
            vec!["Busan".into()],
        );
        g.awaiting_first_board = true;
        persist_active_game(dir.path(), Some(&g));
        let mut st = startup_session(dir.path());
        st.last_game_open = None;
        st.pending_outcome = Some((detect::MatchOutcome::Victory, Instant::now()));
        resume_after_suspend(&mut st, dir.path());
        assert!(
            st.pending_outcome.is_none(),
            "resume drops the volatile pending outcome"
        );
        let opened = st
            .last_game_open
            .expect("resume restores the debounce from opened_at");
        assert_eq!(opened, st.active_game.as_ref().unwrap().opened_at);
        let screen = boundary::StartScreen::HeroBan;
        let now = opened + Duration::from_secs(30);
        assert!(
            start_screen_blocked(
                st.active_game.as_ref(),
                st.last_game_open,
                now,
                Duration::from_secs(120),
                &screen,
            ),
            "a ban inside the restored debounce does not split"
        );
    }

    #[test]
    fn a_pre_0_4_19_skeleton_with_a_map_is_untrusted_text() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut g = ActiveGame::open_now("old".into(), detect::MatchOutcome::Unknown, Vec::new());
        g.map = Some("Busan".into());
        g.map_source = Some(boundary::MapSource::TopBar);
        persist_active_game(dir.path(), Some(&g));
        let path = active_game_path(dir.path());
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        value.as_object_mut().unwrap().remove("map_source");
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let recovered = recover_active_game(dir.path()).expect("recent skeleton recovers");
        assert_eq!(recovered.map.as_deref(), Some("Busan"));
        assert_eq!(
            recovered.map_source,
            Some(boundary::MapSource::TextFallback),
            "a map with no source is untrusted text"
        );

        g.map = None;
        g.map_source = None;
        persist_active_game(dir.path(), Some(&g));
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        value.as_object_mut().unwrap().remove("map_source");
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let recovered = recover_active_game(dir.path()).unwrap();
        assert!(recovered.map.is_none());
        assert!(
            recovered.map_source.is_none(),
            "a skeleton with no map stays unsourced"
        );
    }

    #[test]
    fn a_restart_mid_game_keeps_the_best_guess() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut g = ActiveGame::open_now("guess".into(), detect::MatchOutcome::Unknown, Vec::new());
        let mut row = test_match("guess", "victory");
        row.suspect_fields = vec!["h".into()];
        g.best_guess = Some(row.clone());
        persist_active_game(dir.path(), Some(&g));
        let recovered = recover_active_game(dir.path()).expect("recent skeleton recovers");
        assert_eq!(recovered.best_guess, Some(row));
    }

    #[test]
    fn a_persisted_text_fallback_lock_survives_recovery() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut g =
            ActiveGame::open_now("locked".into(), detect::MatchOutcome::Unknown, Vec::new());
        g.map = Some("Dorado".into());
        g.map_source = Some(boundary::MapSource::TextFallback);
        g.text_fallback_locked = true;
        persist_active_game(dir.path(), Some(&g));
        let recovered = recover_active_game(dir.path()).expect("recent skeleton recovers");
        assert!(
            recovered.text_fallback_locked,
            "a lock written to disk must still be set after recovery"
        );
        assert_eq!(recovered.map.as_deref(), Some("Dorado"));
    }

    /// F252-1: the text-fallback name a banner read replaced survives a
    /// restart, so the game's accolade still counts as this game.
    #[test]
    fn a_replaced_text_fallback_name_survives_recovery() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut g =
            ActiveGame::open_now("renamed".into(), detect::MatchOutcome::Unknown, Vec::new());
        g.map = Some("Ilios".into());
        g.map_source = Some(boundary::MapSource::TopBar);
        g.replaced_text_fallback_map = Some("Busan".into());
        persist_active_game(dir.path(), Some(&g));
        let recovered = recover_active_game(dir.path()).expect("recent skeleton recovers");
        assert_eq!(
            recovered.replaced_text_fallback_map.as_deref(),
            Some("Busan")
        );
        // an older skeleton without the field reads as no rename
        let path = active_game_path(dir.path());
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .remove("replaced_text_fallback_map");
        std::fs::write(&path, value.to_string()).unwrap();
        let old = recover_active_game(dir.path()).expect("older skeleton recovers");
        assert_eq!(old.replaced_text_fallback_map, None);
    }

    #[test]
    fn an_unmappable_hint_timestamp_keeps_the_text_fallback_lock() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut g =
            ActiveGame::open_now("locked".into(), detect::MatchOutcome::Unknown, Vec::new());
        g.map = Some("Dorado".into());
        g.map_source = Some(boundary::MapSource::TextFallback);
        g.text_fallback_locked = true;
        g.result_mark = Some(ResultMark {
            outcome: detect::MatchOutcome::Defeat,
            confirmed: false,
            seen_at: Instant::now(),
        });
        persist_active_game(dir.path(), Some(&g));
        let path = active_game_path(dir.path());
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let future = Utc::now() + chrono::Duration::hours(48);
        value["result_seen_at"] = serde_json::Value::String(future.to_rfc3339());
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let recovered = recover_active_game(dir.path()).expect("the session still loads");
        assert!(
            recovered.text_fallback_locked,
            "dropping the hint must not drop the lock"
        );
        assert!(recovered.result_mark.is_none());
    }

    #[test]
    fn a_main_0_4_18_skeleton_recovers_as_untrusted_unlocked_text() {
        let dir = tempfile::tempdir().expect("tempdir");
        // A whole-second RFC3339 stamp has no fraction. Recovery must still
        // accept it. Anything older than the unfinished idle bound is stale.
        let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        assert!(
            !now.contains('.'),
            "whole seconds have no fraction, got {now}"
        );
        // The eleven keys a 0.4.18 skeleton actually stored, with a mid-match
        // gate and the three hero-authority keys. Later fields stay absent so
        // a missing default fails this load.
        let skeleton = serde_json::json!({
            "session_id": "old",
            "outcome": "unknown",
            "map": "Busan",
            "map_candidates": [],
            "session_created": true,
            "opened_at": now,
            "last_activity": now,
            "outcome_recorded_at": null,
            "gate": {
                "accepted": {
                    "elims": 14,
                    "assists": 22,
                    "deaths": 6,
                    "damage": 2400,
                    "healing": 9800,
                    "mitigation": 400
                },
                "last_raw": {
                    "elims": 14,
                    "assists": 22,
                    "deaths": 6,
                    "damage": 2400,
                    "healing": 9800,
                    "mitigation": 400
                },
                "down_streak_len": [0, 0, 0, 0, 0, 0],
                "down_streak_last": [0, 0, 0, 0, 0, 0],
                "last_raw_suspect": [false, false, false, false, false, false]
            },
            "last_stats_at": now,
            "hero_auth": {
                "career_ever_ok": true,
                "accepted_hero": "Zenyatta",
                "portrait_pending": null
            }
        });
        assert_eq!(
            skeleton.as_object().expect("object").len(),
            11,
            "a 0.4.18 skeleton has these eleven keys and no later fields"
        );
        assert_eq!(
            skeleton["hero_auth"].as_object().expect("hero_auth").len(),
            3
        );
        std::fs::write(
            active_game_path(dir.path()),
            serde_json::to_vec(&skeleton).unwrap(),
        )
        .unwrap();
        let recovered = recover_active_game(dir.path()).expect("a 0.4.18 skeleton recovers");
        assert_eq!(recovered.map.as_deref(), Some("Busan"));
        assert_eq!(
            recovered.map_source,
            Some(boundary::MapSource::TextFallback),
            "a map with no source is untrusted text"
        );
        assert!(!recovered.text_fallback_locked);
        assert!(recovered.result_mark.is_none());
        assert!(!recovered.pending_boundary);
        assert!(!recovered.awaiting_first_board);
        assert!(recovered.session_created);
        assert!(recovered.last_stats_at.is_some());
        assert_eq!(recovered.gate.map(|gate| gate.accepted.elims), Some(14));
        assert!(recovered.hero_auth.career_ever_ok);
        assert_eq!(
            recovered.hero_auth.accepted_hero.as_deref(),
            Some("Zenyatta")
        );
        assert!(recovered.hero_auth.portrait_pending.is_none());
        let gate = recovered.gate.expect("skeleton gate");
        assert!(
            !gate.low_trust,
            "a pre-0.4.21 skeleton has no low-trust bit, so the latch stays trusted"
        );
        assert!(!gate.unconfirmed.iter().any(|&col| col));
    }

    #[test]
    fn a_skipped_capture_report_uses_the_plan_outcome_and_the_callers_career_flag() {
        let plan = boundary::CapturePlan {
            split: false,
            defer: true,
            ignore_row: false,
            skip_store: true,
            clear_hint: false,
            reset_streak: 1,
            reset_baseline: None,
            baseline_row: Some(2),
            refresh_baseline: false,
            count_progress: false,
            stored_outcome: detect::MatchOutcome::Victory,
            deferred_counters: Some(night_counters(2, 1, 0, 100, 40, 10)),
            seal: None,
            close_reason: None,
        };
        let report = skipped_capture_report(
            &plan,
            "sid",
            true,
            HeroAuthState::default(),
            Some("Zenyatta".into()),
            None,
        );
        assert_eq!(report.outcome, detect::MatchOutcome::Victory);
        assert!(report.career_panel);
        assert!(!report.recorded);
        assert!(report.armed_reset);
        assert_eq!(report.held_counters.map(|c| c.elims), Some(2));
        assert_eq!(report.session_id, "sid");
    }

    #[test]
    fn the_confirming_tick_uses_the_carried_map() {
        let now = test_now();
        let mut streak = None;
        let first = resolve_word_tick(
            &mut streak,
            Some((
                detect::MatchOutcome::Victory,
                detect::match_end::OutcomeSource::ResultWord,
            )),
            now,
            Some("Junkertown".into()),
        );
        assert!(!first.signal_confirmed);
        assert_eq!(first.accolade_map.as_deref(), Some("Junkertown"));
        let second = resolve_word_tick(
            &mut streak,
            Some((
                detect::MatchOutcome::Victory,
                detect::match_end::OutcomeSource::ResultWord,
            )),
            now + Duration::from_secs(4),
            None,
        );
        assert!(second.signal_confirmed);
        assert_eq!(
            second.accolade_map.as_deref(),
            Some("Junkertown"),
            "the confirming tick keeps the first read's map"
        );
    }

    #[test]
    fn a_carried_map_requires_the_same_outcome() {
        let now = test_now();
        let mut streak = None;
        note_word_streak(
            &mut streak,
            detect::MatchOutcome::Victory,
            now,
            Some("Junkertown".into()),
        );
        let dropped = note_word_streak(
            &mut streak,
            detect::MatchOutcome::Defeat,
            now + Duration::from_secs(4),
            None,
        );
        assert!(
            dropped.is_none(),
            "a different outcome must not keep the previous map"
        );
    }

    #[test]
    fn a_carried_map_expires_with_the_confirm_window() {
        let now = test_now();
        let mut fresh = None;
        note_word_streak(
            &mut fresh,
            detect::MatchOutcome::Victory,
            now,
            Some("Junkertown".into()),
        );
        assert_eq!(
            note_word_streak(
                &mut fresh,
                detect::MatchOutcome::Victory,
                now + Duration::from_secs(30),
                None,
            )
            .as_deref(),
            Some("Junkertown")
        );
        let mut stale = None;
        note_word_streak(
            &mut stale,
            detect::MatchOutcome::Victory,
            now,
            Some("Junkertown".into()),
        );
        assert!(
            note_word_streak(
                &mut stale,
                detect::MatchOutcome::Victory,
                now + OUTCOME_CONFIRM_WINDOW + Duration::from_secs(1),
                None,
            )
            .is_none(),
            "a map older than the confirm window is not carried"
        );
        let mut edge = None;
        note_word_streak(
            &mut edge,
            detect::MatchOutcome::Victory,
            now,
            Some("Junkertown".into()),
        );
        assert_eq!(
            note_word_streak(
                &mut edge,
                detect::MatchOutcome::Victory,
                now + OUTCOME_CONFIRM_WINDOW,
                None,
            )
            .as_deref(),
            Some("Junkertown"),
            "a map exactly as old as the confirm window is still carried"
        );
        assert_eq!(
            poll_debug_hit(
                Some((
                    detect::MatchOutcome::Victory,
                    detect::match_end::OutcomeSource::ResultWord,
                )),
                Some((detect::MatchOutcome::Victory, OUTCOME_CONFIRM_WINDOW)),
                OUTCOME_CONFIRM_WINDOW,
            ),
            Some((PollDebugHit::Confirm, detect::MatchOutcome::Victory)),
            "the confirm window's exact edge agrees with the carried map"
        );
    }

    #[test]
    fn a_text_fallback_poll_input_is_not_map_trusted() {
        let now = test_now();
        let mut game =
            ActiveGame::open_at("sid".into(), detect::MatchOutcome::Unknown, Vec::new(), now);
        game.map = Some("Busan".into());
        game.map_source = Some(boundary::MapSource::TextFallback);
        let input = poll_input_from_game(&game, None, false, None, None, false, now);
        assert!(
            !input.map_trusted,
            "a text fallback is absent on the poll path"
        );
        game.map_source = Some(boundary::MapSource::TopBar);
        let input = poll_input_from_game(&game, None, false, None, None, false, now);
        assert!(input.map_trusted);
    }

    fn banner_read(key: &str, suspect: bool) -> shadow::banner::MapRead {
        let info = shadow::banner::MAPS.iter().find(|m| m.key == key).unwrap();
        shadow::banner::MapRead {
            map: Some(info),
            best: Some(info),
            score: 0.95,
            margin: 0.2,
            suspect,
            found_banner: true,
        }
    }

    /// `reader = "ocr-v1"` (the default) never reads the banner: the top-bar
    /// map is the OCR label, read or not.
    #[test]
    fn ocr_v1_never_reads_the_banner_for_the_top_bar_map() {
        let called = std::cell::Cell::new(false);
        let read = || {
            called.set(true);
            Some(banner_read("ilios", false))
        };
        let t = choose_top_bar_map(false, read, Some("Busan".into()));
        assert_eq!(t.map.as_deref(), Some("Busan"));
        assert!(!t.from_banner);
        assert_eq!(t.banner, None);
        assert_eq!(choose_top_bar_map(false, read, None).map, None);
        assert!(!called.get(), "the banner was read under ocr-v1");
    }

    #[test]
    fn the_new_reader_takes_a_sure_banner_read_over_the_ocr_label() {
        let t = choose_top_bar_map(
            true,
            || Some(banner_read("ilios", false)),
            Some("Busan".into()),
        );
        assert_eq!(t.map.as_deref(), Some("Ilios"));
        assert!(t.from_banner);
        assert_eq!(
            t.banner,
            Some(Some(banner_read("ilios", false))),
            "kept for the board"
        );
        let top = choose_top_bar_map(true, || Some(banner_read("ilios", false)), None).map;
        assert_eq!(top.as_deref(), Some("Ilios"));
        // The banner read is a top-bar read: a session without a map stores
        // it as MapSource::TopBar, and a session map still wins.
        assert_eq!(
            resolved_map(None, top.as_deref(), "Nepal", &[]),
            ("Ilios".to_string(), Some(boundary::MapSource::TopBar))
        );
        assert_eq!(
            resolved_map(Some("Busan"), top.as_deref(), "", &[]),
            ("Busan".to_string(), None)
        );
        // the vote-candidate veto still applies
        let candidates = vec!["Oasis".to_string(), "Busan".to_string()];
        assert_eq!(
            resolved_map(None, top.as_deref(), "Oasis", &candidates),
            ("Oasis".to_string(), Some(boundary::MapSource::TextFallback))
        );
    }

    /// An abstaining banner (no pack, a flagged read, an unknown name:
    /// `shadow::top_bar_map` returns `None`) leaves the OCR label in place.
    /// The read itself is still handed on, so the board does not read the
    /// frame again.
    #[test]
    fn an_abstaining_banner_falls_back_to_the_ocr_label() {
        let flagged = choose_top_bar_map(
            true,
            || Some(banner_read("ilios", true)),
            Some("Busan".into()),
        );
        assert_eq!(flagged.map.as_deref(), Some("Busan"));
        assert!(!flagged.from_banner);
        assert_eq!(flagged.banner, Some(Some(banner_read("ilios", true))));
        let no_pack = choose_top_bar_map(true, || None, Some("Busan".into()));
        assert_eq!(no_pack.map.as_deref(), Some("Busan"));
        assert_eq!(
            no_pack.banner,
            Some(None),
            "no pack: the board reads no map either"
        );
        assert_eq!(choose_top_bar_map(true, || None, None).map, None);
    }

    /// F251-2: a panic in the banner reader is an abstain. The Tab keeps
    /// the OCR label, and the board reads the banner again under its own
    /// `catch_unwind`.
    #[test]
    fn a_panicking_banner_reader_falls_back_to_the_ocr_label() {
        let t = choose_top_bar_map(
            true,
            || -> Option<shadow::banner::MapRead> { panic!("banner reader") },
            Some("Busan".into()),
        );
        assert_eq!(t.map.as_deref(), Some("Busan"));
        assert!(!t.from_banner);
        assert_eq!(t.banner, None);
    }

    /// Both outcome reads take the FINAL SCORE source from the reader switch
    /// (alpha.7 O3): the poll tick and the Tab frame's vision pass. The Tab
    /// pass no longer reads the scoreboard header (O4) and makes no ocr-v1
    /// only outcome call. Whitespace is ignored.
    #[test]
    fn the_outcome_reads_take_the_final_score_source_from_the_reader_switch() {
        let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
        let full = include_str!("main.rs");
        let src = full.split("\nmod tests {").next().unwrap();
        let at = src.find("\nfn analyze_frame(").expect("analyze_frame");
        let body = norm(src[at..].split("\n}\n").next().unwrap());
        assert!(
            body.contains(
                "let p4 = detect::match_end::FinalScoreSource::for_reader(use_new_reader);"
            )
        );
        // one end-title read per Tab frame (F257-3)
        assert!(body.contains(
            "let frame_outcome = if allow_banner_recovery { \
             detect::match_end::detect_outcome_with(&img, p4, &titles, &words) } else { \
             detect::match_end::detect_outcome_text_with(&img, p4, &titles) };"
        ));
        assert_eq!(body.matches("detect::match_end::detect_outcome").count(), 2);
        let all = norm(src);
        assert!(all.contains(
            "detect::match_end::detect_outcome_signal_polled( &img, &rgb, &mut stability, \
             detect::match_end::FinalScoreSource::for_reader(use_new_reader), &titles, &words, );"
        ));
        for ocr_only in [
            "detect::match_end::detect_outcome(",
            "detect::match_end::detect_outcome_text(",
            "detect::match_end::detect_outcome_signal(",
            "detect::match_end::detect_outcome_signal_with_rgb(",
        ] {
            assert!(!all.contains(ocr_only), "{ocr_only}");
        }
    }

    fn accolade_read(name: &str) -> shadow::AccoladeRead {
        shadow::AccoladeRead::Map(shadow::TopBarMap {
            name: name.into(),
            mode: None,
            score: 0.95,
            margin: 0.3,
        })
    }

    /// `reader = "ocr-v1"` (the default): the accolade map is the title OCR,
    /// on a result-word tick only, and the map pack is never read.
    #[test]
    fn ocr_v1_reads_the_accolade_map_by_ocr_only() {
        use detect::match_end::OutcomeSource as S;
        let pack = || -> shadow::AccoladeRead { panic!("map pack read under ocr-v1") };
        let ocr = || Some("Busan".to_string());
        assert_eq!(
            choose_accolade_map(false, false, Some(S::ResultWord), pack, ocr).as_deref(),
            Some("Busan")
        );
        assert_eq!(
            choose_accolade_map(false, false, Some(S::ResultWord), pack, || None),
            None
        );
        // no result word on the tick, or Tab cell OCR in flight: nothing runs
        let no_ocr = || -> Option<String> { panic!("accolade OCR off a result-word tick") };
        for signal in [
            None,
            Some(S::Banner),
            Some(S::RankScreen),
            Some(S::EndTitle),
        ] {
            assert_eq!(
                choose_accolade_map(false, false, signal, pack, no_ocr),
                None
            );
        }
        assert_eq!(
            choose_accolade_map(false, true, Some(S::ResultWord), pack, no_ocr),
            None
        );
    }

    /// `reader = "new"` with a map pack: the accolade map is the pack's
    /// read, on the same ticks, and the OCR is never run, not even when the
    /// pack abstains.
    #[test]
    fn the_new_reader_reads_the_accolade_map_from_the_pack_and_never_by_ocr() {
        use detect::match_end::OutcomeSource as S;
        let ocr = || -> Option<String> { panic!("accolade OCR under reader = new") };
        assert_eq!(
            choose_accolade_map(
                true,
                false,
                Some(S::ResultWord),
                || accolade_read("Ilios"),
                ocr
            )
            .as_deref(),
            Some("Ilios")
        );
        // an abstain is no map: no fallback
        assert_eq!(
            choose_accolade_map(
                true,
                false,
                Some(S::ResultWord),
                || shadow::AccoladeRead::Abstain,
                ocr
            ),
            None
        );
        // the pack is read on a result-word tick only (a counter, not a
        // panic: a panic in the pack read is caught and would hide a call)
        let reads = std::cell::Cell::new(0);
        let counted = || {
            reads.set(reads.get() + 1);
            accolade_read("Ilios")
        };
        for signal in [
            None,
            Some(S::Banner),
            Some(S::RankScreen),
            Some(S::EndTitle),
        ] {
            assert_eq!(choose_accolade_map(true, false, signal, counted, ocr), None);
        }
        assert_eq!(
            choose_accolade_map(true, true, Some(S::ResultWord), counted, ocr),
            None
        );
        assert_eq!(
            reads.get(),
            0,
            "the map pack was read off a result-word tick"
        );
    }

    /// `reader = "new"` with no map pack loaded (F265-3): nothing read the
    /// frame, so the OCR reads it, as it does under ocr-v1 and as the top
    /// bar, the FINAL SCORE line and the phase titles do without their
    /// packs. Once per result-word tick, on those ticks only, and the
    /// warning is logged once per process.
    #[test]
    fn the_new_reader_reads_the_accolade_map_by_ocr_when_no_map_pack_is_loaded() {
        use detect::match_end::OutcomeSource as S;
        let no_pack = || shadow::AccoladeRead::NoPack;
        let calls = std::cell::Cell::new(0);
        let ocr = |map: Option<&str>| {
            let map = map.map(str::to_string);
            let calls = &calls;
            move || {
                calls.set(calls.get() + 1);
                map
            }
        };
        assert_eq!(
            choose_accolade_map(
                true,
                false,
                Some(S::ResultWord),
                no_pack,
                ocr(Some("Busan"))
            )
            .as_deref(),
            Some("Busan")
        );
        assert_eq!(calls.get(), 1, "one OCR read for the tick");
        // the OCR finds no map either: no map
        assert_eq!(
            choose_accolade_map(true, false, Some(S::ResultWord), no_pack, ocr(None)),
            None
        );
        assert_eq!(calls.get(), 2);
        // off a result-word tick, or with Tab cell OCR in flight: nothing runs
        for (outcome_only, signal) in [
            (false, None),
            (false, Some(S::Banner)),
            (false, Some(S::RankScreen)),
            (false, Some(S::EndTitle)),
            (true, Some(S::ResultWord)),
        ] {
            let pack = || -> shadow::AccoladeRead { panic!("pack read off a result-word tick") };
            assert_eq!(
                choose_accolade_map(true, outcome_only, signal, pack, ocr(Some("Busan"))),
                None
            );
        }
        assert_eq!(calls.get(), 2, "no OCR off a result-word tick");
        // the fallback above has logged its warning; it does not log again
        assert!(!warn_accolade_map_ocr_fallback());
        assert!(!warn_accolade_map_ocr_fallback());
    }

    /// A panic in the accolade map reader is an abstain: the tick keeps its
    /// result word (the poll closure does not unwind) and the OCR stays off.
    #[test]
    fn a_panicking_accolade_reader_is_no_map() {
        use detect::match_end::OutcomeSource as S;
        let map = choose_accolade_map(
            true,
            false,
            Some(S::ResultWord),
            || -> shadow::AccoladeRead { panic!("accolade reader") },
            || -> Option<String> { panic!("accolade OCR under reader = new") },
        );
        assert_eq!(map, None);
    }

    /// The poll tick reads the accolade map only through
    /// `choose_accolade_map`, with the daemon's reader switch, and the
    /// member-edit rule reaches `apply_poll_decision` with the same switch.
    #[test]
    fn the_poll_tick_chooses_the_accolade_map_with_the_reader_switch() {
        let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
        let src = include_str!("main.rs");
        let at = src.find("\nasync fn run_loop(").expect("run_loop");
        let body = norm(src[at..].split("\n}\n").next().unwrap());
        assert!(
            body.contains(
                "choose_accolade_map( use_new_reader, outcome_only, signal.map(|(_, source)| source), || shadow::read_accolade_map(&rgb), || detect::match_end::read_accolade_map(&img), );"
            ),
            "the accolade map goes through choose_accolade_map with the switch"
        );
        assert_eq!(
            body.matches("read_accolade_map").count(),
            2,
            "one call each"
        );
        assert!(body.contains("let use_new_reader = ctx.use_new_reader;"));
        assert!(body.contains("start_screen, now, new_game_debounce, use_new_reader, ) .await;"));
        let tick = src
            .find("\nasync fn apply_poll_tick(")
            .expect("apply_poll_tick");
        let tick_body = norm(src[tick..].split("\n}\n").next().unwrap());
        assert!(
            tick_body.contains(
                "apply_poll_decision(st, store, data_dir, decision, now, use_new_reader)"
            )
        );
    }

    async fn adopt_accolade_map(
        store: &storage::LocalStore,
        dir: &std::path::Path,
        session_id: &str,
        use_new_reader: bool,
    ) -> ActiveGame {
        adopt_accolade_map_over(store, dir, session_id, use_new_reader, None).await
    }

    /// One confirmed result-word tick that read `Ilios` off the accolade
    /// screen, on a session whose map is `fallback` (a text fallback) or
    /// not known.
    async fn adopt_accolade_map_over(
        store: &storage::LocalStore,
        dir: &std::path::Path,
        session_id: &str,
        use_new_reader: bool,
        fallback: Option<&str>,
    ) -> ActiveGame {
        let now = Instant::now();
        let mut g = game(detect::MatchOutcome::Unknown, None, now);
        g.session_id = session_id.into();
        g.session_created = true;
        g.map = fallback.map(str::to_string);
        g.map_source = fallback.map(|_| boundary::MapSource::TextFallback);
        let mut st = session(Some(g), now);
        let decision = {
            let g = st.active_game.as_ref().unwrap();
            let tick = ResolvedWordTick {
                signal: Some(detect::MatchOutcome::Victory),
                signal_confirmed: true,
                accolade_map: Some("Ilios".into()),
            };
            boundary::decide_poll(&poll_input_from_game(
                g,
                tick.signal,
                tick.signal_confirmed,
                tick.accolade_map.as_deref(),
                None,
                false,
                now,
            ))
        };
        apply_poll_decision(&mut st, store, dir, decision, now, use_new_reader).await;
        st.active_game.expect("the accolade stays on this session")
    }

    /// A game with two boards, a member's map pick (`King's Row`) stamped
    /// on both, and a third board captured after the pick with the
    /// session's map (`later_map`): the pick is not carried to later rows.
    async fn insert_game_with_a_mid_game_pick(
        store: &storage::LocalStore,
        session_id: &str,
        later_map: &str,
    ) {
        let t0 = Utc::now() - chrono::Duration::minutes(10);
        let row = |minute: i64, map: &str| {
            let mut row = test_match(session_id, "unknown");
            row.map_name = map.into();
            row.played_at = SurrealDatetime::from(t0 + chrono::Duration::minutes(minute));
            row
        };
        store.insert_match(row(0, later_map)).await.unwrap();
        store.insert_match(row(1, later_map)).await.unwrap();
        store
            .edit_match(
                session_id,
                &storage::MatchEdit {
                    map_name: Some("King's Row".into()),
                    ..storage::MatchEdit::default()
                },
            )
            .await
            .unwrap();
        store.insert_match(row(2, later_map)).await.unwrap();
        let rows = store.get_session_snapshots(session_id).await.unwrap();
        assert_eq!(
            rows.iter()
                .map(|r| r.has_member_map_edit())
                .collect::<Vec<_>>(),
            [true, true, false],
            "the pick is on the rows that existed when it was made"
        );
    }

    /// `reader = "new"`: the accolade map never renames a game whose map a
    /// member chose, and never rewrites the mode stored with that choice.
    /// The session still takes the accolade map for its boundary checks.
    #[tokio::test]
    async fn the_accolade_map_never_renames_a_member_map_pick_under_the_new_reader() {
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        let mut row = test_match("picked", "unknown");
        row.map_name = String::new();
        store.insert_match(row).await.unwrap();
        store
            .edit_match(
                "picked",
                &storage::MatchEdit {
                    map_name: Some("King's Row".into()),
                    ..storage::MatchEdit::default()
                },
            )
            .await
            .unwrap();
        let before = store.get_session_snapshots("picked").await.unwrap();
        assert_eq!(before[0].game_mode, "Hybrid");

        let g = adopt_accolade_map(&store, dir.path(), "picked", true).await;
        assert_eq!(g.map.as_deref(), Some("Ilios"));
        assert_eq!(g.map_source, Some(boundary::MapSource::Accolade));
        let rows = store.get_session_snapshots("picked").await.unwrap();
        assert_eq!(
            rows[0].map_name, "",
            "the read under the pick is not renamed"
        );
        assert_eq!(rows[0].display_map_name(), "King's Row");
        assert_eq!(rows[0].game_mode, "Hybrid", "the pick's mode is kept");
        // the outcome of the same tick is still written
        assert_eq!(rows[0].outcome, "victory");
    }

    /// The member-pick rule is per row (F265-1). A pick made mid-game is on
    /// the rows that existed then; the board captured after it, the game's
    /// newest row, has none. Under `reader = "new"` the accolade map is
    /// written to that row, whether it held nothing or a text fallback, and
    /// the picked rows keep their read and their mode. One picked row does
    /// not stop the write for the session.
    #[tokio::test]
    async fn the_accolade_map_is_written_to_the_rows_a_member_did_not_choose() {
        for later_map in ["", "Busan"] {
            let dir = tempfile::tempdir().unwrap();
            let store = storage::LocalStore::open(dir.path()).await.unwrap();
            insert_game_with_a_mid_game_pick(&store, "mixed", later_map).await;
            for row in store.get_session_snapshots("mixed").await.unwrap() {
                storage::append_match_log(dir.path(), &row);
            }
            let fallback = (!later_map.is_empty()).then_some(later_map);
            let g = adopt_accolade_map_over(&store, dir.path(), "mixed", true, fallback).await;
            assert_eq!(
                g.map.as_deref(),
                Some("Ilios"),
                "session map over {later_map:?}"
            );
            assert_eq!(g.map_source, Some(boundary::MapSource::Accolade));

            let rows = store.get_session_snapshots("mixed").await.unwrap();
            for picked in &rows[..2] {
                assert_eq!(picked.map_name, later_map, "the read under the pick stays");
                assert_eq!(picked.display_map_name(), "King's Row");
                assert_eq!(picked.game_mode, "Hybrid", "the pick's mode is kept");
            }
            let newest = &rows[2];
            assert_eq!(
                newest.map_name, "Ilios",
                "the row after the pick, over {later_map:?}"
            );
            assert_eq!(newest.game_mode, "Control");
            assert_eq!(newest.display_map_name(), "Ilios");
            // the game's row is the newest one: it shows the accolade map
            let shown = storage::latest_per_game(rows.iter().rev().cloned().collect());
            assert_eq!(shown.len(), 1);
            assert_eq!(shown[0].display_map_name(), "Ilios");
            // the outcome of the same tick is written to every row
            assert!(rows.iter().all(|r| r.outcome == "victory"));
            // the match log is rewritten the same way, row for row
            let log = std::fs::read_to_string(dir.path().join("matches.jsonl")).unwrap();
            let logged: Vec<storage::PersonalMatch> = log
                .lines()
                .map(|l| serde_json::from_str(l).unwrap())
                .collect();
            assert_eq!(
                logged
                    .iter()
                    .map(|m| (m.map_name.as_str(), m.game_mode.as_str()))
                    .collect::<Vec<_>>(),
                [
                    (later_map, "Hybrid"),
                    (later_map, "Hybrid"),
                    ("Ilios", "Control")
                ]
            );
        }
        // ocr-v1 is unchanged: every row is written, the picked ones too
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        insert_game_with_a_mid_game_pick(&store, "mixed-v1", "").await;
        adopt_accolade_map(&store, dir.path(), "mixed-v1", false).await;
        let rows = store.get_session_snapshots("mixed-v1").await.unwrap();
        assert!(
            rows.iter()
                .all(|r| r.map_name == "Ilios" && r.game_mode == "Control")
        );
    }

    /// The per-row write itself: every row without a member's map choice
    /// takes the map, with the mode and the sync queue of `set_session_map`;
    /// a session with no choice at all is written by `set_session_map`.
    #[tokio::test]
    async fn set_session_map_keeping_member_choices_writes_the_other_rows() {
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        insert_game_with_a_mid_game_pick(&store, "kept", "").await;
        let before = store.get_session_snapshots("kept").await.unwrap();
        let kept = store
            .set_session_map_keeping_member_choices("kept", "Ilios")
            .await
            .unwrap();
        assert_eq!(kept, 2);
        let rows = store.get_session_snapshots("kept").await.unwrap();
        for (was, now) in before.iter().zip(&rows).take(2) {
            assert_eq!(
                (&now.map_name, &now.game_mode, now.sync_rev),
                (&was.map_name, &was.game_mode, was.sync_rev),
                "a picked row is not touched"
            );
        }
        assert_eq!(
            (rows[2].map_name.as_str(), rows[2].game_mode.as_str()),
            ("Ilios", "Control")
        );
        assert_eq!(rows[2].sync_rev, before[2].sync_rev + 1);
        // a second write of the same map changes nothing
        store
            .set_session_map_keeping_member_choices("kept", "Ilios")
            .await
            .unwrap();
        let again = store.get_session_snapshots("kept").await.unwrap();
        assert_eq!(again[2].sync_rev, rows[2].sync_rev);

        // no choice in the session: every row, as `set_session_map`
        let mut plain = test_match("plain", "unknown");
        plain.map_name = String::new();
        store.insert_match(plain).await.unwrap();
        assert_eq!(
            store
                .set_session_map_keeping_member_choices("plain", "Ilios")
                .await
                .unwrap(),
            0
        );
        let rows = store.get_session_snapshots("plain").await.unwrap();
        assert_eq!(
            (rows[0].map_name.as_str(), rows[0].game_mode.as_str()),
            ("Ilios", "Control")
        );
    }

    /// Without a member's pick the accolade map is written under both
    /// readers, and `reader = "ocr-v1"` writes it over a pick as before.
    #[tokio::test]
    async fn the_accolade_map_is_written_when_no_member_chose_the_map() {
        for use_new_reader in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let store = storage::LocalStore::open(dir.path()).await.unwrap();
            let mut row = test_match("unpicked", "unknown");
            row.map_name = String::new();
            store.insert_match(row).await.unwrap();
            let g = adopt_accolade_map(&store, dir.path(), "unpicked", use_new_reader).await;
            assert_eq!(g.map.as_deref(), Some("Ilios"));
            let rows = store.get_session_snapshots("unpicked").await.unwrap();
            assert_eq!(rows[0].map_name, "Ilios", "reader new = {use_new_reader}");
            assert_eq!(rows[0].game_mode, "Control");
        }
        // ocr-v1 is unchanged: the accolade map is written under a pick too
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        let mut row = test_match("picked-v1", "unknown");
        row.map_name = String::new();
        store.insert_match(row).await.unwrap();
        store
            .edit_match(
                "picked-v1",
                &storage::MatchEdit {
                    map_name: Some("King's Row".into()),
                    ..storage::MatchEdit::default()
                },
            )
            .await
            .unwrap();
        adopt_accolade_map(&store, dir.path(), "picked-v1", false).await;
        let rows = store.get_session_snapshots("picked-v1").await.unwrap();
        assert_eq!(rows[0].map_name, "Ilios");
        assert_eq!(rows[0].game_mode, "Control");
        assert_eq!(rows[0].display_map_name(), "King's Row");
    }

    /// The end title (P3) and the end-of-match wake (P5) go through the
    /// reader switch (alpha.7 O2): on the poll tick and in the Tab frame's
    /// vision pass. Each reads the frame's titles once, and no daemon call
    /// site uses the wake entry point that is hard-wired to OCR. Every
    /// broken wire is named. Whitespace is ignored.
    #[test]
    fn the_end_titles_go_through_the_reader_switch() {
        let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
        let full = include_str!("main.rs");
        let src = full.split("\nmod tests {").next().unwrap();
        let at = src.find("\nfn analyze_frame(").expect("analyze_frame");
        let body = norm(src[at..].split("\n}\n").next().unwrap());
        let all = norm(src);
        let checks = [
            (
                "the Tab pass makes its titles from the switch",
                body.contains(
                    "let titles = detect::match_end::Titles::for_frame(use_new_reader, &img);",
                ),
            ),
            (
                "the Tab pass hands them to both outcome reads",
                body.contains("detect::match_end::detect_outcome_with(&img, p4, &titles, &words)")
                    && body
                        .contains("detect::match_end::detect_outcome_text_with(&img, p4, &titles)"),
            ),
            (
                "the poll tick makes its titles from the switch",
                all.contains(
                    "let titles = detect::match_end::Titles::for_rgb(use_new_reader, &rgb);",
                ),
            ),
            (
                "the poll tick hands them to the outcome signal",
                all.contains(
                    "detect::match_end::detect_outcome_signal_polled( &img, &rgb, &mut stability, \
                     detect::match_end::FinalScoreSource::for_reader(use_new_reader), &titles, \
                     &words, );",
                ),
            ),
            (
                "the poll tick hands them to the wake",
                all.contains("detect::match_end::detect_end_reel_with(&img, &rgb, &titles);")
                    && !all.contains("detect::match_end::detect_end_reel("),
            ),
            (
                "titles are made in those two places only",
                all.matches("detect::match_end::Titles::").count() == 2
                    && !all.contains("shadow::read_end_title"),
            ),
        ];
        let broken: Vec<&str> = checks.iter().filter(|c| !c.1).map(|c| c.0).collect();
        assert!(broken.is_empty(), "{broken:?}");
    }

    /// The accolade and rank-screen result words (P1, P2) go through the
    /// reader switch (alpha.7 O11): on the poll tick and in the Tab frame's
    /// vision pass. Each makes its `ResultWords` from the switch and hands
    /// it to the outcome read, and the poll tick gives the read it took to
    /// the result evidence. Every broken wire is named. Whitespace is
    /// ignored.
    #[test]
    fn the_result_words_go_through_the_reader_switch() {
        let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
        let full = include_str!("main.rs");
        let src = full.split("\nmod tests {").next().unwrap();
        let at = src.find("\nfn analyze_frame(").expect("analyze_frame");
        let body = norm(src[at..].split("\n}\n").next().unwrap());
        let all = norm(src);
        // everything but the Tab pass
        let rest = all.replacen(body.as_str(), "", 1);
        assert!(rest.len() < all.len(), "the Tab pass is part of the source");
        let from_switch = "let words = detect::match_end::ResultWords::for_reader(use_new_reader);";
        let checks = [
            (
                "the Tab pass makes its result words from the switch",
                body.matches(from_switch).count() == 1,
            ),
            (
                "the Tab pass hands them to the outcome read",
                body.contains("detect::match_end::detect_outcome_with(&img, p4, &titles, &words)"),
            ),
            (
                "the poll tick makes its result words from the switch",
                rest.matches(from_switch).count() == 1,
            ),
            (
                "the poll tick hands them to the outcome signal",
                all.contains(
                    "detect::match_end::detect_outcome_signal_polled( &img, &rgb, &mut stability, \
                     detect::match_end::FinalScoreSource::for_reader(use_new_reader), &titles, \
                     &words, );",
                ),
            ),
            (
                "the poll tick's result evidence is the read the signal took",
                all.contains(
                    "let result_read = tick_result_read( use_new_reader, signal.is_some(), \
                     words.into_taken(), || shadow::read_result(&img), );",
                ),
            ),
            (
                "result words are made in those two places only",
                all.matches("ResultWords::").count() == 2,
            ),
            (
                "the frame's result is read in that one place only",
                all.matches("shadow::read_result(").count() == 1
                    && !all.contains("shadow::read_result_screens"),
            ),
        ];
        let broken: Vec<&str> = checks.iter().filter(|c| !c.1).map(|c| c.0).collect();
        assert!(broken.is_empty(), "{broken:?}");
    }

    /// One words-pack read per poll tick: the read the signal detection took
    /// is the tick's result evidence, and the frame is not read again. A
    /// tick whose signal came without one (colour flood, end title, an OCR
    /// word) reads the frame as before. No signal, or `reader = "ocr-v1"`:
    /// no evidence and no read.
    #[test]
    fn a_poll_tick_reads_the_frames_result_words_once() {
        use shadow::result::{ACCOLADE, Outcome, RANK_SCREEN, ResultRead};
        let read_of = |source: &str| ResultRead {
            outcome: Some(Outcome::Victory),
            source: Some(source.into()),
            score: 0.9,
            margin: 0.4,
            suspect: false,
        };
        let no_second_read = || -> ResultRead { panic!("the frame was read a second time") };
        let no_read = || -> ResultRead { panic!("the frame was read with no evidence due") };
        // the signal's own read is the evidence
        assert_eq!(
            tick_result_read(true, true, Some(read_of(ACCOLADE)), no_second_read),
            Some(read_of(ACCOLADE))
        );
        // no read was taken: the frame is read here, once
        let reads = std::cell::Cell::new(0);
        let counted = || {
            reads.set(reads.get() + 1);
            read_of(RANK_SCREEN)
        };
        assert_eq!(
            tick_result_read(true, true, None, counted),
            Some(read_of(RANK_SCREEN))
        );
        assert_eq!(reads.get(), 1);
        // no signal: nothing is added, whether or not a read was taken
        assert_eq!(tick_result_read(true, false, None, no_read), None);
        assert_eq!(
            tick_result_read(true, false, Some(read_of(ACCOLADE)), no_read),
            None
        );
        // ocr-v1: never
        assert_eq!(tick_result_read(false, true, None, no_read), None);
        assert_eq!(
            tick_result_read(false, true, Some(read_of(ACCOLADE)), no_read),
            None
        );
        assert_eq!(tick_result_read(false, false, None, no_read), None);
    }

    /// The accolade map read still starts from a result-word tick under
    /// `reader = "new"` (alpha.7 O11 with O5). The new reader's accolade
    /// signal carries the source ocr-v1's does (`OutcomeSource::ResultWord`;
    /// see `a_sure_words_read_is_the_result_word_signal` in
    /// `detect::match_end` and `tests/result_signal_wiring.rs`). The poll
    /// tick hands the source of its own signal to `choose_accolade_map`, and
    /// that source alone decides whether the map pack is read: a rank-screen
    /// word, an end title, a colour flood and no signal read no map.
    #[test]
    fn a_result_word_tick_still_starts_the_accolade_map_read() {
        use detect::match_end::OutcomeSource as S;
        // The tick's own signal, made with the reader switch's result
        // words, is what the map read is keyed on. Whitespace is ignored.
        let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
        let full = include_str!("main.rs");
        let all = norm(full.split("\nmod tests {").next().unwrap());
        let signal_at = all
            .find("let signal = detect::match_end::detect_outcome_signal_polled(")
            .expect("the poll tick's signal");
        let to_map = all[signal_at..]
            .find(
                "let accolade_map = choose_accolade_map( use_new_reader, outcome_only, \
                 signal.map(|(_, source)| source),",
            )
            .expect("the poll tick's accolade map follows its signal and takes its source");
        assert!(all[signal_at..signal_at + to_map].contains("&titles, &words, );"));

        // A result-word signal under the new reader reads the map pack.
        let ocr = || -> Option<String> { panic!("accolade OCR with a map pack loaded") };
        assert_eq!(
            choose_accolade_map(
                true,
                false,
                Some(S::ResultWord),
                || accolade_read("Ilios"),
                ocr
            )
            .as_deref(),
            Some("Ilios")
        );
        // No other source does (a counter, not a panic: a panic in the pack
        // read is caught and would hide a call).
        let reads = std::cell::Cell::new(0);
        for source in [
            None,
            Some(S::RankScreen),
            Some(S::EndTitle),
            Some(S::Banner),
        ] {
            let counted = || {
                reads.set(reads.get() + 1);
                accolade_read("Ilios")
            };
            assert_eq!(choose_accolade_map(true, false, source, counted, ocr), None);
        }
        assert_eq!(
            reads.get(),
            0,
            "the map pack was read off a result-word tick"
        );
    }

    /// `reader = "new"` on the poll tick (F263-2, F263-6): a sure end title
    /// is nothing but a wake on its first poll, one unconfirmed word (a
    /// hint) on its second, and confirmed on its third, the polls ocr-v1
    /// needs. One sure read of one frame never reaches the session machine.
    #[test]
    fn a_sure_end_title_is_a_word_on_its_second_poll_and_confirms_on_its_third() {
        use detect::match_end::{FinalScoreSource, OutcomeSource, Titles};
        use shadow::result::{END_TITLE, Outcome, ResultRead};
        let rgb = image::RgbImage::from_pixel(640, 360, image::Rgb([10, 10, 10]));
        let img = image::DynamicImage::ImageRgb8(rgb.clone());
        let titles = Titles::Words(ResultRead {
            outcome: Some(Outcome::Defeat),
            source: Some(END_TITLE.to_string()),
            score: 0.9,
            margin: 0.4,
            suspect: false,
        });
        let mut stability = detect::stability::FrameStability::default();
        let mut streak = None;
        let t0 = Instant::now();
        let mut ticks = Vec::new();
        for n in 0..3 {
            let signal = detect::match_end::detect_outcome_signal_polled(
                &img,
                &rgb,
                &mut stability,
                FinalScoreSource::Words,
                &titles,
                &detect::match_end::ResultWords::for_reader(true),
            );
            let wake = detect::match_end::detect_end_reel_with(&img, &rgb, &titles);
            let tick =
                resolve_word_tick(&mut streak, signal, t0 + Duration::from_secs(4 * n), None);
            ticks.push((
                wake,
                signal.map(|s| s.1),
                tick.signal,
                tick.signal_confirmed,
            ));
        }
        let defeat = Some(detect::MatchOutcome::Defeat);
        let title = Some(OutcomeSource::EndTitle);
        assert_eq!(
            ticks,
            vec![
                (true, None, None, false),
                (true, title, defeat, false),
                (true, title, defeat, true),
            ]
        );
    }

    /// The poll tick confirms the vote, ban and select screens with the
    /// source the reader switch picks (alpha.7 O7), and calls no ocr-v1
    /// only phase read. Whitespace is ignored.
    #[test]
    fn the_poll_tick_takes_the_phase_source_from_the_reader_switch() {
        let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
        let full = include_str!("main.rs");
        let src = norm(full.split("\nmod tests {").next().unwrap());
        assert!(src.contains(
            "detect::match_start::detect_phase_polled( &img, &rgb, &mut stability, \
             detect::match_start::PhaseSource::for_reader(use_new_reader), )"
        ));
        for ocr_only in [
            "detect::match_start::detect_phase(",
            "detect::match_start::detect_phase_with_rgb(",
            "PhaseSource::Ocr",
        ] {
            assert!(!src.contains(ocr_only), "{ocr_only}");
        }
    }

    /// The vision pass reads the banner only through `choose_top_bar_map`,
    /// with the daemon's reader switch, and hands the read to the board
    /// read. Whitespace is ignored, so a reflow does not break this;
    /// hard-wiring the switch or a second banner read does.
    #[test]
    fn the_vision_pass_chooses_the_top_bar_map_with_the_reader_switch() {
        let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
        let src = include_str!("main.rs");
        let at = src.find("\nfn analyze_frame(").expect("analyze_frame");
        let body = norm(src[at..].split("\n}\n").next().unwrap());
        assert!(
            body.contains("choose_top_bar_map( use_new_reader, || shadow::read_banner(&img),"),
            "the top-bar map goes through choose_top_bar_map with the switch"
        );
        assert_eq!(
            body.matches("shadow::read_banner").count(),
            1,
            "one banner read"
        );
        // ...one hero icon read (O8) and one board read (O10), which reuses
        // both instead of reading again: the only other shadow reads
        assert_eq!(
            body.matches("shadow::read_heroes").count(),
            1,
            "one hero read"
        );
        assert!(body.contains(
            "let board = read_tab_board(use_new_reader, || { shadow::read_board_with(&img, top_bar.banner, tab_heroes) });"
        ));
        assert_eq!(body.matches("shadow::read_").count(), 3);
        assert!(body.contains("map_from_banner: top_bar.from_banner,"));
        let hc = src
            .find("\nasync fn handle_capture(")
            .expect("handle_capture");
        let hc_body = norm(src[hc..].split("\n}\n").next().unwrap());
        assert!(hc_body.contains("use_new_reader: ctx.use_new_reader,"));
        assert!(hc_body.contains(
            "if ctx.use_new_reader { req.member_map_edit = session_has_member_map_edit(&ctx.store, &req).await; }"
        ));
        assert!(hc_body.contains("panel_from_banner: map_from_banner,"));
        assert!(
            hc_body
                .contains("apply_new_reader( &mut parsed, &req.result_evidence, board_row, own_row_suspect, board, pack_hero.as_deref(), );")
        );
        // the stored report carries the text-fallback replacement
        assert!(hc_body.contains("map_replaces_text_fallback: staged.replaced_text_fallback,"));
        // apply_new_reader reads nothing off the frame: it has no frame
        let an = src
            .find("\nfn apply_new_reader(")
            .expect("apply_new_reader");
        let an_body = norm(src[an..].split("\n}\n").next().unwrap());
        assert_eq!(an_body.matches("shadow::read_").count(), 0);
        assert_eq!(an_body.matches("ocr::").count(), 0);
    }

    /// T5 (O8): the vision pass reads the hero authority only through
    /// `read_own_hero` with the daemon's reader switch, and `handle_capture`
    /// resolves it with the switch. Hard-wiring either, or reading the career
    /// panel outside `read_own_hero`, breaks this.
    #[test]
    fn the_hero_authority_goes_through_the_reader_switch() {
        let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
        let src = include_str!("main.rs");
        let at = src.find("\nfn analyze_frame(").expect("analyze_frame");
        let body = norm(src[at..].split("\n}\n").next().unwrap());
        assert!(body.contains("} = read_own_hero( use_new_reader, || {"));
        assert_eq!(
            body.matches("crop_career_hero").count(),
            1,
            "one career read"
        );
        // the icons are read once, behind the switch, before the own row
        // (O9b: the career hero's row is one of its voices), and that one
        // read is the hero authority
        assert!(body.contains(
            "let tab_heroes = read_tab_heroes(use_new_reader, || { shadow::read_heroes(&img, &scoreboard, team_size) });"
        ));
        assert!(body.contains("tab_heroes, row_idx, );"));
        assert!(
            body.find("read_tab_heroes(").unwrap() < body.find("} = find_own_row(").unwrap(),
            "the icons are read before the own row"
        );
        assert!(body.contains("career_hero, pack_hero, tab_heroes,"));
        let hc = src
            .find("\nasync fn handle_capture(")
            .expect("handle_capture");
        let hc_body = norm(src[hc..].split("\n}\n").next().unwrap());
        assert!(hc_body.contains(
            "if ctx.use_new_reader { hero_auth::resolve_hero_new_reader( pack_hero.as_deref(), portrait, &hero_auth, parse::canonical_hero, ) } else { hero_auth::resolve_hero( career_hero.as_deref(), portrait, &parsed.hero, &hero_auth, parse::canonical_hero, ) };"
        ));
    }

    fn request_with_text_fallback() -> (ActiveGame, Instant) {
        let now = test_now();
        let mut g =
            ActiveGame::open_at("s1".into(), detect::MatchOutcome::Unknown, Vec::new(), now);
        g.map = Some("Busan".into());
        g.map_source = Some(boundary::MapSource::TextFallback);
        // a board of this game was accepted just now
        g.gate = Some(GateState::default());
        g.last_stats_at = Some(now);
        (g, now)
    }

    fn banner_facts(map: &str, from_banner: bool) -> BoardFacts<'_> {
        BoardFacts {
            counters: Counters::default(),
            suspect: NIGHT_CLEAN,
            trusted_cells: true,
            held: [false; capture_gate::GATE_COLS],
            suspect_row: false,
            row_id: Some(2),
            hero: "Ana",
            map_from_panel: Some(map),
            panel_from_banner: from_banner,
            parsed_map: "",
            frame_outcome: detect::MatchOutcome::Unknown,
        }
    }

    /// F251-1: the rule for a sure banner read against the session map.
    #[test]
    fn a_banner_read_replaces_only_an_open_text_fallback_map() {
        let (g, now) = request_with_text_fallback();
        let req = build_capture_request(&g, false, now);
        let ilios = banner_facts("Ilios", true);
        assert_eq!(
            banner_replaces_text_fallback(&req, &ilios, false).as_deref(),
            Some("Ilios")
        );
        // the top-bar OCR, the same map, and a split never replace
        assert_eq!(
            banner_replaces_text_fallback(&req, &banner_facts("Ilios", false), false),
            None
        );
        assert_eq!(
            banner_replaces_text_fallback(&req, &banner_facts("busan", true), false),
            None
        );
        assert_eq!(banner_replaces_text_fallback(&req, &ilios, true), None);
        // a source-less map counts as text, like ActiveGame::map_is_trusted
        let mut no_source = request_with_text_fallback().0;
        no_source.map_source = None;
        let req_none = build_capture_request(&no_source, false, now);
        assert!(banner_replaces_text_fallback(&req_none, &ilios, false).is_some());

        type Edit = fn(&mut ActiveGame);
        let closed: [(&str, Edit); 9] = [
            ("top bar", |g| {
                g.map_source = Some(boundary::MapSource::TopBar)
            }),
            ("accolade", |g| {
                g.map_source = Some(boundary::MapSource::Accolade)
            }),
            ("outcome", |g| g.outcome = detect::MatchOutcome::Victory),
            ("pending boundary", |g| g.pending_boundary = true),
            ("lock", |g| g.text_fallback_locked = true),
            ("reset streak", |g| g.reset_streak = 1),
            ("carried board", |g| g.deferred = Some(Counters::default())),
            ("candidates", |g| {
                g.map_candidates = vec!["Oasis".into(), "Busan".into()]
            }),
            ("hint", |g| {
                g.result_mark = Some(boundary::ResultMark {
                    outcome: detect::MatchOutcome::Defeat,
                    confirmed: false,
                    seen_at: test_now(),
                })
            }),
        ];
        for (what, edit) in closed {
            let mut h = request_with_text_fallback().0;
            edit(&mut h);
            let req = build_capture_request(&h, false, now);
            assert_eq!(
                banner_replaces_text_fallback(&req, &ilios, false),
                None,
                "{what}"
            );
        }
        let mut voted = request_with_text_fallback().0;
        voted.map_candidates = vec!["Ilios".into(), "Busan".into()];
        let req = build_capture_request(&voted, false, now);
        assert!(
            banner_replaces_text_fallback(&req, &ilios, false).is_some(),
            "a candidate passes"
        );

        // F252-3: a member's own map choice is never renamed
        let mut edited = build_capture_request(&request_with_text_fallback().0, false, now);
        edited.member_map_edit = true;
        assert_eq!(banner_replaces_text_fallback(&edited, &ilios, false), None);

        // F252-2: the last board must be at most STAT_SPLIT_MIN_GAP old
        // the game's own clock: test_now() moves between calls
        let (g, at) = request_with_text_fallback();
        let at_bound = build_capture_request(&g, false, at + STAT_SPLIT_MIN_GAP);
        assert!(banner_replaces_text_fallback(&at_bound, &ilios, false).is_some());
        let late = build_capture_request(
            &g,
            false,
            at + STAT_SPLIT_MIN_GAP + Duration::from_millis(1),
        );
        assert_eq!(
            banner_replaces_text_fallback(&late, &ilios, false),
            None,
            "gap"
        );
        let mut no_board = request_with_text_fallback().0;
        no_board.gate = None;
        no_board.last_stats_at = None;
        let req = build_capture_request(&no_board, false, now);
        assert_eq!(
            banner_replaces_text_fallback(&req, &ilios, false),
            None,
            "no board"
        );
    }

    #[test]
    fn vote_candidates_constrain_but_never_become_the_map() {
        let candidates = vec!["Oasis".to_string(), "Busan".to_string()];

        // No trusted read yet: candidates alone must NOT pick a map — the
        // vote winner is unknowable, and detected_maps[0] used to be wrong
        // ~2/3 of the time.
        assert_eq!(resolve_map(None, None, "", &candidates), "");

        // Session's confirmed map always wins.
        assert_eq!(
            resolve_map(Some("Busan"), Some("Oasis"), "Ilios", &candidates),
            "Busan"
        );

        // Panel read accepted when it is a candidate...
        assert_eq!(resolve_map(None, Some("Busan"), "", &candidates), "Busan");
        // ...vetoed when it isn't (misread), falling back to a plausible
        // text read, or to nothing.
        assert_eq!(
            resolve_map(None, Some("Ilios"), "Oasis", &candidates),
            "Oasis"
        );
        assert_eq!(resolve_map(None, Some("Ilios"), "Nepal", &candidates), "");

        // Without candidates, panel > text (unchanged behavior).
        assert_eq!(resolve_map(None, Some("Ilios"), "Nepal", &[]), "Ilios");
        assert_eq!(resolve_map(None, None, "Nepal", &[]), "Nepal");

        let (name, source) = resolved_map(Some("Busan"), Some("Busan"), "Ilios", &[]);
        assert_eq!(name, "Busan");
        assert_eq!(
            source,
            Some(boundary::MapSource::TopBar),
            "a later top bar that agrees upgrades the stored source"
        );
        assert_eq!(
            resolved_map(Some("Busan"), None, "Busan", &[]).1,
            None,
            "full-board text that agrees does not upgrade the source"
        );
    }

    // Transitions below are real (elims, deaths, damage) sequences from the
    // 2026-07-14 session-merge incident (matches.jsonl), where three games
    // were appended to one session because the poller missed every boundary.
    const CLEAN3: (bool, bool, bool) = (false, false, false);

    /// Legacy-shape helper: no active CG-2 latch (so `last_raw == accepted`) and
    /// all reads clean. Under those conditions the raw-continuity vote reduces to
    /// the original accepted-drop check, so these fixtures assert unchanged.
    fn regressed(prev: (u32, u32, u32), cur: (u32, u32, u32)) -> bool {
        stats_regressed(prev, prev, cur, CLEAN3, CLEAN3)
    }

    #[test]
    fn stat_regression_detects_real_game_boundaries() {
        // Colosseo end -> Neon Junction first capture (E11->0, DMG 2740->0;
        // deaths 0->0 is NOT strictly lower — two drops still suffice).
        assert!(regressed((11, 0, 2740), (0, 0, 0)));
        // Neon Junction end -> Dorado first capture: all three drop.
        assert!(regressed((34, 10, 11742), (1, 0, 254)));
    }

    #[test]
    fn stat_regression_ignores_single_field_misreads() {
        // Garbage OCR row mid-game (E29->9 misread, deaths/damage rose):
        // one drop must not split the session.
        assert!(!regressed((29, 5, 9242), (9, 11, 61029)));
        // Inflated elims read (E91) settling back next capture, deaths and
        // damage unchanged: still only one drop.
        assert!(!regressed((91, 3, 9072), (15, 3, 9072)));
        // Normal mid-game progression never regresses.
        assert!(!regressed((5, 1, 1271), (8, 2, 5966)));
        // Identical re-capture of the same board (post-match Tab spam).
        assert!(!regressed((30, 5, 10352), (30, 5, 10352)));
    }

    #[test]
    fn stat_regression_after_garbage_row_needs_the_gap_guard() {
        // A garbage row (E9 D11 DMG61029) followed by the next real capture
        // DOES look like a regression — the STAT_SPLIT_MIN_GAP guard is what
        // prevents this from splitting (captures were 86s apart, gap is 120s).
        assert!(regressed((9, 11, 61029), (3, 5, 10865)));
        assert!(STAT_SPLIT_MIN_GAP > Duration::from_secs(86));
    }

    #[test]
    fn dup1_suspect_columns_do_not_vote_for_a_split() {
        // CG-2 latch: `accepted` sits inflated (E latched 40, DMG latched 35031),
        // so the real reads (E10, DMG5435) look like a two-column regression and
        // would split a NEW session mid-game (Rialto 2026-07-20). Here last_raw
        // also sits high (=accepted), so only the current-suspect strip protects.
        let prev = (40, 4, 35031);
        let cur = (10, 4, 5435);
        assert!(
            stats_regressed(prev, prev, cur, CLEAN3, CLEAN3),
            "with last_raw high and all clean the latched inflation fakes a boundary"
        );
        assert!(
            !stats_regressed(prev, prev, cur, (true, false, true), CLEAN3),
            "suspect E + DMG drops must not vote: no mid-game split"
        );
    }

    #[test]
    fn dup1_genuine_new_game_still_splits_on_clean_resets() {
        // A real new game resets every counter and reads them CLEAN, so the guard
        // leaves the boundary fully detectable: E34->1, D10->0, DMG11742->254.
        assert!(
            regressed((34, 10, 11742), (1, 0, 254)),
            "clean reset reads still trip the split"
        );
        // Even if ONE reset column happened to be suspect, the other two clean
        // drops still carry the 2-of-3 vote.
        assert!(
            stats_regressed(
                (34, 10, 11742),
                (34, 10, 11742),
                (1, 0, 254),
                (false, false, true),
                CLEAN3
            ),
            "one suspect reset column does not block a genuine boundary"
        );
    }

    // --- F-CG2-1: raw-continuity split vote (latch-recovery reads are clean) ---

    #[test]
    fn fcg2_latch_recovery_reads_do_not_split_midgame() {
        // The Rialto-class residual: `accepted` is CG-2-latched high (E40,
        // DMG35031) while `last_raw` tracks reality (E10, DMG10311 from the last
        // real capture). After the ≥120s gap the first clean read (E10,
        // DMG10500) drops versus the latched `accepted` but is CONTINUOUS with
        // last_raw — the un-latch streak is still 0, yet this must NOT split.
        let prev_acc = (40, 4, 35031);
        let prev_raw = (10, 4, 10311);
        let cur = (10, 5, 10500);
        assert!(
            !stats_regressed(prev_acc, prev_raw, cur, CLEAN3, CLEAN3),
            "clean reads continuous with last_raw must not vote for a split"
        );
    }

    #[test]
    fn fcg2_genuine_reset_drops_versus_both_and_splits() {
        // Same latched state, but a genuine new game resets every counter to ~0 —
        // which drops versus BOTH accepted AND last_raw, so the boundary trips.
        let prev_acc = (40, 4, 35031);
        let prev_raw = (10, 4, 10311);
        let cur = (0, 0, 200);
        assert!(
            stats_regressed(prev_acc, prev_raw, cur, CLEAN3, CLEAN3),
            "a real reset drops versus last_raw too and still splits"
        );
    }

    #[test]
    fn fcg2_suspect_prev_raw_falls_back_to_accepted_drop_alone() {
        // When last_raw for a column is itself suspect, it is not a trustworthy
        // continuity anchor, so that column falls back to the DUP-1 rule: an
        // accepted-drop alone may vote. Here E and DMG prev_raw are suspect and
        // the reads drop versus accepted → the two votes split (conservative).
        let prev_acc = (40, 4, 35031);
        let prev_raw = (10, 4, 10311); // values irrelevant on the suspect path
        let cur = (10, 5, 10500); // continuous with last_raw, but that is suspect
        assert!(
            stats_regressed(prev_acc, prev_raw, cur, CLEAN3, (true, false, true)),
            "suspect prev_raw → accepted-drop alone may vote (conservative fallback)"
        );
    }

    #[test]
    fn pending_outcome_expires() {
        let now = test_now();

        let mut fresh = Some((detect::MatchOutcome::Defeat, now - Duration::from_secs(30)));
        assert_eq!(
            take_fresh_pending(&mut fresh, now),
            Some(detect::MatchOutcome::Defeat)
        );
        assert!(fresh.is_none());

        let mut stale = Some((detect::MatchOutcome::Defeat, now - Duration::from_secs(200)));
        assert_eq!(take_fresh_pending(&mut stale, now), None);
        assert!(stale.is_none());

        assert_eq!(take_fresh_pending(&mut None, now), None);
    }

    fn word(
        outcome: detect::MatchOutcome,
    ) -> Option<(detect::MatchOutcome, detect::match_end::OutcomeSource)> {
        Some((outcome, detect::match_end::OutcomeSource::ResultWord))
    }

    #[test]
    fn poll_debug_hit_skips_mid_match_no_signal() {
        assert_eq!(poll_debug_hit(None, None, OUTCOME_CONFIRM_WINDOW), None);
        assert_eq!(
            poll_debug_hit(
                None,
                Some((detect::MatchOutcome::Victory, Duration::from_secs(5))),
                OUTCOME_CONFIRM_WINDOW
            ),
            None
        );
    }

    #[test]
    fn poll_debug_hit_banner_is_confirm() {
        assert_eq!(
            poll_debug_hit(
                Some((
                    detect::MatchOutcome::Victory,
                    detect::match_end::OutcomeSource::Banner
                )),
                None,
                OUTCOME_CONFIRM_WINDOW
            ),
            Some((PollDebugHit::Confirm, detect::MatchOutcome::Victory))
        );
        assert_eq!(
            poll_debug_hit(
                Some((
                    detect::MatchOutcome::Defeat,
                    detect::match_end::OutcomeSource::Banner
                )),
                Some((detect::MatchOutcome::Defeat, Duration::from_secs(1))),
                OUTCOME_CONFIRM_WINDOW
            ),
            Some((PollDebugHit::Confirm, detect::MatchOutcome::Defeat))
        );
    }

    #[test]
    fn poll_debug_hit_first_word_is_streak() {
        assert_eq!(
            poll_debug_hit(
                word(detect::MatchOutcome::Victory),
                None,
                OUTCOME_CONFIRM_WINDOW
            ),
            Some((PollDebugHit::Streak, detect::MatchOutcome::Victory))
        );
        // Expired prior streak is a new first sighting, not a confirm.
        assert_eq!(
            poll_debug_hit(
                word(detect::MatchOutcome::Defeat),
                Some((
                    detect::MatchOutcome::Defeat,
                    OUTCOME_CONFIRM_WINDOW + Duration::from_secs(1)
                )),
                OUTCOME_CONFIRM_WINDOW
            ),
            Some((PollDebugHit::Streak, detect::MatchOutcome::Defeat))
        );
        // Disagreement restarts the streak.
        assert_eq!(
            poll_debug_hit(
                word(detect::MatchOutcome::Draw),
                Some((detect::MatchOutcome::Victory, Duration::from_secs(2))),
                OUTCOME_CONFIRM_WINDOW
            ),
            Some((PollDebugHit::Streak, detect::MatchOutcome::Draw))
        );
    }

    #[test]
    fn poll_debug_hit_second_agreeing_word_is_confirm() {
        assert_eq!(
            poll_debug_hit(
                word(detect::MatchOutcome::Victory),
                Some((detect::MatchOutcome::Victory, Duration::from_secs(10))),
                OUTCOME_CONFIRM_WINDOW
            ),
            Some((PollDebugHit::Confirm, detect::MatchOutcome::Victory))
        );
        assert_eq!(
            poll_debug_hit(
                Some((
                    detect::MatchOutcome::Defeat,
                    detect::match_end::OutcomeSource::RankScreen
                )),
                Some((detect::MatchOutcome::Defeat, Duration::from_secs(1))),
                OUTCOME_CONFIRM_WINDOW
            ),
            Some((PollDebugHit::Confirm, detect::MatchOutcome::Defeat))
        );
        assert_eq!(
            poll_debug_hit(
                Some((
                    detect::MatchOutcome::Draw,
                    detect::match_end::OutcomeSource::EndTitle
                )),
                Some((detect::MatchOutcome::Draw, Duration::from_secs(50))),
                OUTCOME_CONFIRM_WINDOW
            ),
            Some((PollDebugHit::Confirm, detect::MatchOutcome::Draw))
        );
    }

    #[test]
    fn poll_debug_prefix_is_greppable_by_kind_and_outcome() {
        assert_eq!(
            poll_debug_prefix(PollDebugHit::Confirm, detect::MatchOutcome::Victory),
            "poll_confirm_victory"
        );
        assert_eq!(
            poll_debug_prefix(PollDebugHit::Streak, detect::MatchOutcome::Defeat),
            "poll_streak_defeat"
        );
        assert_eq!(
            poll_debug_prefix(PollDebugHit::Confirm, detect::MatchOutcome::Draw),
            "poll_confirm_draw"
        );
    }

    fn png_names(dir: &std::path::Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn tiny_board() -> image::DynamicImage {
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(2, 2, image::Rgb([8, 16, 32])))
    }

    /// A frame whose map-label box holds a small glyph, so a map crop saves.
    fn labelled_frame() -> image::DynamicImage {
        let (w, h) = (1920, 1080);
        let mut img = image::RgbImage::from_pixel(w, h, image::Rgb([0, 0, 0]));
        let rect = stat_tracker::ocr::preprocess::map_name_rect(w, h).expect("map rect");
        for y in rect.y..rect.y + 8.min(rect.h) {
            for x in rect.x..rect.x + 16.min(rect.w) {
                img.put_pixel(x, y, image::Rgb([230, 230, 230]));
            }
        }
        image::DynamicImage::ImageRgb8(img)
    }

    #[tokio::test]
    async fn a_rejected_capture_saves_a_map_crop_only_with_debug_on() {
        // R2 on #244: a rejected frame can be gameplay, where the map-label
        // box covers the kill feed (player names).
        let dir = tempfile::tempdir().unwrap();
        let mapcrops = dir.path().join("debug").join("mapcrops");
        for reason in ["teamsize", "noscoreboard", "noplayerrow", "unreadable"] {
            assert!(
                archive_rejected_map_label_crop(
                    false,
                    dir.path(),
                    "sess-r",
                    &labelled_frame(),
                    reason
                )
                .is_none(),
                "{reason}"
            );
        }
        assert!(
            archive_rejected_map_label_crop(
                true,
                dir.path(),
                "sess-r",
                &labelled_frame(),
                "preflight"
            )
            .is_none()
        );
        assert!(
            png_names(&mapcrops).is_empty(),
            "debug off saves no map crop"
        );

        archive_rejected_map_label_crop(
            true,
            dir.path(),
            "sess-r",
            &labelled_frame(),
            "unreadable",
        )
        .expect("debug on saves the crop")
        .await
        .unwrap();
        assert_eq!(png_names(&mapcrops).len(), 1);

        // An accepted capture still saves one with debug off.
        archive_map_label_crop(dir.path(), "sess-a", &labelled_frame())
            .expect("accepted capture saves the crop")
            .await
            .unwrap();
        assert_eq!(png_names(&mapcrops).len(), 2);
    }

    #[tokio::test]
    async fn accepted_and_rejected_rings_write_nothing_with_debug_off() {
        let dir = tempfile::tempdir().unwrap();
        assert!(save_accepted_frame(false, dir.path(), Arc::new(tiny_board())).is_none());
        assert!(save_rejected_frame(false, dir.path(), tiny_board(), "unreadable").is_none());
        assert!(
            !dir.path().join("debug").exists(),
            "debug off must not create debug/accepted or debug/rejected"
        );
    }

    #[tokio::test]
    async fn accepted_and_rejected_rings_write_with_debug_on() {
        let dir = tempfile::tempdir().unwrap();
        save_accepted_frame(true, dir.path(), Arc::new(tiny_board()))
            .expect("debug on spawns the write")
            .await
            .unwrap();
        save_rejected_frame(true, dir.path(), tiny_board(), "unreadable")
            .expect("debug on spawns the write")
            .await
            .unwrap();
        let accepted = png_names(&dir.path().join("debug").join("accepted"));
        let rejected = png_names(&dir.path().join("debug").join("rejected"));
        assert!(
            accepted.len() == 1 && accepted[0].starts_with("accepted_"),
            "accepted={accepted:?}"
        );
        assert!(
            rejected.len() == 1 && rejected[0].starts_with("rejected_unreadable_"),
            "rejected={rejected:?}"
        );
    }

    #[test]
    fn stale_board_debug_dirs_are_reported_only_with_debug_off_and_never_removed() {
        let dir = tempfile::tempdir().unwrap();
        assert!(stale_board_debug_dirs(false, dir.path()).is_empty());
        let accepted = dir.path().join("debug").join("accepted");
        std::fs::create_dir_all(&accepted).unwrap();
        let keep = accepted.join("accepted_old.png");
        std::fs::write(&keep, b"x").unwrap();
        assert_eq!(
            stale_board_debug_dirs(false, dir.path()),
            vec![accepted.clone()]
        );
        assert!(stale_board_debug_dirs(true, dir.path()).is_empty());
        assert!(keep.exists(), "the user's files are left untouched");
    }

    #[test]
    fn save_frame_ring_uses_on_hit_prefix_and_bounds() {
        let dir = tempfile::tempdir().unwrap();
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            2,
            2,
            image::Rgb([8, 16, 32]),
        ));
        let prefix = poll_debug_prefix(PollDebugHit::Confirm, detect::MatchOutcome::Victory);
        save_frame_ring(dir.path(), &prefix, &img, 2);
        save_frame_ring(
            dir.path(),
            &poll_debug_prefix(PollDebugHit::Streak, detect::MatchOutcome::Defeat),
            &img,
            2,
        );
        let names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            names
                .iter()
                .any(|n| n.starts_with("poll_confirm_victory_") && n.ends_with(".png")),
            "names={names:?}"
        );
        assert!(
            names
                .iter()
                .any(|n| n.starts_with("poll_streak_defeat_") && n.ends_with(".png")),
            "names={names:?}"
        );
        assert_eq!(names.len(), 2);

        // Bounded keep: a third write evicts the oldest PNG in the ring.
        save_frame_ring(dir.path(), "poll_confirm_draw", &img, 2);
        let names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names.len(), 2, "names={names:?}");
    }

    fn upload_ok() -> scuffed_types::api::StatsUploadResponse {
        scuffed_types::api::StatsUploadResponse {
            inserted: 1,
            skipped: 0,
            deleted: 0,
        }
    }

    fn test_match(session_id: &str, outcome: &str) -> storage::PersonalMatch {
        storage::PersonalMatch {
            id: None,
            hero: "Ana".into(),
            map_name: "Busan".into(),
            game_mode: String::new(),
            role: "Support".into(),
            outcome: outcome.into(),
            elims: 10,
            deaths: 2,
            assists: 5,
            damage: 4000,
            healing: 8000,
            mitigation: 0,
            played_at: SurrealDatetime::from(Utc::now()),
            synced: false,
            sync_rev: 0,
            upload_reject: None,
            session_id: session_id.into(),
            corrected_hero: None,
            corrected_role: None,
            corrected_map_name: None,
            corrected_outcome: None,
            corrected_elims: None,
            corrected_deaths: None,
            corrected_assists: None,
            corrected_damage: None,
            corrected_healing: None,
            corrected_mitigation: None,
            edited_fields: Vec::new(),
            edited_at: None,
            heroes_played: Vec::new(),
            segment_resolutions: Vec::new(),
            recognizer: scuffed_types::RECOGNIZER_OCR_V1.to_string(),
            suspect_fields: Vec::new(),
        }
    }

    #[test]
    fn new_reader_drop_from_11_to_1_stays_11_and_flags_elims() {
        let mut parsed = test_match("s", "unknown");
        parsed.recognizer = "cv-v5".into();
        parsed.elims = 1;
        parsed.assists = 4;
        parsed.deaths = 2;
        parsed.damage = 3100;
        parsed.healing = 800;
        parsed.mitigation = 400;
        let prior = capture_gate::GateState {
            accepted: capture_gate::Counters {
                elims: 11,
                assists: 4,
                deaths: 2,
                damage: 3000,
                healing: 800,
                mitigation: 400,
            },
            last_raw: capture_gate::Counters {
                elims: 11,
                assists: 4,
                deaths: 2,
                damage: 3000,
                healing: 800,
                mitigation: 400,
            },
            ..capture_gate::GateState::default()
        };
        let clean = [false; capture_gate::GATE_COLS];
        let held = hold_new_reader_stats(
            &mut parsed,
            Some((prior, Duration::from_secs(20))),
            clean,
            clean,
            false,
            true,
            None,
        );
        assert_eq!(
            parsed.elims, 11,
            "a new-reader 1 does not replace a held 11"
        );
        assert!(
            parsed.suspect_fields.iter().any(|field| field == "e"),
            "elims stays flagged: {:?}",
            parsed.suspect_fields
        );
        assert_eq!(held.accepted.elims, 11);
        // The same hold waits on a lone jump, then stores the matching second read.
        parsed.elims = 18;
        parsed.suspect_fields.clear();
        let jumped = hold_new_reader_stats(
            &mut parsed,
            Some((prior, Duration::from_secs(20))),
            clean,
            clean,
            false,
            true,
            None,
        );
        assert_eq!(parsed.elims, 11, "one 18 does not count");
        assert!(parsed.suspect_fields.iter().any(|field| field == "e"));
        parsed.elims = 18;
        parsed.suspect_fields.clear();
        let matched = hold_new_reader_stats(
            &mut parsed,
            Some((jumped.state, Duration::from_secs(20))),
            clean,
            clean,
            false,
            true,
            None,
        );
        assert_eq!(matched.accepted.elims, 18, "the second 18 counts");
        assert_eq!(parsed.elims, 18);
    }

    #[test]
    fn new_reader_keeps_a_fallback_of_40_18_until_two_clean_reads_of_0_0() {
        let clean = [false; capture_gate::GATE_COLS];
        let mut parsed = test_match("s", "unknown");
        parsed.elims = 2;
        parsed.assists = 40;
        parsed.deaths = 18;
        parsed.damage = 450;
        parsed.healing = 0;
        parsed.mitigation = 2;
        let first = hold_new_reader_stats(&mut parsed, None, clean, clean, false, false, None);
        assert_eq!((parsed.assists, parsed.deaths), (40, 18));
        assert!(first.state.low_trust);
        assert!(first.state.unconfirmed[1] && first.state.unconfirmed[2]);

        parsed.assists = 0;
        parsed.deaths = 0;
        parsed.damage = 1105;
        parsed.healing = 259;
        parsed.mitigation = 450;
        parsed.suspect_fields.clear();
        let second = hold_new_reader_stats(
            &mut parsed,
            Some((first.state, Duration::from_secs(20))),
            clean,
            clean,
            false,
            true,
            None,
        );
        assert_eq!(
            (parsed.assists, parsed.deaths),
            (40, 18),
            "one clean read keeps the fallback"
        );
        assert!(parsed.suspect_fields.iter().any(|field| field == "a"));
        assert!(parsed.suspect_fields.iter().any(|field| field == "d"));

        parsed.assists = 0;
        parsed.deaths = 0;
        parsed.suspect_fields.clear();
        let third = hold_new_reader_stats(
            &mut parsed,
            Some((second.state, Duration::from_secs(20))),
            clean,
            clean,
            false,
            true,
            None,
        );
        assert_eq!((third.accepted.assists, third.accepted.deaths), (0, 0));
        assert_eq!((parsed.assists, parsed.deaths), (0, 0));
        assert!(!third.state.low_trust);
    }

    #[tokio::test]
    async fn handle_capture_new_reader_keeps_11_when_the_next_read_is_1() {
        let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
        fn stat_rows(elims: u32) -> Vec<ocr::RowOcrResult> {
            let values = [
                elims.to_string(),
                "4".into(),
                "2".into(),
                "3000".into(),
                "800".into(),
                "400".into(),
            ];
            (0..10)
                .map(|_| ocr::RowOcrResult {
                    name: Some(ocr::CellOcrResult {
                        value: "PLAYER".into(),
                        confidence: 80,
                        suspect: false,
                    }),
                    name_alternates: Vec::new(),
                    stats: values
                        .iter()
                        .map(|value| ocr::CellOcrResult {
                            value: value.clone(),
                            confidence: 90,
                            suspect: false,
                        })
                        .collect(),
                    mean_confidence: 90,
                })
                .collect()
        }
        fn analysis_for(elims: u32) -> FrameAnalysis {
            let frame = image::DynamicImage::new_rgb8(64, 64);
            FrameAnalysis {
                frame_outcome: detect::MatchOutcome::Unknown,
                ocr: Ok(ocr::OcrResult {
                    raw_text: "Ana\nBusan".into(),
                    confidence: 80,
                }),
                rows: stat_rows(elims),
                portrait_hero: None,
                career_hero: None,
                pack_hero: None,
                board: None,
                map_from_panel: Some("Busan".into()),
                map_panel_raw: Some("BUSAN".into()),
                map_from_banner: false,
                scoreboard: std::sync::Arc::new(frame.clone()),
                player_row_idx: Some(0),
                name_match: None,
                own_row: None,
                own_name_candidate: None,
                own_seed_verdict: detect::own_row::Verdict::Silent,
                team_size: 5,
                frame,
                shadow_input: None,
            }
        }
        fn board_for(elims: u32) -> shadow::BoardRead {
            let mut fields = vec![
                shadow::FieldRead {
                    name: "map".into(),
                    value: Some(shadow::Value::Text("Busan".into())),
                    confidence: 0.9,
                    suspect: false,
                },
                shadow::FieldRead {
                    name: "r0.hero".into(),
                    value: Some(shadow::Value::Text("Ana".into())),
                    confidence: 0.9,
                    suspect: false,
                },
            ];
            for (suffix, value) in [
                ("e", elims),
                ("a", 4),
                ("d", 2),
                ("dmg", 3000),
                ("h", 800),
                ("mit", 400),
            ] {
                fields.push(shadow::FieldRead {
                    name: format!("r0.{suffix}"),
                    value: Some(shadow::Value::Int(value)),
                    confidence: 0.9,
                    suspect: false,
                });
            }
            shadow::BoardRead {
                status: shadow::BoardStatus::Read,
                team_size: Some(5),
                fields,
                elapsed_ms: 1,
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        let ctx = DaemonCtx {
            backend: capture::CaptureBackend::None,
            store: store.clone(),
            sync_client: None,
            player_name: None,
            capture_output: None,
            auto_detect: config::AutoDetectConfig::default(),
            finished_game_close: Duration::from_secs(90),
            game_process_names: Vec::new(),
            portrait_matcher: std::sync::Arc::new(detect::hero_portrait::PortraitMatcher::load(
                dir.path(),
            )),
            collect_portraits: false,
            dump_poll_frames: false,
            debug_ocr: false,
            data_dir: dir.path().to_path_buf(),
            empty_map_reads: std::sync::atomic::AtomicUsize::new(0),
            shadow: None,
            own_name: None,
            use_new_reader: true,
        };
        let mut now = test_now();
        let mut active = game(detect::MatchOutcome::Unknown, None, now);
        active.session_id = "held-11".into();
        active.map = Some("Busan".into());
        active.map_source = Some(boundary::MapSource::TopBar);
        active.session_created = false;
        let mut st = session(Some(active), now);

        for elims in [11u32, 1] {
            *INJECTED_CAPTURE.lock().unwrap() = Some(InjectedCapture {
                analysis: analysis_for(elims),
                board: Some(board_for(elims)),
            });
            let req = build_capture_request(st.active_game.as_ref().unwrap(), false, now);
            let report = handle_capture(&ctx, req).await.unwrap();
            assert!(report.recorded, "elims read {elims} was not stored");
            assert!(!report.split, "elims read {elims} must stay on this game");
            apply_capture_report(
                &mut st,
                &ctx.store,
                &ctx.data_dir,
                "held-11",
                Ok(report),
                now,
            )
            .await;
            now += Duration::from_secs(20);
        }

        let saved = ctx.store.get_session_snapshots("held-11").await.unwrap();
        let elims: Vec<u32> = saved.iter().map(|row| row.elims).collect();
        assert_eq!(elims, vec![11, 11], "a read of 1 must not replace 11");
    }

    /// O1: the same capture, with a sure result in the match's word
    /// evidence, stores that result only with `reader = "new"`. With the
    /// default `ocr-v1` the stored row is ocr-v1's, whatever the evidence.
    #[tokio::test]
    async fn result_evidence_is_saved_only_with_the_new_reader() {
        use shadow::result::{ACCOLADE, Outcome, RANK_SCREEN, ResultRead};
        let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
        let sure = |src: &str| ResultRead {
            outcome: Some(Outcome::Defeat),
            source: Some(src.into()),
            score: 0.97,
            margin: 0.6,
            suspect: false,
        };
        let rows = || -> Vec<ocr::RowOcrResult> {
            (0..10)
                .map(|_| ocr::RowOcrResult {
                    name: Some(ocr::CellOcrResult {
                        value: "PLAYER".into(),
                        confidence: 80,
                        suspect: false,
                    }),
                    name_alternates: Vec::new(),
                    stats: ["7", "4", "2", "3000", "800", "400"]
                        .iter()
                        .map(|value| ocr::CellOcrResult {
                            value: (*value).into(),
                            confidence: 90,
                            suspect: false,
                        })
                        .collect(),
                    mean_confidence: 90,
                })
                .collect()
        };
        let analysis = || {
            let frame = image::DynamicImage::new_rgb8(64, 64);
            FrameAnalysis {
                frame_outcome: detect::MatchOutcome::Unknown,
                ocr: Ok(ocr::OcrResult {
                    raw_text: "Ana\nBusan".into(),
                    confidence: 80,
                }),
                rows: rows(),
                portrait_hero: None,
                career_hero: None,
                pack_hero: None,
                board: None,
                map_from_panel: Some("Busan".into()),
                map_panel_raw: Some("BUSAN".into()),
                scoreboard: std::sync::Arc::new(frame.clone()),
                player_row_idx: Some(0),
                name_match: None,
                own_row: None,
                own_name_candidate: None,
                own_seed_verdict: detect::own_row::Verdict::Silent,
                team_size: 5,
                frame,
                shadow_input: None,
                map_from_banner: false,
            }
        };
        let mut fields = vec![shadow::FieldRead {
            name: "result".into(),
            value: None,
            confidence: 0.0,
            suspect: true,
        }];
        for (suffix, value) in [
            ("e", 7),
            ("a", 4),
            ("d", 2),
            ("dmg", 3000),
            ("h", 800),
            ("mit", 400),
        ] {
            fields.push(shadow::FieldRead {
                name: format!("r0.{suffix}"),
                value: Some(shadow::Value::Int(value)),
                confidence: 0.9,
                suspect: false,
            });
        }
        let board = shadow::BoardRead {
            status: shadow::BoardStatus::Read,
            team_size: Some(5),
            fields,
            elapsed_ms: 1,
        };
        let mut stored = Vec::new();
        for use_new_reader in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let store = storage::LocalStore::open(dir.path()).await.unwrap();
            let ctx = DaemonCtx {
                backend: capture::CaptureBackend::None,
                store: store.clone(),
                sync_client: None,
                player_name: None,
                capture_output: None,
                auto_detect: config::AutoDetectConfig::default(),
                finished_game_close: Duration::from_secs(90),
                game_process_names: Vec::new(),
                portrait_matcher: std::sync::Arc::new(
                    detect::hero_portrait::PortraitMatcher::load(dir.path()),
                ),
                collect_portraits: false,
                dump_poll_frames: false,
                debug_ocr: false,
                data_dir: dir.path().to_path_buf(),
                empty_map_reads: std::sync::atomic::AtomicUsize::new(0),
                shadow: None,
                own_name: None,
                use_new_reader,
            };
            let now = test_now();
            let mut active = game(detect::MatchOutcome::Unknown, None, now);
            active.session_id = "o1-result".into();
            active.map = Some("Busan".into());
            active.map_source = Some(boundary::MapSource::TopBar);
            active.session_created = false;
            active.result_evidence.add(sure(RANK_SCREEN));
            active.result_evidence.add(sure(ACCOLADE));
            let mut st = session(Some(active), now);
            *INJECTED_CAPTURE.lock().unwrap() = Some(InjectedCapture {
                analysis: analysis(),
                board: Some(board.clone()),
            });
            let req = build_capture_request(st.active_game.as_ref().unwrap(), false, now);
            let report = handle_capture(&ctx, req).await.unwrap();
            assert!(report.recorded);
            apply_capture_report(
                &mut st,
                &ctx.store,
                &ctx.data_dir,
                "o1-result",
                Ok(report),
                now,
            )
            .await;
            let saved = ctx.store.get_session_snapshots("o1-result").await.unwrap();
            assert_eq!(saved.len(), 1);
            stored.push((saved[0].outcome.clone(), saved[0].recognizer.clone()));
        }
        assert_ne!(
            stored[0].0, "defeat",
            "ocr-v1 ignores the evidence: {stored:?}"
        );
        assert_eq!(stored[0].1, scuffed_types::RECOGNIZER_OCR_V1);
        assert_eq!(stored[1].0, "defeat", "{stored:?}");
        assert_eq!(stored[1].1, shadow::digits::RECOGNIZER_ID);
    }

    // ---- S1: the OCR on/off switch (`ocr` in config.toml).

    /// A 1080p stand-in for a Tab frame: a bright header band with six dark
    /// stat labels inside the scoreboard window. That is all the preflight
    /// needs to call it a board. Nothing on it can be read.
    fn s1_tab_frame() -> image::DynamicImage {
        let (w, h) = (1920u32, 1080u32);
        let mut img = image::RgbImage::from_pixel(w, h, image::Rgb([12, 14, 22]));
        let (gx, gy, gw, gh) = ocr::preprocess::game_rect_16_9(w, h);
        let bx = gx + (gw as f64 * 0.175) as u32;
        let by = gy + (gh as f64 * 0.15) as u32;
        let bw = (gw as f64 * 0.65) as u32;
        let bh = (gh as f64 * 0.70) as u32;
        let y1 = by + (bh as f64 * 0.030) as u32;
        for py in by..y1 {
            for px in bx..bx + bw {
                img.put_pixel(px, py, image::Rgb([230, 230, 230]));
            }
        }
        let origin = bx + (bw as f64 * 0.40) as u32;
        for i in 0..6u32 {
            let x0 = origin + i * 36;
            for py in by..y1 {
                for px in x0..x0 + 4 {
                    img.put_pixel(px, py, image::Rgb([20, 20, 20]));
                }
            }
        }
        image::DynamicImage::ImageRgb8(img)
    }

    /// [`s1_tab_frame`] with the background of one team 1 row lighter, as
    /// the game draws the member's own row. Still nothing on it can be read.
    fn s1_tab_frame_lit(row: usize) -> image::DynamicImage {
        let frame = s1_tab_frame();
        let (w, h) = (frame.width(), frame.height());
        let (bx, by, bw, bh) = ocr::preprocess::crop_scoreboard_rect(w, h);
        // Where the row is in the crop: `crop_player_row` on an image whose
        // pixels carry their own y.
        let probe = image::RgbImage::from_fn(bw, bh, |_, y| {
            image::Rgb([(y % 256) as u8, (y / 256) as u8, 0])
        });
        let slot = ocr::preprocess::crop_player_row(&image::DynamicImage::ImageRgb8(probe), row, 5)
            .expect("the row is on the board")
            .to_rgb8();
        let [lo, hi, _] = slot.get_pixel(0, 0).0;
        let top = u32::from(lo) + 256 * u32::from(hi);
        let mut img = frame.to_rgb8();
        for py in by + top..by + top + slot.height() {
            for px in bx..bx + bw {
                // The background, three times as bright. The saturation is
                // the same, so the preflight's row scan sees no new row.
                img.put_pixel(px, py, image::Rgb([36, 42, 66]));
            }
        }
        image::DynamicImage::ImageRgb8(img)
    }

    /// With OCR off the vision pass still runs to its end on a Tab frame.
    /// Every OCR read is stopped at the gate and comes back empty, and the
    /// missing board text is not an error (an error would fail the capture).
    #[test]
    fn with_ocr_off_a_tab_frame_is_analysed_without_ocr_and_is_not_a_failed_capture() {
        let frame = s1_tab_frame();
        assert!(
            matches!(
                detect::hero_portrait::preflight_scoreboard(&ocr::preprocess::crop_scoreboard(
                    &frame
                ))
                .0,
                detect::hero_portrait::ScoreboardPreflight::Ready(_)
            ),
            "the synthetic frame has to pass the preflight"
        );
        let dir = tempfile::tempdir().unwrap();
        let matcher = Arc::new(detect::hero_portrait::PortraitMatcher::load(dir.path()));
        let off = ocr::gate::scoped(ocr::gate::OcrMode::Off);
        let analysed = analyze_frame(
            frame,
            matcher,
            Some("PLAYER".into()),
            detect::MatchOutcome::Unknown,
            FrameFlags {
                allow_banner_recovery: true,
                session_map_known: false,
                shadow_on: false,
                // `ocr = "off"` implies the new reader
                use_new_reader: true,
            },
            // no own-name seed yet: a first run
            Vec::new(),
        );
        let FrameAnalysisOutcome::Analyzed(a) = analysed else {
            panic!("the frame was rejected before the reads");
        };
        assert!(
            a.rows
                .iter()
                .all(|row| row.name.is_none() && row.stats.is_empty()),
            "no name and no stat cell was read"
        );
        assert_eq!(a.map_panel_raw, None, "no top-bar label was read");
        assert_eq!(a.career_hero, None);
        assert!(a.name_match.is_none());
        assert_eq!(a.frame_outcome, detect::MatchOutcome::Unknown);
        let text = a
            .ocr
            .as_ref()
            .expect("with OCR off, no board text is not a failed capture");
        assert_eq!((text.raw_text.as_str(), text.confidence), ("", 0));
        // The trust gate downstream is the new reader's board. Nothing on
        // this frame can be read, so the board is refused, not handed to OCR.
        let board = a.board.as_ref().expect("the new reader's board");
        assert!(
            reader_apply::board_refusal(board, a.team_size).is_some(),
            "{:?}",
            board.status
        );
        let blocked = off.blocked();
        assert_eq!(blocked, [], "nothing on the Tab pass went to the gate");
        assert!(
            blocked
                .iter()
                .all(|b| !matches!(b.site.entry, "ocr_with" | "ocr_pool" | "tessdata_lang")),
            "every OCR call stops at its entry point: {blocked:?}"
        );
        // The reads that have a replacement, and the two fallbacks, are not
        // attempted at all: the top-bar label (the banner read, O6), the
        // career-panel title (the hero icons, O8) and the full-board text.
        assert!(
            blocked.iter().all(|b| !matches!(
                b.site.entry,
                "recognize_map_label" | "recognize_region" | "recognize"
            )),
            "a replaced read still went to the gate: {blocked:?}"
        );
    }

    /// The analysis of a Tab nothing was read from: what the vision pass
    /// returns with OCR off for a board no reader could read.
    fn s1_unread_analysis() -> FrameAnalysis {
        FrameAnalysis {
            ocr: Ok(ocr::OcrResult {
                raw_text: String::new(),
                confidence: 0,
            }),
            rows: Vec::new(),
            map_from_panel: None,
            map_panel_raw: None,
            player_row_idx: None,
            ..o8_analysis(None, None)
        }
    }

    /// A Tab nothing was read from is held. It stores no row, and it does
    /// not touch the game that is open: the row already stored, the result
    /// and the map all stay as they were. Nothing is written as Unknown.
    #[tokio::test]
    async fn an_unread_board_is_held_and_leaves_the_open_game_as_it_was() {
        let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let ctx = o8_ctx(dir.path(), true).await;
        let mut now = test_now();
        let mut active = game(detect::MatchOutcome::Victory, Some(5), now);
        active.session_id = "s1".into();
        active.map = Some("Busan".into());
        active.map_source = Some(boundary::MapSource::TopBar);
        active.session_created = false;
        let mut st = session(Some(active), now);

        let tabs = [
            (
                o8_analysis(None, Some("zenyatta")),
                Some(o8_board(Some("zenyatta"))),
                true,
            ),
            (s1_unread_analysis(), None, false),
            (s1_unread_analysis(), None, false),
        ];
        let mut stored_after_first = None;
        for (i, (analysis, board, stored)) in tabs.into_iter().enumerate() {
            *INJECTED_CAPTURE.lock().unwrap() = Some(InjectedCapture { analysis, board });
            let req = build_capture_request(st.active_game.as_ref().unwrap(), false, now);
            let report = handle_capture(&ctx, req)
                .await
                .expect("an unread board is not a failed capture");
            assert_eq!(report.recorded, stored, "tab {i}");
            assert!(!report.split, "tab {i} stays on this game");
            assert_eq!(report.outcome, detect::MatchOutcome::Victory, "tab {i}");
            apply_capture_report(&mut st, &ctx.store, &ctx.data_dir, "s1", Ok(report), now).await;
            let rows = ctx.store.get_session_snapshots("s1").await.unwrap();
            assert_eq!(rows.len(), 1, "tab {i}: only the read board is stored");
            match &stored_after_first {
                None => stored_after_first = Some(rows),
                Some(first) => assert_eq!(&rows, first, "tab {i} changed the stored row"),
            }
            let game = st.active_game.as_ref().expect("the game stays open");
            assert_eq!(game.session_id, "s1");
            assert_eq!(game.outcome, detect::MatchOutcome::Victory);
            assert_eq!(game.map.as_deref(), Some("Busan"));
            now += Duration::from_secs(20);
        }
        let row = &stored_after_first.unwrap()[0];
        assert_eq!(row.hero, "Zenyatta");
        assert_eq!(row.map_name, "Busan");
        assert_eq!((row.elims, row.deaths, row.damage), (10, 2, 3000));
        assert_ne!(row.outcome, "unknown");
    }

    /// The merge gate for `ocr = "off"`, in two halves.
    ///
    /// 1. While the stat cells or the own row are not read without OCR on
    ///    this build, off has to be refused, and the refusal names them.
    /// 2. The Tab pass with OCR off, alone in a process
    ///    ([`own_process_tab_pass_with_ocr_off`]): the real `analyze_frame`
    ///    on synthetic Tabs, with the process-wide mode off, so a read on
    ///    any thread is seen. No read may reach the gate from a read this
    ///    build says it does without OCR. A Tab with a lit row is stored
    ///    with its stats, the own row found by `analyze_frame` itself.
    #[test]
    fn with_ocr_off_a_board_the_new_reader_reads_is_stored_with_its_stats() {
        use stat_tracker::config::OcrSetting;
        use stat_tracker::ocr_switch::{self, PackReadiness, Read};
        let every_pack = PackReadiness {
            heroes: true,
            maps: true,
            words: true,
            result_words: true,
            phase_titles: true,
            end_titles: true,
        };
        let board_reads = [Read::StatCells, Read::OwnRow];
        if board_reads.iter().any(|read| !read.without_ocr()) {
            let decision = ocr_switch::decide(OcrSetting::Off, &every_pack, &Read::needing_ocr());
            assert_eq!(
                decision.effective,
                OcrSetting::On,
                "no board reader without OCR on this build: off has to be refused"
            );
            let refusal = decision.refusal.expect("the refusal says why");
            for read in board_reads.iter().filter(|read| !read.without_ocr()) {
                assert!(refusal.reads.contains(read), "{read:?} is named");
            }
        }

        let exe = std::env::current_exe().expect("this test binary");
        // Empty HOME and XDG dirs: the process-wide reader of that process
        // loads no pack of this machine, and no tracker file is read.
        let home = tempfile::tempdir().unwrap();
        let run = std::process::Command::new(exe)
            .args([
                "--ignored",
                "--exact",
                "tests::own_process_tab_pass_with_ocr_off",
                "--test-threads",
                "1",
                "--nocapture",
            ])
            .env(OWN_PROCESS_ENV, "1")
            .env("HOME", home.path())
            .env("XDG_CONFIG_HOME", home.path().join("config"))
            .env("XDG_DATA_HOME", home.path().join("data"))
            .env("XDG_CACHE_HOME", home.path().join("cache"))
            .env_remove("SCUFFED_TEMPLATE_DIR")
            .output()
            .expect("run the Tab pass in its own process");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&run.stdout),
            String::from_utf8_lossy(&run.stderr)
        );
        assert!(
            run.status.success() && text.contains("1 passed"),
            "the Tab pass with OCR off, in its own process:\n{text}"
        );
    }

    /// Set by [`with_ocr_off_a_board_the_new_reader_reads_is_stored_with_its_stats`]
    /// for the process it starts.
    const OWN_PROCESS_ENV: &str = "STAT_TRACKER_TEST_OWN_PROCESS";

    /// The read behind each `ocr` entry point the Tab pass can call. A
    /// blocked call is allowed only while its read still needs OCR on this
    /// build ([`stat_tracker::ocr_switch::Read::without_ocr`]).
    fn tab_pass_read(entry: &str) -> Option<stat_tracker::ocr_switch::Read> {
        use stat_tracker::ocr_switch::Read;
        Some(match entry {
            "recognize_scoreboard_cells"
            | "recognize_scoreboard_cells_with_team_size"
            | "recognize_scoreboard_cells_pre_cropped"
            | "recognize_scoreboard_cells_pre_cropped_with"
            | "recognize_row"
            | "recognize_row_with"
            | "recognize_cell" => Read::StatCells,
            "recognize_scoreboard_names_pre_cropped" | "recognize_name" => Read::OwnRow,
            "recognize_region" => Read::Hero,
            // The label and the full-board text are fallbacks of the banner
            // read. They are not asked for with OCR off.
            "recognize_map_label" | "read_map_crop" | "recognize" => Read::TopBarMap,
            // The Tab frame's own outcome read.
            "recognize_prepared" => Read::ResultWord,
            "recognize_prepared_lang" => Read::EndTitle,
            "recognize_sparse_region" => Read::MatchStart,
            _ => return None,
        })
    }

    /// Run by [`with_ocr_off_a_board_the_new_reader_reads_is_stored_with_its_stats`],
    /// alone in a process, so the gate's mode and counters are this test's:
    /// a read made on the OCR pool or in `spawn_blocking` is blocked and
    /// counted like one made here (a scoped mode would miss it).
    #[tokio::test]
    #[ignore = "run alone in a process by with_ocr_off_a_board_the_new_reader_reads_is_stored_with_its_stats"]
    async fn own_process_tab_pass_with_ocr_off() {
        use stat_tracker::ocr_switch::Read;
        assert_eq!(
            std::env::var(OWN_PROCESS_ENV).as_deref(),
            Ok("1"),
            "this test sets the process-wide OCR mode: run it through its starter"
        );
        ocr::gate::set_mode(ocr::gate::OcrMode::Off);
        assert_eq!(ocr::gate::mode(), ocr::gate::OcrMode::Off);

        let dir = tempfile::tempdir().unwrap();
        let matcher = Arc::new(detect::hero_portrait::PortraitMatcher::load(dir.path()));
        let analysed = analyze_frame(
            s1_tab_frame(),
            matcher,
            Some("PLAYER".into()),
            detect::MatchOutcome::Unknown,
            FrameFlags {
                allow_banner_recovery: true,
                session_map_known: false,
                shadow_on: false,
                // `ocr = "off"` implies the new reader
                use_new_reader: true,
            },
            // no own-name seed yet: a first run
            Vec::new(),
        );
        let FrameAnalysisOutcome::Analyzed(analysis) = analysed else {
            panic!("the synthetic Tab was rejected before the reads");
        };

        // 1. Every call the pass sent to the gate belongs to a read this
        //    build still needs OCR for. A read marked as done without OCR
        //    that still calls OCR fails here, whatever thread made the call.
        //    Every such call is named, not only the first.
        let check_blocked = |when: &str| {
            let mut wrong = Vec::new();
            for b in ocr::gate::blocked_calls() {
                match tab_pass_read(b.site.entry) {
                    None => wrong.push(format!(
                        "{} is an entry point with no read: add it to tab_pass_read",
                        b.site
                    )),
                    Some(read) if read.without_ocr() => wrong.push(format!(
                        "{read:?} is marked as read without OCR, and {} still asked for it \
                         {} time(s)",
                        b.site, b.count
                    )),
                    Some(_) => {}
                }
            }
            assert!(wrong.is_empty(), "{when}:\n{}", wrong.join("\n"));
        };
        check_blocked("the Tab pass");
        let blocked_for = |read: Read| -> u64 {
            ocr::gate::blocked_calls()
                .iter()
                .filter(|b| tab_pass_read(b.site.entry) == Some(read))
                .map(|b| b.count)
                .sum()
        };
        // The stat cells are read without OCR on this build, so the cell
        // read must not be at the gate at all.
        assert_eq!(
            blocked_for(Read::StatCells),
            0,
            "{:?}",
            ocr::gate::blocked_calls()
        );
        // 2. Nothing of Tesseract was started, on any thread.
        assert_eq!(ocr::gate::leptess_created(), 0, "no LepTess");
        assert_eq!(ocr::gate::tesseract_calls(), 0, "no Tesseract read");
        assert!(!ocr::pool_started(), "no OCR pool");
        assert!(!ocr::tessdata_lang_probed(), "no tessdata lookup");

        // The plain synthetic Tab has no lit row, no seed and no hero icon:
        // no own row, and the name read was not asked for (nothing above
        // went to the gate, so not the name cells either).
        assert_eq!(analysis.player_row_idx, None);
        assert_eq!(
            blocked_for(Read::OwnRow),
            0,
            "{:?}",
            ocr::gate::blocked_calls()
        );

        // 3. A board the new reader read is stored, with its stats, and
        //    nothing of it goes to OCR.
        //
        //    The Tab has a lit row, and the own row is the one the real
        //    pass finds on it, with OCR off. One thing is given, because a
        //    synthetic frame cannot carry it: the digits (the reader's
        //    board, with 10 / 4 / 2 / 3000 / 800 / 400 on the lit row).
        const LIT: usize = 1;
        let matcher = Arc::new(detect::hero_portrait::PortraitMatcher::load(dir.path()));
        let analysed = analyze_frame(
            s1_tab_frame_lit(LIT),
            matcher,
            Some("PLAYER".into()),
            detect::MatchOutcome::Unknown,
            FrameFlags {
                allow_banner_recovery: true,
                session_map_known: false,
                shadow_on: false,
                use_new_reader: true,
            },
            Vec::new(),
        );
        let FrameAnalysisOutcome::Analyzed(analysis) = analysed else {
            panic!("the lit synthetic Tab was rejected before the reads");
        };
        let analysis = *analysis;
        check_blocked("the Tab pass on the lit Tab");
        assert_eq!(
            analysis.player_row_idx,
            Some(LIT),
            "the own row is the pass's own: {:?}",
            analysis.own_row
        );
        assert_eq!(
            analysis.own_row.map(|own| own.source),
            Some(detect::own_row::Source::HighlightOnly)
        );
        assert_eq!(analysis.name_match, None, "no name was read for it");
        let mut board = o8_board(Some("zenyatta"));
        board.team_size = Some(analysis.team_size);
        for field in &mut board.fields {
            if let Some(rest) = field.name.strip_prefix("r0.") {
                field.name = format!("r{LIT}.{rest}");
            }
        }
        let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
        let store_dir = tempfile::tempdir().unwrap();
        let ctx = o8_ctx(store_dir.path(), true).await;
        let (rows, recorded) = o10_tabs_on(&ctx, vec![(analysis, Some(board))], "Busan").await;
        assert_eq!(
            recorded,
            vec![true],
            "with OCR off a board the new reader read has to be stored"
        );
        check_blocked("storing the board");
        assert_eq!(
            ocr::gate::blocked_total(),
            0,
            "nothing of the two passes or of storing was asked of OCR"
        );
        assert_eq!(ocr::gate::leptess_created(), 0);
        assert_eq!(ocr::gate::tesseract_calls(), 0);
        assert!(!ocr::pool_started(), "no OCR pool");
        assert_eq!(rows.len(), 1);
        assert_eq!(
            o10_stats(&rows[0]),
            [10, 4, 2, 3000, 800, 400],
            "the new reader's numbers"
        );
        assert_eq!(rows[0].map_name, "Busan");
    }

    /// F261-9. Startup may wait for the reader packs before it decides the
    /// OCR switch. A stop that comes during that wait ends it at once, and
    /// startup then leaves as after a normal stop: the SIGTERM handler is
    /// registered before the wait, the run marker is finished, and the same
    /// handler goes on to the run loop.
    #[tokio::test]
    async fn a_stop_during_the_startup_pack_wait_ends_it_cleanly() {
        let never = std::future::pending::<()>;
        assert!(
            !stopped_while_waiting(Duration::from_millis(30), never(), never()).await,
            "the limit ends the wait"
        );
        assert!(
            !stopped_while_waiting(Duration::from_secs(30), async {}, never()).await,
            "the packs arriving ends the wait"
        );
        let started = Instant::now();
        assert!(
            stopped_while_waiting(Duration::from_secs(30), never(), async {}).await,
            "a stop ends the wait"
        );
        assert!(started.elapsed() < Duration::from_secs(5));

        // The wait is on the pack job's first try, not on the job: `run`
        // returns only when every pack is settled.
        let config = config::Config {
            data_dir: tempfile::tempdir().unwrap().path().to_path_buf(),
            ..config::Config::default()
        };
        let first_try = |missing: Vec<&'static str>, next_try_in: Option<Duration>| {
            stat_tracker::packs::FirstTry {
                missing,
                next_try_in,
            }
        };
        let limit = Duration::from_secs(45);
        // Every pack arrived, or the job ended, or its next try is far off
        // or past the limit: go on at once.
        for first in [
            first_try(vec![], None),
            first_try(vec![], Some(Duration::from_secs(5))),
            first_try(vec!["maps-v1"], None),
            first_try(vec!["maps-v1"], Some(Duration::from_secs(60))),
            first_try(vec!["maps-v1"], Some(Duration::from_secs(3600))),
        ] {
            assert!(!keep_waiting_for_packs(&first, limit), "{first:?}");
            let (send, recv) = tokio::sync::oneshot::channel();
            send.send(first.clone()).unwrap();
            let started = Instant::now();
            reader_packs_arrive(limit, recv, &config).await;
            assert!(
                started.elapsed() < Duration::from_secs(2),
                "{first:?} does not hold startup"
            );
        }
        // A next try soon and inside what is left: packs can still arrive.
        let soon = first_try(vec!["maps-v1"], Some(Duration::from_secs(10)));
        assert!(keep_waiting_for_packs(&soon, limit));
        assert!(
            !keep_waiting_for_packs(&soon, Duration::from_secs(10)),
            "the next try is not before the wait ends"
        );
        // The job dropped before its first try: nothing to wait for.
        let (send, recv) = tokio::sync::oneshot::channel::<stat_tracker::packs::FirstTry>();
        drop(send);
        reader_packs_arrive(limit, recv, &config).await;

        // The order in `run`, by its source.
        let src = include_str!("main.rs");
        let at = src.find("\nasync fn run(").expect("run");
        let body = &src[at..];
        let body = &body[..body
            .find("\nasync fn stopped_while_waiting")
            .expect("the helper")];
        let wait = body.find("stopped_while_waiting(").expect("the wait");
        let handler = body
            .find("tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())")
            .expect("the early handler");
        let decide = body.find("Switch::start(&config)").expect("the switch");
        assert!(
            handler < wait && wait < decide,
            "the handler is registered before the wait, and the wait is before the decision"
        );
        let first = body.find("extras.first_try_done()").expect("the first try");
        let spawn = body.find("tokio::spawn(extras.run());").expect("the job");
        assert!(
            first < spawn && spawn < wait,
            "the first-try channel is taken before the job starts, and the wait follows"
        );
        assert!(
            body.contains(".is_some_and(|job| job.starts_with_downloads())"),
            "no wait when the pack job's first move is a wait"
        );
        let stop_branch =
            &body[body[wait..].find("if stopped {").expect("the stop branch") + wait..];
        let stop_branch =
            &stop_branch[..stop_branch.find("early_sigterm = Some(sigterm);").unwrap()];
        assert!(
            stop_branch.contains("guard.finish();") && stop_branch.contains("return Ok(());"),
            "a stop during the wait finishes the run marker and returns Ok"
        );
        assert!(
            body.contains("run_loop(ctx, early_sigterm)"),
            "the same handler goes on to the run loop"
        );
    }

    /// With classic OCR off the accolade map is the map pack's read or
    /// nothing. The OCR read behind a missing map pack is not asked for, so
    /// nothing of it reaches the gate
    /// ([`stat_tracker::ocr_switch::Read::AccoladeMap`]). With OCR on that
    /// fallback is as #265 left it.
    #[test]
    fn with_ocr_off_the_accolade_map_is_never_asked_of_ocr() {
        use detect::match_end::OutcomeSource;
        use stat_tracker::ocr_switch::Read;
        let frame = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            1920,
            1080,
            image::Rgb([16, 25, 50]),
        ));
        // The daemon's own OCR read as the fallback, on a result-word tick.
        let choose = |pack: shadow::AccoladeRead| {
            choose_accolade_map(
                true,
                false,
                Some(OutcomeSource::ResultWord),
                || pack,
                || detect::match_end::read_accolade_map(&frame),
            )
        };
        let off = ocr::gate::scoped(ocr::gate::OcrMode::Off);
        // That OCR read does reach the gate when it is called.
        assert_eq!(detect::match_end::read_accolade_map(&frame), None);
        assert_eq!(off.blocked_total(), 1, "the frame reaches the OCR read");
        // No map pack, or a pack that abstains: no map, and no OCR call.
        assert_eq!(choose(shadow::AccoladeRead::NoPack), None);
        assert_eq!(choose(shadow::AccoladeRead::Abstain), None);
        assert_eq!(
            off.blocked_total(),
            1,
            "the accolade map was asked of OCR with classic OCR off: {:?}",
            off.blocked()
        );
        assert!(
            Read::AccoladeMap.without_ocr(),
            "the accolade map is read without OCR on this build"
        );
        drop(off);
        // With OCR on, no map pack still falls back to the OCR read.
        let asked = std::cell::Cell::new(false);
        let map = choose_accolade_map(
            true,
            false,
            Some(OutcomeSource::ResultWord),
            || shadow::AccoladeRead::NoPack,
            || {
                asked.set(true);
                Some("Busan".to_string())
            },
        );
        assert!(asked.get(), "with OCR on the fallback runs");
        assert_eq!(map.as_deref(), Some("Busan"));
    }

    /// F261-1. With classic OCR off the co-op event board check works
    /// from its two sources without OCR
    /// ([`stat_tracker::ocr_switch::Read::PveBoard`]): the word from a sure
    /// `ADLERSBRUNN` banner read (#271) and the teams from the new reader's
    /// board (#266), through `handle_capture`.
    ///
    /// * A board with stats on one team only (Junkenstein) is not stored.
    /// * A board with stats on both teams is stored, as Eichenwalde.
    /// * The limit: the word exists only on a sure banner read. When the
    ///   banner is read unsure the Tab has no top-bar map and the check does
    ///   not fire, so a co-op board is stored in the open game. With OCR on
    ///   the label OCR is a second chance to read the word.
    #[tokio::test]
    async fn with_ocr_off_a_coop_board_is_not_stored_and_a_two_team_adlersbrunn_game_is() {
        use stat_tracker::ocr_switch::Read;
        let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
        // One Tab as the pass with OCR off hands it over: no label OCR and
        // no board text, the rows unread, and the top bar from the banner
        // read alone.
        let tab = |banner_suspect: bool, both_teams: bool| {
            let banner = Some(banner_read(shadow::banner::ADLERSBRUNN_KEY, banner_suspect));
            let top = choose_top_bar_map(true, || banner, None);
            let mut analysis = s1_unread_analysis();
            analysis.rows = ocr::unread_rows(5);
            analysis.player_row_idx = Some(0);
            analysis.pack_hero = Some("ana".into());
            analysis.map_panel_raw = top_bar_raw_label(top.banner.as_ref(), None);
            analysis.map_from_panel = top.map;
            analysis.map_from_banner = top.from_banner;
            let mut board = o8_board(Some("ana"));
            if both_teams {
                for suffix in reader_apply::STAT_SUFFIXES {
                    board.fields.push(shadow::FieldRead {
                        name: format!("r7.{suffix}"),
                        value: Some(shadow::Value::Int(3)),
                        confidence: 0.9,
                        suspect: false,
                    });
                }
            }
            (analysis, Some(board))
        };
        let off = ocr::gate::scoped(ocr::gate::OcrMode::Off);
        let run = |tab: (FrameAnalysis, Option<shadow::BoardRead>)| async move {
            let dir = tempfile::tempdir().unwrap();
            let ctx = o8_ctx(dir.path(), true).await;
            o10_tabs_on(&ctx, vec![tab], "Eichenwalde").await
        };

        // The word arrives without OCR.
        let (sure, _) = tab(false, true);
        assert_eq!(
            sure.map_panel_raw.as_deref(),
            Some(shadow::banner::ADLERSBRUNN)
        );
        assert_eq!(sure.map_from_panel.as_deref(), Some("Eichenwalde"));

        let (rows, recorded) = run(tab(false, true)).await;
        assert_eq!(recorded, vec![true], "two teams: a real game");
        assert_eq!(rows[0].map_name, "Eichenwalde");

        let (rows, recorded) = run(tab(false, false)).await;
        assert_eq!(recorded, vec![false], "one team only: the co-op board");
        assert!(rows.is_empty());

        // The limit. An unsure banner read says no word and gives no map.
        let (unsure, _) = tab(true, false);
        assert_eq!(unsure.map_panel_raw, None);
        assert_eq!(unsure.map_from_panel, None);
        let (_, recorded) = run(tab(true, false)).await;
        assert_eq!(
            recorded,
            vec![true],
            "the limit: without a sure banner read the co-op board is not told apart"
        );

        assert_eq!(off.blocked(), [], "none of it asked anything of OCR");
        assert!(
            Read::PveBoard.without_ocr(),
            "the check has both its sources without OCR on this build"
        );
    }

    // ---- O8: hero authority (OCR site T5) under each reader.

    fn o8_rows() -> Vec<ocr::RowOcrResult> {
        let values = ["10", "4", "2", "3000", "800", "400"];
        (0..10)
            .map(|_| ocr::RowOcrResult {
                name: Some(ocr::CellOcrResult {
                    value: "PLAYER".into(),
                    confidence: 80,
                    suspect: false,
                }),
                name_alternates: Vec::new(),
                stats: values
                    .iter()
                    .map(|value| ocr::CellOcrResult {
                        value: value.to_string(),
                        confidence: 90,
                        suspect: false,
                    })
                    .collect(),
                mean_confidence: 90,
            })
            .collect()
    }

    /// One Tab: the full-board OCR text names Ana (an OCR-text hero), the
    /// career panel (if any) and the heroes-v2 own-row read as given.
    fn o8_analysis(career: Option<&str>, pack: Option<&str>) -> FrameAnalysis {
        let frame = image::DynamicImage::new_rgb8(64, 64);
        FrameAnalysis {
            frame_outcome: detect::MatchOutcome::Unknown,
            ocr: Ok(ocr::OcrResult {
                raw_text: "Ana\nBusan".into(),
                confidence: 80,
            }),
            rows: o8_rows(),
            portrait_hero: None,
            career_hero: career.map(str::to_string),
            pack_hero: pack.map(str::to_string),
            board: None,
            map_from_panel: Some("Busan".into()),
            map_panel_raw: Some("BUSAN".into()),
            map_from_banner: false,
            scoreboard: std::sync::Arc::new(frame.clone()),
            player_row_idx: Some(0),
            name_match: None,
            own_row: None,
            own_name_candidate: None,
            own_seed_verdict: detect::own_row::Verdict::Silent,
            team_size: 5,
            frame,
            shadow_input: None,
        }
    }

    /// The new reader's board for that Tab: digits read, own-row hero as
    /// given (`None`: a suspect read that names no hero).
    fn o8_board(hero: Option<&str>) -> shadow::BoardRead {
        let mut fields = vec![
            shadow::FieldRead {
                name: "map".into(),
                value: Some(shadow::Value::Text("Busan".into())),
                confidence: 0.9,
                suspect: false,
            },
            shadow::FieldRead {
                name: "r0.hero".into(),
                value: hero.map(|h| shadow::Value::Text(h.into())),
                confidence: if hero.is_some() { 0.9 } else { 0.4 },
                suspect: hero.is_none(),
            },
        ];
        for (suffix, value) in [
            ("e", 10),
            ("a", 4),
            ("d", 2),
            ("dmg", 3000),
            ("h", 800),
            ("mit", 400),
        ] {
            fields.push(shadow::FieldRead {
                name: format!("r0.{suffix}"),
                value: Some(shadow::Value::Int(value)),
                confidence: 0.9,
                suspect: false,
            });
        }
        shadow::BoardRead {
            status: shadow::BoardStatus::Read,
            team_size: Some(5),
            fields,
            elapsed_ms: 1,
        }
    }

    async fn o8_ctx(dir: &std::path::Path, use_new_reader: bool) -> DaemonCtx {
        DaemonCtx {
            backend: capture::CaptureBackend::None,
            store: storage::LocalStore::open(dir).await.unwrap(),
            sync_client: None,
            player_name: None,
            capture_output: None,
            auto_detect: config::AutoDetectConfig::default(),
            finished_game_close: Duration::from_secs(90),
            game_process_names: Vec::new(),
            portrait_matcher: std::sync::Arc::new(detect::hero_portrait::PortraitMatcher::load(
                dir,
            )),
            collect_portraits: false,
            dump_poll_frames: false,
            debug_ocr: false,
            data_dir: dir.to_path_buf(),
            empty_map_reads: std::sync::atomic::AtomicUsize::new(0),
            shadow: None,
            own_name: None,
            use_new_reader,
        }
    }

    /// The hero parts of the last Tab's report.
    struct O8Report {
        career_panel: bool,
        hero_auth: HeroAuthState,
    }

    /// Run Tabs through `handle_capture` on one open game and return the
    /// session's stored rows and the last report.
    async fn o8_tabs(
        ctx: &DaemonCtx,
        tabs: Vec<(FrameAnalysis, Option<shadow::BoardRead>)>,
        between: impl AsyncFn(&DaemonCtx, usize),
    ) -> (Vec<storage::PersonalMatch>, O8Report) {
        let mut now = test_now();
        let mut active = game(detect::MatchOutcome::Unknown, None, now);
        active.session_id = "o8".into();
        active.map = Some("Busan".into());
        active.map_source = Some(boundary::MapSource::TopBar);
        active.session_created = false;
        let mut st = session(Some(active), now);
        let mut last = None;
        for (i, (analysis, board)) in tabs.into_iter().enumerate() {
            between(ctx, i).await;
            *INJECTED_CAPTURE.lock().unwrap() = Some(InjectedCapture { analysis, board });
            let req = build_capture_request(st.active_game.as_ref().unwrap(), false, now);
            let report = handle_capture(ctx, req).await.unwrap();
            assert!(report.recorded, "tab {i} was not stored");
            assert!(!report.split, "tab {i} must stay on this game");
            last = Some(O8Report {
                career_panel: report.career_panel,
                hero_auth: report.hero_auth.clone(),
            });
            apply_capture_report(&mut st, &ctx.store, &ctx.data_dir, "o8", Ok(report), now).await;
            now += Duration::from_secs(20);
        }
        let rows = ctx.store.get_session_snapshots("o8").await.unwrap();
        (rows, last.expect("a tab"))
    }

    async fn no_edit(_: &DaemonCtx, _: usize) {}

    /// The own row's board hero with the hero authority as given: a 5v5
    /// board whose own row names `row` while the authority (own row and
    /// career panel portrait) names `authority`.
    fn guard_tab(
        row: Option<&str>,
        authority: Option<&str>,
    ) -> (FrameAnalysis, Option<shadow::BoardRead>) {
        (o8_analysis(None, authority), Some(o8_board(row)))
    }

    async fn guard_game(
        tabs: Vec<(FrameAnalysis, Option<shadow::BoardRead>)>,
    ) -> (Vec<storage::PersonalMatch>, Option<String>) {
        let dir = tempfile::tempdir().unwrap();
        let ctx = o8_ctx(dir.path(), true).await;
        let (rows, _) = o8_tabs(&ctx, tabs, no_edit).await;
        // The game-level hero is the derived timeline's primary, which is
        // what the GUI shows and the upload sends (display_hero).
        let snaps = ctx.store.get_session_snapshots("o8").await.unwrap();
        let shown: Vec<String> = snaps.iter().map(|m| m.display_hero().to_string()).collect();
        assert!(shown.windows(2).all(|w| w[0] == w[1]), "{shown:?}");
        let session = shown.last().cloned();
        (rows, session)
    }

    /// A confident hero on Tab 1, then a 5v5 Tab whose own-row read the
    /// career panel contradicts (the authority abstains): the game keeps
    /// Tab 1's hero and the second row flags `hero`.
    #[tokio::test]
    async fn a_disagreeing_5v5_tab_keeps_the_earlier_confident_hero() {
        let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
        let (rows, session) = guard_game(vec![
            guard_tab(Some("zenyatta"), Some("zenyatta")),
            guard_tab(Some("baptiste"), None),
        ])
        .await;
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.hero == "Zenyatta"), "{rows:?}");
        assert!(!rows[0].suspect_fields.iter().any(|f| f == "hero"));
        assert!(rows[1].suspect_fields.iter().any(|f| f == "hero"));
        assert_eq!(session.as_deref(), Some("Zenyatta"));
    }

    /// The first Tab's own-row hero is rejected (the authority abstains, the
    /// panel names nothing to confirm it): that row stores Unknown with
    /// `hero` flagged, never the rejected read. A later agreeing Tab fills
    /// the game's hero in.
    #[tokio::test]
    async fn a_rejected_first_tab_stores_unknown_and_a_later_agreeing_tab_fills_the_hero() {
        let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
        for first in [Some("baptiste"), None] {
            let (rows, session) = guard_game(vec![
                guard_tab(first, None),
                guard_tab(Some("zenyatta"), Some("zenyatta")),
            ])
            .await;
            assert_eq!(rows.len(), 2, "{first:?}");
            assert_eq!(rows[0].hero, "Unknown", "{first:?}");
            assert!(
                rows[0].suspect_fields.iter().any(|f| f == "hero"),
                "{first:?}"
            );
            assert_eq!(rows[1].hero, "Zenyatta", "{first:?}");
            assert!(
                !rows[1].suspect_fields.iter().any(|f| f == "hero"),
                "{first:?}"
            );
            assert_eq!(session.as_deref(), Some("Zenyatta"), "{first:?}");
        }
    }

    // ---- alpha.7 O9: the own row without name OCR (OCR site T4)

    /// A made-up Tab table (header bar, role column, two blocks of five
    /// rows) with a made-up name of white blocks on `row`. No captured
    /// pixels and no real name.
    fn o9_board(row: usize) -> image::DynamicImage {
        use image::{Rgb, RgbImage};
        let pitch = 88u32;
        let (hh, x0, w) = (34u32, pitch / 2, pitch * 14);
        let mut img = RgbImage::from_pixel(w, hh + 12 * pitch, Rgb([12, 14, 24]));
        for y in 0..hh {
            for x in x0..w - x0 {
                img.put_pixel(x, y, Rgb([205, 208, 212]));
            }
        }
        for r in 0..10u32 {
            let top = hh + (r / 5) * 6 * pitch + (r % 5) * pitch;
            let colour = if r < 5 {
                [20, 110, 120]
            } else {
                [110, 20, 120]
            };
            for y in top..top + pitch - 2 {
                for x in x0..w - x0 {
                    img.put_pixel(x, y, Rgb(colour));
                }
            }
            if r as usize != row {
                continue;
            }
            // six glyphs: a white block with a dark notch, each different
            for g in 0..6u32 {
                let gx = x0 + (2.3 * pitch as f32) as u32 + g * 22;
                for y in 0..30u32 {
                    for x in 0..18u32 {
                        let notch = (6..12).contains(&x) && (y / 5) % 6 == (g * 2 + 1) % 6;
                        if !notch {
                            img.put_pixel(gx + x, top + 24 + y, Rgb([255; 3]));
                        }
                    }
                }
            }
        }
        image::DynamicImage::ImageRgb8(img)
    }

    /// The name line of `row` on [`o9_board`], as the vision pass cuts it.
    fn o9_candidate(row: usize) -> detect::own_row::Candidate {
        detect::own_row::NameBoard::measure(None, &o9_board(row), 5)
            .expect("board")
            .candidate(row)
            .expect("name line")
    }

    /// One Tab of the game `session_id` on `map` through `handle_capture`.
    async fn o9_tab(
        ctx: &DaemonCtx,
        session_id: &str,
        map: &str,
        mut analysis: FrameAnalysis,
        board: Option<shadow::BoardRead>,
    ) -> CaptureReport {
        let now = test_now();
        let mut active = game(detect::MatchOutcome::Unknown, None, now);
        active.session_id = session_id.into();
        active.map = Some(map.into());
        active.map_source = Some(boundary::MapSource::TopBar);
        active.session_created = false;
        analysis.map_from_panel = Some(map.into());
        analysis.map_panel_raw = Some(map.to_uppercase());
        if let Ok(read) = &mut analysis.ocr
            && read.raw_text == "Ana\nBusan"
        {
            read.raw_text = format!("Ana\n{map}");
        }
        *INJECTED_CAPTURE.lock().unwrap() = Some(InjectedCapture { analysis, board });
        let req = build_capture_request(&active, false, now);
        handle_capture(ctx, req).await.unwrap()
    }

    fn o9_store(dir: &std::path::Path) -> Arc<std::sync::Mutex<detect::own_row::SeedStore>> {
        Arc::new(std::sync::Mutex::new(detect::own_row::SeedStore::open(
            dir, None,
        )))
    }

    fn o9_lit(row: usize, lead: f64) -> Option<detect::own_row::Highlight> {
        Some(detect::own_row::Highlight { row, lead })
    }

    /// The seed store opens exactly when the new reader is in effect, and
    /// both read one function (F262-5): whatever turns the new reader on,
    /// the store is there. ocr-v1 never opens or creates the private folder.
    #[test]
    fn the_own_name_seeds_open_exactly_when_the_new_reader_is_in_effect() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = config::Config {
            data_dir: dir.path().to_path_buf(),
            ..config::Config::default()
        };
        use stat_tracker::config::OcrSetting;
        use stat_tracker::ocr_switch::{PackReadiness, decide};
        let ready = PackReadiness {
            heroes: true,
            maps: true,
            words: true,
            result_words: true,
            phase_titles: true,
            end_titles: true,
        };
        // Classic OCR on, as asked or because off was refused: `reader` decides.
        let on = decide(OcrSetting::On, &ready, &[]);
        let refused = decide(OcrSetting::Off, &PackReadiness::default(), &[]);
        // Classic OCR off: the new reader, whatever `reader` says.
        let off = decide(OcrSetting::Off, &ready, &[]);
        assert!(off.effective.is_off() && refused.effective.is_on());
        for switch in [&on, &refused] {
            assert!(
                !new_reader_in_effect(&config, switch),
                "ocr-v1 is the default"
            );
            assert!(open_own_name_seeds(new_reader_in_effect(&config, switch), &config).is_none());
        }
        assert!(
            new_reader_in_effect(&config, &off),
            "off implies the new reader, and its seed store"
        );
        assert!(open_own_name_seeds(new_reader_in_effect(&config, &off), &config).is_some());
        assert!(own_name_seeds(None).is_empty());
        // the bool decides, not the `reader` setting on its own
        assert!(open_own_name_seeds(true, &config).is_some());
        config.reader = config::ReaderSetting::New;
        for switch in [&on, &refused, &off] {
            assert!(new_reader_in_effect(&config, switch));
        }
        let store =
            open_own_name_seeds(new_reader_in_effect(&config, &on), &config).expect("store");
        assert!(own_name_seeds(Some(&store)).is_empty());
        assert!(!dir.path().join("private").exists(), "nothing until a crop");
        // the daemon takes the capture path, the reader packs and the seed
        // store from that one answer
        let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
        let source = norm(include_str!("main.rs"));
        // Each needle is written in two pieces, so this test's own text is
        // not what it finds.
        for pinned in [
            concat!(
                "let use_new_reader = new_reader_in_effect(&config, &ocr_switch.decision); ",
                "if use_new_reader { shadow::init("
            ),
            concat!(
                "use_new_reader, own_name: ",
                "open_own_name_seeds(use_new_reader, &config),"
            ),
        ] {
            assert_eq!(source.matches(pinned).count(), 1, "{pinned}");
        }
    }

    #[test]
    fn find_own_row_reads_only_the_switched_source() {
        use detect::own_row::{OwnRowRead, Source, Trust, combine, combine_with_hero};
        let found = |row| {
            Some(parse::NameMatch {
                row,
                window: 0,
                sole_window: true,
            })
        };
        let seeds = |seed: Option<usize>, lit| OwnRowRead {
            own: combine(seed, lit, true),
            ..OwnRowRead::unread(None, None, true)
        };
        let no_name = || -> Option<parse::NameMatch> { panic!("name read") };
        // ocr-v1: the name match, else the highlight. Neither the seeds, the
        // career hero nor the fallback are touched.
        let v1 = find_own_row(
            false,
            || found(4),
            || panic!("seeds read under ocr-v1"),
            no_name,
            o9_lit(1, 1.5),
            Some(3),
            true,
        );
        assert_eq!(v1.row_idx, Some(4));
        assert_eq!(v1.name_match.map(|m| m.row), Some(4));
        assert!(v1.own_row.is_none() && v1.candidate.is_none());
        let v1 = find_own_row(
            false,
            || None,
            || panic!("seeds read"),
            no_name,
            o9_lit(1, 1.5),
            Some(3),
            true,
        );
        assert_eq!((v1.row_idx, v1.name_match), (Some(1), None));
        let v1 = find_own_row(
            false,
            || None,
            || panic!("seeds"),
            no_name,
            None,
            Some(3),
            true,
        );
        assert_eq!(v1.row_idx, None, "ocr-v1 never takes the career hero's row");
        // new reader: the seeds and the highlight. With a row from them no
        // name is read at all.
        let both = find_own_row(
            true,
            no_name,
            || seeds(Some(2), o9_lit(2, 1.5)),
            no_name,
            o9_lit(2, 1.5),
            None,
            true,
        );
        assert_eq!((both.row_idx, both.name_match), (Some(2), None));
        assert_eq!(both.own_row.map(|o| o.trust), Some(Trust::Sure));
        // seed against a strong highlight: no row, and the name read is not
        // asked to break the tie
        let split = find_own_row(
            true,
            no_name,
            || seeds(Some(2), o9_lit(0, 1.5)),
            no_name,
            o9_lit(0, 1.5),
            None,
            true,
        );
        assert_eq!(split.row_idx, None);
        assert_eq!(split.own_row.map(|o| o.source), Some(Source::Disagree));
        // the seed alone names a row the highlight never saw (team 2)
        let alone = find_own_row(
            true,
            no_name,
            || seeds(Some(7), None),
            no_name,
            None,
            None,
            true,
        );
        assert_eq!(alone.row_idx, Some(7));
        assert!(alone.own_row.is_some_and(|o| o.suspect()));
        // seed and highlight silent, the career hero on one row of the
        // team: that row, marked, and no name is read (O9b)
        let hero = |seeded| OwnRowRead {
            own: combine_with_hero(None, None, Some(3), seeded),
            ..OwnRowRead::unread(None, Some(3), seeded)
        };
        for seeded in [false, true] {
            let by_hero = find_own_row(
                true,
                no_name,
                || hero(seeded),
                no_name,
                None,
                Some(3),
                seeded,
            );
            assert_eq!((by_hero.row_idx, by_hero.name_match), (Some(3), None));
            let own = by_hero.own_row.expect("own row");
            assert_eq!(
                (own.source, own.trust),
                (Source::CareerHero, Trust::Suspect)
            );
            assert!(own.suspect());
        }
        // nothing names a row: main's name read is asked, for this board.
        // With no seed for the board the row is stored as main stores it.
        let silent = |seeded| OwnRowRead {
            own: combine(None, None, seeded),
            ..OwnRowRead::unread(None, None, seeded)
        };
        let named = find_own_row(
            true,
            no_name,
            || silent(false),
            || found(6),
            None,
            None,
            false,
        );
        assert_eq!(named.row_idx, Some(6));
        assert_eq!(named.name_match.map(|m| m.row), Some(6));
        let own = named.own_row.expect("own row");
        assert_eq!(
            (own.source, own.trust),
            (Source::NameFallback, Trust::Unconfirmed)
        );
        assert!(!own.suspect(), "stored as main stores a name-matched row");
        // With a seed for the board that matched no row, the name's row is
        // marked: one voice where two could have spoken (F262-18). The
        // board's own answer decides, not the caller's "any seed exists".
        let named = find_own_row(
            true,
            no_name,
            || silent(true),
            || found(6),
            None,
            None,
            false,
        );
        let own = named.own_row.expect("own row");
        assert_eq!((named.row_idx, own.source), (Some(6), Source::NameFallback));
        assert_eq!(own.trust, Trust::Suspect);
        assert!(own.suspect());
        let named = find_own_row(
            true,
            no_name,
            || silent(false),
            || found(6),
            None,
            None,
            true,
        );
        assert!(!named.own_row.expect("own row").suspect());
        // ...and when that finds no row either (or OCR is off): held
        let none = find_own_row(true, no_name, || silent(true), || None, None, None, true);
        assert_eq!((none.row_idx, none.name_match), (None, None));
        assert_eq!(none.own_row.map(|o| o.source), Some(Source::Neither));
        // a panic in the seed read: the highlight, marked when seeds exist
        let panicked = find_own_row(
            true,
            no_name,
            || panic!("seed reader"),
            no_name,
            o9_lit(3, 1.5),
            Some(1),
            true,
        );
        assert_eq!(panicked.row_idx, Some(3));
        assert_eq!(panicked.own_row.map(|o| o.trust), Some(Trust::Suspect));
        assert!(panicked.candidate.is_none());
        let panicked = find_own_row(
            true,
            no_name,
            || panic!("seed reader"),
            no_name,
            o9_lit(3, 1.5),
            None,
            false,
        );
        assert_eq!(panicked.own_row.map(|o| o.trust), Some(Trust::Unconfirmed));
        // ...and with no highlight either, the career hero's row, marked
        let panicked = find_own_row(
            true,
            no_name,
            || panic!("seed reader"),
            no_name,
            None,
            Some(1),
            false,
        );
        assert_eq!(panicked.row_idx, Some(1));
        assert_eq!(
            panicked.own_row.map(|o| (o.source, o.trust)),
            Some((Source::CareerHero, Trust::Suspect))
        );
    }

    #[test]
    fn the_name_fallback_needs_a_configured_name() {
        // No name, or a blank one: nothing is read (a blank board would give
        // no match anyway; the point is that no window is OCRed).
        let board = o9_board(0);
        assert_eq!(own_row_name_fallback(None, &board, 5), None);
        assert_eq!(own_row_name_fallback(Some("  "), &board, 5), None);
        // With classic OCR off the name read is not asked for at all: no
        // call reaches the gate, so nothing is counted against the own row.
        {
            let off = ocr::gate::scoped(ocr::gate::OcrMode::Off);
            assert_eq!(own_row_name_fallback(Some("PLAYER"), &board, 5), None);
            assert_eq!(off.blocked(), [], "the fallback returns before any OCR");
        }
    }

    /// [`stat_tracker::ocr_switch::Read::OwnRow`] with classic OCR off, and
    /// both sides of its one condition (a hero pack that reads the board),
    /// through the real seed read and the real name fallback.
    ///
    /// * The icons are read: no highlight and no seed, and the career
    ///   panel's hero on one row of the member's team. That row is the own
    ///   row, marked, and no name is read.
    /// * No icon read to go by (no hero pack for the board, or the own icon
    ///   dead or covered): no row. With OCR off the Tab is held and nothing
    ///   goes to the gate: the stated limit. With OCR on the name read is
    ///   asked, which is why off is refused without a hero pack.
    #[test]
    fn with_ocr_off_the_career_hero_names_the_own_row_and_without_it_the_tab_is_held() {
        use detect::own_row::{Source, Trust};
        const OWN: usize = 3;
        let hero_read = |row: usize, hero: Option<&str>| shadow::heroes::HeroRead {
            row,
            class: if hero.is_some() {
                shadow::heroes::RowClass::Hero
            } else {
                shadow::heroes::RowClass::Unknown
            },
            hero: hero.map(str::to_string),
            best: hero.unwrap_or("ana").to_string(),
            score: if hero.is_some() { 0.95 } else { 0.5 },
            lead: 0.4,
            second: "kiriko".into(),
            suspect: hero.is_none(),
        };
        // The career panel shows Ana. `own_icon`: what is read on the
        // member's row (`None`: a skull or an ult ring over the icon).
        let icons = |own_icon: Option<&str>| shadow::TabHeroes {
            team_size: 5,
            read: Some(shadow::heroes::HeroFrameRead {
                board: shadow::heroes::HeroBoard {
                    rows: (0..10)
                        .map(|row| {
                            if row == OWN {
                                hero_read(row, own_icon)
                            } else {
                                hero_read(row, Some("mercy"))
                            }
                        })
                        .collect(),
                    elapsed_us: 1,
                },
                pack: shadow::heroes::HeroPack::V2,
                pad_px: None,
            }),
            panel: Some(shadow::heroes::PanelRead {
                read: hero_read(0, Some("ana")),
                pack: shadow::heroes::HeroPack::V2,
                box_px: (0, 0, 1, 1),
                rel: (0.03, -0.03, 0.16),
            }),
        };
        // What `shadow::read_heroes` returns with no hero pack for the board.
        let no_pack = shadow::TabHeroes {
            team_size: 5,
            read: None,
            panel: None,
        };
        let board = o9_board(OWN);
        let find = |heroes: &shadow::TabHeroes,
                    name_fallback: &dyn Fn() -> Option<parse::NameMatch>| {
            find_own_row(
                true,
                || -> Option<parse::NameMatch> { panic!("ocr-v1's name match") },
                // no highlight on the board, and no seed learned yet
                || detect::own_row::read_own_row(None, &board, 5, None, Some(heroes), &[]),
                name_fallback,
                None,
                detect::own_row::career_hero_row(heroes),
                false,
            )
        };

        {
            let off = ocr::gate::scoped(ocr::gate::OcrMode::Off);
            let the_fallback = || own_row_name_fallback(Some("PLAYER"), &board, 5);

            let by_hero = find(&icons(Some("ana")), &the_fallback);
            assert_eq!((by_hero.row_idx, by_hero.name_match), (Some(OWN), None));
            assert_eq!(
                by_hero.own_row.map(|own| (own.source, own.trust)),
                Some((Source::CareerHero, Trust::Suspect)),
                "the row is marked until a seed agrees"
            );

            // The limit: never the highlight, no seed, the own icon unread.
            for heroes in [icons(None), no_pack.clone()] {
                let held = find(&heroes, &the_fallback);
                assert_eq!((held.row_idx, held.name_match), (None, None));
                assert_eq!(held.own_row.map(|own| own.source), Some(Source::Neither));
            }
            assert_eq!(off.blocked(), [], "none of it asked anything of OCR");
        }

        // OCR on (the run of a daemon that was refused off for the hero
        // pack): with no icon read the name read is asked, once, and its
        // row is the own row.
        assert!(!ocr::gate::is_off());
        let asked = std::cell::Cell::new(0);
        let name_read = || {
            asked.set(asked.get() + 1);
            Some(parse::NameMatch {
                row: 6,
                window: 0,
                sole_window: true,
            })
        };
        let named = find(&no_pack, &name_read);
        assert_eq!(asked.get(), 1);
        assert_eq!(named.row_idx, Some(6));
        assert_eq!(
            named.own_row.map(|own| own.source),
            Some(Source::NameFallback)
        );
        // and where the career hero names the row, no name is read
        let by_hero = find(&icons(Some("ana")), &name_read);
        assert_eq!((by_hero.row_idx, asked.get()), (Some(OWN), 1));
    }

    #[test]
    fn the_text_fallback_gets_no_name_when_the_new_reader_has_no_own_row() {
        let name = Some("Made Up");
        // ocr-v1: unchanged, with or without a row
        assert_eq!(text_fallback_name(false, None, name), name);
        assert_eq!(text_fallback_name(false, Some(2), name), name);
        // new reader: only beside a row the seeds, the highlight or the
        // name fallback found
        assert_eq!(text_fallback_name(true, Some(2), name), name);
        assert_eq!(text_fallback_name(true, None, name), None);
        assert_eq!(text_fallback_name(true, None, None), None);
    }

    /// T4 (O9): the vision pass reads names in the per-cell pass only under
    /// ocr-v1 and finds the own row only through `find_own_row` with the
    /// daemon's reader switch. Whitespace is ignored. Hard-wiring the switch,
    /// reading names in the cell pass under the new reader, a second name
    /// fallback, or matching a name in `apply_new_reader` breaks this.
    #[test]
    fn the_own_row_goes_through_the_reader_switch() {
        let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
        let src = include_str!("main.rs");
        let at = src.find("\nfn analyze_frame(").expect("analyze_frame");
        let body = norm(src[at..].split("\n}\n").next().unwrap());
        // the per-cell pass (names and stat cells) is ocr-v1's only: the new
        // reader gets rows with nothing read on them (O10 took the stat
        // cells out of it, so `ocr::Names::Skip` has no caller here)
        assert!(body.contains(
            "let rows = read_tab_rows( use_new_reader, || ocr::recognize_scoreboard_cells_pre_cropped(&scoreboard, team_size), || ocr::unread_rows(team_size), );"
        ));
        assert_eq!(body.matches("recognize_scoreboard_cells").count(), 1);
        assert_eq!(body.matches("ocr::Names").count(), 0);
        assert!(body.contains("} = find_own_row( use_new_reader, || {"));
        // ocr-v1's match in its closure; the new reader's only through the
        // one named fallback
        assert_eq!(body.matches("match_player_row_by_name").count(), 1);
        assert_eq!(body.matches("own_row_name_fallback(").count(), 1);
        assert_eq!(body.matches("recognize_scoreboard_names").count(), 0);
        // one call in the daemon; the text of this assertion is the second match
        assert_eq!(
            src.matches("recognize_scoreboard_names_pre_cropped(")
                .count(),
            2
        );
        assert_eq!(body.matches("detect::own_row::read_own_row(").count(), 1);
        assert_eq!(body.matches("detect::own_row::seed_gate(").count(), 1);
        // the career hero's row comes from the one icon read, in the seed
        // read and in its panic fallback (O9b)
        assert!(body.contains("highlight, tab_heroes.as_ref(), &own_seeds, ) },"));
        assert!(body.contains(
            "highlight, tab_heroes .as_ref() .and_then(detect::own_row::career_hero_row), !own_seeds.is_empty(), );"
        ));
        assert!(body.contains("player_row_idx: row_idx, name_match, own_row,"));
        let an = src
            .find("\nfn apply_new_reader(")
            .expect("apply_new_reader");
        let an_body = norm(src[an..].split("\n}\n").next().unwrap());
        assert!(!an_body.contains("find_player_row_by_name"));
        assert!(!an_body.contains("player_name"));
        assert!(an_body.contains(
            "if own_row_suspect { reader_apply::mark_own_row_suspect(&mut parsed.suspect_fields); }"
        ));
        let hc = src
            .find("\nasync fn handle_capture(")
            .expect("handle_capture");
        let hc_body = norm(src[hc..].split("\n}\n").next().unwrap());
        assert!(
            hc_body
                .contains("text_fallback_name(ctx.use_new_reader, player_row_idx, player_name),")
        );
        assert!(hc_body.contains(
            "note_own_seed_verdict(ctx.own_name.as_ref(), own_seed_verdict, session_id).await;"
        ));
        // the offer sits on the new reader's own read path (O10)
        assert!(hc_body.contains(
            "if let Some(board) = new_board { let guesses = reader_apply::own_row_guesses(&board, board_row); offer_own_name_seed("
        ));
        // no name is matched for the board's row
        assert!(hc_body.contains(
            "let board_row = reader_apply::own_row(None, None, player_row_idx, team_size);"
        ));
        assert_eq!(hc_body.matches("find_player_row_by_name").count(), 0);
        // a suspect own row is one low-trust bit for the plan and both gates
        assert!(
            hc_body.contains("let own_row_suspect = own_row.is_some_and(|own| own.suspect());")
        );
        assert!(hc_body.contains("suspect_row: own_row_suspect,"));
        assert!(hc_body.contains("trusted_cells && !own_row_suspect,"));
        assert!(hc_body.contains("let own_seeds = own_name_seeds(ctx.own_name.as_ref());"));
    }

    #[tokio::test]
    async fn a_suspect_own_row_marks_every_field_and_is_not_a_trusted_read() {
        use detect::own_row::combine_with_hero;
        let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
        for (seed, lit, hero, seeded, marked) in [
            // seed and highlight agree: nothing is marked
            (Some(0), o9_lit(0, 1.5), None, true, false),
            // only the seed, or only the highlight where a seed exists
            (Some(0), None, None, true, true),
            (None, o9_lit(0, 1.5), None, true, true),
            // a seed over a weak highlight that names another row
            (Some(0), o9_lit(3, 1.1), None, true, true),
            // a strong highlight before any seed exists: as it always was
            (None, o9_lit(0, 1.5), None, false, false),
            // a weak highlight before any seed exists
            (None, o9_lit(0, 1.1), None, false, true),
            // the career hero's row alone (O9b): marked, seeds or not
            (None, None, Some(0), false, true),
            (None, None, Some(0), true, true),
            // a seed the career hero confirms: nothing is marked
            (Some(0), None, Some(0), true, false),
            // a seed the career hero contradicts: the seed's row, marked
            (Some(0), None, Some(2), true, true),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let ctx = o8_ctx(dir.path(), true).await;
            let mut analysis = o8_analysis(None, Some("zenyatta"));
            analysis.own_row = Some(combine_with_hero(seed, lit, hero, seeded));
            assert_eq!(analysis.own_row.unwrap().row, Some(0));
            let board = Some(o8_board(Some("zenyatta")));
            let report = o9_tab(&ctx, "o9", "Busan", analysis, board).await;
            assert!(report.recorded);
            let rows = ctx.store.get_session_snapshots("o9").await.unwrap();
            let flags = &rows[0].suspect_fields;
            let want: Vec<&str> = if marked {
                reader_apply::OWN_ROW_FIELDS.to_vec()
            } else {
                Vec::new()
            };
            assert_eq!(
                flags, &want,
                "seed {seed:?} highlight {lit:?} hero {hero:?} seeded {seeded}"
            );
            // the values themselves are still the row's
            assert_eq!((rows[0].elims, rows[0].damage), (10, 3000));
            // F262-11: a suspect row's counters enter the capture gate as a
            // low-trust read, so they cannot latch
            assert_eq!(
                report.gate_state.expect("gate").low_trust,
                marked,
                "seed {seed:?} highlight {lit:?} hero {hero:?} seeded {seeded}"
            );
        }
    }

    #[tokio::test]
    async fn the_new_reader_without_an_own_row_is_not_rescued_by_the_name_in_the_text() {
        use detect::own_row::combine;
        let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
        // The full-board text holds the configured name with six stats: the
        // text fallback (T7) would take that line as the member's row.
        let tab = || {
            let mut analysis = o8_analysis(None, Some("zenyatta"));
            analysis.player_row_idx = None;
            analysis.ocr = Ok(ocr::OcrResult {
                raw_text: "Ana\nBusan\nMADEUP 10 4 2 3,000 800 400".into(),
                confidence: 80,
            });
            analysis
        };
        // ocr-v1 takes it, as before
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = o8_ctx(dir.path(), false).await;
        ctx.player_name = Some("MadeUp".into());
        assert!(o9_tab(&ctx, "o9", "Busan", tab(), None).await.recorded);
        // the new reader holds the capture: a seed names another row than a
        // strong highlight
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = o8_ctx(dir.path(), true).await;
        ctx.player_name = Some("MadeUp".into());
        let mut analysis = tab();
        analysis.own_row = Some(combine(Some(2), o9_lit(0, 1.5), true));
        assert_eq!(analysis.own_row.unwrap().row, None);
        let board = Some(o8_board(Some("zenyatta")));
        let report = o9_tab(&ctx, "o9", "Busan", analysis, board).await;
        assert!(!report.recorded);
        assert!(
            ctx.store
                .get_session_snapshots("o9")
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn gated_crops_reach_the_seed_store_and_are_trusted_after_two_real_games() {
        let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = o8_ctx(dir.path(), true).await;
        ctx.own_name = Some(o9_store(dir.path()));
        let file = detect::own_row::seed_dir(dir.path()).join("seeds.json");
        // Four boards of one game are one game, and so is a second session
        // on the same map a moment later (a stat split of that game). A
        // session on another map is a second game.
        for (i, (session, map, trusted)) in [
            ("g1", "Busan", 0),
            ("g1", "Busan", 0),
            ("g1", "Busan", 0),
            ("g1-split", "Busan", 0),
            ("g2", "Ilios", 3),
        ]
        .into_iter()
        .enumerate()
        {
            let mut analysis = o8_analysis(None, Some("zenyatta"));
            analysis.own_name_candidate = Some(o9_candidate(i));
            let board = Some(o8_board(Some("zenyatta")));
            let report = o9_tab(&ctx, session, map, analysis, board).await;
            assert!(report.recorded, "tab {i}");
            assert_eq!(
                own_name_seeds(ctx.own_name.as_ref()).len(),
                trusted,
                "tab {i}"
            );
            // a waiting crop is never on disk; the trusted seeds are
            assert_eq!(file.is_file(), trusted > 0, "tab {i}");
        }
        // the seeds the next vision pass gets find the name on a new board
        let seeds = own_name_seeds(ctx.own_name.as_ref());
        let board = detect::own_row::NameBoard::measure(None, &o9_board(7), 5).unwrap();
        assert_eq!(detect::own_row::seed_row(&board.scores(&seeds)), Some(7));
        // a restart reads them back
        assert_eq!(own_name_seeds(Some(&o9_store(dir.path()))).len(), 3);
        // a Tab without a gated crop offers nothing
        let before = std::fs::read(&file).unwrap();
        let analysis = o8_analysis(None, Some("zenyatta"));
        let board = Some(o8_board(Some("zenyatta")));
        o9_tab(&ctx, "g3", "Busan", analysis, board).await;
        assert_eq!(std::fs::read(&file).unwrap(), before);
    }

    /// O9b: a crop from the career hero's row (no highlight) reaches the
    /// store like a highlighted row's, and is trusted only after four
    /// boards from three real games on two career heroes.
    #[tokio::test]
    async fn career_hero_crops_reach_the_seed_store_and_need_three_games_and_a_hero_swap() {
        use detect::own_row::{Proof, combine_with_hero};
        let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = o8_ctx(dir.path(), true).await;
        ctx.own_name = Some(o9_store(dir.path()));
        let file = detect::own_row::seed_dir(dir.path()).join("seeds.json");
        // What trusts three highlighted crops (two games) leaves these
        // waiting; a split of the second game is not a third game; one
        // hero through three games is not enough either.
        for (i, (session, map, hero, trusted)) in [
            ("g1", "Busan", "zenyatta", 0),
            ("g1", "Busan", "zenyatta", 0),
            ("g2", "Ilios", "zenyatta", 0),
            ("g2-split", "Ilios", "zenyatta", 0),
            ("g3", "Oasis", "zenyatta", 0),
            ("g3", "Oasis", "ana", 3),
        ]
        .into_iter()
        .enumerate()
        {
            let mut analysis = o8_analysis(None, Some(hero));
            analysis.own_row = Some(combine_with_hero(None, None, Some(0), false));
            analysis.own_name_candidate =
                Some(o9_candidate(i).proved(Proof::CareerHero, Some(hero)));
            let board = Some(o8_board(Some(hero)));
            let report = o9_tab(&ctx, session, map, analysis, board).await;
            assert!(report.recorded, "tab {i}");
            // the row is stored marked and as a low-trust read
            if i == 0 {
                let rows = ctx.store.get_session_snapshots(session).await.unwrap();
                assert_eq!(
                    rows[0].suspect_fields,
                    reader_apply::OWN_ROW_FIELDS.to_vec()
                );
            }
            assert!(report.gate_state.expect("gate").low_trust, "tab {i}");
            assert_eq!(
                own_name_seeds(ctx.own_name.as_ref()).len(),
                trusted,
                "tab {i}"
            );
            // a waiting crop is never on disk; the trusted seeds are
            assert_eq!(file.is_file(), trusted > 0, "tab {i}");
        }
        let seeds = own_name_seeds(ctx.own_name.as_ref());
        let board = detect::own_row::NameBoard::measure(None, &o9_board(7), 5).unwrap();
        assert_eq!(detect::own_row::seed_row(&board.scores(&seeds)), Some(7));
        assert_eq!(own_name_seeds(Some(&o9_store(dir.path()))).len(), 3);
    }

    #[tokio::test]
    async fn seeds_that_keep_contradicting_a_strong_highlight_are_dropped() {
        use detect::own_row::{DISAGREE_DROPS, Verdict, combine};
        let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = o8_ctx(dir.path(), true).await;
        ctx.own_name = Some(o9_store(dir.path()));
        let file = detect::own_row::seed_dir(dir.path()).join("seeds.json");
        for (i, (session, map)) in [("g1", "Busan"), ("g1", "Busan"), ("g2", "Ilios")]
            .into_iter()
            .enumerate()
        {
            let mut analysis = o8_analysis(None, Some("zenyatta"));
            analysis.own_name_candidate = Some(o9_candidate(i));
            o9_tab(
                &ctx,
                session,
                map,
                analysis,
                Some(o8_board(Some("zenyatta"))),
            )
            .await;
        }
        assert_eq!(own_name_seeds(ctx.own_name.as_ref()).len(), 3);
        // Held boards (a seed against a strong highlight) are not stored,
        // but they are counted: the seeds go on the third in a row.
        let held = || {
            let mut analysis = o8_analysis(None, Some("zenyatta"));
            analysis.player_row_idx = None;
            analysis.own_row = Some(combine(Some(2), o9_lit(0, 1.5), true));
            analysis.own_seed_verdict = Verdict::Disagree(88);
            analysis
        };
        for i in 1..=DISAGREE_DROPS {
            assert_eq!(own_name_seeds(ctx.own_name.as_ref()).len(), 3, "board {i}");
            let report = o9_tab(&ctx, "g3", "Busan", held(), Some(o8_board(None))).await;
            assert!(!report.recorded);
        }
        assert!(own_name_seeds(ctx.own_name.as_ref()).is_empty());
        assert!(!file.exists(), "the dropped seeds are off the disk too");
    }

    #[tokio::test]
    async fn ocr_v1_never_offers_a_crop_or_creates_the_private_folder() {
        let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = o8_ctx(dir.path(), false).await;
        // Even with a store, a crop and a verdict handed in, the ocr-v1
        // path uses none of them.
        ctx.own_name = Some(o9_store(dir.path()));
        for (i, (session, map)) in [("g1", "Busan"), ("g1", "Busan"), ("g2", "Ilios")]
            .into_iter()
            .enumerate()
        {
            let mut analysis = o8_analysis(Some("Ana"), None);
            analysis.own_name_candidate = Some(o9_candidate(i));
            analysis.own_seed_verdict = detect::own_row::Verdict::Disagree(88);
            assert!(o9_tab(&ctx, session, map, analysis, None).await.recorded);
        }
        assert!(own_name_seeds(ctx.own_name.as_ref()).is_empty());
        assert!(!dir.path().join("private").exists());
        let rows = ctx.store.get_session_snapshots("g1").await.unwrap();
        assert!(rows.iter().all(|r| r.suspect_fields.is_empty()));
        // Nor does it judge seeds that are there (a member who switched back
        // from the new reader): verdicts are not counted under ocr-v1.
        {
            let mut store = ctx.own_name.as_ref().unwrap().lock().unwrap();
            for (i, (session, map)) in [("s1", "Busan"), ("s1", "Busan"), ("s2", "Ilios")]
                .into_iter()
                .enumerate()
            {
                let game = detect::own_row::GameTag {
                    session: session.into(),
                    map: map.into(),
                    at: 0,
                };
                store.offer(o9_candidate(i), &game);
            }
            assert_eq!(store.trusted().len(), 3);
        }
        for _ in 0..2 * detect::own_row::DISAGREE_DROPS {
            let mut analysis = o8_analysis(Some("Ana"), None);
            analysis.own_seed_verdict = detect::own_row::Verdict::Disagree(88);
            assert!(o9_tab(&ctx, "g3", "Busan", analysis, None).await.recorded);
        }
        assert_eq!(own_name_seeds(ctx.own_name.as_ref()).len(), 3);
    }

    #[tokio::test]
    async fn new_reader_hero_comes_from_the_pack_read_never_the_career_panel() {
        let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let ctx = o8_ctx(dir.path(), true).await;
        // The career panel says Reinhardt and the OCR text Ana; the heroes-v2
        // own-row read says Zenyatta and is the hero authority.
        let (rows, report) = o8_tabs(
            &ctx,
            vec![(
                o8_analysis(Some("Reinhardt"), Some("zenyatta")),
                Some(o8_board(Some("zenyatta"))),
            )],
            no_edit,
        )
        .await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].hero, "Zenyatta");
        assert!(report.career_panel, "a pack read is the authority");
        assert!(report.hero_auth.career_ever_ok);
        assert_eq!(report.hero_auth.accepted_hero.as_deref(), Some("Zenyatta"));
    }

    #[tokio::test]
    async fn new_reader_abstain_stores_unknown_not_the_career_or_ocr_hero() {
        let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let ctx = o8_ctx(dir.path(), true).await;
        // The own-row read abstained (suspect), so the board names no hero
        // either. The career panel and the OCR text must not stand in.
        let (rows, report) = o8_tabs(
            &ctx,
            vec![(o8_analysis(Some("Reinhardt"), None), Some(o8_board(None)))],
            no_edit,
        )
        .await;
        assert_eq!(rows[0].hero, "Unknown");
        assert!(rows[0].suspect_fields.iter().any(|f| f == "hero"));
        assert!(!report.career_panel);
        assert!(!report.hero_auth.career_ever_ok);
        assert!(report.hero_auth.accepted_hero.is_none());
    }

    #[tokio::test]
    async fn new_reader_abstain_holds_the_hero_already_accepted() {
        let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let ctx = o8_ctx(dir.path(), true).await;
        let (rows, report) = o8_tabs(
            &ctx,
            vec![
                (
                    o8_analysis(None, Some("zenyatta")),
                    Some(o8_board(Some("zenyatta"))),
                ),
                (o8_analysis(Some("Reinhardt"), None), Some(o8_board(None))),
            ],
            no_edit,
        )
        .await;
        let heroes: Vec<&str> = rows.iter().map(|r| r.hero.as_str()).collect();
        assert_eq!(heroes, ["Zenyatta", "Zenyatta"], "abstain holds Zenyatta");
        assert_eq!(report.hero_auth.accepted_hero.as_deref(), Some("Zenyatta"));
    }

    #[tokio::test]
    async fn ocr_v1_hero_still_comes_from_the_career_panel() {
        let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let ctx = o8_ctx(dir.path(), false).await;
        // ocr-v1 ignores a pack read and never reads the board.
        let (rows, report) = o8_tabs(
            &ctx,
            vec![
                (o8_analysis(Some("Reinhardt"), Some("zenyatta")), None),
                // no career read: the held career hero, as before
                (o8_analysis(None, Some("zenyatta")), None),
            ],
            no_edit,
        )
        .await;
        let heroes: Vec<&str> = rows.iter().map(|r| r.hero.as_str()).collect();
        assert_eq!(heroes, ["Reinhardt", "Reinhardt"]);
        assert!(report.hero_auth.career_ever_ok);
        // and with no career read and nothing held, the OCR text, as before
        let dir = tempfile::tempdir().unwrap();
        let ctx = o8_ctx(dir.path(), false).await;
        let (rows, _) = o8_tabs(
            &ctx,
            vec![(o8_analysis(None, Some("zenyatta")), None)],
            no_edit,
        )
        .await;
        assert_eq!(rows[0].hero, "Ana", "ocr-v1 keeps its OCR-text fallback");
    }

    /// A member's own hero pick on the game, made between two Tabs.
    async fn pick_reinhardt(ctx: &DaemonCtx, tab: usize) {
        if tab == 1 {
            ctx.store
                .edit_match(
                    "o8",
                    &storage::MatchEdit {
                        hero: Some("Reinhardt".into()),
                        ..storage::MatchEdit::default()
                    },
                )
                .await
                .unwrap();
        }
    }

    #[tokio::test]
    async fn new_reader_never_overrides_a_member_hero_pick() {
        let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let ctx = o8_ctx(dir.path(), true).await;
        let tab = || {
            (
                o8_analysis(None, Some("zenyatta")),
                Some(o8_board(Some("zenyatta"))),
            )
        };
        let (rows, _) = o8_tabs(&ctx, vec![tab(), tab()], pick_reinhardt).await;
        assert_eq!(rows.len(), 2);
        for row in &rows {
            assert_eq!(row.hero, "Zenyatta", "the read stays as read");
            assert_eq!(row.display_hero(), "Reinhardt", "the member's pick wins");
            assert_eq!(row.display_role(), "Tank");
            assert!(row.field_edited("hero"));
            assert!(!row.suspect_fields.iter().any(|f| f == "hero"));
        }
    }

    #[tokio::test]
    async fn ocr_v1_rows_after_a_hero_pick_are_written_as_before() {
        let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let ctx = o8_ctx(dir.path(), false).await;
        let tab = || (o8_analysis(Some("Zenyatta"), None), None);
        let (rows, _) = o8_tabs(&ctx, vec![tab(), tab()], pick_reinhardt).await;
        assert_eq!(rows[0].display_hero(), "Reinhardt");
        // ocr-v1 is unchanged: a later row carries no overlay of its own
        assert_eq!(rows[1].corrected_hero, None);
    }

    #[test]
    fn read_own_hero_reads_only_the_switched_source() {
        let tab = |hero: Option<&str>| shadow::TabHeroes {
            team_size: 5,
            read: Some(shadow::heroes::HeroFrameRead {
                board: shadow::heroes::HeroBoard {
                    rows: (0..10)
                        .map(|row| shadow::heroes::HeroRead {
                            row,
                            class: if hero.is_some() {
                                shadow::heroes::RowClass::Hero
                            } else {
                                shadow::heroes::RowClass::Unknown
                            },
                            hero: hero.map(str::to_string),
                            best: hero.unwrap_or("ana").to_string(),
                            score: if hero.is_some() { 0.9 } else { 0.5 },
                            lead: 0.2,
                            second: "mercy".into(),
                            suspect: hero.is_none(),
                        })
                        .collect(),
                    elapsed_us: 1,
                },
                pack: shadow::heroes::HeroPack::V2,
                pad_px: None,
            }),
            panel: None,
        };
        // ocr-v1: the career panel, and the icons are never read. (A panic
        // in the closure would be caught and look like "not read", so the
        // closure counts its calls instead.)
        let reads = std::cell::Cell::new(0);
        let icons = read_tab_heroes(false, || {
            reads.set(reads.get() + 1);
            tab(Some("zenyatta"))
        });
        assert_eq!(icons, None);
        assert_eq!(reads.get(), 0, "ocr-v1 must not read the hero icons");
        let v1 = read_own_hero(false, || Some("Reinhardt".into()), icons, Some(0));
        assert_eq!(v1.career_hero.as_deref(), Some("Reinhardt"));
        assert_eq!((v1.pack_hero, v1.tab_heroes), (None, None));
        // ocr-v1 ignores an icon read even if one were handed to it
        let v1 = read_own_hero(false, || None, Some(tab(Some("zenyatta"))), Some(0));
        assert_eq!((v1.pack_hero, v1.tab_heroes), (None, None));
        // new: the icons, and the career panel is never read
        let career =
            || -> Option<String> { panic!("the new reader must not read the career panel") };
        let icons = read_tab_heroes(true, || {
            reads.set(reads.get() + 1);
            tab(Some("zenyatta"))
        });
        assert!(icons.is_some());
        assert_eq!(reads.get(), 1, "one icon read per Tab");
        let new = read_own_hero(true, career, icons, Some(3));
        assert_eq!(new.career_hero, None);
        assert_eq!(new.pack_hero.as_deref(), Some("zenyatta"));
        assert!(new.tab_heroes.is_some(), "kept for the board read");
        // an abstaining own row, an unknown own row, a row off the board
        let career = || -> Option<String> { panic!("no career read") };
        assert_eq!(
            read_own_hero(true, career, Some(tab(None)), Some(3)).pack_hero,
            None
        );
        let career = || -> Option<String> { panic!("no career read") };
        assert_eq!(
            read_own_hero(true, career, Some(tab(Some("zenyatta"))), None).pack_hero,
            None
        );
        let career = || -> Option<String> { panic!("no career read") };
        assert_eq!(
            read_own_hero(true, career, Some(tab(Some("zenyatta"))), Some(10)).pack_hero,
            None
        );
        // the career panel's portrait authenticates when the row abstains
        let panel = || {
            let mut t = tab(None);
            t.panel = Some(shadow::heroes::PanelRead {
                read: shadow::heroes::HeroRead {
                    row: 0,
                    class: shadow::heroes::RowClass::Hero,
                    hero: Some("ana".into()),
                    best: "ana".into(),
                    score: 0.97,
                    lead: 0.4,
                    second: "mercy".into(),
                    suspect: false,
                },
                pack: shadow::heroes::HeroPack::V2,
                box_px: (0, 0, 1, 1),
                rel: (0.03, -0.03, 0.16),
            });
            t
        };
        let career = || -> Option<String> { panic!("no career read") };
        assert_eq!(
            read_own_hero(true, career, Some(panel()), Some(3))
                .pack_hero
                .as_deref(),
            Some("ana")
        );
        // a panic in the icon reader is an abstain, not a failed Tab: no
        // icon read, so no hero authority and no career hero's row
        let career = || -> Option<String> { panic!("no career read") };
        let icons = read_tab_heroes(true, || panic!("icon reader"));
        assert_eq!(icons, None);
        let panicked = read_own_hero(true, career, icons, Some(0));
        assert_eq!((panicked.pack_hero, panicked.tab_heroes), (None, None));
    }

    #[tokio::test]
    async fn blank_map_and_unknown_hero_stay_local_until_picked() {
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        let mut missed_map = test_match("held-map", "victory");
        missed_map.map_name.clear();
        missed_map.game_mode.clear();
        store.insert_match(missed_map).await.unwrap();
        let mut unknown_hero = test_match("held-hero", "victory");
        unknown_hero.hero = "Unknown".into();
        store.insert_match(unknown_hero).await.unwrap();

        let calls = std::sync::Arc::new(std::sync::Mutex::new(0u32));
        let calls_bg = std::sync::Arc::clone(&calls);
        let attempt = try_sync_with(&store, dir.path(), None, move |_matches, _| {
            *calls_bg.lock().unwrap() += 1;
            async { Ok(upload_ok()) }
        })
        .await;
        assert!(matches!(attempt, sync::SyncAttempt::NoServerCall));
        assert_eq!(*calls.lock().unwrap(), 0);
        assert!(
            store
                .get_all_matches()
                .await
                .unwrap()
                .iter()
                .all(|row| !row.synced)
        );

        store
            .edit_match(
                "held-map",
                &storage::MatchEdit {
                    map_name: Some("King's Row".into()),
                    ..storage::MatchEdit::default()
                },
            )
            .await
            .unwrap();
        store
            .edit_match(
                "held-hero",
                &storage::MatchEdit {
                    hero: Some("Ana".into()),
                    ..storage::MatchEdit::default()
                },
            )
            .await
            .unwrap();

        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen_bg = std::sync::Arc::clone(&seen);
        let attempt = try_sync_with(&store, dir.path(), None, move |matches, _| {
            let seen_bg = std::sync::Arc::clone(&seen_bg);
            async move {
                seen_bg.lock().unwrap().extend(matches);
                Ok(upload_ok())
            }
        })
        .await;
        assert!(matches!(attempt, sync::SyncAttempt::Uploaded));
        let got = seen.lock().unwrap().clone();
        let body = serde_json::to_string(&sync::upload_request(&got, &[])).unwrap();
        assert!(!body.contains("\"map_name\":\"\""));
        assert!(!body.contains("\"game_mode\":\"\""));
        assert!(!body.contains("Unknown"));
        let parsed: scuffed_types::api::StatsUploadRequest = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed.matches.len(), 2);
        assert!(
            parsed
                .matches
                .iter()
                .any(|m| m.map_name == "King's Row" && m.game_mode == "Hybrid" && m.hero == "Ana")
        );
        assert!(
            parsed
                .matches
                .iter()
                .any(|m| m.map_name == "Busan" && m.game_mode == "Control" && m.hero == "Ana")
        );
        assert!(
            store
                .get_all_matches()
                .await
                .unwrap()
                .iter()
                .all(|row| row.synced)
        );
    }

    #[tokio::test]
    async fn held_game_stays_unsynced_while_a_ready_game_syncs() {
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        store
            .insert_match(test_match("ready-game", "victory"))
            .await
            .unwrap();
        let mut held = test_match("held-game", "victory");
        held.map_name.clear();
        held.game_mode.clear();
        store.insert_match(held).await.unwrap();

        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen_bg = std::sync::Arc::clone(&seen);
        let attempt = try_sync_with(&store, dir.path(), None, move |matches, _| {
            let seen_bg = std::sync::Arc::clone(&seen_bg);
            async move {
                seen_bg.lock().unwrap().extend(matches);
                Ok(upload_ok())
            }
        })
        .await;
        assert!(matches!(attempt, sync::SyncAttempt::Uploaded));
        let got = seen.lock().unwrap().clone();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].session_id, "ready-game");
        let body = serde_json::to_string(&sync::upload_request(&got, &[])).unwrap();
        assert!(!body.contains("\"map_name\":\"\""));

        let rows = store.get_all_matches().await.unwrap();
        let ready = rows
            .iter()
            .find(|row| row.session_id == "ready-game")
            .unwrap();
        let held = rows
            .iter()
            .find(|row| row.session_id == "held-game")
            .unwrap();
        assert!(ready.synced, "the ready game is marked synced");
        assert!(!held.synced, "the held game stays unsynced");
        assert!(held.map_name.is_empty());
    }

    /// Shadow mode is log only: the same capture stored and uploaded with the
    /// flag off and on gives byte-identical stored rows and upload body. The
    /// hand-off below is the one the accept path runs after `stage_capture`.
    #[tokio::test]
    async fn shadow_flag_on_and_off_store_and_upload_identical_rows() {
        async fn run(flag_on: bool) -> (String, String, bool) {
            let dir = tempfile::tempdir().unwrap();
            let cfg = config::Config {
                data_dir: dir.path().to_path_buf(),
                shadow_recognizer: flag_on,
                ..config::Config::default()
            };
            let worker = shadow::worker::ShadowWorker::start_if_enabled(&cfg);
            assert_eq!(worker.is_some(), flag_on);
            let scoreboard = Arc::new(image::DynamicImage::new_rgb8(1664, 1007));
            let rows: Vec<ocr::RowOcrResult> = (0..10)
                .map(|i| ocr::RowOcrResult {
                    name: None,
                    name_alternates: Vec::new(),
                    stats: ["12", "3", "4", "5,480", "950", "2,347"]
                        .iter()
                        .map(|v| ocr::CellOcrResult {
                            value: v.to_string(),
                            confidence: 80 + i,
                            suspect: false,
                        })
                        .collect(),
                    mean_confidence: 80 + i,
                })
                .collect();
            let rows_before = format!("{rows:?}");
            let input = shadow::worker::ShadowInput::when(worker.is_some(), &scoreboard, 5, 1440);
            if let (Some(w), Some(input)) = (&worker, &input) {
                assert!(w.try_submit(shadow::worker::ShadowJob::new(
                    input,
                    &rows,
                    Some(0),
                    "sess-shadow",
                    Utc::now(),
                )));
            }
            let store = storage::LocalStore::open(dir.path()).await.unwrap();
            let mut row = test_match("sess-shadow", "victory");
            row.played_at = SurrealDatetime::from(
                chrono::DateTime::<Utc>::from_timestamp(1_760_000_000, 0).unwrap(),
            );
            store.insert_match(row).await.unwrap();
            let log = dir.path().join("shadow").join("digits.jsonl");
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while flag_on && !log.exists() && std::time::Instant::now() < deadline {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            drop(worker);
            assert_eq!(
                format!("{rows:?}"),
                rows_before,
                "the hand-off never edits rows"
            );
            let stored = store.get_unsynced().await.unwrap();
            assert_eq!(stored.len(), 1);
            let stored_json = serde_json::to_string(&stored).unwrap();
            let body = serde_json::to_string(&sync::upload_request(&stored, &[])).unwrap();
            (stored_json, body, log.exists())
        }
        let (stored_off, body_off, log_off) = run(false).await;
        let (stored_on, body_on, log_on) = run(true).await;
        assert_eq!(
            stored_off, stored_on,
            "stored rows must not depend on shadow mode"
        );
        assert_eq!(
            body_off, body_on,
            "upload body must not depend on shadow mode"
        );
        assert!(!log_off, "flag off writes no shadow log");
        assert!(log_on, "flag on wrote its log line");
    }

    /// The accept path hands the shadow worker shared borrows only, and the
    /// row commit never sees shadow state.
    #[test]
    fn shadow_hook_only_borrows_and_stays_out_of_the_commit() {
        let src = include_str!("main.rs");
        let hook = src
            .find("worker.try_submit(shadow::worker::ShadowJob::new(")
            .expect("shadow hand-off in the accept path");
        let call = &src[hook..hook + 300];
        assert!(call.contains("&rows,"), "{call}");
        assert!(call.contains("&staged.target_session,"), "{call}");
        let commit = src
            .find("async fn commit_capture_rows(")
            .expect("commit_capture_rows");
        let body = src[commit..].split("\nasync fn ").next().unwrap();
        let body = body.split("\nfn ").next().unwrap();
        assert!(
            !body.contains("shadow"),
            "commit_capture_rows must not read shadow state"
        );
    }

    #[tokio::test]
    async fn local_write_during_inflight_upload_stays_unsynced_until_next_sync() {
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        store
            .insert_match(test_match("s1", "victory"))
            .await
            .unwrap();

        let entered = std::sync::Arc::new(tokio::sync::Notify::new());
        let release = std::sync::Arc::new(tokio::sync::Notify::new());
        let uploaded: std::sync::Arc<std::sync::Mutex<Vec<String>>> =
            std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));

        let store_bg = store.clone();
        let dir_bg = dir.path().to_path_buf();
        let entered_bg = entered.clone();
        let release_bg = release.clone();
        let uploaded_bg = uploaded.clone();
        let sync = tokio::spawn(async move {
            try_sync_with(&store_bg, &dir_bg, None, move |matches, _tombstones| {
                let entered_bg = entered_bg.clone();
                let release_bg = release_bg.clone();
                let uploaded_bg = uploaded_bg.clone();
                async move {
                    uploaded_bg
                        .lock()
                        .unwrap()
                        .push(matches[0].display_outcome().to_string());
                    entered_bg.notify_one();
                    release_bg.notified().await;
                    Ok(upload_ok())
                }
            })
            .await;
        });

        entered.notified().await;
        store.set_session_outcome("s1", "defeat").await.unwrap();
        release.notify_one();
        sync.await.unwrap();

        let mid = store.get_all_matches().await.unwrap();
        assert_eq!(mid[0].outcome, "defeat");
        assert!(
            !mid[0].synced,
            "in-flight mark must not commit the revised row"
        );

        let uploaded_bg = uploaded.clone();
        try_sync_with(&store, dir.path(), None, move |matches, _| {
            let uploaded_bg = uploaded_bg.clone();
            async move {
                uploaded_bg
                    .lock()
                    .unwrap()
                    .push(matches[0].display_outcome().to_string());
                Ok(upload_ok())
            }
        })
        .await;

        assert_eq!(
            uploaded.lock().unwrap().as_slice(),
            ["victory", "defeat"],
            "the revised payload is what the follow-up sync uploads"
        );
        let done = store.get_all_matches().await.unwrap();
        assert!(done[0].synced);
        assert_eq!(done[0].outcome, "defeat");
    }

    #[tokio::test]
    async fn shutdown_joins_inflight_sync_before_marking() {
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        store
            .insert_match(test_match("s1", "victory"))
            .await
            .unwrap();

        SYNC_DRAIN_WAITING.store(0, std::sync::atomic::Ordering::SeqCst);
        let release = std::sync::Arc::new(tokio::sync::Notify::new());
        let uploads = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let in_upload = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let overlapped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

        let inflight = tokio::spawn({
            let store = store.clone();
            let dir = dir.path().to_path_buf();
            let release = release.clone();
            let uploads = uploads.clone();
            let in_upload = in_upload.clone();
            let overlapped = overlapped.clone();
            async move {
                try_sync_with(&store, &dir, None, move |_matches, _tombstones| {
                    let release = release.clone();
                    let uploads = uploads.clone();
                    let in_upload = in_upload.clone();
                    let overlapped = overlapped.clone();
                    async move {
                        if in_upload.swap(true, std::sync::atomic::Ordering::SeqCst) {
                            overlapped.store(true, std::sync::atomic::Ordering::SeqCst);
                        }
                        uploads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        release.notified().await;
                        in_upload.store(false, std::sync::atomic::Ordering::SeqCst);
                        Ok(upload_ok())
                    }
                })
                .await;
            }
        });

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while uploads.load(std::sync::atomic::Ordering::SeqCst) == 0 {
            if std::time::Instant::now() > deadline {
                panic!("in-flight sync never entered upload");
            }
            tokio::task::yield_now().await;
        }

        let shutdown = tokio::spawn({
            let store = store.clone();
            let dir = dir.path().to_path_buf();
            let uploads = uploads.clone();
            let in_upload = in_upload.clone();
            let overlapped = overlapped.clone();
            async move {
                drain_sync_then(Some(inflight), || {
                    let store = store.clone();
                    let dir = dir.clone();
                    let uploads = uploads.clone();
                    let in_upload = in_upload.clone();
                    let overlapped = overlapped.clone();
                    async move {
                        try_sync_with(&store, &dir, None, move |_matches, _tombstones| {
                            let uploads = uploads.clone();
                            let in_upload = in_upload.clone();
                            let overlapped = overlapped.clone();
                            async move {
                                if in_upload.swap(true, std::sync::atomic::Ordering::SeqCst) {
                                    overlapped.store(true, std::sync::atomic::Ordering::SeqCst);
                                }
                                uploads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                                in_upload.store(false, std::sync::atomic::Ordering::SeqCst);
                                Ok(upload_ok())
                            }
                        })
                        .await;
                    }
                })
                .await;
            }
        });

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while SYNC_DRAIN_WAITING.load(std::sync::atomic::Ordering::SeqCst) == 0 {
            if shutdown.is_finished() {
                panic!("shutdown finished before joining the in-flight sync");
            }
            if std::time::Instant::now() > deadline {
                panic!("shutdown never blocked in drain_sync_then");
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(
            uploads.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "final sync uploaded while the in-flight sync was still in flight"
        );
        assert!(!overlapped.load(std::sync::atomic::Ordering::SeqCst));

        release.notify_one();
        shutdown.await.unwrap();

        assert!(!overlapped.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(
            uploads.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "shutdown sync must not upload rows the in-flight sync already marked"
        );
        let rows = store.get_all_matches().await.unwrap();
        assert!(rows.iter().all(|m| m.synced));
    }

    #[test]
    fn unsafe_sync_url_yields_no_client() {
        assert!(open_sync_client(None).is_none());
        let clear = config::SyncConfig {
            server_url: "http://example.com".into(),
            token: "secret".into(),
        };
        assert!(
            open_sync_client(Some(&clear)).is_none(),
            "daemon must not build a client that could send the token"
        );
        let https = config::SyncConfig {
            server_url: "https://crew.example".into(),
            token: "secret".into(),
        };
        assert!(open_sync_client(Some(&https)).is_some());
        for url in ["http://localhost:3030", "http://127.0.0.1", "http://[::1]"] {
            let cfg = config::SyncConfig {
                server_url: url.into(),
                token: "secret".into(),
            };
            assert!(
                open_sync_client(Some(&cfg)).is_some(),
                "{url} is the local-dev exception"
            );
        }
    }

    #[test]
    fn periodic_sync_respects_single_flight_and_backoff() {
        assert!(
            !periodic_sync_due(4, false, true),
            "not yet every N captures"
        );
        assert!(periodic_sync_due(5, false, true));
        assert!(
            !periodic_sync_due(5, true, true),
            "single-flight: an in-flight upload blocks the next spawn"
        );
        assert!(
            !periodic_sync_due(10, false, false),
            "backoff window blocks periodic sync"
        );
        assert!(
            periodic_sync_due(10, false, true),
            "a reset backoff allows the next periodic sync"
        );
    }

    #[test]
    fn quiet_close_waits_for_grace_and_does_not_invent_an_outcome() {
        let now = test_now();
        let close = finished_game_close_after(30);
        assert_eq!(
            close, POST_MATCH_GRACE,
            "a shorter setting still waits out grace"
        );
        let mut finished = game(detect::MatchOutcome::Defeat, Some(0), now);
        finished.last_activity = now;
        finished.outcome_recorded_at = Some(now);
        assert!(
            quiet_close_reason(&finished, now + POST_MATCH_GRACE, close).is_none(),
            "grace is still open at exactly 75s"
        );
        assert_eq!(
            quiet_close_reason(
                &finished,
                now + POST_MATCH_GRACE + Duration::from_secs(1),
                close
            ),
            Some("finished game closed after quiet period")
        );
        let close = finished_game_close_after(config::FINISHED_GAME_CLOSE_DEFAULT_SECS);
        assert!(quiet_close_reason(&finished, now + Duration::from_secs(179), close).is_none());
        assert_eq!(
            quiet_close_reason(&finished, now + Duration::from_secs(180), close),
            Some("finished game closed after quiet period")
        );
        let mut hinted = game(detect::MatchOutcome::Unknown, None, now);
        hinted.last_activity = now;
        hinted.result_mark = Some(ResultMark {
            outcome: detect::MatchOutcome::Defeat,
            confirmed: false,
            seen_at: now,
        });
        assert!(
            quiet_close_reason(&hinted, now + Duration::from_secs(180), close).is_none(),
            "a hint is not a recorded result"
        );
        assert!(
            quiet_close_reason(&hinted, now + UNFINISHED_SESSION_IDLE, close).is_none(),
            "the timer does not close an unfinished game"
        );
        assert!(
            quiet_close_reason(
                &hinted,
                now + UNFINISHED_SESSION_IDLE + Duration::from_secs(1),
                close
            )
            .is_none(),
            "a long match stays open until a result or a later Tab"
        );
        assert!(
            quiet_close_reason(
                &hinted,
                now + UNFINISHED_SESSION_MAX - Duration::from_secs(1),
                close
            )
            .is_none(),
            "just short of the long bound the unfinished game stays open"
        );
        assert_eq!(
            quiet_close_reason(&hinted, now + UNFINISHED_SESSION_MAX, close),
            Some(UNFINISHED_MAX_REASON),
            "after several hours an unfinished game closes"
        );
        assert_eq!(
            session_close_log("finished game closed after quiet period"),
            "finished game closed after quiet period"
        );
        assert_eq!(
            session_close_log("superseded by Tab-opened game"),
            "closed session at new-game boundary"
        );
    }

    /// P1c. A start that the pid guard is going to refuse must not reach
    /// the site or the pack files first, and the reader pack job must be
    /// started for every signed-in daemon, not only when the startup sync
    /// got a list. The crash marker stays where it was: after the guard and
    /// after the vacuum exit.
    #[test]
    fn startup_takes_the_pid_guard_before_the_site_and_always_starts_the_pack_keeper() {
        let src = include_str!("main.rs");
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
            "auth_pause_matches(",
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
        let src = include_str!("main.rs");
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
    }

    /// Production poll cadence. A stable screen is emitted on every tick.
    const NIGHT_TICKS: usize = 4;

    fn night_tick() -> Duration {
        config::AutoDetectConfig::default().poll_interval(false)
    }

    fn night_debounce() -> Duration {
        Duration::from_secs(config::AutoDetectConfig::default().cooldown_secs)
    }

    fn night_counters(e: u32, a: u32, d: u32, dmg: u32, hlg: u32, mit: u32) -> Counters {
        Counters {
            elims: e,
            assists: a,
            deaths: d,
            damage: dmg,
            healing: hlg,
            mitigation: mit,
        }
    }

    const NIGHT_CLEAN: [bool; stat_tracker::capture_gate::GATE_COLS] =
        [false; stat_tracker::capture_gate::GATE_COLS];

    /// Drives the real poll and Tab path at the production cadence, with the
    /// gap, grace, debounce, and idle bounds left on. Time is the injected clock.
    struct Night {
        st: SessionState,
        store: storage::LocalStore,
        dir: tempfile::TempDir,
        now: Instant,
        wall: chrono::DateTime<Utc>,
        syncs: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        uploads: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        /// Overrides the "a row index means per-cell stats" bit for the next
        /// Tab. `None` keeps that bit. `Some(false)` is an identified row
        /// whose cells were unreadable, so the text fallback was used.
        cells_trusted: Option<bool>,
        /// The next Tab's top-bar label is a sure banner read
        /// (`reader = "new"`), not the top-bar OCR.
        panel_from_banner: bool,
    }

    impl Night {
        async fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let store = storage::LocalStore::open(dir.path()).await.unwrap();
            let now = test_now();
            Self {
                st: session(None, now),
                store,
                dir,
                now,
                wall: Utc::now(),
                syncs: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                uploads: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                cells_trusted: None,
                panel_from_banner: false,
            }
        }

        fn sync_count(&self) -> usize {
            self.syncs.load(std::sync::atomic::Ordering::SeqCst)
        }

        fn upload_count(&self) -> usize {
            self.uploads.load(std::sync::atomic::Ordering::SeqCst)
        }

        /// One command tick of the quiet-session timer. Same function the
        /// daemon loop calls. The closure schedules the upload. It is not
        /// the shutdown upload.
        async fn quiet_tick(&mut self) -> bool {
            let syncs = std::sync::Arc::clone(&self.syncs);
            let uploads = std::sync::Arc::clone(&self.uploads);
            let store = self.store.clone();
            let dir = self.dir.path().to_path_buf();
            close_quiet_session_and_sync(
                &mut self.st,
                &self.store,
                self.dir.path(),
                self.now,
                finished_game_close_after(config::FINISHED_GAME_CLOSE_DEFAULT_SECS),
                || {
                    let syncs = std::sync::Arc::clone(&syncs);
                    let uploads = std::sync::Arc::clone(&uploads);
                    let store = store.clone();
                    let dir = dir.clone();
                    async move {
                        syncs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        let _ = try_sync_with(&store, &dir, None, move |_matches, _tombstones| {
                            let uploads = std::sync::Arc::clone(&uploads);
                            async move {
                                uploads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                                Ok(upload_ok())
                            }
                        })
                        .await;
                    }
                },
            )
            .await
        }

        /// Step the injected clock the way the 3-second command tick does,
        /// and run the quiet-close timer on each step.
        async fn idle_through(&mut self, until: Instant) {
            while self.now < until {
                let step = until
                    .saturating_duration_since(self.now)
                    .min(Duration::from_secs(3));
                if step.is_zero() {
                    break;
                }
                self.advance(step);
                self.quiet_tick().await;
            }
        }

        /// What a daemon restart does with the on-disk skeleton.
        async fn restart_from_disk(&self) -> Option<ActiveGame> {
            let syncs = std::sync::Arc::clone(&self.syncs);
            let uploads = std::sync::Arc::clone(&self.uploads);
            let store = self.store.clone();
            let dir = self.dir.path().to_path_buf();
            recover_or_sync_active_game(self.dir.path(), &self.store, |dropped| {
                let syncs = std::sync::Arc::clone(&syncs);
                let uploads = std::sync::Arc::clone(&uploads);
                let store = store.clone();
                let dir = dir.clone();
                async move {
                    syncs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let _ = try_sync_with(&store, &dir, dropped, move |_matches, _tombstones| {
                        let uploads = std::sync::Arc::clone(&uploads);
                        async move {
                            uploads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                            Ok(upload_ok())
                        }
                    })
                    .await;
                }
            })
            .await
            .active_game
        }

        fn advance(&mut self, by: Duration) {
            self.now += by;
            self.wall += chrono::Duration::from_std(by).unwrap();
        }

        fn game(&self) -> &ActiveGame {
            self.st.active_game.as_ref().expect("active game")
        }

        fn id(&self) -> String {
            self.game().session_id.clone()
        }

        /// A match already underway, the way a recovered session looks.
        fn begin_on(&mut self, map: &str, hero: &str) {
            let mut g = ActiveGame::open_at(
                format!("{:016x}", rand_id()),
                detect::MatchOutcome::Unknown,
                Vec::new(),
                self.now,
            );
            g.map = Some(map.to_string());
            g.map_source = Some(boundary::MapSource::TopBar);
            g.hero_auth.accepted_hero = Some(hero.to_string());
            self.st.active_game = Some(g);
            self.st.last_game_open = Some(self.now);
        }

        async fn poll_screen(&mut self, screen: boundary::StartScreen) {
            self.apply_resolved_tick(
                ResolvedWordTick {
                    signal: None,
                    signal_confirmed: false,
                    accolade_map: None,
                },
                Some(screen),
            )
            .await;
        }

        /// Apply one resolved poll tick through [`apply_poll_tick`], the
        /// same function [`run_loop`] calls. [`resolve_word_tick`] has
        /// already noted the streak. When no game is open, this records a
        /// confirmed word as the pending outcome.
        async fn apply_resolved_tick(
            &mut self,
            tick: ResolvedWordTick,
            start_screen: Option<boundary::StartScreen>,
        ) {
            apply_poll_tick(
                &mut self.st,
                &self.store,
                self.dir.path(),
                &tick,
                start_screen,
                self.now,
                night_debounce(),
                // the member-edit rule of `reader = "new"`, as on the Tab path
                true,
            )
            .await;
        }

        /// Several consecutive ticks of one stable screen.
        async fn screen(&mut self, screen: boundary::StartScreen) {
            for i in 0..NIGHT_TICKS {
                self.poll_screen(screen.clone()).await;
                if i + 1 != NIGHT_TICKS {
                    self.advance(night_tick());
                }
            }
        }

        /// One result-word tick through [`resolve_word_tick`], including
        /// when no game is open. Production notes that streak too.
        async fn word_tick(&mut self, outcome: detect::MatchOutcome, map: Option<&str>) {
            let tick = resolve_word_tick(
                &mut self.st.word_outcome_streak,
                Some((outcome, detect::match_end::OutcomeSource::ResultWord)),
                self.now,
                map.map(str::to_string),
            );
            self.apply_resolved_tick(tick, None).await;
        }

        /// A result word held across several ticks. The second agreeing tick confirms.
        async fn word(&mut self, outcome: detect::MatchOutcome, map: Option<&str>) {
            for i in 0..NIGHT_TICKS {
                self.word_tick(outcome, map).await;
                if i + 1 != NIGHT_TICKS {
                    self.advance(night_tick());
                }
            }
        }

        /// A poll tick with no result word and no start screen.
        async fn no_signal_tick(&mut self) {
            self.apply_resolved_tick(
                ResolvedWordTick {
                    signal: None,
                    signal_confirmed: false,
                    accolade_map: None,
                },
                None,
            )
            .await;
        }

        /// One unconfirmed read. Production has not seen the agreeing tick yet.
        async fn word_once(&mut self, outcome: detect::MatchOutcome) {
            self.word_tick(outcome, None).await;
        }

        /// A banner confirms on one tick and has no accolade map. Production
        /// reads the map crop only on result-word ticks.
        async fn banner(&mut self, outcome: detect::MatchOutcome) {
            let tick = resolve_word_tick(
                &mut self.st.word_outcome_streak,
                Some((outcome, detect::match_end::OutcomeSource::Banner)),
                self.now,
                None,
            );
            self.apply_resolved_tick(tick, None).await;
        }

        async fn tab_once(
            &mut self,
            cur: Counters,
            hero: &str,
            row: Option<u32>,
            map: Option<&str>,
            suspect: [bool; stat_tracker::capture_gate::GATE_COLS],
        ) {
            self.tab_read(
                cur,
                hero,
                row,
                (map, map.unwrap_or("")),
                suspect,
                detect::MatchOutcome::Unknown,
            )
            .await;
        }

        /// The top-bar label is a sure banner read (`reader = "new"`).
        async fn tab_banner(&mut self, cur: Counters, hero: &str, row: Option<u32>, map: &str) {
            self.panel_from_banner = true;
            self.tab_once(cur, hero, row, Some(map), NIGHT_CLEAN).await;
        }

        /// The map came from full-board text. There is no top-bar label.
        async fn tab_text(
            &mut self,
            cur: Counters,
            hero: &str,
            row: Option<u32>,
            text_map: &str,
            suspect: [bool; stat_tracker::capture_gate::GATE_COLS],
        ) {
            self.tab_read(
                cur,
                hero,
                row,
                (None, text_map),
                suspect,
                detect::MatchOutcome::Unknown,
            )
            .await;
        }

        /// One Tab through the production request, plan, [`stage_capture`], and report.
        /// `maps` is the top-bar label and the full-board fallback. `frame` is
        /// the header on this board. Unknown is a board with none.
        async fn tab_read(
            &mut self,
            cur: Counters,
            hero: &str,
            row: Option<u32>,
            maps: (Option<&str>, &str),
            suspect: [bool; stat_tracker::capture_gate::GATE_COLS],
            frame: detect::MatchOutcome,
        ) {
            let (panel, text_map) = maps;
            let opened =
                open_fresh_if_tab_starts_one(&mut self.st, &self.store, self.dir.path(), self.now)
                    .await;
            let mut req = build_capture_request(self.game(), opened, self.now);
            req.member_map_edit = session_has_member_map_edit(&self.store, &req).await;
            let (hero_resolved, source, hero_auth) = hero_auth::resolve_hero(
                Some(hero),
                None,
                hero,
                &req.hero_auth,
                parse::canonical_hero,
            );
            let career_panel = matches!(source, HeroSource::CareerPanel);
            let cells_trusted = self.cells_trusted.take().unwrap_or(row.is_some());
            let facts = BoardFacts {
                counters: cur,
                suspect,
                trusted_cells: cells_trusted,
                held: [false; capture_gate::GATE_COLS],
                suspect_row: false,
                row_id: row,
                hero: &hero_resolved,
                map_from_panel: panel,
                panel_from_banner: std::mem::take(&mut self.panel_from_banner),
                parsed_map: text_map,
                frame_outcome: frame,
            };
            let plan = plan_from_board(&req, &facts);
            let sid = req.session_id.clone();
            if plan.skip_store {
                let report = skipped_capture_report(
                    &plan,
                    &sid,
                    career_panel,
                    hero_auth,
                    plan.defer.then(|| hero_resolved.clone()),
                    plan.defer.then_some(self.wall),
                );
                apply_capture_report(
                    &mut self.st,
                    &self.store,
                    self.dir.path(),
                    &sid,
                    Ok(report),
                    self.now,
                )
                .await;
                return;
            }
            let mut staged = stage_capture(&req, &plan, &facts, self.wall);
            let mut parsed = test_match(&staged.target_session, &staged.outcome_label);
            if prepare_capture_row(
                &mut parsed,
                &mut staged,
                req.session_map.as_deref(),
                panel,
                text_map,
                true,
            ) == PreparedRow::Drop
            {
                let report = CaptureReport {
                    best_guess: None,
                    recorded: false,
                    outcome: staged.outcome,
                    map: None,
                    map_source: None,
                    session_id: sid.clone(),
                    split: false,
                    armed_reset: false,
                    ignore_row: false,
                    reset_streak: plan.reset_streak,
                    reset_baseline: plan.reset_baseline,
                    baseline_row: plan.baseline_row,
                    refresh_baseline: false,
                    clear_hint: false,
                    count_progress: false,
                    career_panel,
                    held_counters: None,
                    held_hero: None,
                    gate_state: None,
                    hero_auth,
                    seal: None,
                    close_reason: None,
                    held_at: None,
                    map_replaces_text_fallback: false,
                };
                apply_capture_report(
                    &mut self.st,
                    &self.store,
                    self.dir.path(),
                    &sid,
                    Ok(report),
                    self.now,
                )
                .await;
                return;
            }
            write_staged_rows(
                &self.store,
                &staged,
                &hero_resolved,
                "",
                SurrealDatetime::from(self.wall),
            )
            .await
            .unwrap();
            self.write_board(
                &staged.target_session,
                &hero_resolved,
                &staged.map_name,
                &staged.outcome_label,
                staged.gate.accepted,
                self.wall,
            )
            .await;
            write_carried_after_current(
                &self.store,
                self.dir.path(),
                &staged,
                &hero_resolved,
                true,
            )
            .await
            .unwrap();
            let report = CaptureReport {
                best_guess: None,
                recorded: true,
                outcome: staged.outcome,
                map: staged.recorded_map,
                map_source: staged.map_source,
                session_id: staged.target_session,
                split: staged.split,
                armed_reset: false,
                ignore_row: plan.ignore_row,
                reset_streak: plan.reset_streak,
                reset_baseline: plan.reset_baseline,
                baseline_row: plan.baseline_row,
                refresh_baseline: plan.refresh_baseline,
                clear_hint: plan.clear_hint,
                count_progress: plan.count_progress,
                career_panel,
                held_counters: None,
                held_hero: None,
                gate_state: Some(staged.gate.state),
                hero_auth,
                seal: plan.seal,
                close_reason: plan.close_reason,
                held_at: None,
                map_replaces_text_fallback: staged.replaced_text_fallback,
            };
            apply_capture_report(
                &mut self.st,
                &self.store,
                self.dir.path(),
                &sid,
                Ok(report),
                self.now,
            )
            .await;
        }

        async fn write_board(
            &self,
            id: &str,
            hero: &str,
            map: &str,
            outcome: &str,
            counters: Counters,
            at: chrono::DateTime<Utc>,
        ) {
            store_held_board(
                &self.store,
                self.dir.path(),
                id,
                hero,
                map,
                outcome,
                counters,
                at,
            )
            .await
            .unwrap();
        }

        /// The same scoreboard captured on several ticks, the way a held Tab
        /// is sampled. A fresh board uses [`Self::tab_once`]: a second sample
        /// inside 45 seconds is not a second reset signal.
        async fn tab_held(
            &mut self,
            cur: Counters,
            hero: &str,
            row: Option<u32>,
            map: Option<&str>,
        ) {
            for i in 0..NIGHT_TICKS {
                self.tab_once(cur, hero, row, map, NIGHT_CLEAN).await;
                if i + 1 != NIGHT_TICKS {
                    self.advance(night_tick());
                }
            }
        }

        async fn elims(&self, session: &str) -> Vec<u32> {
            self.store
                .get_session_snapshots(session)
                .await
                .unwrap()
                .into_iter()
                .map(|row| row.elims)
                .collect()
        }
    }

    fn vote(candidates: &[&str]) -> boundary::StartScreen {
        boundary::StartScreen::MapVote {
            candidates: candidates.iter().map(|c| (*c).to_string()).collect(),
        }
    }

    #[tokio::test]
    async fn a_fallback_board_yields_to_two_matching_clean_reads_and_a_later_ghost_is_rate_capped()
    {
        let mut night = Night::new().await;
        night.begin_on("Dorado", "Wrecking Ball");
        night
            .tab_once(
                night_counters(2, 40, 18, 450, 0, 2),
                "Wrecking Ball",
                None,
                Some("Dorado"),
                NIGHT_CLEAN,
            )
            .await;
        let latched = night.game().gate.expect("fallback stored");
        assert!(latched.low_trust);
        assert_eq!(latched.accepted.assists, 40);

        night.advance(Duration::from_secs(20));
        night
            .tab_once(
                night_counters(2, 0, 0, 1105, 259, 450),
                "Wrecking Ball",
                Some(0),
                Some("Dorado"),
                NIGHT_CLEAN,
            )
            .await;
        let once = night.game().gate.expect("first clean read stored");
        assert_eq!(once.accepted.assists, 40, "one low read keeps the latch");
        assert_eq!(once.accepted.deaths, 18);
        assert!(once.low_trust);

        night.advance(Duration::from_secs(20));
        night
            .tab_once(
                night_counters(2, 0, 0, 1105, 259, 450),
                "Wrecking Ball",
                Some(0),
                Some("Dorado"),
                NIGHT_CLEAN,
            )
            .await;
        let cleaned = night.game().gate.expect("second clean read stored");
        assert_eq!(cleaned.accepted.assists, 0);
        assert_eq!(cleaned.accepted.deaths, 0);
        assert_eq!(cleaned.accepted.damage, 1105);
        assert!(!cleaned.low_trust);

        night.advance(Duration::from_secs(20));
        night
            .tab_once(
                night_counters(91, 0, 0, 1105, 259, 450),
                "Wrecking Ball",
                Some(0),
                Some("Dorado"),
                NIGHT_CLEAN,
            )
            .await;
        let ghost = night.game().gate.expect("ghost stored");
        assert_eq!(ghost.accepted.elims, 2, "the rate cap still holds a ghost");
        assert!(!ghost.low_trust);
    }

    #[tokio::test]
    async fn an_identified_row_can_still_be_an_untrusted_fallback() {
        // The row index is set, and the cells were unreadable, so the
        // numbers came from the text fallback. Trust does not follow the
        // row index.
        let mut night = Night::new().await;
        night.begin_on("Dorado", "Wrecking Ball");
        night.cells_trusted = Some(false);
        night
            .tab_once(
                night_counters(2, 40, 18, 450, 0, 2),
                "Wrecking Ball",
                Some(0),
                Some("Dorado"),
                NIGHT_CLEAN,
            )
            .await;
        let latched = night.game().gate.expect("fallback stored");
        assert!(latched.low_trust);
        assert!(latched.unconfirmed.iter().all(|&col| col));
        assert_eq!(latched.accepted.assists, 40);
    }

    #[tokio::test]
    async fn multi_tick_vote_ban_and_select_keep_one_session_and_its_candidates() {
        let mut night = Night::new().await;
        let candidates = ["Junkertown", "Ilios"];
        night.screen(vote(&candidates)).await;
        let id = night.id();
        assert_eq!(night.game().map_candidates, vec!["Junkertown", "Ilios"]);
        night.screen(boundary::StartScreen::HeroBan).await;
        night.screen(boundary::StartScreen::HeroSelect).await;
        assert_eq!(night.id(), id, "a stable start screen must not re-split");
        assert_eq!(
            night.game().map_candidates,
            vec!["Junkertown", "Ilios"],
            "ban and select keep the vote candidates"
        );
        assert!(night.game().awaiting_first_board);
        let rows = night.elims(&id).await;
        assert!(rows.is_empty(), "no false unrecorded split wrote a board");
    }

    #[tokio::test]
    async fn board_one_defeat_then_vote_seals_a_and_opens_b_with_no_tab() {
        let mut night = Night::new().await;
        night.begin_on("Busan", "Zenyatta");
        night
            .tab_held(
                night_counters(14, 22, 6, 2400, 9800, 400),
                "Zenyatta",
                Some(2),
                Some("Busan"),
            )
            .await;
        night.advance(Duration::from_secs(8 * 60));
        night.word_once(detect::MatchOutcome::Defeat).await;
        let a = night.id();
        night.advance(Duration::from_secs(20));
        night.screen(vote(&["Junkertown", "Ilios"])).await;
        night.screen(boundary::StartScreen::HeroBan).await;
        night.screen(boundary::StartScreen::HeroSelect).await;
        assert_ne!(night.id(), a, "the vote opens B");
        assert_eq!(night.elims(&a).await, vec![14, 14, 14, 14]);
        let snaps = night.store.get_session_snapshots(&a).await.unwrap();
        assert!(
            snaps.iter().all(|row| row.outcome == "defeat"),
            "A stays Defeat, got {:?}",
            snaps
                .iter()
                .map(|row| row.outcome.clone())
                .collect::<Vec<_>>()
        );
        night
            .word(detect::MatchOutcome::Victory, Some("Junkertown"))
            .await;
        assert_eq!(night.game().outcome, detect::MatchOutcome::Victory);
        assert_ne!(night.id(), a);
        assert!(
            night.elims(&night.id()).await.is_empty(),
            "B was played with no Tab"
        );
    }

    #[tokio::test]
    async fn one_read_on_b_then_c_does_not_seal_b_onto_a() {
        let mut night = Night::new().await;
        night.begin_on("Busan", "Zenyatta");
        night
            .tab_once(
                night_counters(14, 22, 6, 2400, 9800, 400),
                "Zenyatta",
                Some(2),
                Some("Busan"),
                NIGHT_CLEAN,
            )
            .await;
        night.advance(Duration::from_secs(8 * 60));
        night.word_once(detect::MatchOutcome::Defeat).await;
        let a = night.id();
        night.advance(Duration::from_secs(30));
        night.screen(vote(&["Junkertown"])).await;
        let b = night.id();
        assert_ne!(b, a);
        night.word_once(detect::MatchOutcome::Victory).await;
        night.advance(Duration::from_secs(20));
        night
            .tab_once(
                night_counters(6, 2, 1, 900, 200, 400),
                "Wrecking Ball",
                Some(0),
                Some("Junkertown"),
                NIGHT_CLEAN,
            )
            .await;
        assert_eq!(night.id(), b, "B's first Tab is B's board");
        night.advance(Duration::from_secs(130));
        night
            .tab_once(
                night_counters(2, 1, 0, 350, 60, 800),
                "Reinhardt",
                Some(1),
                Some("Ilios"),
                NIGHT_CLEAN,
            )
            .await;
        assert_ne!(night.id(), b, "C's gap closes B");
        assert_ne!(night.id(), a);
        let snaps = night.store.get_session_snapshots(&a).await.unwrap();
        assert!(
            snaps.iter().all(|row| row.outcome == "defeat"),
            "C's gap must not seal B's word onto A"
        );
        let b_rows = night.store.get_session_snapshots(&b).await.unwrap();
        assert!(
            b_rows.iter().all(|row| row.outcome == "victory"),
            "B's one read is sealed on B"
        );
    }

    #[tokio::test]
    async fn a_tab_on_b_does_not_move_bs_board_onto_c() {
        let mut night = Night::new().await;
        night.begin_on("Busan", "Zenyatta");
        // The debounce runs from when the session opened, the gap from the
        // last board. Open long enough for the vote, then take A's board.
        night.advance(Duration::from_secs(130));
        night
            .tab_once(
                night_counters(14, 22, 6, 2400, 9800, 400),
                "Zenyatta",
                Some(2),
                Some("Busan"),
                NIGHT_CLEAN,
            )
            .await;
        night.advance(Duration::from_secs(20));
        night.word_once(detect::MatchOutcome::Defeat).await;
        let a = night.id();
        night.advance(Duration::from_secs(20));
        night.screen(vote(&["Junkertown"])).await;
        let b = night.id();
        night.advance(Duration::from_secs(20));
        night
            .tab_once(
                night_counters(3, 1, 1, 400, 80, 500),
                "Wrecking Ball",
                Some(0),
                Some("Junkertown"),
                NIGHT_CLEAN,
            )
            .await;
        assert_eq!(night.id(), b, "B's first Tab stays on B");
        assert_eq!(night.elims(&b).await, vec![3]);
        assert_eq!(night.elims(&a).await, vec![14]);
        night.advance(Duration::from_secs(130));
        night.screen(vote(&["Ilios"])).await;
        assert_ne!(night.id(), b);
        assert_eq!(night.elims(&a).await, vec![14]);
        assert_eq!(night.elims(&b).await, vec![3]);
    }

    #[tokio::test]
    async fn fresh_misread_then_hero_swap_does_not_split() {
        let mut night = Night::new().await;
        night.begin_on("Busan", "Zenyatta");
        night
            .tab_once(
                night_counters(14, 22, 6, 2400, 9800, 400),
                "Zenyatta",
                Some(2),
                Some("Busan"),
                NIGHT_CLEAN,
            )
            .await;
        let id = night.id();
        night.advance(Duration::from_secs(60));
        night
            .tab_once(
                night_counters(2, 1, 0, 350, 60, 800),
                "Zenyatta",
                Some(2),
                Some("Busan"),
                NIGHT_CLEAN,
            )
            .await;
        assert_eq!(night.id(), id);
        assert!(night.game().deferred.is_some(), "the misread is held");
        assert_eq!(night.elims(&id).await, vec![14]);
        night.advance(Duration::from_secs(30));
        night.screen(boundary::StartScreen::HeroSelect).await;
        assert_eq!(
            night.id(),
            id,
            "a hero swap after one misread does not split"
        );
        assert_eq!(night.elims(&id).await, vec![14]);
    }

    #[tokio::test]
    async fn ordinary_growth_is_not_the_short_band_and_uses_the_baseline_clock() {
        let mut night = Night::new().await;
        night.begin_on("Busan", "Ana");
        night
            .tab_once(
                night_counters(4, 3, 2, 1500, 400, 300),
                "Ana",
                Some(0),
                Some("Busan"),
                NIGHT_CLEAN,
            )
            .await;
        // E4 at 2:00, E9 at 5:00. Three minutes of ordinary 2-4x growth.
        night.advance(Duration::from_secs(2 * 60));
        night
            .tab_once(
                night_counters(4, 3, 2, 1500, 400, 300),
                "Ana",
                Some(0),
                Some("Busan"),
                NIGHT_CLEAN,
            )
            .await;
        night.advance(Duration::from_secs(3 * 60));
        night
            .tab_once(
                night_counters(9, 7, 4, 3800, 900, 800),
                "Ana",
                Some(0),
                Some("Busan"),
                NIGHT_CLEAN,
            )
            .await;
        assert_eq!(
            night.game().reset_baseline.map(|gate| gate.accepted.elims),
            Some(9),
            "a long-age 2-4x row becomes the baseline"
        );
        // Fresh session for the baseline clock. The first Tab is past the gap,
        // so the gate accepts the raw row instead of latching it to the
        // previous game. A short 2-4x row keeps the baseline. A spike at
        // 6:00 is stored and still is not the baseline. E17 twenty seconds
        // later is ordinary growth from that baseline.
        night.screen(vote(&["Ilios"])).await;
        night.advance(Duration::from_secs(130));
        night
            .tab_once(
                night_counters(4, 3, 2, 1500, 400, 300),
                "Ana",
                Some(0),
                Some("Ilios"),
                NIGHT_CLEAN,
            )
            .await;
        night.advance(Duration::from_secs(30));
        night
            .tab_once(
                night_counters(10, 8, 6, 4000, 1000, 800),
                "Ana",
                Some(0),
                Some("Ilios"),
                NIGHT_CLEAN,
            )
            .await;
        assert_eq!(
            night.game().reset_baseline.map(|gate| gate.accepted.elims),
            Some(4),
            "the short 2-4x band does not move the baseline"
        );
        night.advance(Duration::from_secs(6 * 60 - 30));
        night
            .tab_once(
                night_counters(90, 40, 40, 30000, 50000, 5000),
                "Ana",
                Some(0),
                Some("Ilios"),
                NIGHT_CLEAN,
            )
            .await;
        assert_eq!(
            night.game().reset_baseline.map(|gate| gate.accepted.elims),
            Some(4),
            "the spike is stored and is not the baseline"
        );
        night.advance(Duration::from_secs(20));
        night
            .tab_once(
                night_counters(17, 8, 5, 6100, 1200, 900),
                "Ana",
                Some(0),
                Some("Ilios"),
                NIGHT_CLEAN,
            )
            .await;
        assert_eq!(
            night.game().reset_baseline.map(|gate| gate.accepted.elims),
            Some(17),
            "E17 at 6:20 is normal growth when age is measured from the baseline"
        );
    }

    #[tokio::test]
    async fn a_deferred_board_is_dropped_when_the_match_continues() {
        let mut night = Night::new().await;
        night.begin_on("Busan", "Zenyatta");
        night
            .tab_once(
                night_counters(14, 22, 6, 2400, 9800, 400),
                "Zenyatta",
                Some(2),
                Some("Busan"),
                NIGHT_CLEAN,
            )
            .await;
        night.advance(Duration::from_secs(60));
        night
            .tab_once(
                night_counters(2, 1, 0, 350, 60, 800),
                "Zenyatta",
                Some(2),
                Some("Busan"),
                NIGHT_CLEAN,
            )
            .await;
        assert!(night.game().deferred.is_some());
        night.advance(Duration::from_secs(10));
        night
            .tab_once(
                night_counters(16, 23, 6, 2600, 10000, 420),
                "Zenyatta",
                Some(2),
                Some("Busan"),
                NIGHT_CLEAN,
            )
            .await;
        assert!(night.game().deferred.is_none());
        assert_eq!(night.elims(&night.id()).await, vec![14, 16]);
    }

    #[tokio::test]
    async fn a_carried_deferred_board_is_written_on_the_new_session() {
        let mut night = Night::new().await;
        night.begin_on("Busan", "Zenyatta");
        night
            .tab_once(
                night_counters(18, 7, 9, 6400, 11000, 800),
                "Zenyatta",
                Some(2),
                Some("Busan"),
                NIGHT_CLEAN,
            )
            .await;
        night.advance(Duration::from_secs(30));
        night
            .word(detect::MatchOutcome::Defeat, Some("Busan"))
            .await;
        night.advance(Duration::from_secs(50));
        let first = night_counters(1, 3, 0, 220, 80, 400);
        night
            .tab_once(first, "Wrecking Ball", Some(2), Some("Busan"), NIGHT_CLEAN)
            .await;
        assert_eq!(night.game().deferred, Some(first));
        assert!(!night.game().deferred_imported);
        let held_on = night.id();
        night.advance(Duration::from_secs(20));
        night.screen(boundary::StartScreen::HeroSelect).await;
        let opened = night.id();
        assert_ne!(opened, held_on, "the confirmed game closes");
        assert!(
            night.game().deferred_imported,
            "the held board is carried onto the session the start screen opened"
        );
        assert_eq!(night.game().deferred, Some(first));
        let next = night_counters(6, 2, 1, 900, 200, 400);
        night
            .tab_once(
                next,
                "Wrecking Ball",
                Some(0),
                Some("Junkertown"),
                NIGHT_CLEAN,
            )
            .await;
        assert_eq!(night.id(), opened, "the carried board does not split again");
        let rows = night.elims(&opened).await;
        assert!(
            rows.contains(&1),
            "the carried board is written on the new session, got {rows:?}"
        );
        assert!(rows.contains(&6), "the new session's own board is stored");
        assert!(
            !night.elims(&held_on).await.contains(&1),
            "the held board did not land on the session that deferred it"
        );
    }

    #[tokio::test]
    async fn a_carried_board_is_stored_once_and_the_accolade_can_correct_the_map() {
        let mut night = Night::new().await;
        night.begin_on("Busan", "Zenyatta");
        night
            .tab_once(
                night_counters(18, 7, 9, 6400, 11000, 800),
                "Zenyatta",
                Some(2),
                Some("Busan"),
                NIGHT_CLEAN,
            )
            .await;
        night.advance(Duration::from_secs(30));
        night
            .word(detect::MatchOutcome::Defeat, Some("Busan"))
            .await;
        night.advance(Duration::from_secs(50));
        let carried = night_counters(1, 3, 0, 220, 80, 400);
        night
            .tab_once(
                carried,
                "Wrecking Ball",
                Some(2),
                Some("Busan"),
                NIGHT_CLEAN,
            )
            .await;
        assert_eq!(night.game().deferred, Some(carried));
        night.advance(Duration::from_secs(20));
        night.screen(boundary::StartScreen::HeroSelect).await;
        let opened = night.id();
        assert!(
            night.game().deferred_imported,
            "the held board is carried onto the session the start screen opened"
        );
        night.advance(Duration::from_secs(10));
        night
            .tab_text(
                night_counters(3, 1, 0, 400, 100, 200),
                "Wrecking Ball",
                None,
                "Dorado",
                NIGHT_CLEAN,
            )
            .await;
        assert_eq!(night.id(), opened);
        assert_eq!(night.game().map.as_deref(), Some("Dorado"));
        assert_eq!(
            night.game().map_source,
            Some(boundary::MapSource::TextFallback)
        );
        night.advance(Duration::from_secs(10));
        night
            .tab_text(
                night_counters(4, 2, 1, 500, 150, 250),
                "Wrecking Ball",
                None,
                "Dorado",
                NIGHT_CLEAN,
            )
            .await;
        let rows = night.elims(&opened).await;
        assert_eq!(
            rows.iter().filter(|elims| **elims == 1).count(),
            1,
            "the carried board is stored once, got {rows:?}"
        );
        assert!(
            rows.contains(&3) && rows.contains(&4),
            "each unidentified Tab is stored on its own, got {rows:?}"
        );
        // The hold is still set here unless the first store cleared it. A
        // later identified Tab would clear it by refreshing the baseline,
        // which would hide the leftover flag.
        night.advance(Duration::from_secs(8 * 60));
        night
            .word(detect::MatchOutcome::Victory, Some("Ilios"))
            .await;
        assert_eq!(night.id(), opened, "the accolade stays on this session");
        assert_eq!(night.game().map.as_deref(), Some("Ilios"));
        assert_eq!(night.game().map_source, Some(boundary::MapSource::Accolade));
        let snaps = night.store.get_session_snapshots(&opened).await.unwrap();
        assert!(
            !snaps.is_empty() && snaps.iter().all(|row| row.map_name == "Ilios"),
            "the accolade rewrites the map after the hold is cleared, got {:?}",
            snaps
                .iter()
                .map(|row| row.map_name.clone())
                .collect::<Vec<_>>()
        );
        night.advance(Duration::from_secs(10));
        night
            .tab_text(
                night_counters(5, 2, 1, 600, 180, 280),
                "Wrecking Ball",
                Some(0),
                "Ilios",
                NIGHT_CLEAN,
            )
            .await;
        let rows = night.elims(&opened).await;
        assert_eq!(
            rows.iter().filter(|elims| **elims == 1).count(),
            1,
            "the identified Tab does not store the carried board again, got {rows:?}"
        );
        assert!(
            rows.contains(&5),
            "the identified Tab is stored on its own, got {rows:?}"
        );
    }

    #[tokio::test]
    async fn one_defeat_then_post_match_tab_then_vote_leaves_a_as_defeat() {
        let mut night = Night::new().await;
        night.begin_on("Busan", "Zenyatta");
        night
            .tab_once(
                night_counters(8, 3, 2, 1800, 4000, 100),
                "Zenyatta",
                Some(2),
                Some("Busan"),
                NIGHT_CLEAN,
            )
            .await;
        night.advance(Duration::from_secs(30));
        night.word_once(detect::MatchOutcome::Defeat).await;
        night.advance(Duration::from_secs(20));
        night
            .tab_once(
                night_counters(10, 4, 2, 2200, 4600, 140),
                "Zenyatta",
                Some(2),
                Some("Busan"),
                NIGHT_CLEAN,
            )
            .await;
        assert_eq!(
            night.game().result_mark.map(|mark| mark.outcome),
            Some(detect::MatchOutcome::Defeat),
            "the first progressed board keeps the hint"
        );
        let a = night.id();
        night.advance(Duration::from_secs(130));
        night.screen(vote(&["Junkertown"])).await;
        assert_ne!(night.id(), a);
        let snaps = night.store.get_session_snapshots(&a).await.unwrap();
        assert!(
            snaps.iter().all(|row| row.outcome == "defeat"),
            "the vote seals the hint the post-match Tab kept"
        );
    }

    #[tokio::test]
    async fn a_vote_session_keeps_its_first_tab_and_its_candidates() {
        let mut night = Night::new().await;
        night.begin_on("Busan", "Zenyatta");
        night
            .tab_once(
                night_counters(14, 22, 6, 2400, 9800, 400),
                "Zenyatta",
                Some(2),
                Some("Busan"),
                NIGHT_CLEAN,
            )
            .await;
        let a = night.id();
        night.advance(Duration::from_secs(130));
        night.screen(vote(&["Junkertown", "Ilios"])).await;
        let b = night.id();
        assert_ne!(b, a);
        assert!(night.game().awaiting_first_board);
        night.advance(Duration::from_secs(130));
        night
            .tab_once(
                night_counters(2, 1, 0, 350, 60, 800),
                "Wrecking Ball",
                Some(0),
                Some("Junkertown"),
                NIGHT_CLEAN,
            )
            .await;
        assert_eq!(
            night.id(),
            b,
            "the first Tab stays on the session the vote opened"
        );
        assert_eq!(night.elims(&b).await, vec![2]);
        assert_eq!(night.elims(&a).await, vec![14]);
        assert_eq!(
            night.game().map_candidates,
            vec!["Junkertown".to_string(), "Ilios".to_string()]
        );
        let unrecorded = std::fs::read_to_string(
            night
                .dir
                .path()
                .join("debug")
                .join("unrecorded_games.jsonl"),
        )
        .unwrap_or_default();
        assert!(
            !unrecorded.contains(&b),
            "the vote session was recorded: {unrecorded}"
        );
    }

    #[tokio::test]
    async fn a_normal_night_is_one_session_per_game() {
        let mut night = Night::new().await;
        night.begin_on("Busan", "Zenyatta");
        night
            .tab_held(
                night_counters(10, 4, 3, 2000, 800, 100),
                "Zenyatta",
                Some(0),
                Some("Busan"),
            )
            .await;
        night.advance(Duration::from_secs(8 * 60));
        night
            .word(detect::MatchOutcome::Victory, Some("Busan"))
            .await;
        let busan = night.id();
        night.advance(Duration::from_secs(20));
        night.screen(vote(&["Junkertown", "Ilios"])).await;
        assert_ne!(night.id(), busan);
        assert_eq!(night.game().map_candidates, vec!["Junkertown", "Ilios"]);
        let vote_session = night.id();
        night.advance(Duration::from_secs(150));
        night.screen(boundary::StartScreen::HeroSelect).await;
        night.advance(Duration::from_secs(150));
        night.screen(boundary::StartScreen::HeroSelect).await;
        assert_eq!(
            night.id(),
            vote_session,
            "a hero swap before the first Tab, after the debounce, stays"
        );
        assert_eq!(night.game().map_candidates, vec!["Junkertown", "Ilios"]);
        night.advance(Duration::from_secs(30));
        night
            .tab_held(
                night_counters(4, 6, 2, 900, 3000, 200),
                "Ana",
                Some(1),
                Some("Junkertown"),
            )
            .await;
        let junkertown = night.id();
        assert_eq!(
            junkertown, vote_session,
            "the vote session keeps its first Tab"
        );
        assert_eq!(night.game().map_candidates, vec!["Junkertown", "Ilios"]);
        let unrecorded = std::fs::read_to_string(
            night
                .dir
                .path()
                .join("debug")
                .join("unrecorded_games.jsonl"),
        )
        .unwrap_or_default();
        assert!(
            !unrecorded.contains(&vote_session),
            "the vote session was recorded: {unrecorded}"
        );
        assert_ne!(junkertown, busan);
        night.advance(Duration::from_secs(8 * 60));
        night
            .word(detect::MatchOutcome::Defeat, Some("Junkertown"))
            .await;
        night.advance(Duration::from_secs(20));
        night.screen(boundary::StartScreen::HeroSelect).await;
        night.advance(Duration::from_secs(30));
        night
            .tab_held(
                night_counters(6, 2, 1, 1500, 200, 4000),
                "Reinhardt",
                Some(0),
                Some("Ilios"),
            )
            .await;
        let ilios = night.id();
        assert_ne!(ilios, junkertown);
        night.advance(Duration::from_secs(8 * 60));
        night
            .word(detect::MatchOutcome::Victory, Some("Ilios"))
            .await;
        assert_eq!(night.id(), ilios);
        assert_eq!(night.game().outcome, detect::MatchOutcome::Victory);
        assert_eq!(night.game().map.as_deref(), Some("Ilios"));
        let busan_rows = night.store.get_session_snapshots(&busan).await.unwrap();
        assert!(busan_rows.iter().all(|row| row.outcome == "victory"));
        let junk_rows = night
            .store
            .get_session_snapshots(&junkertown)
            .await
            .unwrap();
        assert!(junk_rows.iter().all(|row| row.outcome == "defeat"));
        assert!(night.elims(&ilios).await.contains(&6));
    }

    #[tokio::test]
    async fn busan_board_one_defeat_then_junkertown_wrecking_ball() {
        let mut night = Night::new().await;
        night.begin_on("Busan", "Zenyatta");
        night
            .tab_once(
                night_counters(14, 22, 6, 2400, 9800, 400),
                "Zenyatta",
                Some(2),
                Some("Busan"),
                NIGHT_CLEAN,
            )
            .await;
        night.advance(Duration::from_secs(8 * 60));
        night.word_once(detect::MatchOutcome::Defeat).await;
        let busan = night.id();
        night.advance(Duration::from_secs(60));
        night
            .tab_once(
                night_counters(4, 3, 2, 1500, 400, 300),
                "Wrecking Ball",
                Some(0),
                Some("Junkertown"),
                NIGHT_CLEAN,
            )
            .await;
        let junk = night.id();
        assert_ne!(junk, busan, "the nine-minute gap splits with the gap on");
        let busan_rows = night.store.get_session_snapshots(&busan).await.unwrap();
        assert!(busan_rows.iter().all(|row| row.outcome == "defeat"));
        assert_eq!(night.elims(&busan).await, vec![14]);
        night.advance(Duration::from_secs(3 * 60));
        night
            .tab_once(
                night_counters(9, 7, 4, 3800, 900, 800),
                "Wrecking Ball",
                Some(0),
                Some("Junkertown"),
                NIGHT_CLEAN,
            )
            .await;
        assert_eq!(
            night.game().reset_baseline.map(|gate| gate.accepted.elims),
            Some(9),
            "a long-age 2-4x row becomes the baseline"
        );
        night.advance(Duration::from_secs(3 * 60));
        let final_board = night_counters(23, 9, 5, 6792, 1463, 3006);
        night
            .tab_once(
                final_board,
                "Wrecking Ball",
                Some(0),
                Some("Junkertown"),
                NIGHT_CLEAN,
            )
            .await;
        assert!(
            night.elims(&junk).await.contains(&23),
            "the final board is stored"
        );
        assert_eq!(
            night.game().gate.map(|gate| gate.accepted),
            Some(final_board)
        );
        night.advance(Duration::from_secs(10 * 60));
        let over = night_counters(100, 20, 12, 14000, 3000, 6000);
        night
            .tab_once(
                over,
                "Wrecking Ball",
                Some(0),
                Some("Junkertown"),
                NIGHT_CLEAN,
            )
            .await;
        assert!(
            night.elims(&junk).await.contains(&100),
            "a long-age row past 4x is stored"
        );
        night
            .word(detect::MatchOutcome::Victory, Some("Junkertown"))
            .await;
        assert_eq!(night.game().outcome, detect::MatchOutcome::Victory);
        assert_eq!(night.game().map.as_deref(), Some("Junkertown"));
        assert_eq!(
            night.game().hero_auth.accepted_hero.as_deref(),
            Some("Wrecking Ball")
        );
    }

    #[tokio::test]
    async fn defeat_word_with_no_board_and_no_start_screen_then_junkertown() {
        let mut night = Night::new().await;
        night.begin_on("Busan", "Zenyatta");
        night.word_once(detect::MatchOutcome::Defeat).await;
        let busan = night.id();
        night.advance(Duration::from_secs(9 * 60));
        night
            .tab_once(
                night_counters(2, 1, 0, 350, 60, 800),
                "Wrecking Ball",
                Some(0),
                Some("Junkertown"),
                NIGHT_CLEAN,
            )
            .await;
        assert_ne!(
            night.id(),
            busan,
            "a hinted session with no board splits when the first Tab names another map"
        );
        let snaps = night.store.get_session_snapshots(&busan).await.unwrap();
        assert!(
            snaps.is_empty() || snaps.iter().all(|row| row.outcome == "defeat"),
            "the hint is sealed on the session that had no board"
        );
        assert_eq!(night.game().map.as_deref(), Some("Junkertown"));
    }

    #[tokio::test]
    async fn a_same_map_tab_after_a_boardless_hint_stays() {
        let mut night = Night::new().await;
        night.begin_on("Busan", "Zenyatta");
        night.word_once(detect::MatchOutcome::Defeat).await;
        let busan = night.id();
        night.advance(Duration::from_secs(9 * 60));
        night
            .tab_once(
                night_counters(8, 3, 2, 1800, 400, 100),
                "Zenyatta",
                Some(2),
                Some("Busan"),
                NIGHT_CLEAN,
            )
            .await;
        assert_eq!(
            night.id(),
            busan,
            "a late Tab of the same map is this match"
        );
        assert_eq!(night.elims(&busan).await, vec![8]);
    }

    #[tokio::test]
    async fn grace_then_idle_each_open_a_fresh_session() {
        let mut night = Night::new().await;
        night.begin_on("Busan", "Ana");
        night
            .tab_once(
                night_counters(8, 3, 2, 1800, 400, 100),
                "Ana",
                Some(0),
                Some("Busan"),
                NIGHT_CLEAN,
            )
            .await;
        night
            .word(detect::MatchOutcome::Victory, Some("Busan"))
            .await;
        let finished = night.id();
        night.advance(Duration::from_secs(80));
        night
            .tab_once(
                night_counters(2, 1, 0, 400, 80, 200),
                "Ana",
                Some(0),
                Some("Ilios"),
                NIGHT_CLEAN,
            )
            .await;
        assert_ne!(
            night.id(),
            finished,
            "past the 75s grace the next Tab is a new session"
        );
        let unfinished = night.id();
        night.advance(Duration::from_secs(21 * 60));
        night
            .tab_once(
                night_counters(4, 2, 1, 900, 100, 300),
                "Ana",
                Some(0),
                Some("Ilios"),
                NIGHT_CLEAN,
            )
            .await;
        assert_ne!(
            night.id(),
            unfinished,
            "an unfinished session idle for 20 minutes does not absorb the next Tab"
        );
    }

    #[tokio::test]
    async fn a_ban_after_a_held_misread_does_not_split() {
        let mut night = Night::new().await;
        night.begin_on("Busan", "Zenyatta");
        night
            .tab_once(
                night_counters(14, 22, 6, 2400, 9800, 400),
                "Zenyatta",
                Some(2),
                Some("Busan"),
                NIGHT_CLEAN,
            )
            .await;
        let id = night.id();
        night.advance(Duration::from_secs(60));
        night
            .tab_once(
                night_counters(2, 1, 0, 350, 60, 800),
                "Zenyatta",
                Some(2),
                Some("Busan"),
                NIGHT_CLEAN,
            )
            .await;
        assert!(night.game().deferred.is_some());
        night.screen(boundary::StartScreen::HeroBan).await;
        assert_eq!(
            night.id(),
            id,
            "a ban after one held misread does not split"
        );
        assert_eq!(night.elims(&id).await, vec![14]);
    }

    async fn arm_busan_defeat(night: &mut Night) -> String {
        night.begin_on("Busan", "Zenyatta");
        night
            .tab_once(
                night_counters(14, 22, 6, 2400, 9800, 400),
                "Zenyatta",
                Some(2),
                Some("Busan"),
                NIGHT_CLEAN,
            )
            .await;
        night.advance(Duration::from_secs(30));
        night.word_once(detect::MatchOutcome::Defeat).await;
        night.screen(boundary::StartScreen::HeroSelect).await;
        assert!(night.game().pending_boundary);
        night.id()
    }

    #[tokio::test]
    async fn an_armed_different_map_end_screen_seals_the_old_hint() {
        let mut night = Night::new().await;
        let busan = arm_busan_defeat(&mut night).await;
        night.advance(Duration::from_secs(20));
        // Two agreeing word reads, the way the poller confirms. The first
        // must not replace Busan's hint.
        night
            .word(detect::MatchOutcome::Victory, Some("Junkertown"))
            .await;
        assert_ne!(night.id(), busan);
        assert_eq!(night.game().outcome, detect::MatchOutcome::Victory);
        assert_eq!(night.game().map.as_deref(), Some("Junkertown"));
        assert_eq!(night.game().map_source, Some(boundary::MapSource::Accolade));
        let snaps = night.store.get_session_snapshots(&busan).await.unwrap();
        assert!(
            snaps.iter().all(|row| row.outcome == "defeat"),
            "B's confirmed result does not overwrite A, got {:?}",
            snaps
                .iter()
                .map(|row| row.outcome.clone())
                .collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn a_single_different_map_word_does_not_replace_the_hint_before_a_vote() {
        let mut night = Night::new().await;
        let busan = arm_busan_defeat(&mut night).await;
        night
            .word_tick(detect::MatchOutcome::Victory, Some("Junkertown"))
            .await;
        assert_eq!(night.id(), busan);
        assert_eq!(
            night.game().result_mark.map(|mark| mark.outcome),
            Some(detect::MatchOutcome::Defeat),
            "one unconfirmed word on the next map leaves A's hint"
        );
        night.advance(Duration::from_secs(150));
        night
            .screen(boundary::StartScreen::MapVote {
                candidates: vec!["Junkertown".into(), "Ilios".into()],
            })
            .await;
        assert_ne!(night.id(), busan);
        assert_eq!(night.game().outcome, detect::MatchOutcome::Unknown);
        let snaps = night.store.get_session_snapshots(&busan).await.unwrap();
        assert!(
            snaps.iter().all(|row| row.outcome == "defeat"),
            "C's vote seals A's defeat, got {:?}",
            snaps
                .iter()
                .map(|row| row.outcome.clone())
                .collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn an_armed_banner_has_no_map_and_seals_onto_the_open_session() {
        let mut night = Night::new().await;
        let busan = arm_busan_defeat(&mut night).await;
        night.banner(detect::MatchOutcome::Victory).await;
        assert_eq!(night.id(), busan);
        assert_eq!(
            night.game().outcome,
            detect::MatchOutcome::Victory,
            "a banner carries no accolade map, so it still seals onto A"
        );
    }

    #[tokio::test]
    async fn an_armed_end_title_without_a_map_does_not_replace_the_hint() {
        let mut night = Night::new().await;
        let busan = arm_busan_defeat(&mut night).await;
        night.word_tick(detect::MatchOutcome::Victory, None).await;
        assert_eq!(night.id(), busan);
        assert_eq!(
            night.game().result_mark.map(|mark| mark.outcome),
            Some(detect::MatchOutcome::Defeat),
            "an end title with no map leaves A's hint"
        );
        night.advance(Duration::from_secs(4));
        night
            .word_tick(detect::MatchOutcome::Victory, Some("Junkertown"))
            .await;
        assert_ne!(night.id(), busan, "the confirming accolade opens B");
        assert_eq!(night.game().outcome, detect::MatchOutcome::Victory);
        assert_eq!(night.game().map.as_deref(), Some("Junkertown"));
        let snaps = night.store.get_session_snapshots(&busan).await.unwrap();
        assert!(
            snaps.iter().all(|row| row.outcome == "defeat"),
            "A keeps its own result, got {:?}",
            snaps
                .iter()
                .map(|row| row.outcome.clone())
                .collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn an_accolade_map_carries_into_the_rank_screen_confirmation() {
        let mut night = Night::new().await;
        let busan = arm_busan_defeat(&mut night).await;
        night
            .word_tick(detect::MatchOutcome::Victory, Some("Junkertown"))
            .await;
        assert_eq!(night.id(), busan);
        assert_eq!(
            night.game().result_mark.map(|mark| mark.outcome),
            Some(detect::MatchOutcome::Defeat)
        );
        night.advance(Duration::from_secs(4));
        night.word_tick(detect::MatchOutcome::Victory, None).await;
        assert_ne!(
            night.id(),
            busan,
            "the rank screen confirms with the map carried from the accolade"
        );
        assert_eq!(night.game().outcome, detect::MatchOutcome::Victory);
        assert_eq!(night.game().map.as_deref(), Some("Junkertown"));
        assert_eq!(night.game().map_source, Some(boundary::MapSource::Accolade));
        let snaps = night.store.get_session_snapshots(&busan).await.unwrap();
        assert!(
            snaps.iter().all(|row| row.outcome == "defeat"),
            "A keeps its own result, got {:?}",
            snaps
                .iter()
                .map(|row| row.outcome.clone())
                .collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn an_unarmed_trusted_difference_splits_and_a_missing_name_seals() {
        let mut split = Night::new().await;
        split.begin_on("Busan", "Zenyatta");
        split
            .tab_once(
                night_counters(14, 22, 6, 2400, 9800, 400),
                "Zenyatta",
                Some(2),
                Some("Busan"),
                NIGHT_CLEAN,
            )
            .await;
        split.word_once(detect::MatchOutcome::Defeat).await;
        let busan = split.id();
        split.advance(Duration::from_secs(4));
        split
            .word(detect::MatchOutcome::Victory, Some("Junkertown"))
            .await;
        assert_ne!(split.id(), busan);
        assert_eq!(split.game().outcome, detect::MatchOutcome::Victory);
        assert_eq!(split.game().map.as_deref(), Some("Junkertown"));
        let snaps = split.store.get_session_snapshots(&busan).await.unwrap();
        assert!(
            snaps.iter().all(|row| row.outcome == "defeat"),
            "an unarmed trusted difference still leaves A's result, got {:?}",
            snaps
                .iter()
                .map(|row| row.outcome.clone())
                .collect::<Vec<_>>()
        );

        let mut seal = Night::new().await;
        seal.begin_on("Busan", "Zenyatta");
        seal.tab_once(
            night_counters(14, 22, 6, 2400, 9800, 400),
            "Zenyatta",
            Some(2),
            Some("Busan"),
            NIGHT_CLEAN,
        )
        .await;
        seal.word_once(detect::MatchOutcome::Defeat).await;
        let busan = seal.id();
        seal.word(detect::MatchOutcome::Victory, None).await;
        assert_eq!(
            seal.id(),
            busan,
            "a confirming word with no trusted name seals onto the open session"
        );
        assert_eq!(seal.game().outcome, detect::MatchOutcome::Victory);
    }

    async fn text_fallback(night: &mut Night, map: &str) -> String {
        let mut g = ActiveGame::open_at(
            format!("{:016x}", rand_id()),
            detect::MatchOutcome::Unknown,
            Vec::new(),
            night.now,
        );
        g.hero_auth.accepted_hero = Some("Zenyatta".into());
        night.st.active_game = Some(g);
        night.st.last_game_open = Some(night.now);
        night
            .tab_text(
                night_counters(14, 22, 6, 2400, 9800, 400),
                "Zenyatta",
                Some(2),
                map,
                NIGHT_CLEAN,
            )
            .await;
        assert_eq!(night.game().map.as_deref(), Some(map));
        assert_eq!(
            night.game().map_source,
            Some(boundary::MapSource::TextFallback)
        );
        night.id()
    }

    #[tokio::test]
    async fn a_text_fallback_sessions_own_accolade_replaces_the_map() {
        let mut night = Night::new().await;
        let dorado = text_fallback(&mut night, "Dorado").await;
        night
            .word_tick(detect::MatchOutcome::Victory, Some("Busan"))
            .await;
        assert_eq!(
            night.id(),
            dorado,
            "this session's own accolade does not split"
        );
        assert_eq!(night.game().map.as_deref(), Some("Busan"));
        assert_eq!(night.game().map_source, Some(boundary::MapSource::Accolade));
        let snaps = night.store.get_session_snapshots(&dorado).await.unwrap();
        assert!(
            !snaps.is_empty() && snaps.iter().all(|row| row.map_name == "Busan"),
            "the adopted accolade rewrites the snapshots, got {:?}",
            snaps
                .iter()
                .map(|row| row.map_name.clone())
                .collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn an_armed_text_fallback_is_not_relabelled_by_the_next_map() {
        let mut night = Night::new().await;
        let busan = text_fallback(&mut night, "Busan").await;
        night.word_once(detect::MatchOutcome::Defeat).await;
        night.screen(boundary::StartScreen::HeroSelect).await;
        assert!(night.game().pending_boundary);
        night
            .word(detect::MatchOutcome::Victory, Some("Junkertown"))
            .await;
        assert_eq!(night.id(), busan);
        assert_eq!(night.game().map.as_deref(), Some("Busan"));
        let snaps = night.store.get_session_snapshots(&busan).await.unwrap();
        assert!(
            snaps.iter().all(|row| row.map_name == "Busan"),
            "the next accolade must not rewrite A's snapshots, got {:?}",
            snaps
                .iter()
                .map(|row| row.map_name.clone())
                .collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn a_text_fallback_hint_is_not_relabelled_by_the_next_result() {
        let mut night = Night::new().await;
        let busan = text_fallback(&mut night, "Busan").await;
        night.word_once(detect::MatchOutcome::Defeat).await;
        night
            .word_tick(detect::MatchOutcome::Victory, Some("Junkertown"))
            .await;
        assert!(night.game().text_fallback_locked);
        night.advance(Duration::from_secs(4));
        night.no_signal_tick().await;
        assert!(
            night.game().text_fallback_locked,
            "a tick with no result word keeps the text-fallback lock"
        );
        night.advance(Duration::from_secs(4));
        night
            .word(detect::MatchOutcome::Victory, Some("Junkertown"))
            .await;
        assert_eq!(night.id(), busan);
        assert_eq!(night.game().map.as_deref(), Some("Busan"));
        let snaps = night.store.get_session_snapshots(&busan).await.unwrap();
        assert!(snaps.iter().all(|row| row.map_name == "Busan"));
    }

    #[tokio::test]
    async fn progressed_boards_clear_the_lock_so_the_accolade_can_correct_the_map() {
        let mut night = Night::new().await;
        let dorado = text_fallback(&mut night, "Dorado").await;
        night.word_once(detect::MatchOutcome::Defeat).await;
        night.word_once(detect::MatchOutcome::Victory).await;
        assert!(
            night.game().text_fallback_locked,
            "a different result replaces the hint and locks the text map"
        );
        night.advance(Duration::from_secs(70));
        night
            .tab_text(
                night_counters(16, 24, 7, 2800, 10400, 500),
                "Zenyatta",
                Some(2),
                "Dorado",
                NIGHT_CLEAN,
            )
            .await;
        assert!(
            night.game().result_mark.is_some(),
            "the first progressed board keeps the hint"
        );
        assert!(night.game().text_fallback_locked);
        night.advance(Duration::from_secs(60));
        night
            .tab_text(
                night_counters(18, 26, 8, 3200, 11000, 600),
                "Zenyatta",
                Some(2),
                "Dorado",
                NIGHT_CLEAN,
            )
            .await;
        assert!(
            night.game().result_mark.is_none(),
            "the second progressed board clears the hint"
        );
        assert!(
            !night.game().text_fallback_locked,
            "clearing the hint clears the text-fallback lock"
        );
        night.advance(Duration::from_secs(5 * 60));
        night
            .word(detect::MatchOutcome::Victory, Some("Busan"))
            .await;
        assert_eq!(night.id(), dorado);
        assert_eq!(night.game().outcome, detect::MatchOutcome::Victory);
        assert_eq!(night.game().map.as_deref(), Some("Busan"));
        assert_eq!(night.game().map_source, Some(boundary::MapSource::Accolade));
        let snaps = night.store.get_session_snapshots(&dorado).await.unwrap();
        assert_eq!(
            snaps.len(),
            3,
            "the opening board and both progressed boards"
        );
        assert!(
            snaps.iter().all(|row| row.map_name == "Busan"),
            "the corrected accolade rewrites every snapshot, got {:?}",
            snaps
                .iter()
                .map(|row| row.map_name.clone())
                .collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn a_finished_text_fallback_session_keeps_its_map() {
        let mut night = Night::new().await;
        let busan = text_fallback(&mut night, "Busan").await;
        night.banner(detect::MatchOutcome::Victory).await;
        assert_eq!(night.game().outcome, detect::MatchOutcome::Victory);
        assert_eq!(night.game().map.as_deref(), Some("Busan"));
        night
            .word(detect::MatchOutcome::Defeat, Some("Junkertown"))
            .await;
        assert_eq!(night.id(), busan);
        assert_eq!(night.game().outcome, detect::MatchOutcome::Victory);
        assert_eq!(night.game().map.as_deref(), Some("Busan"));
        let snaps = night.store.get_session_snapshots(&busan).await.unwrap();
        assert!(snaps.iter().all(|row| row.map_name == "Busan"));
    }

    #[tokio::test]
    async fn a_word_with_no_open_game_still_notes_the_streak() {
        let mut night = Night::new().await;
        night
            .word_tick(detect::MatchOutcome::Victory, Some("Busan"))
            .await;
        assert!(night.st.active_game.is_none());
        assert_eq!(
            night
                .st
                .word_outcome_streak
                .as_ref()
                .map(|streak| streak.outcome),
            Some(detect::MatchOutcome::Victory)
        );
        night.advance(Duration::from_secs(4));
        night.word_tick(detect::MatchOutcome::Victory, None).await;
        assert_eq!(
            night.st.pending_outcome.map(|(outcome, _)| outcome),
            Some(detect::MatchOutcome::Victory)
        );
        assert_eq!(
            night
                .st
                .word_outcome_streak
                .as_ref()
                .and_then(|streak| streak.map.as_deref()),
            Some("Busan")
        );
    }

    #[tokio::test]
    async fn an_accolade_adopts_the_map_source() {
        let mut night = Night::new().await;
        let g = ActiveGame::open_at(
            format!("{:016x}", rand_id()),
            detect::MatchOutcome::Unknown,
            Vec::new(),
            night.now,
        );
        night.st.active_game = Some(g);
        night.st.last_game_open = Some(night.now);
        night
            .word_tick(detect::MatchOutcome::Victory, Some("Busan"))
            .await;
        assert_eq!(night.game().map.as_deref(), Some("Busan"));
        assert_eq!(night.game().map_source, Some(boundary::MapSource::Accolade));
    }

    #[tokio::test]
    async fn a_stat_split_session_keeps_the_top_bar_map_source() {
        let mut night = Night::new().await;
        night.begin_on("Busan", "Zenyatta");
        night
            .tab_once(
                night_counters(14, 22, 6, 2400, 9800, 400),
                "Zenyatta",
                Some(2),
                Some("Busan"),
                NIGHT_CLEAN,
            )
            .await;
        night.word_once(detect::MatchOutcome::Defeat).await;
        let busan = night.id();
        night.advance(Duration::from_secs(150));
        night
            .tab_once(
                night_counters(16, 23, 7, 2800, 10000, 500),
                "Zenyatta",
                Some(2),
                Some("Junkertown"),
                NIGHT_CLEAN,
            )
            .await;
        assert_ne!(night.id(), busan);
        assert_eq!(night.game().map.as_deref(), Some("Junkertown"));
        assert_eq!(night.game().map_source, Some(boundary::MapSource::TopBar));
    }

    #[tokio::test]
    async fn a_later_top_bar_upgrades_a_text_fallback_and_the_next_map_splits() {
        let mut night = Night::new().await;
        let mut g = ActiveGame::open_at(
            format!("{:016x}", rand_id()),
            detect::MatchOutcome::Unknown,
            Vec::new(),
            night.now,
        );
        g.hero_auth.accepted_hero = Some("Zenyatta".into());
        night.st.active_game = Some(g);
        night.st.last_game_open = Some(night.now);
        night
            .tab_text(
                night_counters(14, 22, 6, 2400, 9800, 400),
                "Zenyatta",
                Some(2),
                "Busan",
                NIGHT_CLEAN,
            )
            .await;
        assert_eq!(
            night.game().map_source,
            Some(boundary::MapSource::TextFallback)
        );
        night.advance(Duration::from_secs(10));
        night
            .tab_once(
                night_counters(16, 23, 7, 2600, 10000, 450),
                "Zenyatta",
                Some(2),
                Some("Busan"),
                NIGHT_CLEAN,
            )
            .await;
        assert_eq!(night.game().map.as_deref(), Some("Busan"));
        assert_eq!(night.game().map_source, Some(boundary::MapSource::TopBar));
        night.word_once(detect::MatchOutcome::Defeat).await;
        let busan = night.id();
        night.advance(Duration::from_secs(150));
        night
            .tab_once(
                night_counters(18, 24, 8, 3000, 11000, 520),
                "Zenyatta",
                Some(2),
                Some("Junkertown"),
                NIGHT_CLEAN,
            )
            .await;
        assert_ne!(
            night.id(),
            busan,
            "once the top bar agrees, a later different map still splits"
        );
    }

    #[tokio::test]
    async fn a_different_map_inside_the_gap_stays_one_game() {
        let mut night = Night::new().await;
        night.begin_on("Busan", "Zenyatta");
        night
            .tab_once(
                night_counters(14, 22, 6, 2400, 9800, 400),
                "Zenyatta",
                Some(2),
                Some("Busan"),
                NIGHT_CLEAN,
            )
            .await;
        night.word_once(detect::MatchOutcome::Defeat).await;
        let id = night.id();
        night.advance(Duration::from_secs(60));
        night
            .tab_once(
                night_counters(16, 23, 7, 2800, 10000, 500),
                "Zenyatta",
                Some(2),
                Some("Junkertown"),
                NIGHT_CLEAN,
            )
            .await;
        assert_eq!(night.id(), id, "inside 120s a different top bar stays");
    }

    #[tokio::test]
    async fn a_same_map_tab_after_the_gap_stays_one_game() {
        let mut night = Night::new().await;
        night.begin_on("Busan", "Zenyatta");
        night
            .tab_once(
                night_counters(14, 22, 6, 2400, 9800, 400),
                "Zenyatta",
                Some(2),
                Some("Busan"),
                NIGHT_CLEAN,
            )
            .await;
        night.word_once(detect::MatchOutcome::Defeat).await;
        let id = night.id();
        night.advance(Duration::from_secs(150));
        night
            .tab_once(
                night_counters(16, 23, 7, 2800, 10000, 500),
                "Zenyatta",
                Some(2),
                Some("Busan"),
                NIGHT_CLEAN,
            )
            .await;
        assert_eq!(night.id(), id);
    }

    /// F251-1: a text-fallback session is renamed by a later sure banner
    /// read, rows included, and keeps the new trusted map.
    /// F252-3: a member's map pick on a text-fallback session is never
    /// renamed by a banner read, and its stored mode is left alone.
    #[tokio::test]
    async fn a_banner_read_never_renames_a_member_map_pick() {
        let mut night = Night::new().await;
        let id = text_fallback(&mut night, "Busan").await;
        night
            .store
            .apply_command(&storage::StoreCommand::EditMatch {
                session_id: id.clone(),
                edit: storage::MatchEdit {
                    map_name: Some("King's Row".into()),
                    ..Default::default()
                },
            })
            .await
            .unwrap();
        let before = night.store.get_session_snapshots(&id).await.unwrap();
        assert!(
            before
                .iter()
                .all(|r| r.corrected_map_name.as_deref() == Some("King's Row"))
        );
        night.advance(Duration::from_secs(10));
        night
            .tab_banner(
                night_counters(16, 23, 7, 2600, 10000, 450),
                "Zenyatta",
                Some(2),
                "Ilios",
            )
            .await;
        assert_eq!(night.game().map.as_deref(), Some("Busan"));
        let after = night.store.get_session_snapshots(&id).await.unwrap();
        for row in &after[..before.len()] {
            assert_eq!(row.corrected_map_name.as_deref(), Some("King's Row"));
            assert_ne!(row.map_name, "Ilios");
        }
        for (b, a) in before.iter().zip(&after) {
            assert_eq!(
                a.game_mode, b.game_mode,
                "the member's mode is not rewritten"
            );
        }
    }

    /// F252-2: a sure banner read after a gap longer than the splitter's
    /// STAT_SPLIT_MIN_GAP does not rename the text-fallback session: game 1's
    /// end may have been missed, and this Tab may be game 2.
    #[tokio::test]
    async fn a_banner_read_after_a_long_gap_does_not_rename() {
        let mut night = Night::new().await;
        let id = text_fallback(&mut night, "Busan").await;
        night.advance(STAT_SPLIT_MIN_GAP + Duration::from_secs(30));
        night
            .tab_banner(
                night_counters(16, 23, 7, 2600, 10000, 450),
                "Zenyatta",
                Some(2),
                "Ilios",
            )
            .await;
        let snaps = night.store.get_session_snapshots(&id).await.unwrap();
        assert!(
            snaps.iter().all(|row| row.map_name == "Busan"),
            "{:?}",
            snaps.iter().map(|r| r.map_name.clone()).collect::<Vec<_>>()
        );
        if night.id() == id {
            assert_eq!(night.game().map.as_deref(), Some("Busan"));
        }
    }

    /// F252-1: after a rename, the game's own accolade that still names the
    /// replaced text-fallback map is this game's end screen: the outcome
    /// lands on the renamed session, not on a new empty one.
    #[tokio::test]
    async fn the_accolade_of_a_renamed_game_seals_it() {
        let mut night = Night::new().await;
        let id = text_fallback(&mut night, "Busan").await;
        night.advance(Duration::from_secs(10));
        night
            .tab_banner(
                night_counters(16, 23, 7, 2600, 10000, 450),
                "Zenyatta",
                Some(2),
                "Ilios",
            )
            .await;
        assert_eq!(night.game().map.as_deref(), Some("Ilios"));
        night
            .word(detect::MatchOutcome::Victory, Some("Busan"))
            .await;
        assert_eq!(night.id(), id, "no split on the old name");
        assert_eq!(night.game().map.as_deref(), Some("Ilios"));
        assert_eq!(night.game().outcome, detect::MatchOutcome::Victory);
        let snaps = night.store.get_session_snapshots(&id).await.unwrap();
        assert!(snaps.iter().all(|row| row.map_name == "Ilios"));
    }

    #[tokio::test]
    async fn a_sure_banner_read_replaces_a_text_fallback_session_map() {
        let mut night = Night::new().await;
        let id = text_fallback(&mut night, "Busan").await;
        night.advance(Duration::from_secs(10));
        night
            .tab_banner(
                night_counters(16, 23, 7, 2600, 10000, 450),
                "Zenyatta",
                Some(2),
                "Ilios",
            )
            .await;
        assert_eq!(night.id(), id, "same game, no split");
        assert_eq!(night.game().map.as_deref(), Some("Ilios"));
        assert_eq!(night.game().map_source, Some(boundary::MapSource::TopBar));
        let snaps = night.store.get_session_snapshots(&id).await.unwrap();
        assert!(!snaps.is_empty());
        assert!(
            snaps
                .iter()
                .all(|row| row.map_name == "Ilios" && row.game_mode == "Control"),
            "{:?}",
            snaps.iter().map(|r| r.map_name.clone()).collect::<Vec<_>>()
        );
        // first trusted map wins from here on
        night.advance(Duration::from_secs(10));
        night
            .tab_banner(
                night_counters(18, 24, 8, 2800, 10400, 500),
                "Zenyatta",
                Some(2),
                "Nepal",
            )
            .await;
        assert_eq!(night.game().map.as_deref(), Some("Ilios"));
    }

    /// The other direction: a trusted session map is never replaced, the
    /// top-bar OCR never replaces a text fallback (ocr-v1 unchanged), and a
    /// banner read after a result does not rename the finished game.
    #[tokio::test]
    async fn a_banner_read_keeps_a_trusted_or_closed_session_map() {
        let mut night = Night::new().await;
        night.begin_on("Busan", "Zenyatta");
        night
            .tab_banner(
                night_counters(14, 22, 6, 2400, 9800, 400),
                "Zenyatta",
                Some(2),
                "Ilios",
            )
            .await;
        assert_eq!(
            night.game().map.as_deref(),
            Some("Busan"),
            "trusted top bar"
        );
        assert_eq!(night.game().map_source, Some(boundary::MapSource::TopBar));
        let snaps = night
            .store
            .get_session_snapshots(&night.id())
            .await
            .unwrap();
        assert!(!snaps.is_empty());
        assert!(
            snaps.iter().all(|row| row.map_name == "Busan"),
            "the rows keep the trusted map"
        );

        let mut night = Night::new().await;
        let id = text_fallback(&mut night, "Busan").await;
        night.advance(Duration::from_secs(10));
        night
            .tab_once(
                night_counters(16, 23, 7, 2600, 10000, 450),
                "Zenyatta",
                Some(2),
                Some("Ilios"),
                NIGHT_CLEAN,
            )
            .await;
        assert_eq!(night.game().map.as_deref(), Some("Busan"), "top-bar OCR");
        let snaps = night.store.get_session_snapshots(&id).await.unwrap();
        assert!(snaps.iter().all(|row| row.map_name == "Busan"));

        let mut night = Night::new().await;
        let id = text_fallback(&mut night, "Busan").await;
        night.word_once(detect::MatchOutcome::Defeat).await;
        night.advance(Duration::from_secs(10));
        night
            .tab_banner(
                night_counters(16, 23, 7, 2600, 10000, 450),
                "Zenyatta",
                Some(2),
                "Ilios",
            )
            .await;
        assert_eq!(night.id(), id);
        assert_eq!(night.game().map.as_deref(), Some("Busan"), "after a result");
        let snaps = night.store.get_session_snapshots(&id).await.unwrap();
        assert!(snaps.iter().all(|row| row.map_name == "Busan"));
    }

    #[tokio::test]
    async fn a_text_fallback_map_does_not_split_on_a_later_top_bar() {
        let mut night = Night::new().await;
        let mut g = ActiveGame::open_at(
            format!("{:016x}", rand_id()),
            detect::MatchOutcome::Unknown,
            Vec::new(),
            night.now,
        );
        g.hero_auth.accepted_hero = Some("Zenyatta".into());
        night.st.active_game = Some(g);
        night.st.last_game_open = Some(night.now);
        night
            .tab_text(
                night_counters(14, 22, 6, 2400, 9800, 400),
                "Zenyatta",
                Some(2),
                "Dorado",
                NIGHT_CLEAN,
            )
            .await;
        assert_eq!(night.game().map.as_deref(), Some("Dorado"));
        assert_eq!(
            night.game().map_source,
            Some(boundary::MapSource::TextFallback)
        );
        night.word_once(detect::MatchOutcome::Defeat).await;
        let id = night.id();
        night.advance(Duration::from_secs(150));
        night
            .tab_once(
                night_counters(16, 23, 7, 2800, 10000, 500),
                "Zenyatta",
                Some(2),
                Some("Busan"),
                NIGHT_CLEAN,
            )
            .await;
        assert_eq!(
            night.id(),
            id,
            "a text-fallback map does not make the post-match Tab a new game"
        );
    }

    #[tokio::test]
    async fn an_unidentified_row_leaves_the_held_board_for_the_split() {
        let mut night = Night::new().await;
        night.begin_on("Busan", "Zenyatta");
        night
            .tab_once(
                night_counters(14, 22, 6, 2400, 9800, 400),
                "Zenyatta",
                Some(2),
                Some("Busan"),
                NIGHT_CLEAN,
            )
            .await;
        night.advance(Duration::from_secs(60));
        night
            .tab_once(
                night_counters(2, 1, 0, 200, 40, 80),
                "Zenyatta",
                Some(2),
                Some("Busan"),
                NIGHT_CLEAN,
            )
            .await;
        assert_eq!(night.game().deferred.map(|c| c.elims), Some(2));
        assert!(night.game().reset_streak >= 1);
        night.advance(Duration::from_secs(10));
        night
            .tab_once(
                night_counters(9, 4, 3, 1500, 400, 200),
                "Zenyatta",
                None,
                Some("Busan"),
                NIGHT_CLEAN,
            )
            .await;
        assert_eq!(
            night.game().deferred.map(|c| c.elims),
            Some(2),
            "an unidentified row leaves the held board"
        );
        assert!(night.game().reset_streak >= 1);
        let finished = night.id();
        night.advance(Duration::from_secs(60));
        night
            .tab_once(
                night_counters(1, 0, 0, 80, 10, 20),
                "Zenyatta",
                Some(2),
                Some("Busan"),
                NIGHT_CLEAN,
            )
            .await;
        assert_ne!(night.id(), finished);
        let elims = night.elims(&night.id()).await;
        assert!(
            elims.contains(&2),
            "the split writes the held board onto the new session, got {elims:?}"
        );
    }

    #[tokio::test]
    async fn a_held_board_is_written_on_a_tab_opened_session() {
        let mut night = Night::new().await;
        night.begin_on("Busan", "Zenyatta");
        night
            .tab_once(
                night_counters(14, 22, 6, 2400, 9800, 400),
                "Zenyatta",
                Some(2),
                Some("Busan"),
                NIGHT_CLEAN,
            )
            .await;
        night
            .word(detect::MatchOutcome::Victory, Some("Busan"))
            .await;
        night.advance(Duration::from_secs(60));
        night
            .tab_once(
                night_counters(2, 1, 0, 200, 40, 80),
                "Zenyatta",
                Some(2),
                Some("Busan"),
                NIGHT_CLEAN,
            )
            .await;
        assert!(night.game().deferred.is_some());
        let finished = night.id();
        night.advance(Duration::from_secs(80));
        night
            .tab_once(
                night_counters(4, 2, 1, 900, 100, 200),
                "Ana",
                Some(0),
                Some("Ilios"),
                NIGHT_CLEAN,
            )
            .await;
        assert_ne!(night.id(), finished);
        let elims = night.elims(&night.id()).await;
        assert!(
            elims.contains(&2),
            "grace opens a session and writes the held board, got {elims:?}"
        );
        assert!(!night.elims(&finished).await.contains(&2));
    }

    #[tokio::test]
    async fn a_rejected_frame_adopts_the_outcome_and_clears_the_arm() {
        let mut night = Night::new().await;
        let busan = arm_busan_defeat(&mut night).await;
        let report = CaptureReport {
            best_guess: None,
            recorded: false,
            outcome: detect::MatchOutcome::Victory,
            map: None,
            map_source: None,
            session_id: busan.clone(),
            split: false,
            armed_reset: false,
            ignore_row: true,
            reset_streak: night.game().reset_streak,
            reset_baseline: night.game().reset_baseline,
            baseline_row: night.game().baseline_row,
            refresh_baseline: false,
            clear_hint: false,
            count_progress: false,
            career_panel: false,
            held_counters: None,
            held_hero: None,
            gate_state: None,
            hero_auth: night.game().hero_auth.clone(),
            seal: None,
            close_reason: None,
            held_at: None,
            map_replaces_text_fallback: false,
        };
        apply_capture_report(
            &mut night.st,
            &night.store,
            night.dir.path(),
            &busan,
            Ok(report),
            night.now,
        )
        .await;
        assert_eq!(night.game().outcome, detect::MatchOutcome::Victory);
        assert!(
            !night.game().pending_boundary,
            "adopting the rejected frame's outcome clears the armed boundary"
        );
    }

    /// Move the injected clock to an absolute UTC stamp, measured from `origin`.
    fn jump_to(
        night: &mut Night,
        origin: Instant,
        start: chrono::DateTime<Utc>,
        stamp: &str,
        day: &str,
    ) {
        let when = chrono::DateTime::parse_from_rfc3339(&format!("{day}T{stamp}"))
            .unwrap()
            .with_timezone(&Utc);
        let target = origin + (when - start).to_std().unwrap();
        if target > night.now {
            night.advance(target.saturating_duration_since(night.now));
        }
    }

    #[tokio::test]
    #[allow(clippy::type_complexity)]
    async fn busan_defeat_and_junkertown_victory_stay_two_games() {
        // Boards from a 2026-10-05 night. The file labeled the later boards
        // with the earlier map because they had been merged; the later map
        // is Junkertown. No person's name belongs in this test.
        let njc: &[(u32, u32, u32, u32, u32, u32, &str)] = &[
            (6, 3, 2, 1703, 412, 954, "20:22:13Z"),
            (6, 4, 2, 2121, 459, 1161, "20:23:02Z"),
            (6, 4, 2, 2121, 459, 1161, "20:23:08Z"),
            (6, 4, 2, 2121, 459, 1161, "20:23:13Z"),
            (10, 4, 2, 3352, 509, 1629, "20:23:59Z"),
            (10, 4, 2, 4566, 759, 2221, "20:24:58Z"),
            (10, 4, 2, 5499, 1131, 2821, "20:26:00Z"),
            (1, 5, 2, 5554, 1131, 3146, "20:26:04Z"),
            (1, 5, 2, 5554, 1131, 3146, "20:26:08Z"),
            (1, 5, 2, 5554, 1181, 3146, "20:26:37Z"),
            (1, 5, 2, 5554, 1181, 3146, "20:26:43Z"),
            (13, 7, 2, 6657, 1432, 4188, "20:28:34Z"),
            (14, 8, 2, 6877, 1582, 4434, "20:30:03Z"),
            (18, 9, 4, 8224, 1765, 4726, "20:31:04Z"),
            (19, 9, 4, 8243, 1765, 4726, "20:31:11Z"),
            (19, 9, 4, 8243, 1765, 4726, "20:31:16Z"),
        ];
        let busan: &[(u32, u32, u32, u32, u32, u32, &str, &str)] = &[
            (3, 3, 0, 2905, 691, 0, "20:40:39Z", "Soldier: 76"),
            (4, 3, 0, 2905, 691, 1, "20:42:20Z", "Zenyatta"),
            (4, 4, 2, 3695, 691, 294, "20:43:14Z", "Zenyatta"),
            (4, 4, 5, 4409, 0, 294, "20:43:58Z", "Zenyatta"),
            (6, 4, 5, 4409, 0, 294, "20:44:43Z", "Zenyatta"),
            (6, 4, 6, 4409, 809, 9, "20:45:03Z", "Zenyatta"),
            (8, 4649, 7, 4409, 809, 9, "20:45:33Z", "Zenyatta"),
            (8, 4649, 7, 5, 809, 9, "20:45:44Z", "Zenyatta"),
        ];
        let junk: &[(u32, u32, u32, u32, u32, u32, &str)] = &[
            (10, 4649, 7, 2879, 921, 1679, "21:01:48Z"),
            (14, 4649, 7, 4572, 1044, 2202, "21:03:09Z"),
            (14, 3, 5, 4572, 1044, 2202, "21:03:14Z"),
            (14, 3, 5, 4572, 1044, 2202, "21:03:27Z"),
            (16, 3, 6, 5145, 1099, 2319, "21:05:08Z"),
            (16, 3, 7, 5229, 1149, 2319, "21:05:27Z"),
            (16, 4, 7, 5691, 1349, 2564, "21:06:06Z"),
            (7, 4, 7, 5691, 1349, 2564, "21:06:12Z"),
            (7, 4, 7, 5691, 1349, 2564, "21:06:17Z"),
            (7, 4, 7, 5691, 1349, 2564, "21:06:20Z"),
            (21, 5, 9, 6121, 1413, 2714, "21:07:23Z"),
            (23, 5, 9, 6792, 1463, 3006, "21:08:06Z"),
            (23, 5, 9, 6792, 1463, 3006, "21:08:38Z"),
        ];
        let mut night = Night::new().await;
        night.begin_on("New Junk City", "Wrecking Ball");
        let origin = night.now;
        let start = chrono::DateTime::parse_from_rfc3339("2026-10-05T20:22:13Z")
            .unwrap()
            .with_timezone(&Utc);
        let day = "2026-10-05";
        for (e, d, a, dmg, hlg, mit, stamp) in njc {
            jump_to(&mut night, origin, start, stamp, day);
            night
                .tab_once(
                    night_counters(*e, *a, *d, *dmg, *hlg, *mit),
                    "Wrecking Ball",
                    Some(0),
                    Some("New Junk City"),
                    NIGHT_CLEAN,
                )
                .await;
        }
        let njc_id = night.id();
        jump_to(&mut night, origin, start, "20:34:07Z", day);
        night
            .word(detect::MatchOutcome::Victory, Some("New Junk City"))
            .await;
        assert_eq!(night.id(), njc_id);
        assert_eq!(night.game().outcome, detect::MatchOutcome::Victory);
        for (e, d, a, dmg, hlg, mit, stamp, hero) in busan {
            jump_to(&mut night, origin, start, stamp, day);
            night
                .tab_once(
                    night_counters(*e, *a, *d, *dmg, *hlg, *mit),
                    hero,
                    Some(0),
                    Some("Busan"),
                    NIGHT_CLEAN,
                )
                .await;
        }
        let busan_id = night.id();
        assert_ne!(
            busan_id, njc_id,
            "grace closes the finished New Junk City game"
        );
        jump_to(&mut night, origin, start, "20:46:15Z", day);
        night.word_once(detect::MatchOutcome::Defeat).await;
        assert_eq!(night.id(), busan_id);
        for (e, d, a, dmg, hlg, mit, stamp) in junk {
            jump_to(&mut night, origin, start, stamp, day);
            night
                .tab_once(
                    night_counters(*e, *a, *d, *dmg, *hlg, *mit),
                    "Wrecking Ball",
                    Some(0),
                    Some("Junkertown"),
                    NIGHT_CLEAN,
                )
                .await;
        }
        let junk_id = night.id();
        assert_ne!(
            junk_id, busan_id,
            "the gap seals Busan and opens Junkertown"
        );
        jump_to(&mut night, origin, start, "21:08:47Z", day);
        night
            .word(detect::MatchOutcome::Victory, Some("Junkertown"))
            .await;
        assert_eq!(night.id(), junk_id);
        assert_eq!(night.game().outcome, detect::MatchOutcome::Victory);
        let busan_rows = night.store.get_session_snapshots(&busan_id).await.unwrap();
        assert!(
            busan_rows.iter().all(|row| row.outcome == "defeat"),
            "Busan closes as Defeat"
        );
        assert!(night.elims(&junk_id).await.contains(&23));
        assert!(!night.elims(&busan_id).await.contains(&23));
        assert!(night.elims(&njc_id).await.contains(&19));
        assert!(
            night
                .store
                .get_session_snapshots(&njc_id)
                .await
                .unwrap()
                .iter()
                .all(|row| row.outcome == "victory")
        );
    }

    #[tokio::test]
    async fn neon_junction_side_swap_stays_one_session() {
        // One Hybrid match. A hero-select screen at the side swap used to
        // split it. Elims misreads (9→1, 15→7) are in the row list.
        let rows: &[(u32, u32, u32, u32, u32, u32, &str)] = &[
            (9, 3, 4, 4470, 716, 1880, "21:55:03Z"),
            (9, 4, 4, 5736, 871, 2374, "21:56:54Z"),
            (9, 4, 4, 5736, 871, 2374, "21:56:59Z"),
            (1, 4, 4, 5736, 871, 2374, "21:57:02Z"),
            (1, 4, 4, 5736, 871, 2374, "21:57:07Z"),
            (1, 4, 4, 5736, 871, 2374, "21:57:12Z"),
            (1, 4, 4, 5736, 871, 2374, "21:57:32Z"),
            (1, 4, 4, 5736, 871, 2374, "21:57:55Z"),
            (1, 4, 4, 5736, 871, 2374, "21:58:12Z"),
            (1, 4, 4, 5736, 871, 2374, "21:58:23Z"),
            (1, 4, 4, 5736, 871, 2374, "21:58:31Z"),
            (12, 5, 4, 6660, 1171, 2924, "21:59:39Z"),
            (12, 5, 4, 6660, 1171, 2924, "21:59:44Z"),
            (12, 5, 4, 6660, 1171, 2924, "21:59:51Z"),
            (15, 5, 4, 7646, 1271, 3318, "22:00:48Z"),
            (15, 6, 4, 7859, 1321, 3718, "22:01:27Z"),
            (15, 7, 4, 8793, 1371, 4118, "22:02:05Z"),
            (15, 7, 4, 8793, 1371, 4118, "22:02:11Z"),
            (15, 7, 4, 10131, 1584, 4639, "22:03:05Z"),
            (15, 8, 4, 10165, 1634, 4789, "22:03:15Z"),
            (7, 8, 4, 10165, 1634, 4789, "22:03:21Z"),
            (7, 8, 4, 10165, 1634, 4789, "22:03:28Z"),
            (7, 8, 4, 10310, 1667, 4862, "22:03:44Z"),
            (7, 8, 4, 10654, 1667, 4962, "22:04:04Z"),
            (7, 8, 4, 10965, 1667, 5099, "22:04:27Z"),
            (7, 8, 4, 10965, 1667, 5099, "22:04:39Z"),
            (20, 8, 6, 11867, 1809, 5756, "22:05:17Z"),
            (23, 9, 8, 12469, 1923, 5821, "22:05:59Z"),
            (24, 9, 8, 12915, 2046, 6833, "22:06:52Z"),
            (24, 9, 8, 12915, 2046, 6833, "22:06:59Z"),
            (24, 9, 8, 12915, 2046, 6833, "22:07:04Z"),
            (24, 9, 8, 12921, 2096, 6833, "22:07:10Z"),
            (28, 9, 9, 14579, 2246, 7458, "22:07:48Z"),
            (32, 10, 10, 15143, 2446, 7940, "22:08:31Z"),
            (32, 10, 10, 15143, 2446, 7940, "22:08:44Z"),
            (32, 10, 10, 15143, 2446, 7940, "22:08:48Z"),
        ];
        let mut night = Night::new().await;
        night.begin_on("Neon Junction", "Wrecking Ball");
        let origin = night.now;
        let start = chrono::DateTime::parse_from_rfc3339("2026-09-27T21:55:03Z")
            .unwrap()
            .with_timezone(&Utc);
        let id = night.id();
        for (e, d, a, dmg, hlg, mit, stamp) in rows {
            if *stamp == "22:05:59Z" {
                jump_to(&mut night, origin, start, "22:05:38Z", "2026-09-27");
                night.screen(boundary::StartScreen::HeroSelect).await;
            }
            jump_to(&mut night, origin, start, stamp, "2026-09-27");
            night
                .tab_once(
                    night_counters(*e, *a, *d, *dmg, *hlg, *mit),
                    "Wrecking Ball",
                    Some(0),
                    Some("Neon Junction"),
                    NIGHT_CLEAN,
                )
                .await;
            assert_eq!(night.id(), id, "split at {stamp}");
        }
        assert!(night.elims(&id).await.contains(&32));
    }

    #[tokio::test]
    async fn a_stomp_with_a_short_tab_gap_and_a_hero_select_stays_one_session() {
        let mut night = Night::new().await;
        night.begin_on("Colosseo", "Reinhardt");
        let id = night.id();
        let rows = [
            (0, 0, 0, 120, 40, 80),
            (0, 1, 1, 480, 60, 200),
            (1, 2, 1, 900, 80, 400),
            (1, 3, 2, 1400, 90, 700),
            (2, 4, 2, 2100, 110, 1100),
        ];
        for (i, (e, d, a, dmg, hlg, mit)) in rows.into_iter().enumerate() {
            if i == 3 {
                night.screen(boundary::StartScreen::HeroSelect).await;
            }
            night.advance(Duration::from_secs(40));
            night
                .tab_once(
                    night_counters(e, a, d, dmg, hlg, mit),
                    "Reinhardt",
                    Some(0),
                    Some("Colosseo"),
                    NIGHT_CLEAN,
                )
                .await;
            assert_eq!(night.id(), id);
        }
        assert_eq!(night.elims(&id).await, vec![0, 0, 1, 1, 2]);
    }

    fn close_after() -> Duration {
        finished_game_close_after(config::FINISHED_GAME_CLOSE_DEFAULT_SECS)
    }

    async fn finish_colosseo(night: &mut Night) -> String {
        night.begin_on("Colosseo", "Zenyatta");
        night
            .tab_once(
                night_counters(14, 22, 6, 2400, 9800, 400),
                "Zenyatta",
                Some(2),
                Some("Colosseo"),
                NIGHT_CLEAN,
            )
            .await;
        night
            .word(detect::MatchOutcome::Defeat, Some("Colosseo"))
            .await;
        assert_eq!(night.game().outcome, detect::MatchOutcome::Defeat);
        night.id()
    }

    #[tokio::test]
    async fn last_game_of_the_night_retires_at_three_minutes_and_syncs_once() {
        let mut night = Night::new().await;
        let id = finish_colosseo(&mut night).await;
        let activity = night.game().last_activity;
        let recorded = night.game().outcome_recorded_at.expect("result recorded");
        assert_eq!(activity, recorded);
        assert!(
            night.now < activity + Duration::from_secs(179),
            "the confirming word must not already consume the quiet window"
        );
        night
            .idle_through(activity + close_after() - Duration::from_secs(1))
            .await;
        assert_eq!(night.id(), id, "still the open game just before 180s");
        assert_eq!(night.sync_count(), 0, "no sync before the quiet window");
        assert_eq!(night.upload_count(), 0);
        night.idle_through(activity + close_after()).await;
        assert!(
            night.st.active_game.is_none(),
            "the last game is retired at 180s without another Tab"
        );
        assert_eq!(night.sync_count(), 1, "deleting the sync call must fail");
        assert_eq!(night.upload_count(), 1);
        night
            .idle_through(activity + Duration::from_secs(10 * 60))
            .await;
        assert!(
            night.st.active_game.is_none(),
            "the retired game stays closed through the rest of the night"
        );
        assert!(
            !active_game_path(night.dir.path()).exists(),
            "active_game.json is cleared with the session"
        );
        assert_eq!(night.sync_count(), 1, "deleting the sync call must fail");
        assert_eq!(
            night.upload_count(),
            1,
            "the unsynced rows upload exactly once"
        );
        let rows = night.store.get_session_snapshots(&id).await.unwrap();
        assert!(
            !rows.is_empty(),
            "the scoreboard rows are still in the store"
        );
        assert!(rows.iter().all(|row| row.outcome == "defeat"));
        assert!(rows.iter().all(|row| row.synced));
        assert_eq!(night.elims(&id).await, vec![14]);
    }

    #[tokio::test]
    async fn post_match_tab_inside_grace_delays_the_quiet_close() {
        let mut night = Night::new().await;
        let id = finish_colosseo(&mut night).await;
        let recorded = night.game().outcome_recorded_at.expect("result recorded");
        night.idle_through(recorded + Duration::from_secs(60)).await;
        assert_eq!(night.id(), id);
        assert_eq!(night.sync_count(), 0);
        night
            .tab_once(
                night_counters(16, 23, 6, 2600, 10000, 420),
                "Zenyatta",
                Some(2),
                Some("Colosseo"),
                NIGHT_CLEAN,
            )
            .await;
        assert_eq!(
            night.id(),
            id,
            "a Tab at +60s still lands on the finished game"
        );
        assert!(night.elims(&id).await.contains(&16));
        let activity = night.game().last_activity;
        assert!(activity >= recorded + Duration::from_secs(60));
        night.idle_through(recorded + close_after()).await;
        assert_eq!(
            night.id(),
            id,
            "180s after the result is not enough once a later Tab moved last activity"
        );
        assert_eq!(night.sync_count(), 0);
        night
            .idle_through(activity + close_after() - Duration::from_secs(1))
            .await;
        assert_eq!(night.id(), id);
        night.idle_through(activity + close_after()).await;
        assert!(night.st.active_game.is_none());
        assert_eq!(night.sync_count(), 1);
        assert_eq!(night.upload_count(), 1);
        let rows = night.store.get_session_snapshots(&id).await.unwrap();
        assert!(rows.iter().all(|row| row.outcome == "defeat"));
        assert!(night.elims(&id).await.contains(&16));
    }

    #[tokio::test]
    async fn tab_after_the_quiet_close_opens_a_new_session() {
        let mut night = Night::new().await;
        let id = finish_colosseo(&mut night).await;
        let activity = night.game().last_activity;
        night.idle_through(activity + close_after()).await;
        assert!(night.st.active_game.is_none());
        assert_eq!(night.upload_count(), 1);
        let before = night.elims(&id).await;
        night
            .idle_through(night.now + Duration::from_secs(5 * 60))
            .await;
        assert_eq!(
            night.upload_count(),
            1,
            "sitting after the close does not upload again"
        );
        night
            .tab_once(
                night_counters(2, 1, 0, 350, 60, 800),
                "Reinhardt",
                Some(0),
                Some("Ilios"),
                NIGHT_CLEAN,
            )
            .await;
        assert_ne!(night.id(), id, "the Tab opens a new session");
        assert_eq!(
            night.elims(&id).await,
            before,
            "the retired game is not written again"
        );
        let old = night.store.get_session_snapshots(&id).await.unwrap();
        assert!(old.iter().all(|row| row.outcome == "defeat"));
        night
            .word(detect::MatchOutcome::Victory, Some("Ilios"))
            .await;
        assert_eq!(night.game().outcome, detect::MatchOutcome::Victory);
        assert_ne!(night.id(), id);
        let old = night.store.get_session_snapshots(&id).await.unwrap();
        assert!(
            old.iter().all(|row| row.outcome == "defeat"),
            "a later result is not sealed onto the retired game"
        );
    }

    #[tokio::test]
    async fn unfinished_game_idle_for_21_minutes_stays_open() {
        let mut night = Night::new().await;
        night.begin_on("Busan", "Ana");
        night
            .tab_once(
                night_counters(8, 3, 2, 1800, 400, 100),
                "Ana",
                Some(0),
                Some("Busan"),
                NIGHT_CLEAN,
            )
            .await;
        let id = night.id();
        assert!(!night.game().finished());
        let activity = night.game().last_activity;
        night.idle_through(activity + UNFINISHED_SESSION_IDLE).await;
        assert_eq!(night.id(), id, "exactly 20 minutes is still this game");
        night
            .idle_through(activity + Duration::from_secs(21 * 60))
            .await;
        assert_eq!(
            night.id(),
            id,
            "the timer must not close an unfinished game"
        );
        assert_eq!(night.sync_count(), 0);
        assert!(active_game_path(night.dir.path()).exists());
        let rows = night.store.get_session_snapshots(&id).await.unwrap();
        assert!(rows.iter().all(|row| row.outcome == "unknown"));
    }

    #[tokio::test]
    async fn victory_after_22_minutes_idle_stays_on_the_same_session() {
        let mut night = Night::new().await;
        night.begin_on("Circuit Royal", "Ana");
        night
            .tab_once(
                night_counters(8, 3, 2, 1800, 400, 100),
                "Ana",
                Some(0),
                Some("Circuit Royal"),
                NIGHT_CLEAN,
            )
            .await;
        let id = night.id();
        let activity = night.game().last_activity;
        night
            .idle_through(activity + Duration::from_secs(22 * 60))
            .await;
        assert_eq!(night.id(), id);
        night
            .word(detect::MatchOutcome::Victory, Some("Circuit Royal"))
            .await;
        assert_eq!(night.id(), id);
        assert_eq!(night.game().outcome, detect::MatchOutcome::Victory);
        let recorded = night.game().last_activity;
        night.idle_through(recorded + close_after()).await;
        assert!(night.st.active_game.is_none());
        assert_eq!(night.upload_count(), 1);
        let rows = night.store.get_session_snapshots(&id).await.unwrap();
        assert!(rows.iter().any(|row| row.elims == 8));
        assert!(rows.iter().all(|row| row.map_name == "Circuit Royal"));
        assert!(rows.iter().all(|row| row.outcome == "victory"));
    }

    #[tokio::test]
    async fn quiet_close_carries_a_held_board_onto_the_next_session() {
        let mut night = Night::new().await;
        let id = finish_colosseo(&mut night).await;
        {
            let g = night.st.active_game.as_mut().expect("finished game");
            g.deferred = Some(night_counters(1, 0, 0, 100, 0, 0));
            g.deferred_hero = Some("Ana".into());
            g.deferred_at = Some(night.wall);
        }
        let activity = night.game().last_activity;
        night.idle_through(activity + close_after()).await;
        assert_eq!(night.upload_count(), 1);
        let opened = night.game();
        assert_ne!(opened.session_id, id);
        assert!(opened.deferred_imported);
        assert_eq!(opened.deferred.map(|c| c.elims), Some(1));
        let new_id = night.id();
        night
            .tab_once(
                night_counters(9, 1, 0, 400, 0, 0),
                "Ana",
                Some(0),
                Some("Ilios"),
                NIGHT_CLEAN,
            )
            .await;
        assert!(night.elims(&id).await.iter().all(|elims| *elims != 1));
        let held = night.elims(&new_id).await;
        assert!(
            held.contains(&1),
            "held board landed on the new session: {held:?}"
        );
        assert!(held.contains(&9), "{held:?}");
    }

    #[tokio::test]
    async fn stored_game_mode_follows_the_session_map() {
        let mut night = Night::new().await;
        night.begin_on("Busan", "Ana");
        night
            .tab_text(
                night_counters(4, 1, 0, 800, 200, 0),
                "Ana",
                Some(0),
                "Dorado",
                NIGHT_CLEAN,
            )
            .await;
        let rows = night
            .store
            .get_session_snapshots(&night.id())
            .await
            .unwrap();
        assert_eq!(rows[0].map_name, "Busan");
        assert_eq!(rows[0].game_mode, "Control");
    }

    #[test]
    fn end_screen_split_log_uses_the_pending_boundary_and_prints_none() {
        let open = boundary::OpenNew {
            reason: boundary::CloseReason::EndScreenMap,
            seal_outcome: Some(detect::MatchOutcome::Defeat),
            new_outcome: detect::MatchOutcome::Victory,
            new_map: Some("Ilios".into()),
            candidates: Vec::new(),
            new_result: None,
        };
        let split = end_screen_split_log(true, Some(&open), Some(detect::MatchOutcome::Defeat));
        assert!(split.contains("armed=true"), "{split}");
        assert!(split.contains("accolade_map=Ilios"), "{split}");
        assert!(split.contains("sealed_hint="), "{split}");
        assert!(!split.contains("sealed_hint=none"), "{split}");
        let mut missing = open;
        missing.new_map = None;
        let split = end_screen_split_log(false, Some(&missing), None);
        assert!(split.contains("accolade_map=none"), "{split}");
        assert!(split.contains("sealed_hint=none"), "{split}");
        assert!(split.contains("armed=false"), "{split}");
    }

    fn rejected_report(session_id: &str) -> CaptureReport {
        CaptureReport {
            best_guess: None,
            recorded: false,
            outcome: detect::MatchOutcome::Unknown,
            map: None,
            map_source: None,
            session_id: session_id.to_string(),
            split: false,
            armed_reset: false,
            ignore_row: false,
            reset_streak: 0,
            reset_baseline: None,
            baseline_row: None,
            refresh_baseline: false,
            clear_hint: false,
            count_progress: false,
            career_panel: false,
            held_counters: None,
            held_hero: None,
            gate_state: None,
            hero_auth: HeroAuthState::default(),
            seal: None,
            close_reason: None,
            held_at: None,
            map_replaces_text_fallback: false,
        }
    }

    #[tokio::test]
    async fn a_rejected_or_failed_tab_keeps_the_carried_board() {
        let mut night = Night::new().await;
        night.begin_on("Ilios", "Ana");
        {
            let g = night.st.active_game.as_mut().expect("game");
            g.deferred = Some(night_counters(1, 0, 0, 50, 0, 0));
            g.deferred_hero = Some("Ana".into());
            g.deferred_at = Some(night.wall);
            g.deferred_imported = true;
        }
        let sid = night.id();
        apply_capture_report(
            &mut night.st,
            &night.store,
            night.dir.path(),
            &sid,
            Ok(rejected_report(&sid)),
            night.now,
        )
        .await;
        assert!(night.game().deferred_imported);
        assert_eq!(night.game().deferred.map(|c| c.elims), Some(1));
        apply_capture_report(
            &mut night.st,
            &night.store,
            night.dir.path(),
            &sid,
            Err("capture cycle failed".into()),
            night.now,
        )
        .await;
        assert_eq!(night.game().deferred.map(|c| c.elims), Some(1));
        night
            .tab_once(
                night_counters(3, 1, 0, 200, 40, 0),
                "Ana",
                Some(0),
                Some("Ilios"),
                NIGHT_CLEAN,
            )
            .await;
        let elims = night.elims(&sid).await;
        assert_eq!(
            elims.iter().filter(|elims| **elims == 1).count(),
            1,
            "{elims:?}"
        );
        assert!(elims.contains(&3), "{elims:?}");
        assert!(night.game().deferred.is_none());
    }

    #[tokio::test]
    async fn carried_board_is_not_stored_when_the_current_insert_fails() {
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        let raw = night_counters(1, 0, 0, 10, 0, 0);
        let gate = capture_gate::apply_gate(None, raw, NIGHT_CLEAN, false);
        let staged = StagedCapture {
            target_session: "held-fail".into(),
            create_session: true,
            map_name: "Busan".into(),
            map_source: None,
            outcome: detect::MatchOutcome::Victory,
            outcome_label: "victory".into(),
            gate,
            carried: Some(raw),
            carried_hero: Some("Ana".into()),
            carried_at: Utc::now(),
            split: false,
            recorded_map: Some("Busan".into()),
            replaced_text_fallback: false,
        };
        write_staged_rows(
            &store,
            &staged,
            "Ana",
            "Support",
            SurrealDatetime::from(Utc::now()),
        )
        .await
        .unwrap();
        write_carried_after_current(&store, dir.path(), &staged, "Ana", false)
            .await
            .unwrap();
        let rows = store.get_session_snapshots("held-fail").await.unwrap();
        assert!(
            rows.is_empty(),
            "a refused current insert must not store the carried board: {rows:?}"
        );
        assert!(!should_write_carried(false, true));
        assert!(should_write_carried(true, true));
    }

    #[tokio::test]
    async fn spawned_upload_joins_the_previous_task_before_it_runs() {
        let order = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let previous_order = std::sync::Arc::clone(&order);
        let previous = tokio::spawn(async move {
            rx.await.ok();
            previous_order.lock().unwrap().push("prev");
        });
        let upload_order = std::sync::Arc::clone(&order);
        let handle = spawn_after_previous(Some(previous), move || {
            let upload_order = std::sync::Arc::clone(&upload_order);
            async move {
                upload_order.lock().unwrap().push("upload");
            }
        });
        tokio::task::yield_now().await;
        assert!(
            order.lock().unwrap().is_empty(),
            "the upload must wait for the previous task"
        );
        tx.send(()).unwrap();
        handle.await.unwrap();
        assert_eq!(*order.lock().unwrap(), ["prev", "upload"]);
    }

    #[tokio::test]
    async fn begin_daemon_session_returns_before_the_previous_sync_finishes() {
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let mut sync_task = Some(tokio::spawn(async move {
            let _ = rx.await;
        }));
        let backoff = std::sync::Arc::new(std::sync::Mutex::new(sync::SyncBackoff::default()));
        let st = begin_daemon_session(dir.path(), &mut sync_task, &backoff, &store, None).await;
        assert!(st.active_game.is_none());
        assert!(
            !sync_task.as_ref().unwrap().is_finished(),
            "startup must return while the previous upload is still running"
        );
        tx.send(()).unwrap();
        sync_task.take().unwrap().await.unwrap();
    }

    #[tokio::test]
    async fn deathmatch_rows_stay_local() {
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        let mut guillard = test_match("dm", "victory");
        guillard.map_name = "Château Guillard".into();
        guillard.game_mode = "Deathmatch".into();
        let mut busan = test_match("busan-row", "victory");
        busan.map_name = "Busan".into();
        busan.game_mode = "Control".into();
        let mut corrected = test_match("edited-to-dm", "victory");
        corrected.map_name = "Busan".into();
        corrected.game_mode = "Control".into();
        corrected.corrected_map_name = Some("Château Guillard".into());
        store.insert_match(guillard).await.unwrap();
        store.insert_match(busan).await.unwrap();
        store.insert_match(corrected).await.unwrap();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen_upload = std::sync::Arc::clone(&seen);
        try_sync_with(&store, dir.path(), None, move |matches, _tombstones| {
            let seen_upload = std::sync::Arc::clone(&seen_upload);
            async move {
                *seen_upload.lock().unwrap() =
                    matches.into_iter().map(|row| row.map_name).collect();
                Ok(upload_ok())
            }
        })
        .await;
        assert_eq!(*seen.lock().unwrap(), vec!["Busan".to_string()]);
        let only = {
            let mut row = test_match("dm-only", "victory");
            row.map_name = "Château Guillard".into();
            row.game_mode = "Deathmatch".into();
            row
        };
        store.insert_match(only).await.unwrap();
        let called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let called_upload = std::sync::Arc::clone(&called);
        try_sync_with(&store, dir.path(), None, move |_matches, _tombstones| {
            let called_upload = std::sync::Arc::clone(&called_upload);
            async move {
                called_upload.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(upload_ok())
            }
        })
        .await;
        assert!(
            !called.load(std::sync::atomic::Ordering::SeqCst),
            "a deathmatch-only batch must not call upload"
        );
    }

    #[tokio::test]
    async fn arcade_and_stadium_rows_stay_local() {
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        let local = [
            ("assault", "Hanamura", "Assault"),
            ("elim", "Black Forest", "Elimination"),
            ("ctf", "Ayutthaya", "Capture the Flag"),
            ("race", "Powder Keg Mine", "Payload Race"),
            ("shop", "Workshop Chamber", "Workshop"),
            ("stadium", "Arena Victoriae", "Stadium"),
            ("temple", "Temple of Anubis", "Assault"),
        ];
        for (id, map, mode) in local {
            let mut row = test_match(id, "victory");
            row.map_name = map.into();
            row.game_mode = mode.into();
            store.insert_match(row).await.unwrap();
        }
        let mut busan = test_match("busan-row", "victory");
        busan.map_name = "Busan".into();
        busan.game_mode = "Control".into();
        store.insert_match(busan).await.unwrap();
        let mut mode_only = test_match("mode-assault", "victory");
        mode_only.map_name = "Busan".into();
        mode_only.game_mode = "Assault".into();
        store.insert_match(mode_only).await.unwrap();

        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen_upload = std::sync::Arc::clone(&seen);
        try_sync_with(&store, dir.path(), None, move |matches, _| {
            let seen_upload = std::sync::Arc::clone(&seen_upload);
            async move {
                *seen_upload.lock().unwrap() =
                    matches.into_iter().map(|row| row.session_id).collect();
                Ok(upload_ok())
            }
        })
        .await;
        assert_eq!(*seen.lock().unwrap(), vec!["busan-row".to_string()]);
        let rows = store.get_all_matches().await.unwrap();
        assert!(
            rows.iter().all(|row| row.synced),
            "local-only rows are marked synced and not left pending: {rows:?}"
        );
        assert!(store.get_unsynced().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn one_bad_row_uploads_the_rest_and_stays_quarantined() {
        let n = 8_i64;
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        for i in 0..n {
            let mut row = test_match(&format!("s{i}"), "victory");
            row.played_at = SurrealDatetime::from(Utc::now() + chrono::Duration::seconds(i));
            store.insert_match(row).await.unwrap();
        }
        let bad = "s3";
        let uploaded = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let uploaded_pass = std::sync::Arc::clone(&uploaded);
        let attempt = try_sync_with(&store, dir.path(), None, move |matches, _tombstones| {
            let uploaded_pass = std::sync::Arc::clone(&uploaded_pass);
            let bad_here = matches.iter().any(|row| row.session_id == bad);
            let ids: Vec<String> = matches.iter().map(|row| row.session_id.clone()).collect();
            async move {
                if bad_here {
                    Err(sync::SyncUploadError {
                        message: "matches[0]: hero is not allowed".into(),
                        status: Some(400),
                        retry_after: None,
                    })
                } else {
                    uploaded_pass.lock().unwrap().extend(ids);
                    Ok(upload_ok())
                }
            }
        })
        .await;
        assert!(matches!(attempt, sync::SyncAttempt::Uploaded));
        let mut got = uploaded.lock().unwrap().clone();
        got.sort();
        got.dedup();
        assert_eq!(got.len(), (n as usize) - 1);
        assert!(!got.iter().any(|id| id == bad));
        let rows = store.get_all_matches().await.unwrap();
        assert!(
            rows.iter()
                .filter(|row| row.session_id != bad)
                .all(|row| row.synced),
            "the other rows upload"
        );
        let bad_row = rows.iter().find(|row| row.session_id == bad).unwrap();
        assert!(!bad_row.synced);
        assert_eq!(bad_row.upload_rejection(), Some("hero is not allowed"));
        assert!(
            rows.iter()
                .filter(|row| row.session_id != bad)
                .all(|row| row.synced && row.upload_rejection().is_none()),
            "only the refused row is quarantined"
        );
        assert!(store.get_unsynced().await.unwrap().is_empty());

        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let calls_pass = std::sync::Arc::clone(&calls);
        let again = try_sync_with(&store, dir.path(), None, move |_matches, _| {
            let calls_pass = std::sync::Arc::clone(&calls_pass);
            async move {
                calls_pass.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(upload_ok())
            }
        })
        .await;
        assert!(matches!(again, sync::SyncAttempt::NoServerCall));
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);

        store
            .apply_command(&storage::StoreCommand::RetryUpload {
                session_id: bad.into(),
            })
            .await
            .unwrap();
        let retried = try_sync_with(&store, dir.path(), None, move |matches, _| async move {
            assert_eq!(matches.len(), 1);
            assert_eq!(matches[0].session_id, bad);
            Ok(upload_ok())
        })
        .await;
        assert!(matches!(retried, sync::SyncAttempt::Uploaded));
        let done = store.get_all_matches().await.unwrap();
        assert!(
            done.iter()
                .find(|row| row.session_id == bad)
                .unwrap()
                .synced
        );
    }

    #[tokio::test]
    async fn server_error_still_retries_the_whole_batch() {
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        store
            .insert_match(test_match("s1", "victory"))
            .await
            .unwrap();
        store
            .insert_match(test_match("s2", "defeat"))
            .await
            .unwrap();
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let calls_pass = std::sync::Arc::clone(&calls);
        let attempt = try_sync_with(&store, dir.path(), None, move |_matches, _| {
            let calls_pass = std::sync::Arc::clone(&calls_pass);
            async move {
                calls_pass.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Err(sync::SyncUploadError {
                    message: "down".into(),
                    status: Some(500),
                    retry_after: None,
                })
            }
        })
        .await;
        assert!(matches!(
            attempt,
            sync::SyncAttempt::ServerError { retry_after: None }
        ));
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        let rows = store.get_all_matches().await.unwrap();
        assert!(
            rows.iter()
                .all(|row| !row.synced && row.upload_rejection().is_none())
        );
        assert_eq!(store.get_unsynced().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn all_rows_rejected_stays_queued_and_backs_off() {
        let n = 8_i64;
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        for i in 0..n {
            let mut row = test_match(&format!("s{i}"), "victory");
            row.played_at = SurrealDatetime::from(Utc::now() + chrono::Duration::seconds(i));
            store.insert_match(row).await.unwrap();
        }
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let calls_pass = std::sync::Arc::clone(&calls);
        let attempt = try_sync_with(&store, dir.path(), None, move |_matches, _| {
            let calls_pass = std::sync::Arc::clone(&calls_pass);
            async move {
                calls_pass.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Err(sync::SyncUploadError {
                    message: "matches[0]: recognizer is not allowed".into(),
                    status: Some(422),
                    retry_after: None,
                })
            }
        })
        .await;
        assert!(matches!(
            attempt,
            sync::SyncAttempt::ServerError { retry_after: None }
        ));
        assert!(calls.load(std::sync::atomic::Ordering::SeqCst) > 1);
        let rows = store.get_all_matches().await.unwrap();
        assert_eq!(rows.len(), n as usize);
        assert!(
            rows.iter()
                .all(|row| !row.synced && row.upload_rejection().is_none()),
            "a server-wide refusal quarantines nothing"
        );
        assert_eq!(store.get_unsynced().await.unwrap().len(), n as usize);
        let mut backoff = sync::SyncBackoff::default();
        let now = std::time::Instant::now();
        backoff.observe(attempt, now);
        assert!(!backoff.should_attempt(now));
        assert!(backoff.retry_after(now) >= sync::SYNC_BACKOFF_BASE);
        assert_eq!(backoff.failures(), 1);
    }

    #[tokio::test]
    async fn pending_deletes_upload_when_no_match_rows_are_queued() {
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        store.delete_session("gone").await.unwrap();
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let calls_pass = std::sync::Arc::clone(&calls);
        let attempt = try_sync_with(&store, dir.path(), None, move |matches, tombstones| {
            let calls_pass = std::sync::Arc::clone(&calls_pass);
            async move {
                calls_pass.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                assert!(matches.is_empty());
                assert!(tombstones.iter().any(|id| id == "gone"));
                Ok(upload_ok())
            }
        })
        .await;
        assert!(matches!(attempt, sync::SyncAttempt::Uploaded));
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(store.get_pending_tombstones().await.unwrap().is_empty());
    }

    fn unfinished_row(session_id: &str) -> storage::PersonalMatch {
        let mut row = test_match(session_id, "unknown");
        row.map_name = "Busan".into();
        row.game_mode = "Control".into();
        row
    }

    /// While the session is still open, an unknown row stays unsynced so a
    /// late result can replace it, and no request is made. Closing the
    /// session marks it synced locally and still does not call upload.
    async fn assert_unknown_is_held_locally_on_close(
        store: &storage::LocalStore,
        dir: &std::path::Path,
        session_id: &str,
    ) {
        let called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let called_upload = std::sync::Arc::clone(&called);
        let open = try_sync_with(store, dir, None, move |_matches, _| {
            let called_upload = std::sync::Arc::clone(&called_upload);
            async move {
                called_upload.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(upload_ok())
            }
        })
        .await;
        assert!(
            matches!(open, sync::SyncAttempt::NoServerCall),
            "an open unfinished game makes no request"
        );
        assert!(
            !called.load(std::sync::atomic::Ordering::SeqCst),
            "an open unfinished game must not call upload"
        );
        let rows = store.get_all_matches().await.unwrap();
        assert!(
            rows.iter()
                .any(|row| row.session_id == session_id && !row.synced),
            "an open unfinished game stays unsynced so a late result can attach"
        );
        let settled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let settled_upload = std::sync::Arc::clone(&settled);
        let close = try_sync_with(
            store,
            dir,
            Some(session_id.to_string()),
            move |_matches, _| {
                let settled_upload = std::sync::Arc::clone(&settled_upload);
                async move {
                    settled_upload.store(true, std::sync::atomic::Ordering::SeqCst);
                    Ok(upload_ok())
                }
            },
        )
        .await;
        assert!(
            matches!(close, sync::SyncAttempt::NoServerCall),
            "holding an unknown row locally makes no request"
        );
        assert!(
            !settled.load(std::sync::atomic::Ordering::SeqCst),
            "an unknown row must not be sent"
        );
        let rows = store.get_all_matches().await.unwrap();
        assert!(
            rows.iter().any(|row| {
                row.session_id == session_id && row.synced && row.outcome == "unknown"
            }),
            "the close marks the unknown row synced without sending it"
        );
    }

    #[tokio::test]
    async fn shutdown_keeps_an_unfinished_game_so_a_restart_resumes_it() {
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        store
            .insert_match(unfinished_row("quit-after-match"))
            .await
            .unwrap();
        let now = test_now();
        let mut g = game(detect::MatchOutcome::Unknown, None, now);
        g.session_id = "quit-after-match".into();
        g.map = Some("Busan".into());
        let mut st = session(Some(g), now);
        persist_active_game(dir.path(), st.active_game.as_ref());
        shutdown_from_signal(
            &mut st,
            &mut None,
            &mut None,
            None,
            &std::sync::Mutex::new(sync::SyncBackoff::default()),
            &store,
            dir.path(),
        )
        .await;
        assert_eq!(
            st.active_game.as_ref().map(|g| g.session_id.as_str()),
            Some("quit-after-match"),
            "a clean shutdown keeps the unfinished skeleton"
        );
        assert!(active_game_path(dir.path()).exists());
        assert!(st.settle_closed_unknown.is_none());
        let restored = startup_session(dir.path());
        assert_eq!(
            restored.active_game.as_ref().map(|g| g.session_id.as_str()),
            Some("quit-after-match"),
            "a GUI restart resumes the same session"
        );
        let rows = store.get_all_matches().await.unwrap();
        assert!(
            rows.iter()
                .any(|row| row.session_id == "quit-after-match" && !row.synced),
            "the resumed row stays unsynced so a late result can attach"
        );
    }

    #[tokio::test]
    async fn a_new_tab_after_idle_closes_an_unfinished_game_and_holds_it() {
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        store
            .insert_match(unfinished_row("left-open"))
            .await
            .unwrap();
        let now = test_now();
        let mut g = game(detect::MatchOutcome::Unknown, None, now);
        g.session_id = "left-open".into();
        g.map = Some("Busan".into());
        g.last_activity = now - UNFINISHED_SESSION_IDLE - Duration::from_secs(1);
        let mut st = session(Some(g), now);
        persist_active_game(dir.path(), st.active_game.as_ref());
        let opened = open_fresh_if_tab_starts_one(&mut st, &store, dir.path(), now).await;
        assert!(opened, "the idle bound opens the next session");
        assert_ne!(
            st.active_game.as_ref().map(|g| g.session_id.as_str()),
            Some("left-open")
        );
        assert_eq!(st.settle_closed_unknown.as_deref(), Some("left-open"));
        assert_unknown_is_held_locally_on_close(&store, dir.path(), "left-open").await;
    }

    #[tokio::test]
    async fn restart_holds_a_stale_unfinished_game_and_keeps_a_fresh_one() {
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        store
            .insert_match(unfinished_row("stale-unfinished"))
            .await
            .unwrap();
        let when = Utc::now() - chrono::Duration::hours(9);
        let stale = PersistedGame {
            replaced_text_fallback_map: None,
            best_guess: None,
            session_id: "stale-unfinished".into(),
            outcome: detect::MatchOutcome::Unknown,
            map: Some("Busan".into()),
            map_source: None,
            map_candidates: Vec::new(),
            session_created: true,
            opened_at: when - chrono::Duration::minutes(20),
            last_activity: when,
            outcome_recorded_at: None,
            gate: None,
            last_stats_at: None,
            hero_auth: HeroAuthState::default(),
            result_outcome: None,
            result_confirmed: false,
            result_seen_at: None,
            pending_boundary: false,
            awaiting_first_board: false,
            reset_streak: 0,
            reset_baseline: None,
            baseline_row: None,
            deferred: None,
            deferred_hero: None,
            deferred_at: None,
            deferred_imported: false,
            progressed_boards: 0,
            baseline_at: None,
            text_fallback_locked: false,
        };
        std::fs::write(
            active_game_path(dir.path()),
            serde_json::to_vec(&stale).unwrap(),
        )
        .unwrap();
        let dropped = retire_stale_skeleton(dir.path());
        assert_eq!(
            dropped.as_ref().map(|drop| drop.session_id.as_str()),
            Some("stale-unfinished")
        );
        assert!(dropped.as_ref().is_some_and(|drop| drop.archive.is_none()));
        assert!(!active_game_path(dir.path()).exists());
        let restored = startup_session(dir.path());
        assert!(restored.active_game.is_none());
        assert_unknown_is_held_locally_on_close(&store, dir.path(), "stale-unfinished").await;

        let fresh_dir = tempfile::tempdir().unwrap();
        let fresh_store = storage::LocalStore::open(fresh_dir.path()).await.unwrap();
        fresh_store
            .insert_match(unfinished_row("still-this-match"))
            .await
            .unwrap();
        let now = Utc::now();
        let fresh = PersistedGame {
            replaced_text_fallback_map: None,
            session_id: "still-this-match".into(),
            last_activity: now,
            opened_at: now - chrono::Duration::minutes(5),
            ..stale
        };
        std::fs::write(
            active_game_path(fresh_dir.path()),
            serde_json::to_vec(&fresh).unwrap(),
        )
        .unwrap();
        assert!(
            retire_stale_skeleton(fresh_dir.path()).is_none(),
            "a restart inside the idle bound resumes the match"
        );
        let resumed = startup_session(fresh_dir.path());
        assert_eq!(
            resumed.active_game.as_ref().map(|g| g.session_id.as_str()),
            Some("still-this-match")
        );
        let seen = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen_upload = std::sync::Arc::clone(&seen);
        try_sync_with(&fresh_store, fresh_dir.path(), None, move |matches, _| {
            let seen_upload = std::sync::Arc::clone(&seen_upload);
            async move {
                seen_upload.store(matches.len(), std::sync::atomic::Ordering::SeqCst);
                Ok(upload_ok())
            }
        })
        .await;
        assert_eq!(seen.load(std::sync::atomic::Ordering::SeqCst), 0);
        let rows = fresh_store.get_all_matches().await.unwrap();
        assert!(
            rows.iter()
                .any(|row| row.session_id == "still-this-match" && !row.synced),
            "a resumed match stays unsynced so a late result can attach"
        );
    }

    #[tokio::test]
    async fn set_outcome_on_the_open_session_uploads_on_the_next_sync() {
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        store
            .insert_match(unfinished_row("open-set"))
            .await
            .unwrap();
        let now = test_now();
        let mut g = game(detect::MatchOutcome::Unknown, None, now);
        g.session_id = "open-set".into();
        g.map = Some("Busan".into());
        let mut st = session(Some(g), now);
        assert!(st.settle_closed_unknown.is_none());
        if let Some(active) = st.active_game.as_mut() {
            active.record_outcome(detect::MatchOutcome::Victory);
        }
        store
            .apply_command(&storage::StoreCommand::SetOutcome {
                session_id: "open-set".into(),
                outcome: "victory".into(),
            })
            .await
            .unwrap();
        assert_eq!(
            st.active_game
                .as_ref()
                .map(|active| active.session_id.as_str()),
            Some("open-set"),
            "SetOutcome does not close the open session"
        );
        assert!(st.settle_closed_unknown.is_none());
        assert!(st.active_game.as_ref().unwrap().finished());
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen_upload = std::sync::Arc::clone(&seen);
        let attempt = try_sync_with(&store, dir.path(), None, move |matches, _| {
            let seen_upload = std::sync::Arc::clone(&seen_upload);
            async move {
                *seen_upload.lock().unwrap() = matches
                    .into_iter()
                    .map(|row| {
                        let outcome = row.display_outcome().to_string();
                        let edited = row.is_edited();
                        (row.session_id, outcome, edited)
                    })
                    .collect();
                Ok(upload_ok())
            }
        })
        .await;
        assert!(
            matches!(attempt, sync::SyncAttempt::Uploaded),
            "the next sync uploads the open session"
        );
        assert_eq!(
            *seen.lock().unwrap(),
            vec![("open-set".to_string(), "victory".to_string(), true)]
        );
    }

    #[tokio::test]
    async fn restart_syncs_a_stale_finished_skeleton_without_losing_rows() {
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        store
            .insert_match(test_match("colosseo-defeat", "defeat"))
            .await
            .unwrap();
        let when = Utc::now() - chrono::Duration::hours(9);
        let stale = PersistedGame {
            replaced_text_fallback_map: None,
            best_guess: None,
            session_id: "colosseo-defeat".into(),
            outcome: detect::MatchOutcome::Defeat,
            map: Some("Colosseo".into()),
            map_source: None,
            map_candidates: Vec::new(),
            session_created: true,
            opened_at: when - chrono::Duration::minutes(20),
            last_activity: when,
            outcome_recorded_at: Some(when),
            gate: None,
            last_stats_at: None,
            hero_auth: HeroAuthState::default(),
            result_outcome: Some(detect::MatchOutcome::Defeat),
            result_confirmed: true,
            result_seen_at: Some(when),
            pending_boundary: false,
            awaiting_first_board: false,
            reset_streak: 0,
            reset_baseline: None,
            baseline_row: None,
            deferred: None,
            deferred_hero: None,
            deferred_at: None,
            deferred_imported: false,
            progressed_boards: 0,
            baseline_at: None,
            text_fallback_locked: false,
        };
        std::fs::write(
            active_game_path(dir.path()),
            serde_json::to_vec(&stale).unwrap(),
        )
        .unwrap();
        let uploads = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let sync_with = |uploads: std::sync::Arc<std::sync::atomic::AtomicUsize>,
                         store: storage::LocalStore,
                         dir: std::path::PathBuf| {
            move |dropped: Option<String>| {
                let uploads = std::sync::Arc::clone(&uploads);
                let store = store.clone();
                let dir = dir.clone();
                async move {
                    let _ = try_sync_with(&store, &dir, dropped, move |matches, _tombstones| {
                        let uploads = std::sync::Arc::clone(&uploads);
                        async move {
                            assert_eq!(matches.len(), 1);
                            assert_eq!(matches[0].session_id, "colosseo-defeat");
                            assert_eq!(matches[0].outcome, "defeat");
                            uploads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                            Ok(upload_ok())
                        }
                    })
                    .await;
                }
            }
        };
        let restored = recover_or_sync_active_game(
            dir.path(),
            &store,
            sync_with(
                std::sync::Arc::clone(&uploads),
                store.clone(),
                dir.path().to_path_buf(),
            ),
        )
        .await;
        assert!(
            restored.active_game.is_none() && restored.last_game_open.is_none(),
            "a 9-hour-old skeleton is not the open game"
        );
        assert_eq!(uploads.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(!active_game_path(dir.path()).exists());
        let rows = store.get_all_matches().await.unwrap();
        assert_eq!(
            rows.len(),
            1,
            "dropping the skeleton must not drop the rows"
        );
        assert_eq!(rows[0].session_id, "colosseo-defeat");
        assert_eq!(rows[0].outcome, "defeat");
        assert_eq!(rows[0].elims, 10);
        assert!(rows[0].synced);

        let again = recover_or_sync_active_game(
            dir.path(),
            &store,
            sync_with(
                std::sync::Arc::clone(&uploads),
                store.clone(),
                dir.path().to_path_buf(),
            ),
        )
        .await;
        assert!(again.active_game.is_none());
        assert_eq!(
            uploads.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "a second start must not upload the same game again"
        );
        assert_eq!(store.get_all_matches().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn quiet_close_persists_so_a_restart_does_not_retire_twice() {
        let mut night = Night::new().await;
        let id = finish_colosseo(&mut night).await;
        let activity = night.game().last_activity;
        night.idle_through(activity + close_after()).await;
        assert!(night.st.active_game.is_none());
        assert!(!active_game_path(night.dir.path()).exists());
        assert_eq!(night.upload_count(), 1);
        let uploads = night.upload_count();
        let recovered = night.restart_from_disk().await;
        assert!(
            recovered.is_none(),
            "the cleared skeleton must not come back as the open game"
        );
        assert!(recover_active_game(night.dir.path()).is_none());
        assert_eq!(
            night.upload_count(),
            uploads,
            "restart must not upload again"
        );
        assert!(
            !night.quiet_tick().await,
            "a second quiet tick must not close the session again"
        );
        assert_eq!(night.upload_count(), uploads);
        let rows = night.store.get_session_snapshots(&id).await.unwrap();
        assert!(!rows.is_empty());
        assert!(rows.iter().all(|row| row.outcome == "defeat" && row.synced));
    }

    fn fresh_counters(elims: u32) -> Counters {
        Counters {
            elims,
            assists: 0,
            deaths: 0,
            damage: 100,
            healing: 0,
            mitigation: 0,
        }
    }

    fn staged_row(
        session: &str,
        map: &str,
        source: Option<boundary::MapSource>,
    ) -> (StagedCapture, storage::PersonalMatch) {
        let raw = fresh_counters(14);
        let gate = capture_gate::apply_gate(
            None,
            raw,
            [false; stat_tracker::capture_gate::GATE_COLS],
            false,
        );
        let mut parsed = test_match(session, "victory");
        parsed.map_name = map.to_string();
        parsed.elims = raw.elims;
        parsed.played_at = SurrealDatetime::from(Utc::now());
        let staged = StagedCapture {
            target_session: session.to_string(),
            create_session: true,
            map_name: map.to_string(),
            map_source: source,
            outcome: detect::MatchOutcome::Victory,
            outcome_label: "victory".into(),
            gate,
            carried: None,
            carried_hero: None,
            carried_at: Utc::now() - chrono::Duration::seconds(30),
            split: false,
            recorded_map: Some(map.to_string()),
            replaced_text_fallback: false,
        };
        (staged, parsed)
    }

    fn loopback_client(port: u16) -> sync::SyncClient {
        sync::SyncClient::try_new(config::SyncConfig {
            server_url: format!("http://127.0.0.1:{port}"),
            token: "test-token".into(),
        })
        .unwrap()
    }

    fn http_request_ready(buf: &[u8]) -> bool {
        let Some(split) = buf.windows(4).position(|w| w == b"\r\n\r\n") else {
            return false;
        };
        let headers = String::from_utf8_lossy(&buf[..split]).to_ascii_lowercase();
        let body = &buf[split + 4..];
        for line in headers.lines() {
            if let Some(rest) = line.trim().strip_prefix("content-length:")
                && let Ok(len) = rest.trim().parse::<usize>()
            {
                return body.len() >= len;
            }
        }
        false
    }

    fn spawn_upload_sink() -> (u16, std::sync::mpsc::Receiver<String>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let Ok((mut sock, _)) = listener.accept() else {
                return;
            };
            sock.set_read_timeout(Some(std::time::Duration::from_secs(3)))
                .ok();
            let mut buf = Vec::new();
            let mut tmp = [0u8; 4096];
            loop {
                match std::io::Read::read(&mut sock, &mut tmp) {
                    Ok(0) => break,
                    Ok(n) => {
                        buf.extend_from_slice(&tmp[..n]);
                        if http_request_ready(&buf) {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let body = b"{\"inserted\":1,\"skipped\":0,\"deleted\":0}";
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = std::io::Write::write_all(&mut sock, header.as_bytes());
            let _ = std::io::Write::write_all(&mut sock, body);
            let _ = tx.send(String::from_utf8_lossy(&buf).into_owned());
        });
        (port, rx)
    }

    #[tokio::test]
    async fn shutdown_uploads_the_finished_board_not_a_fresh_hold() {
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        let mut row = test_match("busan-win", "victory");
        row.map_name = "Busan".into();
        row.game_mode = "Control".into();
        row.elims = 14;
        store.insert_match(row).await.unwrap();
        let now = test_now();
        let mut g = game(detect::MatchOutcome::Victory, Some(10), now);
        g.session_id = "busan-win".into();
        g.map = Some("Busan".into());
        g.deferred = Some(fresh_counters(2));
        g.deferred_hero = Some("Tracer".into());
        g.deferred_imported = false;
        let mut st = session(Some(g), now);
        shutdown_from_signal(
            &mut st,
            &mut None,
            &mut None,
            None,
            &std::sync::Mutex::new(sync::SyncBackoff::default()),
            &store,
            dir.path(),
        )
        .await;
        let held = st
            .active_game
            .as_ref()
            .expect("fresh hold opens the next session");
        assert_ne!(held.session_id, "busan-win");
        assert!(held.deferred_imported);
        assert_eq!(held.deferred.map(|c| c.elims), Some(2));
        let busan = store.get_session_snapshots("busan-win").await.unwrap();
        assert_eq!(busan.len(), 1);
        assert_eq!(busan[0].elims, 14);
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen_upload = std::sync::Arc::clone(&seen);
        try_sync_with(&store, dir.path(), None, move |matches, _| {
            let seen_upload = std::sync::Arc::clone(&seen_upload);
            async move {
                *seen_upload.lock().unwrap() = matches
                    .into_iter()
                    .map(|row| (row.session_id, row.outcome, row.elims, row.map_name))
                    .collect();
                Ok(upload_ok())
            }
        })
        .await;
        assert_eq!(
            *seen.lock().unwrap(),
            vec![(
                "busan-win".to_string(),
                "victory".to_string(),
                14,
                "Busan".to_string()
            )]
        );
        let resumed = startup_session(dir.path());
        assert_eq!(
            resumed.active_game.as_ref().map(|g| g.session_id.as_str()),
            Some(held.session_id.as_str()),
            "a GUI restart resumes the next session, not a rewritten Busan row"
        );
        let called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let called_upload = std::sync::Arc::clone(&called);
        try_sync_with(&store, dir.path(), None, move |matches, _| {
            let called_upload = std::sync::Arc::clone(&called_upload);
            async move {
                assert!(
                    matches.iter().all(|row| row.elims != 2),
                    "restart must not upload the held board as Busan: {matches:?}"
                );
                called_upload.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(upload_ok())
            }
        })
        .await;
        assert!(
            !called.load(std::sync::atomic::Ordering::SeqCst),
            "the finished row was already uploaded"
        );
        let all = store.get_all_matches().await.unwrap();
        assert!(all.iter().all(|row| row.elims != 2));
    }

    #[tokio::test]
    async fn shutdown_writes_an_imported_hold_and_does_not_warn() {
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        let mut row = test_match("carried-in", "victory");
        row.map_name = "Busan".into();
        row.game_mode = "Control".into();
        row.hero = "Ana".into();
        row.elims = 9;
        store.insert_match(row).await.unwrap();
        let now = test_now();
        let mut g = game(detect::MatchOutcome::Victory, Some(10), now);
        g.session_id = "carried-in".into();
        g.map = Some("Busan".into());
        g.session_created = false;
        g.deferred = Some(fresh_counters(3));
        g.deferred_hero = None;
        g.deferred_imported = true;
        let mut st = session(Some(g), now);
        close_session_on_shutdown(&mut st, &store, dir.path()).await;
        let rows = store.get_session_snapshots("carried-in").await.unwrap();
        assert!(
            rows.iter().any(|row| row.elims == 3 && row.hero == "Ana"),
            "an imported hold is written, and the hero falls back to the newest row: {rows:?}"
        );
        let note = dir.path().join("debug").join("unrecorded_games.jsonl");
        if let Ok(text) = std::fs::read_to_string(&note) {
            assert!(
                !text.contains("carried-in"),
                "writing the board must not warn that the game had no captures: {text}"
            );
        }
    }

    #[tokio::test]
    async fn a_failed_shutdown_write_keeps_an_imported_hold_for_the_next_start() {
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        let mut row = test_match("held-session-fail", "victory");
        row.map_name = "Busan".into();
        row.game_mode = "Control".into();
        row.hero = "Ana".into();
        row.elims = 9;
        store.insert_match(row).await.unwrap();
        let now = test_now();
        let mut g = game(detect::MatchOutcome::Victory, Some(10), now);
        g.session_id = "held-session-fail".into();
        g.map = Some("Busan".into());
        g.deferred = Some(fresh_counters(4));
        g.deferred_hero = Some("Tracer".into());
        g.deferred_at = Some(Utc::now());
        g.deferred_imported = true;
        let mut st = session(Some(g), now);
        *FAIL_HELD_BOARD_SESSION.lock().unwrap() = Some("held-session-fail".into());
        close_session_on_shutdown(&mut st, &store, dir.path()).await;
        let held = st.active_game.as_ref().expect("the hold stays open");
        assert!(held.deferred_imported);
        assert_eq!(held.deferred.map(|c| c.elims), Some(4));
        let rows = store
            .get_session_snapshots("held-session-fail")
            .await
            .unwrap();
        assert!(
            rows.iter().all(|row| row.elims != 4),
            "a failed write must not drop the hold by storing it: {rows:?}"
        );
        assert!(active_game_path(dir.path()).exists());
        let mut sync_task = None;
        let backoff = std::sync::Arc::new(std::sync::Mutex::new(sync::SyncBackoff::default()));
        let restarted =
            begin_daemon_session(dir.path(), &mut sync_task, &backoff, &store, None).await;
        let rows = store
            .get_session_snapshots("held-session-fail")
            .await
            .unwrap();
        assert!(
            rows.iter().all(|row| row.elims != 4),
            "a resume within 20 minutes must leave the hold pending: {rows:?}"
        );
        let held = restarted
            .active_game
            .as_ref()
            .expect("the session is still open");
        assert!(held.deferred_imported);
        assert_eq!(held.deferred.map(|c| c.elims), Some(4));
    }

    #[tokio::test]
    async fn a_stale_restart_keeps_a_held_board_as_an_unknown_session() {
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        let mut decided = test_match("already-decided", "victory");
        decided.map_name = "Busan".into();
        decided.game_mode = "Control".into();
        decided.elims = 14;
        store.insert_match(decided).await.unwrap();
        let when = Utc::now() - chrono::Duration::hours(9);
        let stale = PersistedGame {
            replaced_text_fallback_map: None,
            best_guess: None,
            session_id: "next-game".into(),
            outcome: detect::MatchOutcome::Unknown,
            map: None,
            map_source: None,
            map_candidates: Vec::new(),
            session_created: false,
            opened_at: when - chrono::Duration::minutes(20),
            last_activity: when,
            outcome_recorded_at: None,
            gate: None,
            last_stats_at: None,
            hero_auth: HeroAuthState::default(),
            result_outcome: None,
            result_confirmed: false,
            result_seen_at: None,
            pending_boundary: false,
            awaiting_first_board: false,
            reset_streak: 0,
            reset_baseline: None,
            baseline_row: None,
            deferred: Some(fresh_counters(2)),
            deferred_hero: Some("Tracer".into()),
            deferred_at: Some(when),
            deferred_imported: true,
            progressed_boards: 0,
            baseline_at: None,
            text_fallback_locked: false,
        };
        std::fs::write(
            active_game_path(dir.path()),
            serde_json::to_vec(&stale).unwrap(),
        )
        .unwrap();
        let mut sync_task = None;
        let backoff = std::sync::Arc::new(std::sync::Mutex::new(sync::SyncBackoff::default()));
        let st = begin_daemon_session(dir.path(), &mut sync_task, &backoff, &store, None).await;
        assert!(st.active_game.is_none());
        assert!(!active_game_path(dir.path()).exists());
        let rows = store.get_all_matches().await.unwrap();
        let held = rows
            .iter()
            .find(|row| row.elims == 2)
            .expect("the hold is in local history");
        assert_eq!(held.outcome, "unknown");
        assert_eq!(held.session_id, "next-game");
        assert_eq!(held.hero, "Tracer");
        assert!(held.map_name.is_empty());
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen_upload = std::sync::Arc::clone(&seen);
        try_sync_with(
            &store,
            dir.path(),
            Some("next-game".into()),
            move |matches, _| {
                let seen_upload = std::sync::Arc::clone(&seen_upload);
                async move {
                    *seen_upload.lock().unwrap() = matches
                        .into_iter()
                        .map(|row| (row.session_id, row.elims, row.map_name, row.outcome))
                        .collect();
                    Ok(upload_ok())
                }
            },
        )
        .await;
        assert_eq!(
            *seen.lock().unwrap(),
            vec![(
                "already-decided".to_string(),
                14,
                "Busan".to_string(),
                "victory".to_string()
            )]
        );
        let kept = store
            .get_all_matches()
            .await
            .unwrap()
            .into_iter()
            .find(|row| row.elims == 2)
            .unwrap();
        assert!(kept.synced, "the unknown hold is kept locally and not sent");
    }

    fn persisted_with_hold(
        session_id: &str,
        outcome: detect::MatchOutcome,
        map: Option<&str>,
        imported: bool,
        elims: u32,
        last_activity: chrono::DateTime<Utc>,
    ) -> PersistedGame {
        PersistedGame {
            replaced_text_fallback_map: None,
            best_guess: None,
            session_id: session_id.into(),
            outcome,
            map: map.map(str::to_string),
            map_source: None,
            map_candidates: Vec::new(),
            session_created: false,
            opened_at: last_activity - chrono::Duration::minutes(20),
            last_activity,
            outcome_recorded_at: None,
            gate: None,
            last_stats_at: None,
            hero_auth: HeroAuthState::default(),
            result_outcome: None,
            result_confirmed: false,
            result_seen_at: None,
            pending_boundary: false,
            awaiting_first_board: false,
            reset_streak: 0,
            reset_baseline: None,
            baseline_row: None,
            deferred: Some(fresh_counters(elims)),
            deferred_hero: Some("Tracer".into()),
            deferred_at: Some(last_activity),
            deferred_imported: imported,
            progressed_boards: 0,
            baseline_at: None,
            text_fallback_locked: false,
        }
    }

    /// A game on disk with a stored board, last captured `ago` before now.
    fn restart_skeleton(map: Option<&str>, ago: chrono::Duration) -> PersistedGame {
        let mut p = persisted_with_hold(
            "before-crash",
            detect::MatchOutcome::Unknown,
            map,
            false,
            0,
            Utc::now() - ago,
        );
        p.session_created = true;
        p.deferred = None;
        p.deferred_hero = None;
        p.deferred_at = None;
        p
    }

    /// Restart from `p`, then ask whether the first Tab opens a new game.
    fn first_tab_after_restart_starts_new_game(p: &PersistedGame) -> bool {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(active_game_path(dir.path()), serde_json::to_vec(p).unwrap()).unwrap();
        let st = startup_session(dir.path());
        should_start_fresh_session(st.active_game.as_ref(), Instant::now())
    }

    /// QA: same map, last capture recent, no result seen: the restart
    /// resumes the game, so the next Tab stays on it.
    #[test]
    fn a_restart_resumes_the_same_recent_game_with_no_result() {
        let p = restart_skeleton(Some("Busan"), chrono::Duration::seconds(40));
        assert_eq!(restart_resume_refusal(&p, Utc::now()), None);
        assert!(!first_tab_after_restart_starts_new_game(&p));
    }

    /// QA: a long gap since the last capture starts a new game, even on the
    /// same map, well inside the old 20 minute idle bound.
    #[test]
    fn a_restart_after_a_long_gap_starts_a_new_game() {
        let gap = chrono::Duration::from_std(RESUME_AFTER_RESTART_MAX_GAP).unwrap()
            + chrono::Duration::seconds(30);
        assert!(gap.to_std().unwrap() < UNFINISHED_SESSION_IDLE);
        let p = restart_skeleton(Some("Busan"), gap);
        assert_eq!(
            restart_resume_refusal(&p, Utc::now()),
            Some("last capture too long ago")
        );
        assert!(first_tab_after_restart_starts_new_game(&p));
    }

    /// A game with a board but no map cannot be matched, so it is not
    /// resumed. A clock that moved back is not resumed either.
    #[test]
    fn a_restart_without_a_map_or_with_a_future_capture_starts_a_new_game() {
        let p = restart_skeleton(None, chrono::Duration::seconds(40));
        assert!(restart_resume_refusal(&p, Utc::now()).is_some());
        assert!(first_tab_after_restart_starts_new_game(&p));
        let p = restart_skeleton(Some("Busan"), chrono::Duration::seconds(-120));
        assert!(restart_resume_refusal(&p, Utc::now()).is_some());
        assert!(first_tab_after_restart_starts_new_game(&p));
    }

    /// QA: a result screen was seen before the crash. The restart keeps the
    /// game so its result is not lost, and the first Tab past the
    /// post-match grace starts a new game. Inside the grace the post-match
    /// scoreboard still belongs to it.
    #[test]
    fn a_restart_after_a_result_screen_starts_a_new_game() {
        let mut p = restart_skeleton(Some("Busan"), chrono::Duration::seconds(30));
        p.result_outcome = Some(detect::MatchOutcome::Victory);
        p.result_seen_at = Some(Utc::now() - chrono::Duration::seconds(100));
        assert!(first_tab_after_restart_starts_new_game(&p));

        p.result_seen_at = Some(Utc::now() - chrono::Duration::seconds(10));
        assert!(
            !first_tab_after_restart_starts_new_game(&p),
            "the post-match scoreboard inside the grace stays on the game"
        );

        let mut p = restart_skeleton(Some("Busan"), chrono::Duration::seconds(100));
        p.outcome = detect::MatchOutcome::Defeat;
        p.outcome_recorded_at = Some(Utc::now() - chrono::Duration::seconds(100));
        assert!(first_tab_after_restart_starts_new_game(&p));
    }

    /// QA: two real games in a row on the same map stay separate across a
    /// crash. Game 1 on Busan saw its result, the tracker restarted, and
    /// game 2 on Busan opens its own session; game 2's restart then resumes
    /// game 2, not game 1.
    #[test]
    fn two_games_in_a_row_on_the_same_map_stay_separate_across_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let mut game1 = restart_skeleton(Some("Busan"), chrono::Duration::seconds(150));
        game1.session_id = "game-1".into();
        game1.result_outcome = Some(detect::MatchOutcome::Victory);
        game1.result_seen_at = Some(Utc::now() - chrono::Duration::seconds(150));
        std::fs::write(
            active_game_path(dir.path()),
            serde_json::to_vec(&game1).unwrap(),
        )
        .unwrap();
        let mut st = startup_session(dir.path());
        assert_eq!(
            st.active_game.as_ref().map(|g| g.session_id.as_str()),
            Some("game-1")
        );
        let now = Instant::now();
        assert!(should_start_fresh_session(st.active_game.as_ref(), now));
        let mut game2 = ActiveGame::open_at(
            "game-2".into(),
            detect::MatchOutcome::Unknown,
            Vec::new(),
            now,
        );
        game2.map = Some("Busan".into());
        game2.session_created = true;
        st.active_game = Some(game2);
        assert!(!should_start_fresh_session(st.active_game.as_ref(), now));

        persist_active_game(dir.path(), st.active_game.as_ref());
        let st = startup_session(dir.path());
        assert_eq!(
            st.active_game.as_ref().map(|g| g.session_id.as_str()),
            Some("game-2"),
            "a restart during game 2 resumes game 2"
        );
        assert!(!should_start_fresh_session(
            st.active_game.as_ref(),
            Instant::now()
        ));
    }

    async fn begin_from_persisted(
        dir: &std::path::Path,
        store: &storage::LocalStore,
        game: PersistedGame,
    ) -> SessionState {
        std::fs::write(active_game_path(dir), serde_json::to_vec(&game).unwrap()).unwrap();
        let mut sync_task = None;
        let backoff = std::sync::Arc::new(std::sync::Mutex::new(sync::SyncBackoff::default()));
        begin_daemon_session(dir, &mut sync_task, &backoff, store, None).await
    }

    #[tokio::test]
    async fn a_stale_fresh_hold_does_not_keep_the_previous_map() {
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        let when = Utc::now() - chrono::Duration::hours(9);
        begin_from_persisted(
            dir.path(),
            &store,
            persisted_with_hold(
                "busan-game",
                detect::MatchOutcome::Victory,
                Some("Busan"),
                false,
                3,
                when,
            ),
        )
        .await;
        let rows = store.get_all_matches().await.unwrap();
        let held = rows
            .iter()
            .find(|row| row.elims == 3)
            .expect("the fresh hold is archived");
        assert_ne!(held.session_id, "busan-game");
        assert!(held.map_name.is_empty(), "fresh hold map: {held:?}");
        assert_eq!(held.outcome, "unknown");
        store
            .apply_command(&storage::StoreCommand::SetOutcome {
                session_id: held.session_id.clone(),
                outcome: "victory".into(),
            })
            .await
            .unwrap();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen_upload = std::sync::Arc::clone(&seen);
        try_sync_with(&store, dir.path(), None, move |matches, _| {
            let seen_upload = std::sync::Arc::clone(&seen_upload);
            async move {
                *seen_upload.lock().unwrap() = matches
                    .into_iter()
                    .map(|row| (row.session_id, row.elims, row.map_name, row.outcome))
                    .collect();
                Ok(upload_ok())
            }
        })
        .await;
        let uploaded = seen.lock().unwrap().clone();
        assert!(
            uploaded.is_empty(),
            "a later GUI result still does not send a blank map: {uploaded:?}"
        );
        let kept = store
            .get_all_matches()
            .await
            .unwrap()
            .into_iter()
            .find(|row| row.elims == 3)
            .unwrap();
        assert!(!kept.synced, "the blank map stays unsynced for a pick");
        assert!(
            kept.map_name.is_empty(),
            "the fresh hold must not become Busan"
        );
        assert_eq!(kept.outcome, "victory");
    }

    #[tokio::test]
    async fn a_stale_imported_hold_keeps_the_session_outcome() {
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        let when = Utc::now() - chrono::Duration::hours(9);
        begin_from_persisted(
            dir.path(),
            &store,
            persisted_with_hold(
                "decided-session",
                detect::MatchOutcome::Victory,
                Some("Busan"),
                true,
                6,
                when,
            ),
        )
        .await;
        let held = store
            .get_all_matches()
            .await
            .unwrap()
            .into_iter()
            .find(|row| row.elims == 6)
            .expect("the imported hold is archived");
        assert_eq!(held.session_id, "decided-session");
        assert_eq!(held.map_name, "Busan");
        assert_eq!(held.outcome, "victory");
    }

    #[tokio::test]
    async fn a_resume_within_twenty_minutes_leaves_an_imported_hold_pending() {
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        let when = Utc::now() - chrono::Duration::minutes(5);
        let st = begin_from_persisted(
            dir.path(),
            &store,
            persisted_with_hold(
                "still-open",
                detect::MatchOutcome::Unknown,
                Some("Busan"),
                true,
                4,
                when,
            ),
        )
        .await;
        let held = st.active_game.as_ref().expect("the session resumes");
        assert!(held.deferred_imported);
        assert_eq!(held.deferred.map(|c| c.elims), Some(4));
        let rows = store.get_all_matches().await.unwrap();
        assert!(
            rows.iter().all(|row| row.elims != 4),
            "resume must not write the hold: {rows:?}"
        );
    }

    #[tokio::test]
    async fn a_backwards_clock_archives_an_imported_hold_with_its_persisted_map() {
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        let when = Utc::now() + chrono::Duration::hours(3);
        begin_from_persisted(
            dir.path(),
            &store,
            persisted_with_hold(
                "clock-back",
                detect::MatchOutcome::Defeat,
                Some("Nepal"),
                true,
                8,
                when,
            ),
        )
        .await;
        let held = store
            .get_all_matches()
            .await
            .unwrap()
            .into_iter()
            .find(|row| row.elims == 8)
            .expect("a backwards clock archives the hold");
        assert_eq!(held.session_id, "clock-back");
        assert_eq!(held.map_name, "Nepal");
        assert_eq!(held.outcome, "defeat");
        assert!(!active_game_path(dir.path()).exists());
    }

    #[tokio::test]
    async fn a_stat_reset_on_untrusted_guillard_stays_unknown_and_does_not_upload_as_busan() {
        let mut night = Night::new().await;
        night.begin_on("Busan", "Zenyatta");
        night
            .tab_once(
                night_counters(14, 22, 6, 2400, 9800, 400),
                "Zenyatta",
                Some(2),
                Some("Busan"),
                NIGHT_CLEAN,
            )
            .await;
        let busan = night.id();
        night.advance(Duration::from_secs(60));
        night
            .tab_once(
                night_counters(1, 3, 0, 220, 80, 400),
                "Zenyatta",
                Some(2),
                Some("Busan"),
                NIGHT_CLEAN,
            )
            .await;
        assert_eq!(
            night.id(),
            busan,
            "the first fresh board only arms the reset"
        );
        assert!(night.game().reset_streak >= 1);
        night.advance(Duration::from_secs(20));
        let guillard = parse::match_map_in_text("guillard").expect("text guillard");
        assert_eq!(guillard, "Château Guillard");
        night
            .tab_text(
                night_counters(2, 1, 0, 300, 40, 80),
                "Zenyatta",
                Some(2),
                &guillard,
                NIGHT_CLEAN,
            )
            .await;
        let opened = night.id();
        assert_ne!(
            opened, busan,
            "the second fresh board is the stat reset split"
        );
        assert!(
            night.game().map.is_none(),
            "an untrusted guillard read must not name the new session Busan, got {:?}",
            night.game().map
        );
        let snaps = night.store.get_session_snapshots(&opened).await.unwrap();
        assert!(
            !snaps.is_empty() && snaps.iter().all(|row| row.map_name.is_empty()),
            "the new session is stored with an empty map: {snaps:?}"
        );
        assert!(snaps.iter().all(|row| row.game_mode.is_empty()));
        night
            .store
            .set_session_outcome(&opened, "victory")
            .await
            .unwrap();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen_upload = std::sync::Arc::clone(&seen);
        try_sync_with(
            &night.store,
            night.dir.path(),
            night.st.settle_closed_unknown.clone(),
            move |matches, _| {
                let seen_upload = std::sync::Arc::clone(&seen_upload);
                async move {
                    *seen_upload.lock().unwrap() = matches
                        .into_iter()
                        .map(|row| (row.session_id, row.map_name, row.game_mode))
                        .collect();
                    Ok(upload_ok())
                }
            },
        )
        .await;
        let uploaded = seen.lock().unwrap().clone();
        assert!(
            uploaded.is_empty(),
            "the empty map stays local and is not sent as Busan: {uploaded:?}"
        );
        let kept = night.store.get_session_snapshots(&opened).await.unwrap();
        assert!(
            !kept.is_empty()
                && kept
                    .iter()
                    .all(|row| !row.synced && row.map_name.is_empty() && row.game_mode.is_empty()),
            "the split stays unsynced with no map until a pick: {kept:?}"
        );
    }

    #[test]
    fn an_untrusted_guillard_read_does_not_replace_the_session_map() {
        let (mut staged, mut parsed) = staged_row(
            "busan-keep",
            "Château Guillard",
            Some(boundary::MapSource::TextFallback),
        );
        let reject = prepare_capture_row(
            &mut parsed,
            &mut staged,
            Some("Busan"),
            Some("GAILLARD"),
            "player GAILLARD",
            true,
        );
        assert_eq!(reject, PreparedRow::Store);
        assert_eq!(staged.map_name, "Busan");
        assert_eq!(parsed.map_name, "Busan");
        assert_eq!(parsed.game_mode, "Control");
        assert!(staged.map_source.is_none());
    }

    #[test]
    fn the_adlersbrunn_gate_rejects_a_one_letter_misread_without_both_teams() {
        let (mut staged, mut parsed) =
            staged_row("ew", "Eichenwalde", Some(boundary::MapSource::TopBar));
        assert_eq!(
            prepare_capture_row(
                &mut parsed,
                &mut staged,
                None,
                Some("ADLERSBRUNM"),
                "",
                false,
            ),
            PreparedRow::Drop
        );
        let (mut staged, mut parsed) =
            staged_row("ew", "Eichenwalde", Some(boundary::MapSource::TopBar));
        assert_eq!(
            prepare_capture_row(
                &mut parsed,
                &mut staged,
                None,
                Some("ADLERSBRUNM"),
                "",
                true,
            ),
            PreparedRow::Store
        );
        let (mut staged, mut parsed) =
            staged_row("ew", "Eichenwalde", Some(boundary::MapSource::TopBar));
        assert_eq!(
            prepare_capture_row(
                &mut parsed,
                &mut staged,
                None,
                Some("EICHENWALDE"),
                "",
                false,
            ),
            PreparedRow::Store
        );
        let (mut staged, mut parsed) =
            staged_row("ew", "Eichenwalde", Some(boundary::MapSource::TopBar));
        assert_eq!(
            prepare_capture_row(
                &mut parsed,
                &mut staged,
                None,
                Some("EICHENWALD"),
                "",
                false,
            ),
            PreparedRow::Store,
            "a one-letter Eichenwalde misread is not an Adlersbrunn alias"
        );
    }

    // ---- M1: Adlersbrunn on the Tab banner without OCR.

    /// One Tab of the new reader with no OCR text at all (OCR off): the
    /// banner read is the only source. Returns the top-bar map and the
    /// capture row decision for a board with stats on one or both teams.
    fn adlersbrunn_tab(
        banner: Option<shadow::banner::MapRead>,
        both_teams: bool,
    ) -> (Option<String>, PreparedRow, String, String) {
        let top = choose_top_bar_map(true, || banner, None);
        let label = top_bar_raw_label(top.banner.as_ref(), None);
        let (name, source) = resolved_map(None, top.map.as_deref(), "", &[]);
        let (mut staged, mut parsed) = staged_row("m1", &name, source);
        let row = prepare_capture_row(
            &mut parsed,
            &mut staged,
            None,
            label.as_deref(),
            "",
            both_teams,
        );
        (top.map, row, parsed.map_name, parsed.game_mode)
    }

    /// The banner spelling is text ocr-v1's alias check accepts, and it is
    /// the label the tracker's map table stores as Eichenwalde.
    #[test]
    fn the_banner_spelling_of_adlersbrunn_is_an_ocr_v1_alias() {
        let label = shadow::banner::ADLERSBRUNN;
        assert!(parse::is_adlersbrunn_alias(label));
        assert_eq!(
            parse::match_map_in_text(label).as_deref(),
            Some("Eichenwalde")
        );
    }

    /// With no OCR text, a sure `ADLERSBRUNN` banner read gives what the
    /// OCR label gives under ocr-v1: the map is Eichenwalde (Hybrid), a
    /// board with stats on both teams is stored, and a board with stats on
    /// one team only (Junkenstein) is dropped.
    #[test]
    fn a_sure_adlersbrunn_banner_read_feeds_the_pve_gate_without_ocr() {
        let key = shadow::banner::ADLERSBRUNN_KEY;
        let (map, row, stored, mode) = adlersbrunn_tab(Some(banner_read(key, false)), true);
        assert_eq!(map.as_deref(), Some("Eichenwalde"));
        assert_eq!(row, PreparedRow::Store);
        assert_eq!((stored.as_str(), mode.as_str()), ("Eichenwalde", "Hybrid"));
        let (map, row, ..) = adlersbrunn_tab(Some(banner_read(key, false)), false);
        assert_eq!(map.as_deref(), Some("Eichenwalde"));
        assert_eq!(row, PreparedRow::Drop, "a Junkenstein board is dropped");

        // the same two boards through ocr-v1's label
        for (both_teams, want) in [(true, PreparedRow::Store), (false, PreparedRow::Drop)] {
            let v1 = choose_top_bar_map(false, || None, parse::match_map_in_text("ADLERSBRUNN"));
            assert_eq!(v1.map.as_deref(), Some("Eichenwalde"));
            let label = top_bar_raw_label(v1.banner.as_ref(), Some("ADLERSBRUNN".into()));
            let (mut staged, mut parsed) =
                staged_row("m1", "Eichenwalde", Some(boundary::MapSource::TopBar));
            let row = prepare_capture_row(
                &mut parsed,
                &mut staged,
                None,
                label.as_deref(),
                "",
                both_teams,
            );
            assert_eq!(row, want);
            assert_eq!(
                (parsed.map_name.as_str(), parsed.game_mode.as_str()),
                ("Eichenwalde", "Hybrid")
            );
        }
    }

    /// A real Eichenwalde banner is not the label: a board with stats on
    /// one team is stored, as ocr-v1 stores it. A flagged `ADLERSBRUNN`
    /// read names nothing and drops nothing.
    #[test]
    fn an_eichenwalde_or_flagged_banner_read_does_not_feed_the_pve_gate() {
        let (map, row, stored, _) = adlersbrunn_tab(Some(banner_read("eichenwalde", false)), false);
        assert_eq!(map.as_deref(), Some("Eichenwalde"));
        assert_eq!(row, PreparedRow::Store);
        assert_eq!(stored, "Eichenwalde");
        let key = shadow::banner::ADLERSBRUNN_KEY;
        let (map, row, stored, _) = adlersbrunn_tab(Some(banner_read(key, true)), false);
        assert_eq!(map, None, "a flagged read is no top-bar map");
        assert_eq!(row, PreparedRow::Store);
        assert_eq!(stored, "");
        let (map, row, ..) = adlersbrunn_tab(None, false);
        assert_eq!((map, row), (None, PreparedRow::Store), "no map pack");
    }

    /// Only a sure `ADLERSBRUNN` banner read replaces the OCR label. Every
    /// other read, and ocr-v1, hand the OCR label on untouched.
    #[test]
    fn only_a_sure_adlersbrunn_banner_read_becomes_the_raw_label() {
        let key = shadow::banner::ADLERSBRUNN_KEY;
        let sure = Some(banner_read(key, false));
        assert_eq!(
            top_bar_raw_label(Some(&sure), None).as_deref(),
            Some("ADLERSBRUNN")
        );
        assert_eq!(
            top_bar_raw_label(Some(&sure), Some("EICHENWALDE".into())).as_deref(),
            Some("ADLERSBRUNN"),
            "the sure banner read wins over the OCR label, as it does for the map"
        );
        let untouched = [
            Some(Some(banner_read(key, true))),
            Some(Some(banner_read("eichenwalde", false))),
            Some(Some(banner_read("ilios", false))),
            Some(None),
            None,
        ];
        for banner in &untouched {
            for ocr in [
                None,
                Some("ADLERSBRUNN"),
                Some("EICHENWALDE"),
                Some("BUSAN"),
            ] {
                assert_eq!(
                    top_bar_raw_label(banner.as_ref(), ocr.map(String::from)).as_deref(),
                    ocr,
                    "{banner:?}"
                );
            }
        }
        let mut no_banner = banner_read(key, false);
        no_banner.found_banner = false;
        assert_eq!(top_bar_raw_label(Some(&Some(no_banner)), None), None);
    }

    /// The vision pass hands the label on from the banner it already read,
    /// before the board takes that read. Whitespace is ignored.
    #[test]
    fn the_vision_pass_takes_the_raw_label_from_the_banner_read() {
        let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
        let full = include_str!("main.rs");
        let src = full.split("\nmod tests {").next().unwrap();
        let at = src.find("\nfn analyze_frame(").expect("analyze_frame");
        let body = norm(src[at..].split("\n}\n").next().unwrap());
        let call = "let map_panel_raw = top_bar_raw_label(top_bar.banner.as_ref(), map_panel_raw);";
        assert_eq!(body.matches(call).count(), 1);
        let chosen = body.find("let top_bar = choose_top_bar_map(").unwrap();
        let built = body.find("FrameAnalysis {").unwrap();
        let at = body.find(call).unwrap();
        assert!(chosen < at && at < built);
        assert!(body[at..].contains(" map_panel_raw, "));
    }

    #[tokio::test]
    async fn a_trusted_guillard_row_is_kept_locally_and_not_uploaded() {
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        let (mut staged, mut parsed) = staged_row(
            "dm-keep",
            "Château Guillard",
            Some(boundary::MapSource::TopBar),
        );
        assert_eq!(
            prepare_capture_row(
                &mut parsed,
                &mut staged,
                None,
                Some("CHATEAU GUILLARD"),
                "",
                true,
            ),
            PreparedRow::Store
        );
        assert_eq!(parsed.map_name, "Château Guillard");
        assert_eq!(parsed.game_mode, "Deathmatch");
        commit_capture_rows(
            &store,
            dir.path(),
            &staged,
            &parsed,
            SurrealDatetime::from(Utc::now()),
        )
        .await
        .unwrap();
        let rows = store.get_session_snapshots("dm-keep").await.unwrap();
        assert_eq!(rows.len(), 1);
        let called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let called_upload = std::sync::Arc::clone(&called);
        try_sync_with(&store, dir.path(), None, move |_matches, _| {
            let called_upload = std::sync::Arc::clone(&called_upload);
            async move {
                called_upload.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(upload_ok())
            }
        })
        .await;
        assert!(!called.load(std::sync::atomic::Ordering::SeqCst));
        let rows = store.get_session_snapshots("dm-keep").await.unwrap();
        assert!(rows.iter().all(|row| row.synced));
    }

    #[tokio::test]
    async fn a_deathmatch_row_stays_local_when_the_new_reader_names_another_map() {
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        let (mut staged, mut parsed) = staged_row(
            "dm-new",
            "Château Guillard",
            Some(boundary::MapSource::TopBar),
        );
        assert_eq!(
            prepare_capture_row(
                &mut parsed,
                &mut staged,
                None,
                Some("CHATEAU GUILLARD"),
                "",
                true,
            ),
            PreparedRow::Store
        );
        assert_eq!(parsed.game_mode, "Deathmatch");
        let board = {
            let mut fields = vec![
                shadow::FieldRead {
                    name: "map".into(),
                    value: Some(shadow::Value::Text("Ilios".into())),
                    confidence: 0.95,
                    suspect: false,
                },
                shadow::FieldRead {
                    name: "mode".into(),
                    value: Some(shadow::Value::Text("Control".into())),
                    confidence: 0.95,
                    suspect: false,
                },
            ];
            fields.push(shadow::FieldRead {
                name: "r0.hero".into(),
                value: Some(shadow::Value::Text("Kiriko".into())),
                confidence: 0.95,
                suspect: false,
            });
            for (suffix, value) in [
                ("e", 21u32),
                ("a", 9),
                ("d", 3),
                ("dmg", 9100),
                ("h", 1200),
                ("mit", 50),
            ] {
                fields.push(shadow::FieldRead {
                    name: format!("r0.{suffix}"),
                    value: Some(shadow::Value::Int(value)),
                    confidence: 0.95,
                    suspect: false,
                });
            }
            shadow::BoardRead {
                status: shadow::BoardStatus::Read,
                team_size: Some(5),
                fields,
                elapsed_ms: 1,
            }
        };
        let saved = reader_apply::merge_saved(
            &reader_apply::PolicyRow {
                map: parsed.map_name.clone(),
                mode: parsed.game_mode.clone(),
                result: parsed.outcome.clone(),
                hero: parsed.hero.clone(),
                elims: parsed.elims,
                assists: parsed.assists,
                deaths: parsed.deaths,
                damage: parsed.damage,
                healing: parsed.healing,
                mitigation: parsed.mitigation,
            },
            reader_apply::OwnRow::Identified {
                index: 0,
                team_size: 5,
            },
            Some(&board),
        );
        parsed.map_name = saved.map;
        parsed.game_mode = saved.mode;
        parsed.hero = saved.hero;
        parsed.elims = saved.elims;
        parsed.recognizer = saved.recognizer.to_string();
        parsed.suspect_fields = saved.suspect_fields;
        assert_eq!(parsed.map_name, "Château Guillard");
        assert_eq!(parsed.game_mode, "Deathmatch");
        assert!(parsed.suspect_fields.iter().any(|name| name == "map"));
        commit_capture_rows(
            &store,
            dir.path(),
            &staged,
            &parsed,
            SurrealDatetime::from(Utc::now()),
        )
        .await
        .unwrap();
        let called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let called_upload = std::sync::Arc::clone(&called);
        try_sync_with(&store, dir.path(), None, move |_matches, _| {
            let called_upload = std::sync::Arc::clone(&called_upload);
            async move {
                called_upload.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(upload_ok())
            }
        })
        .await;
        assert!(
            !called.load(std::sync::atomic::Ordering::SeqCst),
            "reader = new must not upload a Deathmatch row"
        );
    }

    #[tokio::test]
    async fn practice_range_never_uploads_and_is_not_held_for_review() {
        let name = parse::canonical_map("PRACTICE RANGE").expect("practice range");
        assert_eq!(name, "Practice Range");
        assert_eq!(parse::stored_game_mode(&name), "Practice");

        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        let (mut staged, mut parsed) =
            staged_row("practice", &name, Some(boundary::MapSource::TopBar));
        assert_eq!(
            prepare_capture_row(
                &mut parsed,
                &mut staged,
                None,
                Some("PRACTICE RANGE"),
                "",
                true,
            ),
            PreparedRow::Store
        );
        assert_eq!(parsed.map_name, "Practice Range");
        assert_eq!(parsed.game_mode, "Practice");
        assert!(
            parse::review_suspect_fields(&parsed.map_name, &parsed.game_mode, &parsed.hero)
                .is_empty(),
            "a known practice map with a mode is not a blank-map review hold"
        );
        commit_capture_rows(
            &store,
            dir.path(),
            &staged,
            &parsed,
            SurrealDatetime::from(Utc::now()),
        )
        .await
        .unwrap();

        // No open map: do not store a blank row. A blank map uploads, and
        // the review hold would keep it until someone picked a map.
        let (mut fuzzy, mut fuzzy_parsed) = staged_row(
            "practice-fuzzy",
            &name,
            Some(boundary::MapSource::TextFallback),
        );
        assert_eq!(
            prepare_capture_row(
                &mut fuzzy_parsed,
                &mut fuzzy,
                None,
                Some("PRACTICE"),
                "",
                true
            ),
            PreparedRow::Drop
        );
        assert!(fuzzy.map_name.is_empty());

        // An open game keeps its map. The practice misread must not replace it.
        let (mut open, mut open_parsed) =
            staged_row("busan-open", &name, Some(boundary::MapSource::TextFallback));
        assert_eq!(
            prepare_capture_row(
                &mut open_parsed,
                &mut open,
                Some("Busan"),
                Some("PRACTICE RANGE"),
                "",
                true,
            ),
            PreparedRow::Store
        );
        assert_eq!(open_parsed.map_name, "Busan");
        assert_eq!(open_parsed.game_mode, "Control");
        commit_capture_rows(
            &store,
            dir.path(),
            &open,
            &open_parsed,
            SurrealDatetime::from(Utc::now()),
        )
        .await
        .unwrap();

        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen_upload = std::sync::Arc::clone(&seen);
        try_sync_with(&store, dir.path(), None, move |matches, _| {
            let seen_upload = std::sync::Arc::clone(&seen_upload);
            async move {
                *seen_upload.lock().unwrap() = matches
                    .into_iter()
                    .map(|row| (row.session_id, row.map_name, row.game_mode))
                    .collect();
                Ok(upload_ok())
            }
        })
        .await;
        let uploaded = seen.lock().unwrap().clone();
        assert_eq!(
            uploaded,
            vec![("busan-open".into(), "Busan".into(), "Control".into())]
        );
        assert!(
            uploaded
                .iter()
                .all(|(_, map, mode)| map != "Practice Range" && mode != "Practice")
        );

        let practice = store.get_session_snapshots("practice").await.unwrap();
        assert_eq!(practice.len(), 1);
        assert!(
            practice[0].synced,
            "practice is kept locally, not left unsynced"
        );
        assert!(practice[0].upload_rejection().is_none());
        assert!(
            parse::review_suspect_fields(
                practice[0].display_map_name(),
                &practice[0].game_mode,
                practice[0].display_hero(),
            )
            .is_empty()
        );
        assert!(
            store
                .get_session_snapshots("practice-fuzzy")
                .await
                .unwrap()
                .is_empty(),
            "an untrusted practice read is not stored as a blank map"
        );
        assert!(store.get_unsynced().await.unwrap().is_empty());

        let again = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let again_upload = std::sync::Arc::clone(&again);
        let second = try_sync_with(&store, dir.path(), None, move |_, _| {
            let again_upload = std::sync::Arc::clone(&again_upload);
            async move {
                again_upload.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(upload_ok())
            }
        })
        .await;
        assert!(matches!(second, sync::SyncAttempt::NoServerCall));
        assert!(!again.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test]
    async fn an_untrusted_guillard_report_does_not_rename_an_open_game() {
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        let now = test_now();
        let mut g = game(detect::MatchOutcome::Unknown, None, now);
        g.session_id = "busan-open".into();
        g.map = Some("Busan".into());
        g.map_source = Some(boundary::MapSource::TopBar);
        let mut st = session(Some(g), now);
        let mut report = rejected_report("busan-open");
        report.map = Some("Château Guillard".into());
        report.map_source = Some(boundary::MapSource::TextFallback);
        apply_capture_report(&mut st, &store, dir.path(), "busan-open", Ok(report), now).await;
        assert_eq!(
            st.active_game.as_ref().and_then(|g| g.map.as_deref()),
            Some("Busan")
        );

        let mut empty = game(detect::MatchOutcome::Unknown, None, now);
        empty.session_id = "no-map".into();
        empty.map = None;
        let mut st = session(Some(empty), now);
        let mut fuzzy = rejected_report("no-map");
        fuzzy.map = Some("Château Guillard".into());
        fuzzy.map_source = Some(boundary::MapSource::TextFallback);
        apply_capture_report(&mut st, &store, dir.path(), "no-map", Ok(fuzzy), now).await;
        assert!(st.active_game.as_ref().unwrap().map.is_none());

        let mut fresh = game(detect::MatchOutcome::Unknown, None, now);
        fresh.session_id = "dm-note".into();
        fresh.map = None;
        let mut st = session(Some(fresh), now);
        for _ in 0..2 {
            let mut noted = rejected_report("dm-note");
            noted.map = Some("Château Guillard".into());
            noted.map_source = Some(boundary::MapSource::Accolade);
            apply_capture_report(&mut st, &store, dir.path(), "dm-note", Ok(noted), now).await;
        }
        assert_eq!(
            st.active_game.as_ref().and_then(|g| g.map.as_deref()),
            Some("Château Guillard")
        );
        let text = std::fs::read_to_string(dir.path().join("debug").join("unrecorded_games.jsonl"))
            .unwrap();
        assert_eq!(
            text.lines().filter(|line| line.contains("dm-note")).count(),
            1
        );
    }

    #[tokio::test]
    async fn commit_capture_rows_writes_the_current_row_then_the_carried_board() {
        let src = include_str!("main.rs");
        let at = src
            .find("async fn commit_capture_rows(")
            .expect("commit helper");
        let body = src[at..]
            .split("\nasync fn handle_capture(")
            .next()
            .unwrap();
        let insert = body.find("insert_match(").expect("insert");
        let carried = body
            .find("write_carried_after_current(")
            .expect("carried write");
        assert!(
            insert < carried,
            "the carried board is written after the insert"
        );
        assert!(body.contains("should_write_carried(true,"));

        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        let (mut staged, mut parsed) =
            staged_row("carry-order", "Busan", Some(boundary::MapSource::TopBar));
        assert_eq!(
            prepare_capture_row(
                &mut parsed,
                &mut staged,
                Some("Busan"),
                Some("BUSAN"),
                "",
                true,
            ),
            PreparedRow::Store
        );
        let current = Utc::now();
        staged.carried = Some(fresh_counters(2));
        staged.carried_hero = Some("Tracer".into());
        staged.carried_at = current - chrono::Duration::seconds(40);
        parsed.played_at = SurrealDatetime::from(current);
        parsed.elims = 14;
        commit_capture_rows(
            &store,
            dir.path(),
            &staged,
            &parsed,
            SurrealDatetime::from(current),
        )
        .await
        .unwrap();
        let rows = store.get_session_snapshots("carry-order").await.unwrap();
        assert!(
            rows.iter()
                .any(|row| row.elims == 14 && row.game_mode == "Control")
        );
        assert!(
            rows.iter()
                .any(|row| row.elims == 2 && row.hero == "Tracer")
        );
        store.export_snapshot(dir.path()).await.unwrap();
        let snap = storage::read_snapshot(dir.path()).unwrap();
        let session = snap
            .sessions
            .iter()
            .find(|session| session.session_id == "carry-order")
            .unwrap();
        let stamp: chrono::DateTime<Utc> = session.last_capture_at.into();
        assert!(
            stamp + chrono::Duration::seconds(2) >= current,
            "a carried board must not rewind last_capture_at: {stamp} vs {current}"
        );
    }

    #[tokio::test]
    async fn append_capture_keeps_the_newer_session_stamp() {
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        let current = Utc::now();
        store
            .create_session(&storage::MatchSession {
                session_id: "stamp".into(),
                hero: "Ana".into(),
                map_name: "Busan".into(),
                role: "Support".into(),
                started_at: SurrealDatetime::from(current),
                last_capture_at: SurrealDatetime::from(current),
                capture_count: 1,
                final_outcome: "victory".into(),
            })
            .await
            .unwrap();
        store
            .append_capture(
                "stamp",
                SurrealDatetime::from(current - chrono::Duration::minutes(5)),
                "victory",
            )
            .await
            .unwrap();
        store.export_snapshot(dir.path()).await.unwrap();
        let snap = storage::read_snapshot(dir.path()).unwrap();
        let stamp: chrono::DateTime<Utc> = snap.sessions[0].last_capture_at.into();
        assert!(stamp + chrono::Duration::seconds(2) >= current);
    }

    #[tokio::test]
    async fn set_session_map_updates_game_mode() {
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        let mut row = test_match("mode-fix", "victory");
        row.map_name = "Dorado".into();
        row.game_mode = "Escort".into();
        store.insert_match(row).await.unwrap();
        store.set_session_map("mode-fix", "Busan").await.unwrap();
        let rows = store.get_session_snapshots("mode-fix").await.unwrap();
        assert_eq!(rows[0].map_name, "Busan");
        assert_eq!(rows[0].game_mode, "Control");
        assert_eq!(
            parse::uploaded_game_mode(rows[0].display_map_name(), &rows[0].game_mode),
            "Control"
        );
    }

    #[tokio::test]
    async fn end_screen_split_log_uses_the_open_session_at_the_call_site() {
        LAST_END_SCREEN_SPLIT.with(|slot| slot.set(None));
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        let now = test_now();
        let mut g = game(detect::MatchOutcome::Unknown, None, now);
        g.pending_boundary = true;
        g.map = Some("Busan".into());
        g.map_source = Some(boundary::MapSource::TopBar);
        let mut st = session(Some(g), now);
        apply_poll_decision(
            &mut st,
            &store,
            dir.path(),
            boundary::PollDecision::Open(boundary::OpenNew {
                reason: boundary::CloseReason::EndScreenMap,
                seal_outcome: Some(detect::MatchOutcome::Victory),
                new_outcome: detect::MatchOutcome::Unknown,
                new_map: Some("Ilios".into()),
                candidates: Vec::new(),
                new_result: None,
            }),
            now,
            false,
        )
        .await;
        let logged = LAST_END_SCREEN_SPLIT.with(|slot| slot.take()).unwrap();
        assert!(logged.contains("armed=true"), "{logged}");
        assert!(logged.contains("accolade_map=Ilios"), "{logged}");
    }

    #[tokio::test]
    async fn set_outcome_on_the_command_tick_uploads_the_open_session() {
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        store
            .insert_match(unfinished_row("open-cmd"))
            .await
            .unwrap();
        let now = test_now();
        let mut g = game(detect::MatchOutcome::Unknown, None, now);
        g.session_id = "open-cmd".into();
        g.map = Some("Busan".into());
        let mut st = session(Some(g), now);
        storage::queue_command(
            dir.path(),
            &storage::StoreCommand::SetOutcome {
                session_id: "open-cmd".into(),
                outcome: "victory".into(),
            },
        )
        .unwrap();
        let (port, rx) = spawn_upload_sink();
        let client = loopback_client(port);
        let backoff = std::sync::Arc::new(std::sync::Mutex::new(sync::SyncBackoff::default()));
        let mut sync_task = None;
        apply_queued_commands(
            &mut st,
            &store,
            dir.path(),
            &mut sync_task,
            &backoff,
            Some(&client),
        )
        .await;
        assert!(st.active_game.as_ref().unwrap().finished());
        let handle = sync_task.expect("SetOutcome schedules an upload");
        handle.await.unwrap();
        let body = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert!(body.contains("open-cmd"), "{body}");
        assert!(body.contains("victory"), "{body}");
        assert!(body.contains("\"edited\":true"), "{body}");
    }

    #[tokio::test]
    async fn a_delete_command_does_not_schedule_an_upload() {
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        store
            .insert_match(test_match("do-not-upload", "victory"))
            .await
            .unwrap();
        let now = test_now();
        let mut st = session(None, now);
        storage::queue_command(
            dir.path(),
            &storage::StoreCommand::DeleteSession {
                session_id: "do-not-upload".into(),
            },
        )
        .unwrap();
        let client = loopback_client(9);
        let backoff = std::sync::Arc::new(std::sync::Mutex::new(sync::SyncBackoff::default()));
        let mut sync_task = None;
        apply_queued_commands(
            &mut st,
            &store,
            dir.path(),
            &mut sync_task,
            &backoff,
            Some(&client),
        )
        .await;
        assert!(sync_task.is_none());
    }

    #[tokio::test]
    async fn schedule_store_sync_joins_the_previous_task() {
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = std::sync::Arc::clone(&done);
        let previous = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        let backoff = std::sync::Arc::new(std::sync::Mutex::new(sync::SyncBackoff::default()));
        let mut sync_task = Some(previous);
        schedule_store_sync(&mut sync_task, &backoff, &store, None, dir.path(), None);
        tokio::time::sleep(std::time::Duration::from_millis(40)).await;
        assert!(
            !sync_task.as_ref().unwrap().is_finished(),
            "the new upload must still be joining the previous task"
        );
        sync_task.take().unwrap().await.unwrap();
        assert!(done.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test]
    async fn scheduled_sync_feeds_the_backoff_clock() {
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        let mut row = test_match("busan-fail", "victory");
        row.map_name = "Busan".into();
        row.game_mode = "Control".into();
        store.insert_match(row).await.unwrap();
        let client = loopback_client(9);
        let backoff = std::sync::Arc::new(std::sync::Mutex::new(sync::SyncBackoff::default()));
        let mut sync_task = None;
        schedule_store_sync(
            &mut sync_task,
            &backoff,
            &store,
            Some(&client),
            dir.path(),
            None,
        );
        sync_task.take().unwrap().await.unwrap();
        assert!(backoff.lock().unwrap().failures() >= 1);
    }

    #[tokio::test]
    async fn command_tick_settles_only_the_closed_session() {
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        store
            .insert_match(unfinished_row("closed-one"))
            .await
            .unwrap();
        store
            .insert_match(unfinished_row("still-open"))
            .await
            .unwrap();
        let now = test_now();
        let mut st = session(None, now);
        st.settle_closed_unknown = Some("closed-one".into());
        let client = loopback_client(9);
        let backoff = std::sync::Arc::new(std::sync::Mutex::new(sync::SyncBackoff::default()));
        let mut sync_task = None;
        schedule_settled_unknown(
            &mut st,
            &mut sync_task,
            &backoff,
            &store,
            Some(&client),
            dir.path(),
        );
        assert!(st.settle_closed_unknown.is_none());
        sync_task.take().unwrap().await.unwrap();
        let rows = store.get_all_matches().await.unwrap();
        assert!(
            rows.iter()
                .any(|row| row.session_id == "closed-one" && row.synced)
        );
        assert!(
            rows.iter()
                .any(|row| row.session_id == "still-open" && !row.synced)
        );
    }

    #[tokio::test]
    async fn an_unfinished_game_closes_after_six_hours() {
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        store
            .insert_match(unfinished_row("long-busan"))
            .await
            .unwrap();
        let now = test_now();
        let mut g = game(detect::MatchOutcome::Unknown, None, now);
        g.session_id = "long-busan".into();
        g.map = Some("Busan".into());
        g.last_activity = now - UNFINISHED_SESSION_MAX;
        let mut st = session(Some(g), now);
        let closed = close_quiet_session_and_sync(
            &mut st,
            &store,
            dir.path(),
            now,
            finished_game_close_after(config::FINISHED_GAME_CLOSE_DEFAULT_SECS),
            || async {},
        )
        .await;
        assert!(closed);
        assert!(st.active_game.is_none());
        assert_eq!(st.settle_closed_unknown.as_deref(), Some("long-busan"));
    }

    // ---- O10: cell trust (OCR sites T3, T7, T2) under each reader.

    /// A row OCR that must not run.
    fn o10_never() -> Vec<ocr::RowOcrResult> {
        panic!("this read must not run")
    }

    #[test]
    fn the_new_reader_reads_no_stat_cell_and_calibrates_nothing() {
        let named = || {
            vec![ocr::RowOcrResult {
                name: None,
                name_alternates: Vec::new(),
                stats: Vec::new(),
                mean_confidence: 0,
            }]
        };
        // new: the cell pass (calibration, then every stat cell) never runs
        let rows = read_tab_rows(true, o10_never, named);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].stats.is_empty());
        // ocr-v1: the cell pass runs, the name-only pass does not
        let rows = read_tab_rows(false, o8_rows, o10_never);
        assert_eq!(rows.len(), 10);
        assert_eq!(rows[0].stats.len(), 6);
    }

    #[test]
    fn the_new_reader_never_runs_the_full_board_ocr() {
        type Text = Result<ocr::OcrResult, Box<dyn std::error::Error + Send + Sync>>;
        let text = |raw: &str| -> Text {
            Ok(ocr::OcrResult {
                raw_text: raw.into(),
                confidence: 77,
            })
        };
        // new: neither the "is it needed" check nor the OCR runs, even on a
        // Tab where ocr-v1 would need it
        let read = full_board_text(
            true,
            || panic!("needed() must not run"),
            || panic!("the full-board OCR must not run"),
        )
        .unwrap();
        assert_eq!((read.raw_text.as_str(), read.confidence), ("", 0));
        // ocr-v1, needed: the OCR text, or its error, comes back as it is
        let read = full_board_text(false, || true, || text("~ Defeat")).unwrap();
        assert_eq!((read.raw_text.as_str(), read.confidence), ("~ Defeat", 77));
        let failed = full_board_text(false, || true, || -> Text { Err("tesseract".into()) });
        assert_eq!(failed.unwrap_err().to_string(), "tesseract");
        // ocr-v1, not needed: skipped, the empty text
        let read = full_board_text(false, || false, || panic!("not needed")).unwrap();
        assert_eq!((read.raw_text.as_str(), read.confidence), ("", 0));
    }

    #[test]
    fn the_board_is_read_only_under_the_new_reader() {
        assert_eq!(
            read_tab_board(false, || panic!("ocr-v1 reads no board")),
            None
        );
        let board = read_tab_board(true, || o8_board(None)).unwrap();
        assert_eq!(board.status, shadow::BoardStatus::Read);
        // a panic in the reader is "no board", not a failed Tab
        let board = read_tab_board(true, || panic!("board reader")).unwrap();
        assert_eq!(board.status, shadow::BoardStatus::NotFound);
        assert_eq!(board.team_size, None);
    }

    /// OCR sites T2, T3 and T7 run only through the reader switch, and the
    /// trust gate and the stats take the board under the new reader.
    /// Whitespace is ignored; hard-wiring a switch or a second OCR call
    /// breaks this.
    #[test]
    fn the_cell_ocr_sites_go_through_the_reader_switch() {
        let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
        let src = include_str!("main.rs");
        let at = src.find("\nfn analyze_frame(").expect("analyze_frame");
        let body = norm(src[at..].split("\n}\n").next().unwrap());
        assert!(body.contains(
            "let rows = read_tab_rows( use_new_reader, || ocr::recognize_scoreboard_cells_pre_cropped(&scoreboard, team_size), || ocr::unread_rows(team_size), );"
        ));
        assert_eq!(body.matches("recognize_scoreboard_cells").count(), 1);
        assert!(body.contains("let ocr = full_board_text( use_new_reader, || {"));
        assert!(body.contains("|| ocr::recognize(&img), );"));
        assert_eq!(body.matches("ocr::recognize(").count(), 1);
        let hc = src
            .find("\nasync fn handle_capture(")
            .expect("handle_capture");
        let hc_body = norm(src[hc..].split("\n}\n").next().unwrap());
        assert!(hc_body.contains(
            "let looks_like_scoreboard = match &new_board { Some(_) => board_refusal.is_none(), None => parse::looks_like_scoreboard(&rows), };"
        ));
        assert!(hc_body.contains(
            "let scoreboard_read = match &new_board { Some(board) => reader_apply::read_own_row_stats(board, board_row, &outcome_label) .map(|read| (read.matched, true, read.held)), None => parse::read_scoreboard("
        ));
        assert_eq!(hc_body.matches("parse::read_scoreboard(").count(), 1);
        assert!(hc_body.contains(
            "let suspect = if ctx.use_new_reader { held_cols } else { parse::player_row_suspect_mask(&rows, player_row_idx) };"
        ));
        assert!(hc_body.contains(
            "match &new_board { Some(board) => reader_apply::both_teams_have_stats(board, team_size), None => parse::both_teams_have_stats(&rows, team_size), },"
        ));
        assert!(hc_body.contains("let new_board = ctx.use_new_reader.then(|| {"));
    }

    /// `o8_board` with the own row's stats as given: `(value, suspect)` per
    /// column, `None` for a cell with no number.
    fn o10_board(cells: [Option<(u32, bool)>; 6]) -> shadow::BoardRead {
        let mut board = o8_board(Some("ana"));
        for (suffix, cell) in reader_apply::STAT_SUFFIXES.iter().zip(cells) {
            let name = format!("r0.{suffix}");
            let field = board.fields.iter_mut().find(|f| f.name == name).unwrap();
            field.value = cell.map(|(value, _)| shadow::Value::Int(value));
            field.suspect = cell.is_none_or(|(_, suspect)| suspect);
            field.confidence = if field.suspect { 0.2 } else { 0.9 };
        }
        board
    }

    /// Run Tabs through `handle_capture` on one open game. Returns the
    /// stored rows and, per Tab, whether it was recorded.
    async fn o10_tabs(
        ctx: &DaemonCtx,
        tabs: Vec<(FrameAnalysis, Option<shadow::BoardRead>)>,
    ) -> (Vec<storage::PersonalMatch>, Vec<bool>) {
        o10_tabs_on(ctx, tabs, "Busan").await
    }

    /// [`o10_tabs`] on a game whose trusted map is `map`.
    async fn o10_tabs_on(
        ctx: &DaemonCtx,
        tabs: Vec<(FrameAnalysis, Option<shadow::BoardRead>)>,
        map: &str,
    ) -> (Vec<storage::PersonalMatch>, Vec<bool>) {
        let mut now = test_now();
        let mut active = game(detect::MatchOutcome::Unknown, None, now);
        active.session_id = "o10".into();
        active.map = Some(map.into());
        active.map_source = Some(boundary::MapSource::TopBar);
        active.session_created = false;
        let mut st = session(Some(active), now);
        let mut recorded = Vec::new();
        for (i, (analysis, board)) in tabs.into_iter().enumerate() {
            *INJECTED_CAPTURE.lock().unwrap() = Some(InjectedCapture { analysis, board });
            let req = build_capture_request(st.active_game.as_ref().unwrap(), false, now);
            let report = handle_capture(ctx, req).await.unwrap();
            assert!(!report.split, "tab {i} must stay on this game");
            recorded.push(report.recorded);
            apply_capture_report(&mut st, &ctx.store, &ctx.data_dir, "o10", Ok(report), now).await;
            now += Duration::from_secs(20);
        }
        let rows = ctx.store.get_session_snapshots("o10").await.unwrap();
        (rows, recorded)
    }

    fn o10_stats(row: &storage::PersonalMatch) -> [u32; 6] {
        [
            row.elims,
            row.assists,
            row.deaths,
            row.damage,
            row.healing,
            row.mitigation,
        ]
    }

    fn sure(value: u32) -> Option<(u32, bool)> {
        Some((value, false))
    }

    /// The whole point of O10: a cell the digit reader does not vouch for is
    /// held at the game's last accepted value. The OCR cells of the same Tab
    /// (`o8_rows`: 10 4 2 3000 800 400) are never the number stored.
    #[tokio::test]
    async fn new_reader_holds_a_suspect_cell_and_never_takes_the_ocr_cell() {
        let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let ctx = o8_ctx(dir.path(), true).await;
        let tabs = vec![
            // all six sure
            (
                o8_analysis(None, Some("ana")),
                Some(o10_board([
                    sure(12),
                    sure(5),
                    sure(3),
                    sure(3500),
                    sure(700),
                    sure(300),
                ])),
            ),
            // healing suspect (a plausible 750), mitigation with no number
            (
                o8_analysis(None, Some("ana")),
                Some(o10_board([
                    sure(13),
                    sure(5),
                    sure(3),
                    sure(3700),
                    Some((750, true)),
                    None,
                ])),
            ),
            // both read sure again
            (
                o8_analysis(None, Some("ana")),
                Some(o10_board([
                    sure(13),
                    sure(6),
                    sure(3),
                    sure(3900),
                    sure(1000),
                    sure(500),
                ])),
            ),
        ];
        let (rows, recorded) = o10_tabs(&ctx, tabs).await;
        assert_eq!(recorded, vec![true, true, true]);
        assert_eq!(o10_stats(&rows[0]), [12, 5, 3, 3500, 700, 300]);
        assert!(
            rows[0].suspect_fields.is_empty(),
            "{:?}",
            rows[0].suspect_fields
        );
        // held at the last accepted 700 and 300: not the suspect 750, not
        // OCR's 800 and 400 (plausible growth, so no other hold would stop
        // them), and not a 0 written over a good value
        assert_eq!(o10_stats(&rows[1]), [13, 5, 3, 3700, 700, 300]);
        assert_eq!(
            rows[1].suspect_fields,
            vec!["h".to_string(), "mit".to_string()]
        );
        assert_eq!(rows[1].recognizer, shadow::digits::RECOGNIZER_ID);
        assert_eq!(o10_stats(&rows[2]), [13, 6, 3, 3900, 1000, 500]);
        assert!(
            rows[2].suspect_fields.is_empty(),
            "{:?}",
            rows[2].suspect_fields
        );
    }

    /// A Tab the new reader's board does not vouch for stores nothing, even
    /// when ocr-v1's cells and text read it perfectly. ocr-v1 itself, on the
    /// same Tabs, stores its own read and never looks at the board.
    #[tokio::test]
    async fn new_reader_refuses_a_tab_its_board_does_not_vouch_for() {
        let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
        let not_found = || shadow::reader::not_found(shadow::BoardStatus::NotFound);
        let unsure_row = || {
            o10_board([
                Some((10, true)),
                Some((4, true)),
                Some((2, true)),
                Some((3000, true)),
                None,
                None,
            ])
        };
        let six_v_six = || {
            let mut board = o8_board(Some("ana"));
            board.team_size = Some(6);
            board
        };
        let tabs = || {
            vec![
                (o8_analysis(None, Some("ana")), Some(not_found())),
                (o8_analysis(None, Some("ana")), None),
                (o8_analysis(None, Some("ana")), Some(unsure_row())),
                (o8_analysis(None, Some("ana")), Some(six_v_six())),
            ]
        };
        let dir = tempfile::tempdir().unwrap();
        let ctx = o8_ctx(dir.path(), true).await;
        let (rows, recorded) = o10_tabs(&ctx, tabs()).await;
        assert_eq!(recorded, vec![false, false, false, false]);
        assert!(rows.is_empty(), "nothing is written: {}", rows.len());

        let dir = tempfile::tempdir().unwrap();
        let ctx = o8_ctx(dir.path(), false).await;
        let (rows, recorded) = o10_tabs(&ctx, tabs()).await;
        assert_eq!(recorded, vec![true, true, true, true]);
        for row in &rows {
            assert_eq!(o10_stats(row), [10, 4, 2, 3000, 800, 400]);
            assert_eq!(row.recognizer, scuffed_types::RECOGNIZER_OCR_V1);
            assert!(row.suspect_fields.is_empty());
        }
    }

    /// Under the new reader the Tab's name-only rows have no stat cell, and
    /// the board alone carries the capture: acceptance, stats, both teams.
    #[tokio::test]
    async fn new_reader_stores_from_the_board_with_no_ocr_cells_or_text() {
        let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let ctx = o8_ctx(dir.path(), true).await;
        // what analyze_frame hands over under the new reader
        let mut analysis = o8_analysis(None, Some("ana"));
        analysis.rows = ocr::unread_rows(5);
        analysis.ocr = Ok(ocr::OcrResult {
            raw_text: String::new(),
            confidence: 0,
        });
        analysis.board = Some(o10_board([
            sure(12),
            sure(5),
            sure(3),
            sure(3500),
            sure(900),
            sure(450),
        ]));
        let (rows, recorded) = o10_tabs(&ctx, vec![(analysis, None)]).await;
        assert_eq!(recorded, vec![true]);
        assert_eq!(o10_stats(&rows[0]), [12, 5, 3, 3500, 900, 450]);
        assert_eq!(rows[0].recognizer, shadow::digits::RECOGNIZER_ID);
        assert_eq!(rows[0].hero, "Ana");
        assert_eq!(rows[0].map_name, "Busan");
        // the same Tab under ocr-v1 has nothing to read
        let dir = tempfile::tempdir().unwrap();
        let ctx = o8_ctx(dir.path(), false).await;
        let mut analysis = o8_analysis(None, Some("ana"));
        analysis.rows = ocr::unread_rows(5);
        let (rows, recorded) = o10_tabs(&ctx, vec![(analysis, None)]).await;
        assert_eq!(recorded, vec![false]);
        assert!(rows.is_empty());
    }

    /// The Junkenstein gate (an Adlersbrunn label with stats on one team only
    /// is not stored as Eichenwalde) counts the teams on the board under the
    /// new reader: its Tab rows have no stat cell to count.
    #[tokio::test]
    async fn new_reader_counts_both_teams_on_the_board_for_the_pve_gate() {
        let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
        let tab = |both_teams: bool| {
            let mut analysis = o8_analysis(None, Some("ana"));
            analysis.rows = ocr::unread_rows(5);
            analysis.map_from_panel = Some("Eichenwalde".into());
            analysis.map_panel_raw = Some("ADLERSBRUNN".into());
            let mut board = o8_board(Some("ana"));
            if both_teams {
                for suffix in reader_apply::STAT_SUFFIXES {
                    board.fields.push(shadow::FieldRead {
                        name: format!("r7.{suffix}"),
                        value: Some(shadow::Value::Int(3)),
                        confidence: 0.9,
                        suspect: false,
                    });
                }
            }
            (analysis, Some(board))
        };
        let dir = tempfile::tempdir().unwrap();
        let ctx = o8_ctx(dir.path(), true).await;
        let (rows, recorded) = o10_tabs_on(&ctx, vec![tab(true)], "Eichenwalde").await;
        assert_eq!(
            recorded,
            vec![true],
            "a board with both teams is a real game"
        );
        assert_eq!(rows[0].map_name, "Eichenwalde");
        let dir = tempfile::tempdir().unwrap();
        let ctx = o8_ctx(dir.path(), true).await;
        let (rows, recorded) = o10_tabs_on(&ctx, vec![tab(false)], "Eichenwalde").await;
        assert_eq!(recorded, vec![false], "one team only: the PvE board");
        assert!(rows.is_empty());
    }

    #[test]
    fn fill_held_cells_puts_the_last_accepted_value_in_held_columns_only() {
        let mut parsed = test_match("s", "unknown");
        (parsed.elims, parsed.assists, parsed.deaths) = (13, 5, 3);
        (parsed.damage, parsed.healing, parsed.mitigation) = (3700, 0, 0);
        let held = [false, false, false, false, true, true];
        let prior = capture_gate::Counters {
            elims: 12,
            assists: 5,
            deaths: 3,
            damage: 3500,
            healing: 900,
            mitigation: 450,
        };
        fill_held_cells(&mut parsed, held, Some(prior));
        assert_eq!(o10_stats(&parsed), [13, 5, 3, 3700, 900, 450]);
        // no earlier board: 0, and a sure column is never touched
        fill_held_cells(&mut parsed, [true, false, false, false, true, false], None);
        assert_eq!(o10_stats(&parsed), [0, 5, 3, 3700, 0, 450]);
    }

    /// The own row's six stat cells for [`o10_board`]: `(value, suspect)`,
    /// `None` for a cell with no number.
    type O10Cells = [Option<(u32, bool)>; 6];

    /// What one Tab of [`o10_run`] did.
    #[derive(Debug, Clone, PartialEq)]
    struct O10Tab {
        recorded: bool,
        /// Read and gated, no row yet (`CaptureReport::pending`).
        pending: bool,
        split: bool,
        /// Held off the game as a first fresh-match board.
        deferred: bool,
        /// The board became the reset baseline.
        refresh_baseline: bool,
        /// The session the Tab was written to, or asked for.
        session_id: String,
    }

    /// Run Tabs through `handle_capture` and `apply_capture_report` the way
    /// the run loop does, on a game open as "o10" on Busan. Each Tab comes
    /// with the seconds since the one before it. Unlike [`o10_tabs`] a Tab
    /// may split, be deferred or stay pending.
    async fn o10_run(ctx: &DaemonCtx, tabs: Vec<(u64, O10Cells)>) -> (SessionState, Vec<O10Tab>) {
        let tabs = tabs
            .into_iter()
            .map(|(gap, cells)| (gap, cells, None))
            .collect();
        o10_run_rows(ctx, tabs).await
    }

    /// [`o10_run`] with the own row O9 found on each Tab (`None`: the row
    /// is taken as found, not suspect).
    async fn o10_run_rows(
        ctx: &DaemonCtx,
        tabs: Vec<(u64, O10Cells, Option<detect::own_row::OwnRow>)>,
    ) -> (SessionState, Vec<O10Tab>) {
        let mut now = test_now();
        let mut active = game(detect::MatchOutcome::Unknown, None, now);
        active.session_id = "o10".into();
        active.map = Some("Busan".into());
        active.map_source = Some(boundary::MapSource::TopBar);
        active.session_created = false;
        let mut st = session(Some(active), now);
        let mut seen = Vec::new();
        for (gap, cells, own_row) in tabs {
            now += Duration::from_secs(gap);
            let mut analysis = o8_analysis(None, Some("ana"));
            analysis.own_row = own_row;
            *INJECTED_CAPTURE.lock().unwrap() = Some(InjectedCapture {
                analysis,
                board: Some(o10_board(cells)),
            });
            let req = build_capture_request(st.active_game.as_ref().unwrap(), false, now);
            let sid = req.session_id.clone();
            let report = handle_capture(ctx, req).await.unwrap();
            seen.push(O10Tab {
                recorded: report.recorded,
                pending: report.pending(),
                split: report.split,
                deferred: report.armed_reset,
                refresh_baseline: report.refresh_baseline,
                session_id: report.session_id.clone(),
            });
            apply_capture_report(&mut st, &ctx.store, &ctx.data_dir, &sid, Ok(report), now).await;
        }
        (st, seen)
    }

    async fn o10_rows(ctx: &DaemonCtx, session_id: &str) -> Vec<[u32; 6]> {
        ctx.store
            .get_session_snapshots(session_id)
            .await
            .unwrap()
            .iter()
            .map(o10_stats)
            .collect()
    }

    fn all_sure(v: [u32; 6]) -> O10Cells {
        v.map(sure)
    }

    /// F266-1: a stat column the game has never read is a placeholder, and a
    /// placeholder is never stored or uploaded as a number. The board is
    /// read and gated but no row is written; the game's row appears with
    /// the first board that leaves no column unread, with no rate cap or
    /// jump hold against the placeholder.
    #[tokio::test]
    async fn new_reader_stores_no_row_while_a_stat_column_is_unread() {
        let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let ctx = o8_ctx(dir.path(), true).await;
        let first = [
            Some((27, true)),
            sure(5),
            sure(3),
            sure(3500),
            Some((351, true)),
            sure(450),
        ];
        // elims still not read on the second Tab; healing is
        let second = [None, sure(5), sure(3), sure(3520), sure(3400), sure(450)];
        let (st, seen) = o10_run(&ctx, vec![(0, first), (20, second)]).await;
        for tab in &seen {
            assert!(tab.pending && !tab.recorded && !tab.split, "{tab:?}");
        }
        assert_eq!(ctx.store.match_count().await.unwrap(), 0, "no row");
        assert!(ctx.store.get_all_sessions().await.unwrap().is_empty());
        assert!(ctx.store.get_unsynced().await.unwrap().is_empty());
        let game = st.active_game.as_ref().unwrap();
        assert!(!game.session_created, "the session row is not made yet");
        let gate = game.gate.expect("the game goes on from the gate");
        assert_eq!(gate.unread_cols(), vec!["e"], "healing has been read");
        assert_eq!(gate.accepted.healing, 3400);
        // the persisted game carries the pending state across a restart
        let persisted: serde_json::Value =
            serde_json::from_slice(&std::fs::read(active_game_path(dir.path())).unwrap()).unwrap();
        assert_eq!(
            persisted["gate"]["unread"],
            serde_json::json!([true, false, false, false, false, false])
        );
        assert_eq!(persisted["session_created"], false);

        // one more Tab reads elims: the row appears, complete and unflagged,
        // and the healing held on it carries the value read on Tab 2
        let dir = tempfile::tempdir().unwrap();
        let ctx = o8_ctx(dir.path(), true).await;
        let third = [
            sure(25),
            sure(5),
            sure(3),
            sure(3550),
            Some((99, true)),
            sure(450),
        ];
        let (st, seen) = o10_run(&ctx, vec![(0, first), (20, second), (20, third)]).await;
        assert_eq!(
            seen.iter().map(|tab| tab.recorded).collect::<Vec<_>>(),
            vec![false, false, true]
        );
        let rows = ctx.store.get_session_snapshots("o10").await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(o10_stats(&rows[0]), [25, 5, 3, 3550, 3400, 450]);
        assert_eq!(rows[0].suspect_fields, vec!["h".to_string()]);
        assert!(st.active_game.as_ref().unwrap().session_created);
        assert_eq!(ctx.store.get_all_sessions().await.unwrap().len(), 1);
        // what would be uploaded has the read numbers and no placeholder
        let mut row = rows[0].clone();
        row.outcome = "victory".into();
        let upload = sync::upload_request(&[row], &[]);
        assert_eq!(upload.matches.len(), 1);
        assert_eq!(
            (upload.matches[0].elims, upload.matches[0].healing),
            (25, 3400)
        );
    }

    /// alpha.8 (replaces F266-1 at game end): a game where a column never
    /// had a sure read stores the reader's best guess when it closes, and
    /// lists the column in `suspect_fields` so the app marks it unsure.
    /// While the game runs no row is written (the Tabs stay pending), so a
    /// later sure read is still taken as the column's first read.
    #[tokio::test]
    async fn new_reader_game_with_a_never_read_column_stores_its_best_guess() {
        let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let ctx = o8_ctx(dir.path(), true).await;
        let tab = |e: u32, dmg: u32, h: u32| {
            [
                sure(e),
                sure(5),
                sure(3),
                sure(dmg),
                Some((h, true)),
                sure(450),
            ]
        };
        let (mut st, seen) = o10_run(
            &ctx,
            vec![(0, tab(12, 3500, 7000)), (20, tab(13, 3700, 7300))],
        )
        .await;
        assert!(seen.iter().all(|tab| tab.pending), "{seen:?}");
        assert_eq!(
            ctx.store.match_count().await.unwrap(),
            0,
            "nothing mid-game"
        );
        retire_active_game(&mut st, &ctx.store, dir.path(), None, "test: game over").await;
        let rows = ctx.store.get_all_matches().await.unwrap();
        assert_eq!(rows.len(), 1, "the best guess is stored at game end");
        let row = &rows[0];
        assert_eq!(row.session_id, "o10");
        // The newest Tab's guess, the sure columns as read.
        assert_eq!((row.elims, row.damage, row.healing), (13, 3700, 7300));
        assert_eq!(
            crate::reader_apply::upload_suspect_fields(&row.suspect_fields),
            vec!["h".to_string()]
        );
        assert!(
            !dir.path()
                .join("debug")
                .join("unrecorded_games.jsonl")
                .exists(),
            "a stored game is not an unrecorded one"
        );

        // ocr-v1 on the same Tabs has a number for every cell and stores both
        let dir = tempfile::tempdir().unwrap();
        let ctx = o8_ctx(dir.path(), false).await;
        let (_, seen) = o10_run(
            &ctx,
            vec![(0, tab(12, 3500, 7000)), (20, tab(13, 3700, 7300))],
        )
        .await;
        assert!(seen.iter().all(|tab| tab.recorded && !tab.pending));
        assert_eq!(ctx.store.match_count().await.unwrap(), 2);
    }

    /// R2-1 of the #266 review, and what S1 decides for it. A game is left
    /// pending under the new reader (healing never read), the member turns
    /// classic OCR back on with `reader = "ocr-v1"` in the file, restarts,
    /// and the game is resumed. `active_game.json` has no reader tag, so the
    /// ocr-v1 run finds a gate state only the new reader writes.
    ///
    /// The decision: the saved state is honoured as it is. The unread column
    /// is neither cleared on load nor ignored under ocr-v1, because either
    /// would let the placeholder 0 be stored as a number.
    ///
    /// * A Tab on which another column drops: the first read of healing is
    ///   not taken and no row is written. (A game that was ocr-v1 from its
    ///   start stores that Tab. This is the one way an ocr-v1 run holds a
    ///   board on gate state.)
    /// * The first Tab with no drop: OCR's number is the column's first
    ///   read, the row is written, and nothing on it is flagged.
    #[tokio::test]
    async fn a_pending_game_resumed_under_ocr_v1_keeps_its_unread_column() {
        async fn ocr_v1_tab(
            ctx: &DaemonCtx,
            st: &mut SessionState,
            cells: [u32; 6],
            now: Instant,
        ) -> bool {
            let mut analysis = o8_analysis(None, Some("ana"));
            for row in &mut analysis.rows {
                for (cell, value) in row.stats.iter_mut().zip(cells) {
                    cell.value = value.to_string();
                }
            }
            *INJECTED_CAPTURE.lock().unwrap() = Some(InjectedCapture {
                analysis,
                board: None,
            });
            let req = build_capture_request(st.active_game.as_ref().unwrap(), false, now);
            let sid = req.session_id.clone();
            let report = handle_capture(ctx, req).await.unwrap();
            assert!(!report.split, "the Tab must stay on this game");
            let recorded = report.recorded;
            apply_capture_report(st, &ctx.store, &ctx.data_dir, &sid, Ok(report), now).await;
            recorded
        }

        let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = o8_ctx(dir.path(), true).await;
        let pending = [
            sure(12),
            sure(5),
            sure(3),
            sure(3500),
            Some((7000, true)),
            sure(450),
        ];
        let (mut st, seen) = o10_run(&ctx, vec![(0, pending)]).await;
        assert!(seen[0].pending, "{:?}", seen[0]);

        // What the restart finds on disk: the gate, and no word of which
        // reader wrote it.
        let persisted: serde_json::Value =
            serde_json::from_slice(&std::fs::read(active_game_path(dir.path())).unwrap()).unwrap();
        assert_eq!(persisted.get("reader"), None);
        let saved: capture_gate::GateState =
            serde_json::from_value(persisted["gate"].clone()).unwrap();
        assert_eq!(saved.unread_cols(), vec!["h"]);
        st.active_game.as_mut().unwrap().gate = Some(saved);

        // The restart: classic OCR on, `reader = "ocr-v1"`.
        ctx.use_new_reader = false;
        let now = test_now() + Duration::from_secs(20);

        // Damage reads below the accepted 3500 on this Tab.
        let recorded = ocr_v1_tab(&ctx, &mut st, [13, 5, 3, 3400, 7300, 450], now).await;
        assert!(!recorded, "healing's first read is not taken on a drop");
        assert_eq!(ctx.store.match_count().await.unwrap(), 0, "no row");
        let gate = st.active_game.as_ref().unwrap().gate.expect("the gate");
        assert_eq!(gate.unread_cols(), vec!["h"], "still unread");

        // No drop: the column is read, and the row is OCR's.
        let now = now + Duration::from_secs(20);
        let recorded = ocr_v1_tab(&ctx, &mut st, [14, 5, 3, 3700, 7400, 450], now).await;
        assert!(recorded);
        let rows = ctx.store.get_session_snapshots("o10").await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(o10_stats(&rows[0]), [14, 5, 3, 3700, 7400, 450]);
        assert!(
            rows[0].suspect_fields.is_empty(),
            "{:?}",
            rows[0].suspect_fields
        );
        let gate = st.active_game.as_ref().unwrap().gate.expect("the gate");
        assert!(gate.unread_cols().is_empty());
    }

    /// F266-2, the reviewer's trace through `handle_capture`: game 1 has one
    /// Tab with healing held, its end is missed, and game 2's first Tab
    /// comes 60 s later with every cell sure. That board used to be appended
    /// to game 1 and its healing stored there unflagged. Now the reset plan
    /// sees the sure columns fall and holds the board off game 1; the next
    /// fresh board splits, and game 2 gets both boards.
    #[tokio::test]
    async fn new_reader_next_games_first_board_does_not_fill_an_unread_column() {
        let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let ctx = o8_ctx(dir.path(), true).await;
        let game_one = [
            sure(30),
            sure(12),
            sure(9),
            sure(14000),
            Some((5000, true)),
            sure(900),
        ];
        let (st, seen) = o10_run(
            &ctx,
            vec![
                (0, game_one),
                (60, all_sure([1, 0, 0, 300, 100, 0])),
                (30, all_sure([3, 1, 0, 900, 400, 0])),
            ],
        )
        .await;
        assert!(seen[0].pending, "{:?}", seen[0]);
        assert!(
            seen[1].deferred && !seen[1].recorded && !seen[1].pending,
            "game 2's first board is held off game 1: {:?}",
            seen[1]
        );
        assert!(seen[2].split && seen[2].recorded, "{:?}", seen[2]);
        // game 1 never had a sure healing read: at its close it stores its
        // own best guess (5000, marked unsure), and above all not game 2's
        // 100 (alpha.8)
        assert_eq!(
            o10_rows(&ctx, "o10").await,
            vec![[30, 12, 9, 14000, 5000, 900]]
        );
        let new_id = st.active_game.as_ref().unwrap().session_id.clone();
        assert_ne!(new_id, "o10");
        assert_eq!(seen[2].session_id, new_id);
        assert_eq!(
            o10_rows(&ctx, &new_id).await,
            vec![[1, 0, 0, 300, 100, 0], [3, 1, 0, 900, 400, 0]]
        );
        assert_eq!(ctx.store.match_count().await.unwrap(), 3);

        // 30 s after game 1's Tab is too soon for the reset rule. The gate
        // is then the guard: the read columns drop, so the first read of
        // healing is not taken and game 1 still has no row.
        let dir = tempfile::tempdir().unwrap();
        let ctx = o8_ctx(dir.path(), true).await;
        let (st, seen) = o10_run(
            &ctx,
            vec![(0, game_one), (30, all_sure([1, 0, 0, 300, 100, 0]))],
        )
        .await;
        assert!(seen[1].pending && !seen[1].deferred, "{:?}", seen[1]);
        assert_eq!(ctx.store.match_count().await.unwrap(), 0);
        let gate = st.active_game.as_ref().unwrap().gate.unwrap();
        assert_eq!(gate.unread_cols(), vec!["h"]);
        assert_eq!(gate.accepted.healing, 0);
        assert_eq!(gate.accepted.elims, 30);
        // At close the game stores game 1's own guess. The second Tab's
        // healing read was not taken (the other columns dropped), so its
        // 100 is not the guess (alpha.8).
        let mut st = st;
        retire_active_game(&mut st, &ctx.store, dir.path(), None, "test: game over").await;
        assert_eq!(
            o10_rows(&ctx, "o10").await,
            vec![[30, 12, 9, 14000, 5000, 900]]
        );
    }

    /// A split under `reader = "new"`, with the carried board (F266-1's
    /// folded case, and two of the reviewer's mutants). Game 1 is stored;
    /// two fresh-match boards follow and the second has healing held. The
    /// held cell abstains from the reset, so the split still happens. The
    /// new game's rows come from the new game only: the stored-row gate
    /// starts over at the split, and the held healing takes the carried
    /// board's 120, not a 0 under it and not game 1's 5000.
    #[tokio::test]
    async fn new_reader_split_with_a_held_cell_takes_the_carried_board() {
        let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let ctx = o8_ctx(dir.path(), true).await;
        let held_healing = [
            sure(3),
            sure(1),
            sure(0),
            sure(900),
            Some((9, true)),
            sure(0),
        ];
        let (st, seen) = o10_run(
            &ctx,
            vec![
                (0, all_sure([30, 12, 9, 14000, 5000, 900])),
                (60, all_sure([1, 0, 0, 300, 120, 0])),
                (30, held_healing),
            ],
        )
        .await;
        assert!(seen[0].recorded, "{:?}", seen[0]);
        assert!(seen[1].deferred, "{:?}", seen[1]);
        assert!(seen[2].split && seen[2].recorded, "{:?}", seen[2]);
        assert_eq!(
            o10_rows(&ctx, "o10").await,
            vec![[30, 12, 9, 14000, 5000, 900]],
            "game 1 keeps its one row"
        );
        let new_id = st.active_game.as_ref().unwrap().session_id.clone();
        let rows = ctx.store.get_session_snapshots(&new_id).await.unwrap();
        assert_eq!(
            rows.iter().map(o10_stats).collect::<Vec<_>>(),
            vec![[1, 0, 0, 300, 120, 0], [3, 1, 0, 900, 120, 0]]
        );
        assert_eq!(rows[1].suspect_fields, vec!["h".to_string()]);
        let gate = st.active_game.as_ref().unwrap().gate.unwrap();
        assert!(gate.unread_cols().is_empty());
        assert_eq!(gate.accepted.healing, 120);

        // a fresh-match board with a held cell is the first reset signal
        // but is not carried: it has no number for that cell. The split on
        // the next board opens the game from that board alone.
        let dir = tempfile::tempdir().unwrap();
        let ctx = o8_ctx(dir.path(), true).await;
        let (st, seen) = o10_run(
            &ctx,
            vec![
                (0, all_sure([30, 12, 9, 14000, 5000, 900])),
                (60, held_healing),
                (30, all_sure([4, 1, 0, 1200, 300, 0])),
            ],
        )
        .await;
        assert!(seen[1].deferred && seen[2].split, "{seen:?}");
        let new_id = st.active_game.as_ref().unwrap().session_id.clone();
        assert_eq!(o10_rows(&ctx, &new_id).await, vec![[4, 1, 0, 1200, 300, 0]]);
        assert_eq!(ctx.store.match_count().await.unwrap(), 2);
    }

    /// A split whose first board has a held cell and no carried board: the
    /// old game is closed, the new one is open and pending, and no row of
    /// the new game exists until its column is read.
    #[tokio::test]
    async fn new_reader_split_onto_a_pending_game_stores_no_row_yet() {
        let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let ctx = o8_ctx(dir.path(), true).await;
        let held_healing = |e: u32, dmg: u32| {
            [
                sure(e),
                sure(1),
                sure(0),
                sure(dmg),
                Some((9, true)),
                sure(0),
            ]
        };
        let (st, seen) = o10_run(
            &ctx,
            vec![
                (0, all_sure([30, 12, 9, 14000, 5000, 900])),
                (60, held_healing(1, 300)),
                (30, held_healing(3, 900)),
                (20, all_sure([4, 1, 0, 1300, 250, 0])),
            ],
        )
        .await;
        assert!(seen[1].deferred, "{:?}", seen[1]);
        assert!(
            seen[2].split && seen[2].pending && !seen[2].recorded,
            "{:?}",
            seen[2]
        );
        assert!(seen[3].recorded && !seen[3].split, "{:?}", seen[3]);
        let new_id = st.active_game.as_ref().unwrap().session_id.clone();
        assert_eq!(seen[2].session_id, new_id);
        assert_eq!(seen[3].session_id, new_id);
        assert_eq!(
            o10_rows(&ctx, "o10").await,
            vec![[30, 12, 9, 14000, 5000, 900]]
        );
        assert_eq!(o10_rows(&ctx, &new_id).await, vec![[4, 1, 0, 1300, 250, 0]]);
        assert!(st.active_game.as_ref().unwrap().session_created);
        // the pending split made no session row; the first stored row did
        let sessions = ctx.store.get_all_sessions().await.unwrap();
        assert!(
            sessions.iter().any(|s| s.session_id == new_id),
            "the new game's session row exists"
        );
        assert_eq!(sessions.len(), 2);

        // a fresh-match board that is not carried leaves no half of a held
        // board behind: no counters, and no hero or time for them
        let dir = tempfile::tempdir().unwrap();
        let ctx = o8_ctx(dir.path(), true).await;
        let (st, seen) = o10_run(
            &ctx,
            vec![
                (0, all_sure([30, 12, 9, 14000, 5000, 900])),
                (60, held_healing(1, 300)),
            ],
        )
        .await;
        assert!(seen[1].deferred, "{:?}", seen[1]);
        let game = st.active_game.as_ref().unwrap();
        assert_eq!(game.reset_streak, 1, "it is the first reset signal");
        assert!(game.deferred.is_none());
        assert!(game.deferred_hero.is_none() && game.deferred_at.is_none());
    }

    /// O9 and O10 on one board: O9 says which row is the member's and how
    /// sure it is, O10 reads that row's cells. A suspect own row may be
    /// another player's, so it is a low-trust read all the way: every field
    /// off the row is flagged, the capture gate does not latch its counters,
    /// and the boundary plan does not take it as a reset row. A held cell on
    /// it is held like any other.
    #[tokio::test]
    async fn new_reader_suspect_own_row_with_held_cells_is_low_trust_and_flagged() {
        use detect::own_row::combine;
        let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
        let sure_row = combine(Some(0), o9_lit(0, 1.5), true);
        let suspect_row = combine(Some(0), None, true);
        assert!(suspect_row.suspect() && !sure_row.suspect());
        let (sure_row, suspect_row) = (Some(sure_row), Some(suspect_row));
        let held_healing = |e: u32, dmg: u32| {
            [
                sure(e),
                sure(6),
                sure(3),
                sure(dmg),
                Some((77, true)),
                sure(450),
            ]
        };

        // a game with a row; then a suspect row with healing held
        let dir = tempfile::tempdir().unwrap();
        let ctx = o8_ctx(dir.path(), true).await;
        let (st, seen) = o10_run_rows(
            &ctx,
            vec![
                (0, all_sure([12, 5, 3, 3500, 700, 300]), sure_row),
                (20, held_healing(13, 3900), suspect_row),
            ],
        )
        .await;
        assert!(
            seen[0].recorded && seen[0].refresh_baseline,
            "{:?}",
            seen[0]
        );
        assert!(
            seen[1].recorded && seen[1].refresh_baseline,
            "a suspect row is stored, and on the baseline's row it anchors: {:?}",
            seen[1]
        );
        let rows = ctx.store.get_session_snapshots("o10").await.unwrap();
        // the sure cells are the board's, the held one keeps the last
        // accepted 700, and every field read off the row is flagged
        assert_eq!(o10_stats(&rows[1]), [13, 6, 3, 3900, 700, 450]);
        assert_eq!(
            rows[1].suspect_fields,
            reader_apply::OWN_ROW_FIELDS.to_vec()
        );
        let gate = st.active_game.as_ref().unwrap().gate.unwrap();
        assert!(gate.low_trust, "the counters did not latch");
        // the moved columns are unconfirmed over the values they left; the
        // held column was not read and stays confirmed
        assert!(gate.unconfirmed[0] && gate.unconfirmed[3]);
        assert_eq!(gate.confirmed_floor[0], 12);
        assert!(!gate.unconfirmed[4] && !gate.has_confirmed_floor[4]);

        // a game whose first board is a suspect row with healing held: no
        // row. Nothing of the game is stored and nothing in it was read off
        // a trusted row, so the member's real row, on the first trusted
        // board, starts the game over: its numbers are the row, unflagged.
        let dir = tempfile::tempdir().unwrap();
        let ctx = o8_ctx(dir.path(), true).await;
        let real = all_sure([12, 6, 3, 3500, 700, 450]);
        let (st, seen) = o10_run_rows(
            &ctx,
            vec![
                (0, held_healing(20, 8000), suspect_row),
                (20, real, sure_row),
                (20, real, sure_row),
            ],
        )
        .await;
        assert!(seen[0].pending, "{:?}", seen[0]);
        assert!(seen[1].recorded && seen[2].recorded, "{seen:?}");
        let rows = ctx.store.get_session_snapshots("o10").await.unwrap();
        assert_eq!(rows.len(), 2);
        for row in &rows {
            assert_eq!(o10_stats(row), [12, 6, 3, 3500, 700, 450]);
            assert!(row.suspect_fields.is_empty(), "{:?}", row.suspect_fields);
        }
        assert!(!st.active_game.as_ref().unwrap().gate.unwrap().low_trust);
    }

    /// R262-E: the name fallback's rows index the digit reader's board, so
    /// a set that is not the whole board names no row. Today's names read
    /// always returns the whole board (R2-6); the rule is a guard.
    #[test]
    fn the_name_fallback_matches_only_on_a_full_set_of_rows() {
        let named = |name: &str| ocr::RowOcrResult {
            name: Some(ocr::CellOcrResult {
                value: name.into(),
                confidence: 90,
                suspect: false,
            }),
            name_alternates: Vec::new(),
            stats: Vec::new(),
            mean_confidence: 0,
        };
        let mut rows: Vec<ocr::RowOcrResult> = (0..10).map(|_| named("OTHER")).collect();
        rows[3] = named("MADEUP");
        assert_eq!(
            name_on_a_full_board(&rows, "MadeUp", 5).map(|m| m.row),
            Some(3)
        );
        // a set with a row missing is not matched at all
        rows.remove(0);
        assert_eq!(name_on_a_full_board(&rows, "MadeUp", 5), None);
        // and a 5v5 set is not a 6v6 board
        let rows: Vec<ocr::RowOcrResult> = (0..10).map(|_| named("MADEUP")).collect();
        assert_eq!(name_on_a_full_board(&rows, "MadeUp", 6), None);
    }

    /// Under the new reader every cell whose stored value is not this
    /// capture's read is flagged, a rate-cap hold too (F266-3). ocr-v1
    /// leaves a rate-cap hold unflagged, as before.
    #[tokio::test]
    async fn new_reader_flags_a_rate_cap_hold_and_ocr_v1_does_not() {
        let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let ctx = o8_ctx(dir.path(), true).await;
        let (_, seen) = o10_run(
            &ctx,
            vec![
                (0, all_sure([10, 4, 2, 3000, 800, 400])),
                // +40 elims in 20 s, the rest of the board moving
                (20, all_sure([50, 6, 3, 3900, 1500, 800])),
            ],
        )
        .await;
        assert!(seen.iter().all(|tab| tab.recorded), "{seen:?}");
        let rows = ctx.store.get_session_snapshots("o10").await.unwrap();
        assert_eq!(o10_stats(&rows[1]), [10, 6, 3, 3900, 1500, 800]);
        assert_eq!(rows[1].suspect_fields, vec!["e".to_string()]);

        let mut parsed = test_match("s", "unknown");
        (parsed.elims, parsed.assists, parsed.deaths) = (10, 6, 3);
        let kept = capture_gate::Counters {
            elims: 10,
            assists: 6,
            deaths: 3,
            damage: parsed.damage,
            healing: parsed.healing,
            mitigation: parsed.mitigation,
        };
        let holds = [capture_gate::Hold {
            col: 0,
            kind: capture_gate::HoldKind::RateCap,
            raw: 50,
            held: 10,
        }];
        note_gate_suspects(&mut parsed, kept, &capture_gate::unsure_fields(&holds));
        assert!(parsed.suspect_fields.is_empty(), "ocr-v1's list");
        note_gate_suspects(&mut parsed, kept, &capture_gate::held_fields(&holds));
        assert_eq!(parsed.suspect_fields, vec!["e".to_string()]);
    }

    /// A 1080p stand-in the capture preflight takes for a Tab: a bright
    /// header with six dark labels in the scoreboard window (the lib's
    /// `map_label_fixtures::frame_1080`). Not a captured frame.
    fn o10_tab_frame() -> image::DynamicImage {
        let (w, h) = (1920u32, 1080u32);
        let mut img = image::RgbImage::from_pixel(w, h, image::Rgb([12, 14, 22]));
        let bx = (w as f64 * 0.175) as u32;
        let by = (h as f64 * 0.15) as u32;
        let bw = (w as f64 * 0.65) as u32;
        let bh = (h as f64 * 0.70) as u32;
        let y1 = by + (bh as f64 * 0.030) as u32;
        for py in by..y1 {
            for px in bx..bx + bw {
                img.put_pixel(px, py, image::Rgb([230, 230, 230]));
            }
        }
        let origin = bx + (bw as f64 * 0.40) as u32;
        for i in 0..6u32 {
            let x0 = origin + i * 36;
            for py in by..y1 {
                for px in x0..x0 + 4 {
                    img.put_pixel(px, py, image::Rgb([20, 20, 20]));
                }
            }
        }
        image::DynamicImage::ImageRgb8(img)
    }

    /// `analyze_frame` itself under `reader = "new"`, on a frame the
    /// preflight takes for a Tab: the vision pass comes back with no OCR
    /// stat cell, no name cell (no player name, or a blank one), no board
    /// text, and the reader's board. Until now this rested on the helper
    /// tests and on two source-text counts.
    #[test]
    fn analyze_frame_under_the_new_reader_reads_no_cell_and_no_board_text() {
        let dir = tempfile::tempdir().unwrap();
        for name in [None, Some("  ".to_string())] {
            let matcher =
                std::sync::Arc::new(detect::hero_portrait::PortraitMatcher::load(dir.path()));
            let flags = FrameFlags {
                allow_banner_recovery: false,
                session_map_known: false,
                shadow_on: false,
                use_new_reader: true,
            };
            let out = analyze_frame(
                o10_tab_frame(),
                matcher,
                name,
                detect::MatchOutcome::Unknown,
                flags,
                Vec::new(),
            );
            let FrameAnalysisOutcome::Analyzed(analysis) = out else {
                panic!("the stand-in frame did not pass the Tab preflight");
            };
            let analysis = *analysis;
            assert!(analysis.own_row.is_some(), "the own row is O9's");
            assert_eq!(analysis.rows.len(), analysis.team_size * 2);
            for row in &analysis.rows {
                assert!(row.stats.is_empty(), "an OCR stat cell was read");
                assert!(row.name.is_none() && row.name_alternates.is_empty());
            }
            let text = analysis.ocr.expect("no OCR, so no OCR error");
            assert_eq!((text.raw_text.as_str(), text.confidence), ("", 0));
            assert!(analysis.board.is_some(), "the board is the reader's");
            assert_eq!(analysis.career_hero, None);
            assert_eq!(analysis.name_match, None);
        }
    }

    /// A held cell abstains from the reset and the row still re-anchors the
    /// reset baseline (F266-2). Before, `row_counts` was false for any board
    /// with a held cell: no baseline, so no reset for the whole game.
    #[test]
    fn a_board_with_a_held_cell_still_anchors_the_reset_baseline() {
        let (game, now) = request_with_text_fallback();
        let req = build_capture_request(&game, false, now);
        let clean = banner_facts("Busan", true);
        assert!(plan_from_board(&req, &clean).refresh_baseline);
        let mut held = banner_facts("Busan", true);
        held.held[4] = true;
        held.suspect[4] = true;
        let plan = plan_from_board(&req, &held);
        assert!(plan.refresh_baseline, "{plan:?}");
        assert_eq!(plan.baseline_row, Some(2));
        // a raw-text fallback row still does not
        let mut fallback = banner_facts("Busan", true);
        fallback.trusted_cells = false;
        assert!(!plan_from_board(&req, &fallback).refresh_baseline);

        // a game with boards read and no row yet can still gap-split: it
        // has a gate of its own, so "no session row" is not "no board yet"
        let mut pending = game;
        pending.session_created = false;
        pending.map_source = Some(boundary::MapSource::TopBar);
        pending.gate = Some(GateState {
            accepted: Counters {
                elims: 30,
                assists: 12,
                deaths: 9,
                damage: 14000,
                healing: 0,
                mitigation: 900,
            },
            last_raw: Counters {
                elims: 30,
                assists: 12,
                deaths: 9,
                damage: 14000,
                healing: 0,
                mitigation: 900,
            },
            unread: [false, false, false, false, true, false],
            ..GateState::default()
        });
        let later = now + Duration::from_secs(200);
        let req = build_capture_request(&pending, false, later);
        assert!(req.create_session);
        let mut next = banner_facts("Ilios", true);
        next.hero = "Reinhardt";
        next.counters = Counters {
            elims: 1,
            assists: 0,
            deaths: 0,
            damage: 300,
            healing: 100,
            mitigation: 0,
        };
        let plan = plan_from_board(&req, &next);
        assert!(plan.split, "{plan:?}");
        assert_eq!(
            plan.close_reason,
            Some(boundary::CloseReason::StatRegression)
        );
    }
    /// The fields that are lower on a row than on the row before it in the
    /// same session and are not flagged there. Empty is the rule: a stat
    /// never goes down unflagged in a stored row.
    fn o10_unflagged_drops(rows: &[storage::PersonalMatch]) -> Vec<String> {
        let mut found = Vec::new();
        for (n, pair) in rows.windows(2).enumerate() {
            let (before, after) = (o10_stats(&pair[0]), o10_stats(&pair[1]));
            for (col, name) in capture_gate::COL_FIELD.iter().enumerate() {
                let flagged = pair[1].suspect_fields.iter().any(|f| f == name);
                if after[col] < before[col] && !flagged {
                    found.push(format!(
                        "row {} {name}: {} to {}",
                        n + 1,
                        before[col],
                        after[col]
                    ));
                }
            }
        }
        found
    }

    /// R3-1, claude-r3's experiment e4 through `handle_capture`: a game whose
    /// every Tab has a suspect own row, its end missed, then the next game
    /// on the same map and hero with sure rows 40 s apart. Before, the
    /// suspect rows set no reset baseline, so nothing split: one session,
    /// whose last row was `30 12 1 14000 5000 150` with deaths (9, 0, 1) and
    /// mitigation (900, 0, 150) gone down unflagged. Now the suspect rows
    /// anchor the baseline, the first sure board of game 2 is held off game
    /// 1, and the second splits.
    #[tokio::test]
    async fn new_reader_all_suspect_game_then_a_sure_game_splits() {
        use detect::own_row::combine;
        let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
        let suspect = Some(combine(Some(0), None, true));
        let sure_row = Some(combine(Some(0), o9_lit(0, 1.5), true));
        let game_one = [
            [10, 4, 3, 5000, 1800, 300],
            [20, 8, 6, 9500, 3400, 600],
            [30, 12, 9, 14000, 5000, 900],
        ];
        let game_two = [
            [1, 0, 0, 300, 100, 0],
            [3, 1, 0, 900, 400, 0],
            [5, 2, 1, 1900, 700, 100],
            [7, 3, 1, 2800, 900, 150],
        ];
        let tabs = |second: Option<detect::own_row::OwnRow>| {
            let mut tabs = vec![
                (0, all_sure(game_one[0]), suspect),
                (60, all_sure(game_one[1]), suspect),
                (60, all_sure(game_one[2]), suspect),
                (200, all_sure(game_two[0]), second),
            ];
            for board in &game_two[1..] {
                tabs.push((40, all_sure(*board), second));
            }
            tabs
        };

        // e4: game 2 has sure rows
        let dir = tempfile::tempdir().unwrap();
        let ctx = o8_ctx(dir.path(), true).await;
        let (st, seen) = o10_run_rows(&ctx, tabs(sure_row)).await;
        for tab in &seen[..3] {
            assert!(tab.recorded && tab.refresh_baseline, "{tab:?}");
        }
        assert!(seen[3].deferred && !seen[3].recorded, "{:?}", seen[3]);
        assert!(seen[4].split && seen[4].recorded, "{:?}", seen[4]);
        let one = ctx.store.get_session_snapshots("o10").await.unwrap();
        assert_eq!(
            one.iter().map(o10_stats).collect::<Vec<_>>(),
            game_one.to_vec(),
            "game 1 keeps its own three rows"
        );
        for row in &one {
            assert_eq!(row.suspect_fields, reader_apply::OWN_ROW_FIELDS.to_vec());
        }
        let new_id = st.active_game.as_ref().unwrap().session_id.clone();
        assert_ne!(new_id, "o10");
        let two = ctx.store.get_session_snapshots(&new_id).await.unwrap();
        assert_eq!(
            two.iter().map(o10_stats).collect::<Vec<_>>(),
            game_two.to_vec(),
            "game 2 has its own session, with all four boards"
        );
        assert_eq!(ctx.store.get_all_sessions().await.unwrap().len(), 2);
        assert_eq!(o10_unflagged_drops(&one), Vec::<String>::new());
        assert_eq!(o10_unflagged_drops(&two), Vec::<String>::new());

        // e1: game 2 has suspect rows too. A suspect row never commits the
        // split by itself, so two boards are held off game 1 and the third
        // splits. Nothing of game 2 lands in game 1.
        let dir = tempfile::tempdir().unwrap();
        let ctx = o8_ctx(dir.path(), true).await;
        let (st, seen) = o10_run_rows(&ctx, tabs(suspect)).await;
        assert!(seen[3].deferred && seen[4].deferred, "{seen:?}");
        assert!(seen[5].split && seen[5].recorded, "{:?}", seen[5]);
        assert_eq!(
            o10_rows(&ctx, "o10").await,
            game_one.to_vec(),
            "game 1 keeps its own three rows"
        );
        let new_id = st.active_game.as_ref().unwrap().session_id.clone();
        let two = ctx.store.get_session_snapshots(&new_id).await.unwrap();
        assert_eq!(
            two.iter().map(o10_stats).collect::<Vec<_>>(),
            game_two[2..].to_vec(),
            "a suspect fresh board is not carried: game 2 starts at the board that split"
        );
        for row in &two {
            assert_eq!(row.suspect_fields, reader_apply::OWN_ROW_FIELDS.to_vec());
        }
        assert_eq!(ctx.store.match_count().await.unwrap(), 5);
    }

    /// One wrong suspect row in the middle of a sure game does not split it.
    /// The row looks like a fresh match on the member's own row index: it is
    /// held off the game as a reset signal, nothing of it is stored, and the
    /// next real board stays in the game.
    #[tokio::test]
    async fn new_reader_one_wrong_suspect_row_does_not_split_a_sure_game() {
        use detect::own_row::combine;
        let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
        let suspect = Some(combine(Some(0), None, true));
        let sure_row = Some(combine(Some(0), o9_lit(0, 1.5), true));
        let dir = tempfile::tempdir().unwrap();
        let ctx = o8_ctx(dir.path(), true).await;
        let (st, seen) = o10_run_rows(
            &ctx,
            vec![
                (0, all_sure([12, 5, 3, 3500, 700, 300]), sure_row),
                (60, all_sure([14, 6, 3, 4300, 900, 400]), sure_row),
                (60, all_sure([1, 0, 1, 300, 0, 0]), suspect),
                (60, all_sure([15, 6, 4, 4700, 950, 450]), sure_row),
                (60, all_sure([16, 7, 4, 5100, 1000, 500]), sure_row),
            ],
        )
        .await;
        assert!(
            seen[2].deferred && !seen[2].recorded && !seen[2].split,
            "{:?}",
            seen[2]
        );
        assert!(seen.iter().all(|tab| !tab.split), "{seen:?}");
        assert!(seen[3].recorded && seen[4].recorded, "{seen:?}");
        let game = st.active_game.as_ref().unwrap();
        assert_eq!(game.session_id, "o10");
        assert_eq!(game.reset_streak, 0, "the real board ended the streak");
        assert!(game.deferred.is_none(), "a suspect board is not carried");
        let rows = ctx.store.get_session_snapshots("o10").await.unwrap();
        assert_eq!(
            rows.iter().map(o10_stats).collect::<Vec<_>>(),
            vec![
                [12, 5, 3, 3500, 700, 300],
                [14, 6, 3, 4300, 900, 400],
                [15, 6, 4, 4700, 950, 450],
                [16, 7, 4, 5100, 1000, 500],
            ]
        );
        assert_eq!(ctx.store.get_all_sessions().await.unwrap().len(), 1);
        assert!(rows.iter().all(|row| row.suspect_fields.is_empty()));
    }

    /// A stat never goes down unflagged in a stored row, whatever the trust
    /// bits say (R3-1). A suspect row with higher numbers is stored flagged
    /// and leaves its columns unconfirmed; the member's real row then reads
    /// lower. The first lower read is held (flagged), the second matching
    /// one replaces all six numbers with lower ones. That replacement is not
    /// a hold, and it used to store the lower numbers with no flag.
    #[tokio::test]
    async fn new_reader_flags_every_stat_that_goes_down_in_a_stored_row() {
        use detect::own_row::combine;
        let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
        let suspect = Some(combine(Some(0), None, true));
        let sure_row = Some(combine(Some(0), o9_lit(0, 1.5), true));
        let dir = tempfile::tempdir().unwrap();
        let ctx = o8_ctx(dir.path(), true).await;
        let (_, seen) = o10_run_rows(
            &ctx,
            vec![
                (0, all_sure([12, 5, 3, 3500, 700, 300]), sure_row),
                (20, all_sure([20, 9, 5, 8000, 2000, 900]), suspect),
                (20, all_sure([13, 5, 3, 3700, 720, 300]), sure_row),
                (20, all_sure([13, 5, 3, 3700, 720, 300]), sure_row),
                (20, all_sure([14, 6, 3, 4000, 800, 350]), sure_row),
            ],
        )
        .await;
        assert!(
            seen.iter().all(|tab| tab.recorded && !tab.split),
            "{seen:?}"
        );
        let rows = ctx.store.get_session_snapshots("o10").await.unwrap();
        let six: Vec<String> = capture_gate::COL_FIELD.map(String::from).to_vec();
        // the suspect row: stored as read, every own-row field flagged
        assert_eq!(o10_stats(&rows[1]), [20, 9, 5, 8000, 2000, 900]);
        assert_eq!(
            rows[1].suspect_fields,
            reader_apply::OWN_ROW_FIELDS.to_vec()
        );
        // the first real read: held at the suspect row's numbers, flagged
        assert_eq!(o10_stats(&rows[2]), [20, 9, 5, 8000, 2000, 900]);
        assert_eq!(rows[2].suspect_fields, six);
        // the second: all six go down, and all six are flagged
        assert_eq!(o10_stats(&rows[3]), [13, 5, 3, 3700, 720, 300]);
        assert_eq!(rows[3].suspect_fields, six);
        // from there the game is the member's again, unflagged
        assert_eq!(o10_stats(&rows[4]), [14, 6, 3, 4000, 800, 350]);
        assert!(
            rows[4].suspect_fields.is_empty(),
            "{:?}",
            rows[4].suspect_fields
        );
        assert_eq!(o10_unflagged_drops(&rows), Vec::<String>::new());
    }

    /// R3-2, claude-r3's experiment h1: a suspect, higher first board with
    /// healing held, then four complete sure reads of the member's row with
    /// moving numbers. That stored 0 rows from 5 Tabs. A game with no row
    /// yet and only low-trust reads is now started over by the first
    /// trusted board, so the four real boards are stored.
    #[tokio::test]
    async fn new_reader_first_trusted_board_restarts_a_game_only_suspect_rows_read() {
        use detect::own_row::combine;
        let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
        let suspect = Some(combine(Some(0), None, true));
        let sure_row = Some(combine(Some(0), o9_lit(0, 1.5), true));
        let first = [
            sure(20),
            sure(6),
            sure(3),
            sure(8000),
            Some((77, true)),
            sure(450),
        ];
        let real = [
            [12, 6, 3, 3500, 700, 450],
            [13, 6, 3, 3700, 720, 450],
            [14, 7, 4, 4100, 800, 500],
            [15, 7, 4, 4500, 900, 520],
        ];
        let dir = tempfile::tempdir().unwrap();
        let ctx = o8_ctx(dir.path(), true).await;
        let mut tabs = vec![(0, first, suspect)];
        tabs.extend(real.iter().map(|board| (30, all_sure(*board), sure_row)));
        let (_, seen) = o10_run_rows(&ctx, tabs).await;
        assert!(seen[0].pending, "{:?}", seen[0]);
        assert!(
            seen[1..].iter().all(|tab| tab.recorded && !tab.split),
            "{seen:?}"
        );
        let rows = ctx.store.get_session_snapshots("o10").await.unwrap();
        assert_eq!(
            rows.iter().map(o10_stats).collect::<Vec<_>>(),
            real.to_vec()
        );
        assert!(rows.iter().all(|row| row.suspect_fields.is_empty()));
    }

    /// R3-3: nothing that runs before the capture gate reads a held cell.
    /// The boundary plan gives the same plan whatever number sits in a held
    /// column: the game's last value (what `fill_held_cells` puts there), a
    /// 0, or garbage. That is why `fill_held_cells` is a belt and not a rule.
    #[test]
    fn the_plan_does_not_read_a_held_cell() {
        let (mut game, now) = request_with_text_fallback();
        let accepted = Counters {
            elims: 30,
            assists: 12,
            deaths: 9,
            damage: 14000,
            healing: 5000,
            mitigation: 900,
        };
        let state = GateState {
            accepted,
            last_raw: accepted,
            ..GateState::default()
        };
        game.gate = Some(state);
        game.reset_baseline = Some(state);
        game.baseline_row = Some(2);
        game.baseline_at = Some(now);
        let req = build_capture_request(&game, false, now + Duration::from_secs(60));
        let arr = |c: Counters| {
            [
                c.elims,
                c.assists,
                c.deaths,
                c.damage,
                c.healing,
                c.mitigation,
            ]
        };
        // (the sure cells, the held columns)
        let boards: [(Counters, &[usize]); 4] = [
            // the game goes on, healing held
            (
                Counters {
                    elims: 31,
                    assists: 12,
                    deaths: 9,
                    damage: 14400,
                    healing: 0,
                    mitigation: 900,
                },
                &[4],
            ),
            // a fresh-match board, mitigation and assists held
            (
                Counters {
                    elims: 1,
                    assists: 0,
                    deaths: 0,
                    damage: 300,
                    healing: 100,
                    mitigation: 0,
                },
                &[1, 5],
            ),
            // a fresh-match board with elims held: deaths and damage decide
            (
                Counters {
                    elims: 0,
                    assists: 0,
                    deaths: 0,
                    damage: 300,
                    healing: 100,
                    mitigation: 0,
                },
                &[0],
            ),
            // damage held on a board that otherwise moved on
            (
                Counters {
                    elims: 33,
                    assists: 13,
                    deaths: 10,
                    damage: 0,
                    healing: 5600,
                    mitigation: 1000,
                },
                &[3],
            ),
        ];
        for (sure_cells, held_cols) in boards {
            let plan_with = |fill: u32| {
                let mut facts = banner_facts("Busan", true);
                let mut cells = arr(sure_cells);
                for &col in held_cols {
                    cells[col] = fill;
                    facts.held[col] = true;
                    facts.suspect[col] = true;
                }
                facts.counters = Counters {
                    elims: cells[0],
                    assists: cells[1],
                    deaths: cells[2],
                    damage: cells[3],
                    healing: cells[4],
                    mitigation: cells[5],
                };
                plan_from_board(&req, &facts)
            };
            let filled = {
                let mut facts = banner_facts("Busan", true);
                let mut cells = arr(sure_cells);
                for &col in held_cols {
                    cells[col] = arr(accepted)[col];
                    facts.held[col] = true;
                    facts.suspect[col] = true;
                }
                facts.counters = Counters {
                    elims: cells[0],
                    assists: cells[1],
                    deaths: cells[2],
                    damage: cells[3],
                    healing: cells[4],
                    mitigation: cells[5],
                };
                plan_from_board(&req, &facts)
            };
            for fill in [0, 1, 77, 99_999] {
                assert_eq!(
                    plan_with(fill),
                    filled,
                    "held {held_cols:?} filled with {fill}"
                );
            }
        }
    }
    /// CR266-2, as a property of the code: under `reader = "new"` a stat
    /// that is lower on a stored row than on the row before it in the same
    /// game is flagged on that row. Random games through the stored row's
    /// gate (`hold_new_reader_stats`): growth, clips, ghost digits, resets,
    /// held cells, suspect (untrusted) rows, splits with and without a
    /// carried board. A row counts as stored when the game has no unread
    /// column, which is the capture path's rule.
    #[test]
    fn new_reader_never_lowers_a_stored_stat_without_a_flag_on_random_games() {
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let counters = |a: [u32; 6]| Counters {
            elims: a[0],
            assists: a[1],
            deaths: a[2],
            damage: a[3],
            healing: a[4],
            mitigation: a[5],
        };
        let (mut rows, mut lowered, mut pending, mut untrusted) = (0u64, 0u64, 0u64, 0u64);
        for game in 0..4_000u32 {
            let mut gate: Option<GateState> = None;
            // the row stored just before in this game, if any
            let mut before: Option<[u32; 6]> = None;
            let mut cur = [0u32; 6];
            for step in 0..(4 + next() % 12) {
                for (col, value) in cur.iter_mut().enumerate() {
                    let wide = col >= 3;
                    let grow = if wide { next() % 1500 } else { next() % 4 } as u32;
                    *value = match next() % 100 {
                        0..=59 => value.saturating_add(grow),
                        60..=74 => *value,
                        // another row's numbers, a clip, a ghost digit, a reset
                        75..=82 => *value / 2,
                        83..=87 => *value / 10,
                        88..=91 => value.saturating_mul(10).saturating_add(next() as u32 % 10),
                        92..=95 => next() as u32 % if wide { 30_000 } else { 60 },
                        _ => 0,
                    };
                }
                let held: [bool; 6] = std::array::from_fn(|_| next() % 8 == 0);
                let trusted = next() % 5 != 0;
                let split = gate.is_some() && next() % 25 == 0;
                let opens = split || gate.is_none();
                let carried = (opens && next() % 2 == 0).then(|| {
                    let board: [u32; 6] =
                        std::array::from_fn(|col| next() as u32 % if col >= 3 { 3_000 } else { 6 });
                    (counters(board), Duration::from_secs(40))
                });
                let secs = [3u64, 20, 20, 45, 120, 400][(next() % 6) as usize];
                let mut parsed = test_match("s", "unknown");
                parsed.suspect_fields = Vec::new();
                (parsed.elims, parsed.assists, parsed.deaths) = (cur[0], cur[1], cur[2]);
                (parsed.damage, parsed.healing, parsed.mitigation) = (cur[3], cur[4], cur[5]);
                let out = hold_new_reader_stats(
                    &mut parsed,
                    gate.map(|state| (state, Duration::from_secs(secs))),
                    held,
                    held,
                    split,
                    trusted,
                    carried,
                );
                if opens {
                    // a carried board is the row stored just before this one
                    before = carried.map(|(board, _)| {
                        [
                            board.elims,
                            board.assists,
                            board.deaths,
                            board.damage,
                            board.healing,
                            board.mitigation,
                        ]
                    });
                }
                gate = Some(out.state);
                untrusted += u64::from(!trusted);
                if !out.state.unread_cols().is_empty() {
                    // no row yet: with a carried board every held column is
                    // seeded, so this is a game with nothing stored
                    assert!(carried.is_none() || !opens, "game {game} step {step}");
                    pending += 1;
                    continue;
                }
                let stored = o10_stats(&parsed);
                if let Some(before) = before {
                    for (col, name) in capture_gate::COL_FIELD.iter().enumerate() {
                        if stored[col] < before[col] {
                            lowered += 1;
                            assert!(
                                parsed.suspect_fields.iter().any(|f| f == name),
                                "game {game} step {step}: {name} went {} to {} with flags {:?}",
                                before[col],
                                stored[col],
                                parsed.suspect_fields
                            );
                        }
                    }
                }
                before = Some(stored);
                rows += 1;
            }
        }
        // the run really has stored rows, lowered stats and the hard cases
        assert!(rows > 10_000, "{rows}");
        assert!(lowered > 100, "{lowered}");
        assert!(pending > 100 && untrusted > 1_000, "{pending} {untrusted}");
    }
}
