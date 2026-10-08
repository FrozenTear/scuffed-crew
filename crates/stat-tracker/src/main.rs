use stat_tracker::boundary::{self, ResultMark};
use stat_tracker::capture_gate::{self, Counters, GateState};
use stat_tracker::hero_auth::{self, HeroAuthState, HeroSource};
use stat_tracker::{capture, config, detect, ocr, parse, setup, storage, sync};

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
const REJECTED_KEEP: usize = 30;

/// How many ACCEPTED scoreboard crops are kept in `<data_dir>/debug/accepted/`.
/// A silently-corrupt accepted board (OCR drift that still passed every trust
/// gate) is otherwise unrecoverable for calibration retuning. Bounded so the
/// always-on ring never grows without limit.
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
                tracing::info!("SIGTERM received — shutting down");
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
                    "evdev init failed — no keyboard detected"
                );
                if attempt >= max_attempts {
                    anyhow::bail!(
                        "no keyboard available after {attempt} attempts ({e}) — \
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
    match text.trim().parse::<u32>() {
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
    /// and first word-OCR streak — not every mid-match tick.
    debug_ocr: bool,
    data_dir: std::path::PathBuf,
    /// Consecutive scoreboard captures that parsed but resolved no map. Drives
    /// the `debug/mapmiss/` region dump (see `EMPTY_MAP_DUMP_THRESHOLD`).
    empty_map_reads: std::sync::atomic::AtomicUsize,
}

/// Per-capture parameters decided by the session state machine at Tab time.
struct CaptureRequest {
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
    /// Board held off the previous session. Written onto the session this
    /// capture actually stores, at `carried_deferred_at`.
    carried_deferred: Option<Counters>,
    carried_deferred_hero: Option<String>,
    carried_deferred_at: Option<chrono::DateTime<Utc>>,
}

/// Package version shown by `--version` / `--help`.
///
/// Release CI sets `SST_RELEASE_VERSION` from the git tag so the binary
/// reports the tagged release version; local/dev builds fall back to
/// `CARGO_PKG_VERSION`.
fn package_version() -> &'static str {
    option_env!("SST_RELEASE_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    lock_down_umask();
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

    // One client for the whole process. An unsafe URL logs once here and
    // leaves sync off — including the startup player-name fetch — so the
    // bearer token is never sent. The daemon keeps running.
    let sync_client = open_sync_client(config.sync.as_ref());
    if let Some(client) = &sync_client {
        let creds = client.credentials();
        if sync::auth_pause_matches(&config.data_dir, &creds.server_url, &creds.token) {
            tracing::error!(
                "sync paused — the server rejected this token. Update the token in Settings, then restart the tracker or save the new URL or token."
            );
        } else {
            fetch_player_name_if_needed(&mut config, client).await;
        }
    }

    // Single wiring point for OCR debug dumps — the lib reads this switch
    // instead of re-loading config on first use; dumps land under data_dir.
    ocr::set_debug_ocr(config.debug_ocr_enabled());
    ocr::set_debug_dir(config.data_dir.join("debug"));
    // OCR pool size (Tesseract instances ≈ workers). Must run before first OCR.
    let ocr_threads = config.ocr_threads_resolved();
    ocr::set_ocr_threads(ocr_threads);
    tracing::info!(
        ocr_threads,
        configured = ?config.ocr_threads,
        "OCR workers (each holds ~23MB tessdata; set ocr_threads / STAT_TRACKER_OCR_THREADS / --ocr-threads)"
    );

    // Refuse to start alongside a live daemon. On drop the guard removes
    // daemon.pid only if the file still names this process. Held across
    // --vacuum / auto-vacuum so no second instance starts mid-compaction.
    // The store-directory flock (see LocalStore::open) is the single-writer
    // lock; this file is how the GUI finds the process.
    let _pid_guard = acquire_pid_guard(&config.data_dir)?;

    // Maintenance mode: compact the store and exit (see LocalStore::vacuum).
    if maybe_vacuum(&config.data_dir).await? {
        return Ok(());
    }

    // Log rotate, prune old pre-vacuum backups, auto-vacuum if store is bloated.
    // Must run before open_store (single-writer SurrealKV).
    if let Err(e) = storage::maintain::startup_maintenance(&config.data_dir).await {
        tracing::warn!(error = %e, "startup maintenance failed — continuing with live store");
    }

    // Tessdata generation is triggered manually via --generate-tessdata or the GUI button.
    // Don't run it at daemon startup — it can take minutes and blocks Tab capture.

    let backend = capture::detect_backend().await;
    tracing::info!(?backend, "capture backend selected");

    log_selected_output(backend, &config).await;

    let store = open_store(&config.data_dir).await?;

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
    });
    run_loop(ctx).await
}

/// Handle flags that must run before ANY initialization (version/help probes
/// used by smoke tests and humans). Returns true if a flag was handled and
/// `main` should exit successfully.
fn handle_preinit_flags() -> bool {
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("scuffed-stat-tracker {}", package_version());
        return true;
    }
    if std::env::args().any(|a| a == "--help" || a == "-h") {
        println!(
            "scuffed-stat-tracker {} — Overwatch 2 scoreboard OCR daemon\n\n\
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
             \x20 --ocr-threads N       OCR workers 1..=8 (RAM vs speed; also config/env)\n\n\
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
                eprintln!("no capture backend available — cannot list outputs");
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
                "sync disabled — refusing to send the bearer token over an unsafe server URL"
            );
            None
        }
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
                    "server has no player_name configured — set it in the web UI under My Stats → Settings"
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
                    "sync token rejected by daemon-config — pausing sync until the URL or token changes in Settings"
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
            "another daemon is already running — stop it first"
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
            poll_secs = config.auto_detect.poll_interval_secs,
            cooldown_secs = config.auto_detect.cooldown_secs,
            "auto-detect mode enabled — polling for match end screens"
        );
    }
    if config.game_process_names.is_empty() {
        tracing::info!("game-process gate disabled (game_process_names is empty)");
    } else {
        tracing::info!(
            processes = ?config.game_process_names,
            "captures gated on game process — set game_process_names in config.toml if yours differs"
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
    tracing::info!("daemon ready — press Tab in-game to capture scoreboard");

    if collect_portraits {
        tracing::info!(
            "portrait collection mode enabled — only fills missing portraits and the Doctrine stand-in; it never overwrites an existing reference"
        );
    }
}

/// The game currently in progress. Opened when the poller sees a game-start
/// screen (map vote / hero select / ban) and reused for every Tab capture until
/// the next game starts — so captures taken across hero swaps all land in one
/// session. `outcome` is filled in when the poller reads the post-match screens
/// (or recovered from a captured frame), then back-filled onto the snapshots.
struct ActiveGame {
    session_id: String,
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
}

fn active_game_path(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir.join("active_game.json")
}

/// Persist (or clear) the open-game skeleton. Fire-and-forget: losing this
/// file only degrades restart recovery, never the capture itself.
/// A game that closes without a single accepted Tab capture never reached the
/// store: `session_created` stays false, the outcome/map the poller saw live
/// only in `active_game.json` and are overwritten by the next game. Until the
/// 2026-09-01 zero-games night this was silent at INFO level (the poller had
/// happily logged "outcome confirmed" for a game that was then discarded).
/// Say so at WARN, and keep a one-line record in `debug/unrecorded_games.jsonl`
/// so the evening can be reconstructed from disk after the journal rotates.
fn note_unrecorded_game(data_dir: &std::path::Path, g: &ActiveGame, reason: &str) {
    if g.session_created {
        return;
    }
    tracing::warn!(
        session_id = %g.session_id,
        outcome = %g.outcome,
        map = g.map.as_deref().unwrap_or("?"),
        age_secs = g.opened_at.elapsed().as_secs(),
        reason,
        "game closed with NO Tab captures — it was not recorded (no scoreboard seen: Tab not pressed, keyboard grabbed, or capture rejected)"
    );
    let row = serde_json::json!({
        "closed_at": Utc::now().to_rfc3339(),
        "session_id": g.session_id,
        "outcome": g.outcome.to_string(),
        "map": g.map,
        "age_secs": g.opened_at.elapsed().as_secs(),
        "reason": reason,
    });
    let dir = data_dir.join("debug");
    if std::fs::create_dir_all(&dir).is_ok()
        && let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("unrecorded_games.jsonl"))
    {
        use std::io::Write;
        let _ = writeln!(f, "{row}");
    }
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
    if g.session_created && g.finished() && !had_outcome {
        if let Err(e) = store
            .set_session_outcome(&g.session_id, &g.outcome.to_string())
            .await
        {
            tracing::warn!(error = %e, "failed to seal session outcome");
        }
        refresh_snapshot(store, data_dir).await;
    }
    if g.session_created {
        tracing::info!(
            session_id = %g.session_id,
            outcome = %g.outcome,
            map = g.map.as_deref().unwrap_or("?"),
            reason,
            "closed session at new-game boundary"
        );
    } else {
        note_unrecorded_game(data_dir, &g, reason);
    }
}

/// Close a quiet session the way a boundary does, then upload.
///
/// The session id is left on the stored rows. `active_game.json` is removed
/// before the upload so a restart cannot close the same session again.
/// `sync_now` is the shutdown upload ([`finish_sync_on_shutdown`]). No
/// screen capture and no OCR.
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
    tracing::info!(
        session_id = %session_id,
        reason,
        "closing quiet session"
    );
    // No seal. A finished game already has its outcome. An unfinished game
    // stays Unknown — the same write a later Tab would leave behind.
    retire_active_game(st, store, data_dir, None, reason).await;
    persist_active_game(data_dir, None);
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
) {
    if let boundary::PollDecision::IgnoreContradictory { kept, ignored } = &decision {
        tracing::info!(
            kept = %kept,
            ignored = %ignored,
            session_id = st.active_game.as_ref().map(|g| g.session_id.as_str()),
            "result word ignored — it does not override the open session"
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
    let (candidates, end_screen) = match &decision {
        boundary::PollDecision::Open(open)
            if open.reason == boundary::CloseReason::EndScreenMap =>
        {
            let (armed, map) = boundary::end_screen_log_fields(current.pending_boundary, open)
                .expect("an end-screen split carries its accolade map");
            (open.candidates.clone(), Some((armed, map)))
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
                    "auto-detect: map vote — new game"
                );
            }
            boundary::CloseReason::HeroSelectOrBan => {
                tracing::info!(
                    session_id = %new_id,
                    "auto-detect: hero select/ban — new game (map vote missed)"
                );
            }
            boundary::CloseReason::EndScreenMap => {
                let (armed, accolade_map) =
                    end_screen.expect("an end-screen close was captured with its accolade map");
                let split = boundary::format_end_screen_split(armed, closed.seal, &accolade_map);
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
            if let Err(e) = store.set_session_map(&g.session_id, &map).await {
                tracing::warn!(error = %e, "failed to set session map");
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

/// What [`admit_persisted_game`] decided about `active_game.json`.
enum ActiveAdmission {
    Open(Box<ActiveGame>),
    /// The file parsed, and the game is past the idle bound (or its clock
    /// cannot be mapped onto this boot). The rows are still in the store.
    Stale {
        session_id: String,
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
    match active_game_from_persisted(persisted) {
        Some(game) => ActiveAdmission::Open(Box::new(game)),
        None => ActiveAdmission::Stale { session_id },
    }
}

fn active_game_from_persisted(p: PersistedGame) -> Option<ActiveGame> {
    let to_instant =
        |w: chrono::DateTime<Utc>| Instant::now().checked_sub((Utc::now() - w).to_std().ok()?);
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
    })
}

/// Upload unsynced rows, then restore the session with [`startup_session`].
///
/// A stale skeleton is deleted before the upload so the next start cannot
/// drop it a second time. [`startup_session`] then sees no file and returns
/// an empty session. An open game is left on disk and restored afterwards,
/// including `last_game_open`, so the debounce survives a restart. The
/// upload is the same path as shutdown. Rows stay in the local store.
async fn recover_or_sync_active_game<F, Fut>(
    data_dir: &std::path::Path,
    sync_now: F,
) -> SessionState
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let dropped = match admit_persisted_game(data_dir) {
        ActiveAdmission::Stale { session_id } => {
            persist_active_game(data_dir, None);
            Some(session_id)
        }
        ActiveAdmission::Open(_) | ActiveAdmission::Absent => None,
    };
    if let Some(session_id) = &dropped {
        tracing::info!(
            session_id = %session_id,
            "dropped stale active game — syncing unsynced rows so the match is not lost"
        );
    }
    sync_now().await;
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

/// Quiet time before a finished game is closed when no further capture
/// arrives. Counted from the last capture (the result itself, or a
/// post-match Tab). Never shorter than [`POST_MATCH_GRACE`].
fn finished_game_close_after(configured_secs: u64) -> std::time::Duration {
    std::time::Duration::from_secs(configured_secs.max(POST_MATCH_GRACE.as_secs()))
}

/// Why the command-tick timer is closing the open session. Not a new-game
/// boundary: the session id stays, and no outcome is invented.
fn quiet_close_reason(
    game: &ActiveGame,
    now: Instant,
    close_after: std::time::Duration,
) -> Option<&'static str> {
    let close_after = close_after.max(POST_MATCH_GRACE);
    if game.finished() {
        let grace_expired = game
            .outcome_recorded_at
            .is_none_or(|recorded| now.saturating_duration_since(recorded) > POST_MATCH_GRACE);
        let quiet = now.saturating_duration_since(game.last_activity) >= close_after;
        if grace_expired && quiet {
            return Some("finished game closed after quiet period");
        }
        return None;
    }
    if now.saturating_duration_since(game.last_activity) > UNFINISHED_SESSION_IDLE {
        return Some("unfinished game closed after idle bound");
    }
    None
}

/// How long an unfinished session stays reusable with no recorded activity.
/// If the poller misses the outcome AND the next game's start screens (likely
/// under Tab starvation, or with auto-detect off), an unbounded session would
/// absorb every capture that follows — yesterday's unfinished game swallowing
/// today's first Tab. Sized comfortably above a long match plus queue time.
const UNFINISHED_SESSION_IDLE: std::time::Duration = std::time::Duration::from_secs(20 * 60);

/// Minimum age of a poller-opened game before a banner-color outcome read off
/// a Tab frame is trusted. The color-flood detector can false-positive on
/// heavy mid-fight red vignettes; a real banner can't appear this early into
/// a match. The result-header text path stays available regardless, and a
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

/// Whether a Tab capture should open a fresh session instead of reusing the
/// active one.
fn should_start_fresh_session(game: Option<&ActiveGame>, now: Instant) -> bool {
    match game {
        // No game open — daemon started mid-match or the start screen was missed.
        None => true,
        // Mid-game capture — unless the session has been idle so long it
        // can't plausibly be the same game.
        Some(g) if !g.finished() => now.duration_since(g.last_activity) > UNFINISHED_SESSION_IDLE,
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
        tracing::info!(?candidates, session_id = %sid, "auto-detect: map vote — new game");
    } else {
        tracing::info!(
            session_id = %sid,
            "auto-detect: hero select/ban — new game (map vote missed)"
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
                "map read is not a vote candidate — dropping as misread"
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
    let gate_prev = gate_prev_for_store(req.prev_gate, req.awaiting_first_board);
    let gate = capture_gate::apply_gate_with_trust(
        gate_prev,
        facts.counters,
        facts.suspect,
        split,
        facts.trusted_cells,
    );
    let (map_name, map_source) = if split {
        resolved_map(None, facts.map_from_panel, facts.parsed_map, &[])
    } else {
        resolved_map(
            req.session_map.as_deref(),
            facts.map_from_panel,
            facts.parsed_map,
            &req.map_candidates,
        )
    };
    let carried = carried_counters_to_write(req.carried_deferred, split, req.deferred_imported);
    let carried_at = req
        .carried_deferred_at
        .unwrap_or(captured_at - chrono::Duration::seconds(1));
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
    }
}

/// Everything the blocking vision/OCR pass extracts from one Tab frame.
/// Replaces a 9-tuple whose adjacent same-typed fields were swap-prone.
struct FrameAnalysis {
    /// Result read off this frame only. A split stores this and never the
    /// inherited session outcome.
    frame_outcome: detect::MatchOutcome,
    /// Full-image OCR (hero/map name lookup); the per-cell rows carry stats.
    ocr: Result<ocr::OcrResult, Box<dyn std::error::Error + Send + Sync>>,
    /// Column-calibrated per-cell OCR rows, one per scoreboard row.
    rows: Vec<ocr::RowOcrResult>,
    /// Portrait template match: (hero file stem, confidence).
    portrait_hero: Option<(String, f64)>,
    /// Career-panel hero title (most reliable source when present).
    career_hero: Option<String>,
    /// Top-bar map label OCR.
    map_from_panel: Option<String>,
    /// Cropped scoreboard (portrait auto-collection reads from it).
    scoreboard: image::DynamicImage,
    /// The player's row index, by name match or brightness highlight.
    player_row_idx: Option<usize>,
    /// Detected team size (5 or 6) — portrait geometry depends on it.
    team_size: usize,
    /// The full frame, kept for the rejected-capture archive.
    frame: image::DynamicImage,
}

/// Result of the blocking vision pass: a full analysis, or a cheap rejection
/// by the pre-OCR preflight before any Tesseract/portrait work ran.
enum FrameAnalysisOutcome {
    Analyzed(Box<FrameAnalysis>),
    /// The frame lacks scoreboard row structure (menu, transition, black
    /// frame) — carried back with the frame so it lands in debug/rejected.
    NotAScoreboard {
        outcome: detect::MatchOutcome,
        frame: image::DynamicImage,
        dip_count: usize,
    },
}

/// What a Tab capture actually did, reported back to the session state machine.
struct CaptureReport {
    /// A snapshot row was written (and the session row created if this was the
    /// session's first capture). False = rejected by a trust gate.
    recorded: bool,
    /// Outcome stored on the snapshot — may have been recovered from the frame
    /// itself (banner colors / header text) when the game's outcome was still
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
        carried_deferred: g.deferred,
        carried_deferred_hero: g.deferred_hero.clone(),
        carried_deferred_at: g.deferred_at,
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
    /// The boundary plan's `row_counts` is this same bit: a raw-text
    /// fallback does not count as a stat-reset row.
    trusted_cells: bool,
    row_id: Option<u32>,
    hero: &'a str,
    map_from_panel: Option<&'a str>,
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
    boundary::plan_capture(&boundary::CapturePlanInput {
        prev_gate: prev_gate.as_ref(),
        baseline: baseline.as_ref(),
        streak: req.reset_streak,
        cur: facts.counters,
        suspect: facts.suspect,
        create_session: req.create_session,
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
    })
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
        Ok(report) if !report.recorded => {
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
                    "outcome recovered from rejected capture frame — adopting"
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
            if let Some(g) = st.active_game.as_mut().filter(|g| g.session_id == sid) {
                g.session_created = true;
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
                if g.deferred_imported {
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
                        if let Err(e) = store.set_session_map(&g.session_id, &map).await {
                            tracing::warn!(error = %e, "failed to set session map");
                        }
                    } else if agrees && untrusted && incoming_trusted {
                        // A later top bar that names the map already stored
                        // upgrades a text fallback. The name stays.
                        g.map_source = report.map_source;
                    }
                }
                if !g.finished() && !matches!(report.outcome, detect::MatchOutcome::Unknown) {
                    g.record_outcome_at(report.outcome, now);
                    g.note_result(report.outcome, true);
                    g.pending_boundary = false;
                    tracing::info!(
                        outcome = ?report.outcome,
                        session_id = %g.session_id,
                        "outcome recovered from captured frame — back-filling session"
                    );
                    if let Err(e) = store
                        .set_session_outcome(&g.session_id, &g.outcome.to_string())
                        .await
                    {
                        tracing::warn!(error = %e, "failed to back-fill session outcome");
                    }
                }
                persist_active_game(data_dir, Some(g));
            }
            true
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
                    "result word read — awaiting agreeing read"
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
        accolade_map,
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
async fn apply_poll_tick(
    st: &mut SessionState,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    tick: &ResolvedWordTick,
    start_screen: Option<boundary::StartScreen>,
    now: Instant,
    debounce: std::time::Duration,
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
        apply_poll_decision(st, store, data_dir, decision, now).await;
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
    }
}

/// Resume after a suspend. Drops the volatile windows and re-admits the
/// active game through the same recovery startup uses, so `last_game_open`
/// is `opened_at` again and a ban inside the debounce does not split.
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
async fn run_loop(ctx: Arc<DaemonCtx>) -> anyhow::Result<()> {
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
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    // Failed open used to wait only on Ctrl+C. SIGTERM was registered above
    // and never polled, so `systemctl stop` hit TimeoutStopSec then SIGKILL,
    // and exit 0 kept Restart=on-failure from trying again. The pid file
    // (acquired in main) made the GUI show "running" the whole time.
    let mut kbd = match open_keyboard_or_shutdown(&mut sigterm).await? {
        Some(stream) => stream,
        None => return Ok(()),
    };

    let poll_interval = tokio::time::Duration::from_secs(auto_detect.poll_interval_secs);
    let new_game_debounce = std::time::Duration::from_secs(auto_detect.cooldown_secs);
    let tab_debounce = std::time::Duration::from_secs(3);
    let finished_close = ctx.finished_game_close;

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
            "sync paused — token rejected. Update the token in Settings; sync resumes when the URL or token changes."
        );
    }

    // Startup upload uses the shutdown path: join nothing, then upload once,
    // including when a stale skeleton was just dropped. Auth rejection skips
    // the upload inside `finish_sync_on_shutdown`. The session comes from
    // `startup_session`, so `last_game_open` stays the open instant and the
    // debounce survives the restart.
    let startup_client = sync_client.cloned();
    let startup_backoff = Arc::clone(&sync_backoff);
    let startup_store = store.clone();
    let startup_dir = data_dir.to_path_buf();
    let mut st = recover_or_sync_active_game(data_dir, || {
        let startup_client = startup_client.clone();
        let startup_backoff = Arc::clone(&startup_backoff);
        let startup_store = startup_store.clone();
        let startup_dir = startup_dir.clone();
        async move {
            finish_sync_on_shutdown(
                startup_client.as_ref(),
                &startup_backoff,
                None,
                &startup_store,
                &startup_dir,
            )
            .await;
        }
    })
    .await;
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
                            tracing::debug!("Tab ignored — game process not running");
                            continue;
                        }

                        if let Some(last) = st.last_tab_capture
                            && last.elapsed() < tab_debounce {
                                tracing::debug!("Tab debounced — ignoring rapid press");
                                continue;
                            }

                        // Single-flight: OCR of the previous Tab may still be
                        // running (it takes seconds) — don't stack captures.
                        if capture_task.as_ref().is_some_and(|t| !t.is_finished()) {
                            tracing::debug!("Tab ignored — a capture is already in progress");
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
                        tracing::error!(error = %e, "keyboard devices lost — attempting to reopen");
                        match detect::MultiKeyboardStream::open() {
                            Ok(new_kbd) => {
                                kbd = new_kbd;
                                tracing::info!("keyboard monitoring reopened");
                            }
                            Err(e2) => {
                                tracing::error!(error = %e2, "failed to reopen keyboard — exiting");
                                drain_capture(capture_task.take()).await;
                                let task = sync_task.take();
                                if let Some(client) = sync_client {
                                    drain_sync_then(task, || try_sync(store, client, data_dir)).await;
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
                    tracing::debug!("poll tick skipped — Tab capture in flight");
                    continue;
                }
                let outcome_only = poll_outcome_only_while_tab_busy(tab_in_flight, &st, now);
                if outcome_only {
                    tracing::debug!(
                        "cheap outcome poll — Tab capture in flight during end-reel wake"
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
                        tracing::trace!("poll tick skipped — adaptive slow cadence mid-match");
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
                        let (signal, phase, accolade_map, end_reel, stability) = tokio::task::spawn_blocking(move || {
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
                            let signal =
                                detect::match_end::detect_outcome_signal_polled(&img, &rgb, &mut stability);
                            // Cheap wake path: skip phase + accolade-map OCR
                            // so Tab cell OCR keeps the pool. Outcome signal
                            // and end-reel refresh still run (same confirm /
                            // on-hit dump machinery below).
                            let accolade_map = if outcome_only {
                                // Tab cell OCR is in flight. This tick still
                                // reads the outcome word, but not the map, so
                                // two of these ticks confirm with no map and
                                // seal onto the open session.
                                None
                            } else {
                                // Only a result-word tick reads the map. The
                                // rank screen does not. A banner tick is the
                                // gameplay HUD under a result flash, so that
                                // crop is not a map.
                                match &signal {
                                    Some((_, detect::match_end::OutcomeSource::ResultWord)) => {
                                        detect::match_end::read_accolade_map(&img)
                                    }
                                    _ => None,
                                }
                            };
                            let phase = if outcome_only {
                                detect::GamePhase::Unknown
                            } else {
                                detect::match_start::detect_phase_polled(&img, &rgb, &mut stability)
                            };
                            // Wake hint only — does not confirm an outcome.
                            // Ban Heroes is a distinct hook (`detect_ban_screen`)
                            // and is excluded inside detect_end_reel; do not
                            // treat GamePhase::HeroBan as end_reel_wake_until.
                            let end_reel = detect::match_end::detect_end_reel(&img, &rgb);
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
                            (signal, phase, accolade_map, end_reel, stability)
                        }).await.unwrap_or_else(|_| {
                            (None, detect::GamePhase::Unknown, None, false, detect::stability::FrameStability::default())
                        });
                        st.ocr_stability = stability;

                        if end_reel {
                            let already_awake = st
                                .end_reel_wake_until
                                .is_some_and(|t| Instant::now() < t);
                            st.end_reel_wake_until = Some(Instant::now() + END_REEL_WAKE);
                            if !already_awake {
                                tracing::info!(
                                    hold_secs = END_REEL_WAKE.as_secs(),
                                    "end-reel / POTG — holding full poll cadence"
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
                // Tab. The upload is the shutdown sync path.
                {
                    let client = sync_client.cloned();
                    let backoff = Arc::clone(&sync_backoff);
                    let store_for_sync = store.clone();
                    let dir_for_sync = data_dir.to_path_buf();
                    close_quiet_session_and_sync(
                        &mut st,
                        store,
                        data_dir,
                        Instant::now(),
                        finished_close,
                        || {
                            let task = sync_task.take();
                            let client = client.clone();
                            let backoff = Arc::clone(&backoff);
                            let store_for_sync = store_for_sync.clone();
                            let dir_for_sync = dir_for_sync.clone();
                            async move {
                                finish_sync_on_shutdown(
                                    client.as_ref(),
                                    &backoff,
                                    task,
                                    &store_for_sync,
                                    &dir_for_sync,
                                )
                                .await;
                            }
                        },
                    )
                    .await;
                }
                maybe_resume_sync_after_settings_change(sync_client, &sync_backoff, data_dir);
                // Suspend detection (m4): after a sleep, every Instant-based
                // window believes no time passed. Treat resume like a daemon
                // restart — drop the volatile windows and re-admit the active
                // game only through the wall-clock recovery bound (the on-disk
                // skeleton was persisted with correct wall times pre-suspend).
                // A skeleton past that bound is dropped and its rows are synced.
                let mono = st.suspend_probe.0.elapsed();
                let wall = (Utc::now() - st.suspend_probe.1).to_std().unwrap_or(mono);
                if wall > mono + SUSPEND_RESET_GAP {
                    tracing::info!(
                        gap_secs = (wall - mono).as_secs(),
                        "suspend/clock-jump detected — resetting session windows"
                    );
                    // Upload first, including a stale skeleton. Then
                    // `resume_after_suspend` re-admits through `startup_session`,
                    // so the debounce comes back from `opened_at`.
                    let client = sync_client.cloned();
                    let backoff = Arc::clone(&sync_backoff);
                    let store_for_sync = store.clone();
                    let dir_for_sync = data_dir.to_path_buf();
                    let _restored = recover_or_sync_active_game(data_dir, || {
                        let task = sync_task.take();
                        let client = client.clone();
                        let backoff = Arc::clone(&backoff);
                        let store_for_sync = store_for_sync.clone();
                        let dir_for_sync = dir_for_sync.clone();
                        async move {
                            finish_sync_on_shutdown(
                                client.as_ref(),
                                &backoff,
                                task,
                                &store_for_sync,
                                &dir_for_sync,
                            )
                            .await;
                        }
                    })
                    .await;
                    resume_after_suspend(&mut st, data_dir);
                }
                st.suspend_probe = (Instant::now(), Utc::now());

                let cmds = storage::read_commands(data_dir);
                if !cmds.is_empty() {
                    for (cmd_file, cmd) in &cmds {
                        tracing::info!(?cmd, "applying GUI command");
                        // Keep the in-memory game consistent when the command
                        // targets the active session, so the poller can't
                        // overwrite a manual edit or resurrect a deleted game.
                        // (Idempotent — safe to re-run if the apply below
                        // fails and the command retries next tick.)
                        match cmd {
                            storage::StoreCommand::SetOutcome { session_id, outcome } => {
                                if let Some(g) = st.active_game.as_mut()
                                    && g.session_id == *session_id {
                                        g.record_outcome(outcome.parse().unwrap_or(detect::MatchOutcome::Unknown));
                                        persist_active_game(data_dir, Some(g));
                                    }
                            }
                            storage::StoreCommand::DeleteSession { session_id } => {
                                if st.active_game.as_ref().is_some_and(|g| g.session_id == *session_id) {
                                    st.active_game = None;
                                    persist_active_game(data_dir, None);
                                }
                            }
                            storage::StoreCommand::EditMatch { session_id, edit } => {
                                // If the edit corrects the outcome of the still-active
                                // game, keep the in-memory copy consistent so the poller
                                // can't overwrite it (mirrors SetOutcome). Numeric/label
                                // edits target finished games and need no in-memory sync.
                                if let Some(g) = st.active_game.as_mut()
                                    && g.session_id == *session_id
                                    && let Some(outcome) = &edit.outcome {
                                        g.record_outcome(outcome.parse().unwrap_or(detect::MatchOutcome::Unknown));
                                        persist_active_game(data_dir, Some(g));
                                    }
                            }
                            storage::StoreCommand::ResolveSegment { .. } => {
                                // Hero-segment confirm/dismiss only relabels a
                                // finished game's derived timeline; the active
                                // game holds no timeline, so nothing to sync.
                            }
                        }
                        // Two-phase: the file is only removed after a
                        // successful apply, so a crash or store error here
                        // retries the edit instead of losing it.
                        match store.apply_command(cmd).await {
                            Ok(()) => storage::remove_command_file(cmd_file),
                            Err(e) => {
                                tracing::warn!(error = %e, "GUI command failed — will retry")
                            }
                        }
                    }
                    refresh_snapshot(store, data_dir).await;
                } else {
                    // Trailing edge of the snapshot debounce: a refresh that
                    // was deferred inside the window flushes here once due,
                    // so the GUI never waits on the *next* mutation.
                    flush_snapshot_if_due(store, data_dir).await;
                }
            }
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("shutting down");
                drain_capture(capture_task.take()).await;
                let task = sync_task.take();
                finish_sync_on_shutdown(sync_client, &sync_backoff, task, store, data_dir).await;
                flush_snapshot_if_dirty(store, data_dir).await;
                return Ok(());
            }
            _ = sigterm.recv() => {
                tracing::info!("SIGTERM received — shutting down");
                drain_capture(capture_task.take()).await;
                let task = sync_task.take();
                finish_sync_on_shutdown(sync_client, &sync_backoff, task, store, data_dir).await;
                flush_snapshot_if_dirty(store, data_dir).await;
                return Ok(());
            }
        }
    }
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
    allow_banner_recovery: bool,
    session_map_known: bool,
) -> FrameAnalysisOutcome {
    // Outcome: prefer the open game's result (read off the accolade
    // screen by the poller); else color-flood detection (only when the
    // caller deems a banner plausible — see MIN_BANNER_SESSION_AGE); else
    // read the VICTORY/DEFEAT header text off this frame. The last step
    // recovers the case where the poller missed the screens and we're
    // sitting on a post-match scoreboard that prints the result header.
    let detected = if allow_banner_recovery {
        detect::match_end::detect_outcome(&img)
    } else {
        detect::MatchOutcome::Unknown
    };
    let frame_outcome = if matches!(detected, detect::MatchOutcome::Unknown) {
        detect::match_end::detect_outcome_text(&img)
    } else {
        detected
    };
    // The open session's outcome wins on a capture that stays in that session.
    // A gap split keeps this frame's own header. A reset split stores Unknown.
    let outcome = if game_outcome.is_decided() {
        game_outcome
    } else {
        frame_outcome
    };

    let scoreboard = ocr::preprocess::crop_scoreboard(&img);
    // Pre-OCR preflight (H1): a few milliseconds of pixel work that
    // rejects menus/transitions/gameplay/black frames before the
    // expensive portrait-match + calibration + row-OCR pipeline runs.
    // Two independent signals, either accepts: the saturation row-dip
    // scan (fails on the desaturated endorse-phase board) or the
    // brightness-based header stat labels (fail on some vivid boards).
    // Validated on 38 captured frames: all real boards pass, 29/33
    // garbage frames rejected. The OCR-based looks_like_scoreboard gate
    // downstream stays as the final arbiter for frames that pass.
    let row_scan = detect::hero_portrait::scan_rows(&scoreboard);
    if !row_scan.looks_like_scoreboard()
        && !(3..=10).contains(&ocr::preprocess::header_label_groups(&scoreboard).len())
    {
        return FrameAnalysisOutcome::NotAScoreboard {
            outcome,
            frame: img,
            dip_count: row_scan.dip_count,
        };
    }
    let team_size = row_scan.team_size();
    // Pass team_size into portrait match + cell OCR so neither re-detects
    // size or re-crops the full scoreboard (P7).
    let player_match = matcher.match_player_hero_with_team_size(&scoreboard, team_size);
    let portrait_match = player_match
        .as_ref()
        .map(|(name, conf, _)| (name.clone(), *conf));
    let brightness_row_idx = player_match.map(|(_, _, idx)| idx);

    let rows = ocr::recognize_scoreboard_cells_pre_cropped(&scoreboard, team_size);

    // Player row: if a player name is configured, scan ALL rows (both teams)
    // for a name match — this handles replays and post-match screens where the
    // player may be on team 2. Fall back to brightness-detected row otherwise.
    let row_idx = player_name_owned
        .as_deref()
        .and_then(|name| parse::find_player_row_by_name(&rows, name))
        .or(brightness_row_idx);

    // Career-panel hero title. Guard against garbage OCR (happens when there
    // is no career panel — replay, post-match — by requiring the result to
    // actually match a known hero name, which match_hero_in_text already does).
    let career_hero = ocr::recognize_region(&ocr::preprocess::crop_career_hero(&img))
        .ok()
        .and_then(|t| parse::match_hero_in_text(&t));
    let map_from_panel = ocr::recognize_region(&ocr::preprocess::crop_map_name(&img))
        .ok()
        .and_then(|t| parse::match_map_in_text(&t));

    // Full-board OCR exists only to supply raw text for hero/map name
    // lookup and the name-in-raw-text stats fallback. On the happy path —
    // player row found and parseable, hero identified (career panel or
    // portrait match), map already known — it is pure redundancy
    // (adaptive preprocessing plus up to three threshold sweeps), so run
    // it lazily. A portrait match alone satisfies the hero requirement:
    // replay/post-match layouts have no career panel, and the raw-text
    // hero guess loses to the portrait in the priority order anyway (H6).
    let cells_parse = parse::parse_scoreboard_cells(&rows, row_idx, "", "unknown", None).is_some();
    let hero_identified = career_hero.is_some() || portrait_match.is_some();
    let need_full_ocr =
        !cells_parse || !hero_identified || (!session_map_known && map_from_panel.is_none());
    let ocr = if need_full_ocr {
        ocr::recognize(&img)
    } else {
        tracing::debug!("skipping full-board OCR — cell path supplied everything");
        Ok(ocr::OcrResult {
            raw_text: String::new(),
            confidence: 0,
        })
    };

    FrameAnalysisOutcome::Analyzed(Box::new(FrameAnalysis {
        frame_outcome,
        ocr,
        rows,
        portrait_hero: portrait_match,
        career_hero,
        map_from_panel,
        scoreboard,
        player_row_idx: row_idx,
        team_size,
        frame: img,
    }))
}

/// Row id passed into [`boundary::plan_capture`]. `None` when the player
/// row was not identified.
fn identified_row_id(player_row_idx: Option<usize>) -> Option<u32> {
    player_row_idx.map(|idx| idx as u32)
}

/// Session a stat split just opened. A decided `outcome` is the current
/// frame's header (gap split) and stamps grace. Unknown is a reset split.
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
    let played_at = SurrealDatetime::from(played_at);
    let row = storage::PersonalMatch {
        id: None,
        hero: hero.to_string(),
        map_name: map_name.to_string(),
        game_mode: String::new(),
        role: parse::guess_role_public(hero),
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
    }
}

/// Session row and carried board for one staged capture. Production and the
/// night harness both write through this. The current board's own snapshot
/// stays with the caller: production inserts the parsed row, the harness
/// inserts the accepted counters.
async fn write_staged_rows(
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    staged: &StagedCapture,
    hero: &str,
    role: &str,
    now: SurrealDatetime,
) -> anyhow::Result<bool> {
    let mut create = staged.create_session;
    if let Some(counters) = staged.carried {
        let played_at = staged.carried_at;
        if create {
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
            played_at,
        )
        .await?;
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

async fn handle_capture(ctx: &DaemonCtx, req: CaptureRequest) -> anyhow::Result<CaptureReport> {
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

    tracing::info!("Tab detected — capturing screen (hold Tab to keep scoreboard visible)");
    let img = capture::capture_screen_output(backend, capture_output)
        .await
        .map_err(anyhow::Error::from_boxed)
        .context("screen capture failed")?;

    let matcher = Arc::clone(&ctx.portrait_matcher);
    // Clone player_name so the blocking closure can own it.
    let player_name_owned = player_name.map(|s| s.to_string());
    let session_map_known = session_map.is_some();
    let analysis = tokio::task::spawn_blocking(move || {
        analyze_frame(
            img,
            matcher,
            player_name_owned,
            game_outcome,
            allow_banner_recovery,
            session_map_known,
        )
    })
    .await?;
    let analysis = match analysis {
        FrameAnalysisOutcome::Analyzed(a) => *a,
        FrameAnalysisOutcome::NotAScoreboard {
            outcome,
            frame,
            dip_count,
        } => {
            tracing::warn!(
                dip_count,
                "capture rejected by pre-OCR preflight — no scoreboard row structure (saved to debug/rejected)"
            );
            save_rejected_frame(data_dir, frame, "preflight");
            return Ok(CaptureReport {
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
            });
        }
    };
    let FrameAnalysis {
        mut frame_outcome,
        ocr,
        rows,
        portrait_hero,
        career_hero,
        map_from_panel,
        scoreboard: scoreboard_img,
        player_row_idx,
        team_size,
        frame: frame_img,
    } = analysis;
    let ocr_result = ocr
        .map_err(anyhow::Error::from_boxed)
        .context("full-board OCR failed")?;

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
    tracing::info!(
        confidence = ocr_result.confidence,
        text_preview = &ocr_result.raw_text[..preview_end],
        "OCR result"
    );

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

    // Trust gate: don't parse frames that don't look like a scoreboard (menus,
    // replay browser, desktop). Better to record nothing than to scrape stats
    // out of a random screen.
    if !parse::looks_like_scoreboard(&rows) {
        tracing::warn!(
            rows = rows.len(),
            "capture rejected — frame does not look like a scoreboard (saved to debug/rejected)"
        );
        save_rejected_frame(data_dir, frame_img, "noscoreboard");
        return Ok(CaptureReport {
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
        });
    }

    let mut outcome_label = outcome.to_string();
    // CG-4 C: mutates across this capture; carried back on CaptureReport.
    let mut hero_auth = req.hero_auth.clone();

    let scoreboard_read = parse::read_scoreboard(
        &rows,
        player_row_idx,
        &ocr_result.raw_text,
        &outcome_label,
        player_name,
    );
    match scoreboard_read {
        Ok(parsed_read) => {
            let trusted_cells = parsed_read.trusted_cells;
            let mut parsed = parsed_read.matched;
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
            let portrait = portrait_hero
                .as_ref()
                .map(|(name, conf)| (name.as_str(), *conf));
            let (hero, source, next_auth) = hero_auth::resolve_hero(
                career_hero.as_deref(),
                portrait,
                &parsed.hero,
                &hero_auth,
                parse::canonical_hero,
            );
            hero_auth = next_auth;
            parsed.hero = hero;
            parsed.role = parse::guess_role_public(&parsed.hero);
            let source_label = match source {
                HeroSource::CareerPanel => "career_panel",
                HeroSource::Portrait => "portrait",
                HeroSource::Held => "held",
                HeroSource::OcrText => "ocr_text",
            };
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
            // template itself).
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
            let suspect = parse::player_row_suspect_mask(&rows, player_row_idx);

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
                row_id: identified_row_id(player_row_idx),
                hero: &parsed.hero,
                map_from_panel: map_from_panel.as_deref(),
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
                return Ok(skipped_capture_report(
                    &plan,
                    session_id,
                    matches!(source, HeroSource::CareerPanel),
                    hero_auth,
                    plan.defer.then(|| parsed.hero.clone()),
                    plan.defer.then_some(captured_at),
                ));
            }
            let staged = stage_capture(&req, &plan, &facts, captured_at);
            let split = staged.split;
            outcome = staged.outcome;
            outcome_label = staged.outcome_label.clone();
            parsed.outcome = outcome_label.clone();
            let target_session = staged.target_session.clone();
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
            parsed.elims = gate.accepted.elims;
            parsed.assists = gate.accepted.assists;
            parsed.deaths = gate.accepted.deaths;
            parsed.damage = gate.accepted.damage;
            parsed.healing = gate.accepted.healing;
            parsed.mitigation = gate.accepted.mitigation;

            parsed.map_name = staged.map_name.clone();

            // The session is owned by the active game (map-vote → accolade). The
            // first capture creates the session row; later captures (including hero
            // swaps and the post-match scoreboard) append to the same session.
            parsed.session_id = target_session.clone();
            // A board deferred on this same session is dropped when the capture
            // stays. A split, or a board carried in from the session that closed,
            // is written onto the session this row lands on. The same function
            // writes the night harness.
            let created_this_capture =
                write_staged_rows(store, data_dir, &staged, &parsed.hero, &parsed.role, now)
                    .await?;

            tracing::info!(
                hero = %parsed.hero,
                map = %parsed.map_name,
                elims = parsed.elims,
                deaths = parsed.deaths,
                "parsed scoreboard"
            );
            let recorded_map = staged.recorded_map.clone();
            storage::append_match_log(data_dir, &parsed);
            store
                .insert_match(parsed)
                .await
                .map_err(anyhow::Error::from_boxed)
                .context("store insert failed")?;

            // Dump the accepted scoreboard crop to a bounded ring so a corrupt
            // ACCEPTED board is diagnosable after the fact: tonight's corruption
            // was undiagnosable because only rejected frames were ever saved.
            save_accepted_frame(data_dir, scoreboard_img);

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

            Ok(CaptureReport {
                recorded: true,
                outcome,
                map: recorded_map,
                map_source: staged.map_source,
                session_id: target_session,
                split,
                armed_reset: false,
                ignore_row: plan.ignore_row,
                reset_streak: plan.reset_streak,
                reset_baseline: plan.reset_baseline,
                baseline_row: plan.baseline_row,
                refresh_baseline: plan.refresh_baseline,
                clear_hint: plan.clear_hint,
                count_progress: plan.count_progress,
                career_panel: matches!(source, HeroSource::CareerPanel),
                held_counters: None,
                held_hero: None,
                gate_state: Some(staged.gate.state),
                hero_auth,
                seal: plan.seal,
                close_reason: plan.close_reason,
                held_at: None,
            })
        }
        Err(miss) => {
            // Recording another row's stats would be worse than recording nothing.
            // A found row whose cells did not parse is not the same failure as a
            // frame where the row itself is missing. CellsUnreadable also fires
            // when the name shows up only in chat or the kill feed.
            let reason = match miss {
                parse::ScoreboardMiss::PlayerRowNotFound => {
                    tracing::warn!(
                        "capture rejected: player row not found (saved to debug/rejected; \
                     set player_name in config.toml if it is missing)"
                    );
                    "noplayerrow"
                }
                parse::ScoreboardMiss::CellsUnreadable => {
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
                        "capture rejected: stat cells unreadable (saved to debug/rejected)"
                    );
                    "unreadable"
                }
            };
            save_rejected_frame(data_dir, frame_img, reason);
            Ok(CaptureReport {
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

/// Archive a frame whose capture was rejected by a trust gate, for diagnosis.
/// Runs the PNG encode off the async runtime; fire-and-forget.
fn save_rejected_frame(data_dir: &std::path::Path, img: image::DynamicImage, reason: &'static str) {
    let dir = data_dir.join("debug").join("rejected");
    tokio::task::spawn_blocking(move || {
        save_frame_ring(&dir, &format!("rejected_{reason}"), &img, REJECTED_KEEP);
    });
}

/// Archive the scoreboard crop of an ACCEPTED capture into a bounded ring, so a
/// board that OCR'd wrong but still passed every trust gate can be inspected
/// after the fact (the capture gate holds bad cells, but the underlying crop is
/// the only way to retune calibration). Fire-and-forget; encode off-runtime.
fn save_accepted_frame(data_dir: &std::path::Path, board: image::DynamicImage) {
    let dir = data_dir.join("debug").join("accepted");
    tokio::task::spawn_blocking(move || {
        save_frame_ring(&dir, "accepted", &board, ACCEPTED_KEEP);
    });
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
) -> sync::SyncAttempt {
    let client = client.clone();
    let outcome = try_sync_with(store, data_dir, {
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
            tracing::debug!("sync skipped — backing off before the next attempt");
        }
        return;
    }
    let store = store.clone();
    let client = client.clone();
    let data_dir = data_dir.to_path_buf();
    let backoff = Arc::clone(backoff);
    *sync_task = Some(tokio::spawn(async move {
        let outcome = try_sync(&store, &client, &data_dir).await;
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
                "sync upload failed — backing off so a down server is not hammered"
            );
        }
        sync::SyncAttempt::RateLimited { .. } => {
            tracing::warn!(
                retry_in_secs = clock.retry_after(now).as_secs(),
                "sync rate-limited — backing off before the next upload"
            );
        }
        sync::SyncAttempt::AuthRejected => {
            tracing::error!(
                "sync token rejected — pausing until the server URL or token changes in Settings"
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
            tracing::info!("sync resumed — server URL or token changed");
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                "sync stays paused — the saved server URL is not safe for the token"
            );
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
    let rejected = backoff
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .auth_rejected();
    if rejected {
        tracing::warn!("sync paused — token rejected; skipping the shutdown upload");
        drain_sync_then(task, || std::future::ready(())).await;
        return;
    }
    if let Some(client) = client {
        drain_sync_then(task, || try_sync(store, client, data_dir)).await;
    } else {
        drain_sync_then(task, || std::future::ready(())).await;
    }
}

/// Read unsynced rows, upload, then mark synced only where `sync_rev` is
/// still the revision captured here. `upload` is the HTTP call (or a test
/// double). It runs after the read and before the mark, which is the window
/// a local outcome/map/hero/GUI write can revise a row.
async fn try_sync_with<F, Fut>(
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    upload: F,
) -> sync::SyncAttempt
where
    F: FnOnce(Vec<storage::PersonalMatch>, Vec<String>) -> Fut,
    Fut: std::future::Future<
            Output = Result<scuffed_types::api::StatsUploadResponse, sync::SyncUploadError>,
        >,
{
    // Errors are stringified immediately: `Box<dyn Error>` isn't `Send`, and
    // this future runs on a spawned task.
    let unsynced = match store.get_unsynced().await.map_err(|e| e.to_string()) {
        Ok(u) => u,
        Err(e) => {
            tracing::error!(error = %e, "failed to query unsynced matches");
            return sync::SyncAttempt::NoServerCall;
        }
    };
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
    let to_upload = storage::latest_per_game(newest_first);
    tracing::info!(
        rows = claims.len(),
        games = to_upload.len(),
        tombstones = tombstones.len(),
        "syncing unsynced matches"
    );
    match upload(to_upload, tombstones.clone()).await {
        Ok(resp) => {
            tracing::info!(
                inserted = resp.inserted,
                skipped = resp.skipped,
                deleted = resp.deleted,
                "sync complete"
            );
            if let Err(e) = store.mark_synced(&claims).await.map_err(|e| e.to_string()) {
                tracing::error!(error = %e, "failed to mark matches as synced");
            }
            if let Err(e) = store
                .clear_tombstones(tombstones)
                .await
                .map_err(|e| e.to_string())
            {
                tracing::error!(error = %e, "failed to clear acknowledged tombstones");
            }
            // Sync flips `synced` flags — GUI must see that promptly.
            refresh_snapshot_force(store, data_dir).await;
            sync::SyncAttempt::Uploaded
        }
        Err(e) => {
            let attempt = e.attempt();
            match attempt {
                sync::SyncAttempt::RateLimited { .. } => {
                    tracing::warn!(error = %e, "sync rate-limited — will retry after backoff");
                }
                sync::SyncAttempt::AuthRejected => {
                    tracing::error!(
                        error = %e,
                        "sync token rejected — will not retry until the URL or token changes"
                    );
                }
                _ => tracing::error!(error = %e, "sync upload failed"),
            }
            attempt
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

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
            || Err::<(), _>("no keyboard device found — ensure user is in the 'input' group"),
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
        apply_poll_decision(&mut st, &store, dir.path(), decision, Instant::now()).await;
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

    #[test]
    fn a_main_0_4_18_skeleton_recovers_as_untrusted_unlocked_text() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Main persists through chrono's serde, which is RFC3339 with `Z`
        // and a fractional second whenever the instant is not whole.
        let when = Utc::now();
        let when = if when.timestamp_subsec_nanos() == 0 {
            when + chrono::Duration::nanoseconds(1)
        } else {
            when
        };
        let stamped = serde_json::to_value(when).expect("stamp");
        let text = stamped.as_str().expect("rfc3339 string");
        let parsed: chrono::DateTime<Utc> =
            serde_json::from_value(stamped.clone()).expect("a skeleton timestamp parses");
        assert_eq!(
            serde_json::to_value(parsed).expect("round trip"),
            stamped,
            "a skeleton timestamp round-trips through chrono's serde"
        );
        assert!(
            text.contains('.'),
            "a real skeleton timestamp carries a fraction, got {text}"
        );
        let now = text;
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
            "suspect E + DMG drops must not vote — no mid-game split"
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
        }
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
            try_sync_with(&store_bg, &dir_bg, move |matches, _tombstones| {
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
        try_sync_with(&store, dir.path(), move |matches, _| {
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
                try_sync_with(&store, &dir, move |_matches, _tombstones| {
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
                        try_sync_with(&store, &dir, move |_matches, _tombstones| {
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
            "the 20 minute bound is exclusive"
        );
        assert_eq!(
            quiet_close_reason(
                &hinted,
                now + UNFINISHED_SESSION_IDLE + Duration::from_secs(1),
                close
            ),
            Some("unfinished game closed after idle bound")
        );
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
        let window = &run_loop[call..call + 1200];
        assert!(
            window.contains("finish_sync_on_shutdown("),
            "deleting the quiet-close sync call must fail"
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
            run_loop.contains("recover_or_sync_active_game("),
            "startup must sync when it drops a stale skeleton"
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
        let resume_at = after_suspend
            .find("resume_after_suspend(")
            .expect("suspend must resume through resume_after_suspend");
        assert!(
            after_suspend[..resume_at].contains("recover_or_sync_active_game("),
            "a stale skeleton dropped on resume must be synced"
        );
        assert!(
            after_suspend[..resume_at].contains("finish_sync_on_shutdown("),
            "resume sync must use the shutdown path"
        );
    }

    /// Production poll cadence. A stable screen is emitted on every tick.
    const NIGHT_TICKS: usize = 4;

    fn night_tick() -> Duration {
        Duration::from_secs(config::AutoDetectConfig::default().poll_interval_secs)
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
            }
        }

        fn sync_count(&self) -> usize {
            self.syncs.load(std::sync::atomic::Ordering::SeqCst)
        }

        fn upload_count(&self) -> usize {
            self.uploads.load(std::sync::atomic::Ordering::SeqCst)
        }

        /// One command tick of the quiet-session timer. Same function the
        /// daemon loop calls; the closure is the shutdown upload.
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
                        let _ = try_sync_with(&store, &dir, move |_matches, _tombstones| {
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
            recover_or_sync_active_game(self.dir.path(), || {
                let syncs = std::sync::Arc::clone(&syncs);
                let uploads = std::sync::Arc::clone(&uploads);
                let store = store.clone();
                let dir = dir.clone();
                async move {
                    syncs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let _ = try_sync_with(&store, &dir, move |_matches, _tombstones| {
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
            let req = build_capture_request(self.game(), opened, self.now);
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
                row_id: row,
                hero: &hero_resolved,
                map_from_panel: panel,
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
            let staged = stage_capture(&req, &plan, &facts, self.wall);
            write_staged_rows(
                &self.store,
                self.dir.path(),
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
            let report = CaptureReport {
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
    async fn a_fallback_board_yields_to_one_clean_read_and_a_later_ghost_is_rate_capped() {
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
        let cleaned = night.game().gate.expect("clean read stored");
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
    async fn unfinished_game_idle_for_21_minutes_retires_as_unknown_and_syncs() {
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
        assert_eq!(night.sync_count(), 0);
        night
            .idle_through(activity + Duration::from_secs(21 * 60))
            .await;
        assert!(night.st.active_game.is_none());
        assert_eq!(night.sync_count(), 1, "deleting the sync call must fail");
        assert!(
            !active_game_path(night.dir.path()).exists(),
            "the unfinished skeleton is retired"
        );
        let rows = night.store.get_session_snapshots(&id).await.unwrap();
        assert!(!rows.is_empty());
        assert!(
            rows.iter().all(|row| row.outcome == "unknown"),
            "idle close does not invent an outcome, got {:?}",
            rows.iter()
                .map(|row| row.outcome.clone())
                .collect::<Vec<_>>()
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
            move || {
                let uploads = std::sync::Arc::clone(&uploads);
                let store = store.clone();
                let dir = dir.clone();
                async move {
                    let _ = try_sync_with(&store, &dir, move |matches, _tombstones| {
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
}
