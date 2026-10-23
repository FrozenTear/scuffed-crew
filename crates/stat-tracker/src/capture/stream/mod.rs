//! One long-lived xdg-desktop-portal ScreenCast session.
//!
//! The cursor mode is Hidden. Metadata is never requested: that is the
//! KWin crash when a screenshot and a voice-app share run together. The
//! restore token is stored in the data dir so the prompt is once per
//! machine, not once per boot. A backend that rejects the token is asked
//! again without it. If the portal is down, or its cursor-mode property
//! cannot be read, the open is logged as unavailable and the token is kept.
//! If the session cannot start, or a grab sees no frame within
//! [`SCREENCAST_FRAME_WAIT`], a screenshot is used only where that is safe
//! (niri, wlroots, forced GNOME). KDE skips it and logs. A stream that is
//! dead, waiting for a reopen, or quiet for [`QUIET_STALL`] while Overwatch
//! runs follows the same rule for Tab and poll grabs. A failed start is
//! not retried for [`START_BACKOFF`]. Cancelling or closing the permission
//! dialog is a decline: asking stops until the next Overwatch launch, KDE
//! takes no captures, and the restore token stays. A permission dialog that
//! times out is that same No only when no restore token was sent. A timeout
//! after a token was sent waits out the backoff and is not a No. Shutdown
//! aborts the dialog.
//!
//! [`QUIET_STALL`]: policy::QUIET_STALL

mod fit;
mod framebuf;
pub(super) mod pause;
mod pod;
pub(super) mod policy;
pub(super) mod portal_backend;
pub(super) mod pw;
mod session;
mod token;

pub use policy::{
    OPEN_IN_PROGRESS, STREAM_MISS, capture_now_blocked_by_daemon_share,
    capture_now_blocked_message, is_declined_capture, is_share_closed_until_game, is_share_opening,
    is_stream_miss, share_close_due, should_close_share,
};
pub use token::{remove_restore_token, restore_token_saved};

use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use image::DynamicImage;

use super::portal;
use super::types::CaptureKind;

use fit::{fit_stream, save_compare};
use policy::{
    FRESH_MISS_LIMIT, NEVER_STREAMING_WAIT, Observe, OpenErrorAction, OpenPermit,
    PERMISSION_DIALOG_TIMEOUT, REOPEN_NO_TOKEN, ReopenStep, ReopenWatch, SCREENCAST_FRAME_WAIT,
    SHARE_CLOSED_UNTIL_GAME, STALL_WARN_WINDOW, START_BACKOFF, ShotMethod, Stall, StallSite,
    StreamHealth, TabTimeoutAction, capture_while_declined, classify_stall, clears_share_decline,
    decline_notice, drop_unstarted_stream, fallback_line_for, first_shot_method,
    fresh_miss_should_restart, health_is_dead, may_open_share, open_error_action, open_permit,
    poll_wait, portal_warn_due, quiet_poll_shot_due, reopen_step, reset_stall_on_game_start,
    screenshot_fallback_is_safe, screenshot_skip_step, share_prompt_allowed, should_try_stream,
    stall_fallback_at, tab_timeout_action, warn_share_declined, watch_after_close,
};
use session::{clear_portal_warn, open_session};
use token::read_token;

/// Health for a decision, after the pause fail-safe has had its look: a
/// resume the compositor never answered trips there first, so a stream the
/// tracker paused is not reopened with pausing still on.
/// A portal session that closed ends the stream from outside: tell the
/// pause fail-safe before it judges a resume or a death.
fn mark_if_portal_closed(stream: &pw::Handle) {
    mark_external_if(portal_session_closed(), stream);
}

fn mark_external_if(closed: bool, stream: &pw::Handle) {
    if closed {
        stream.mark_external_end();
    }
}

fn health_now(stream: &pw::Handle) -> StreamHealth {
    mark_if_portal_closed(stream);
    stream.check_stuck_resume();
    health_of(stream.stream_report())
}

fn health_of(report: pw::StreamReport) -> StreamHealth {
    if portal_session_closed() && report.has_streamed {
        return StreamHealth::PortalClosed;
    }
    if report.core_fault && report.has_streamed {
        return StreamHealth::CoreFault;
    }
    match report.phase {
        pw::ReportedPhase::Error => StreamHealth::Error,
        pw::ReportedPhase::Unconnected if report.has_streamed => StreamHealth::Unconnected,
        // The tracker's own pause between grabs is a healthy stream.
        pw::ReportedPhase::Paused if report.has_streamed && report.self_paused => {
            StreamHealth::Streaming
        }
        pw::ReportedPhase::Paused if report.has_streamed => StreamHealth::Paused {
            paused_for: report.paused_for.unwrap_or(Duration::ZERO),
        },
        pw::ReportedPhase::Streaming => StreamHealth::Streaming,
        _ => StreamHealth::Connecting,
    }
}

const COMPARE_LIMIT: u32 = 4;

struct Live {
    _fd: std::os::fd::OwnedFd,
    stream: Option<std::sync::Arc<pw::Handle>>,
    /// Dropping this asks the task that owns the portal session to call
    /// `Session::close`. Drop alone does not close it.
    close_session: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Drop for Live {
    fn drop(&mut self) {
        // Quit and join the PipeWire thread before Session.Close, so the
        // consumer is gone before the portal session ends. A session the
        // portal already closed is not a failed pause.
        if let Some(stream) = self.stream.take() {
            mark_if_portal_closed(&stream);
            drop(stream);
        }
        if let Some(tx) = self.close_session.take() {
            let _ = tx.send(());
        }
    }
}

static SESSION_OPEN: AtomicBool = AtomicBool::new(false);

pub fn session_is_open() -> bool {
    SESSION_OPEN.load(Ordering::Acquire)
}

struct CastState {
    live: Option<Live>,
    /// Bumped when an open starts and when the share is closed. An open
    /// that finishes for an older generation drops its session.
    open_gen: u64,
    /// True while `open_session` is in progress. The permission dialog runs
    /// in that task, not under this mutex.
    opening: bool,
    last_fail: Option<Instant>,
    warned: bool,
    compare_saved: u32,
    compare_warned: bool,
    last_timing_log: Instant,
    /// The task that is waiting on the permission dialog. Shutdown aborts it.
    open_task: Option<tokio::task::JoinHandle<()>>,
    /// Consecutive grabs that found no usable buffer while the session was up.
    fresh_misses: u32,
    /// Dead-stream and Tab soft-miss reopen state. Silence does not move it.
    watch: ReopenWatch,
    /// Data dir for the status file. Set on each capture.
    data_dir: Option<PathBuf>,
    /// The last screenshot a poll took for a quiet stream.
    last_quiet_shot: Option<Instant>,
}

impl Default for CastState {
    fn default() -> Self {
        Self {
            live: None,
            open_gen: 0,
            opening: false,
            last_fail: None,
            warned: false,
            compare_saved: 0,
            compare_warned: false,
            last_timing_log: Instant::now(),
            open_task: None,
            fresh_misses: 0,
            watch: ReopenWatch::default(),
            data_dir: None,
            last_quiet_shot: None,
        }
    }
}

static CAST: LazyLock<tokio::sync::Mutex<CastState>> =
    LazyLock::new(|| tokio::sync::Mutex::new(CastState::default()));

/// Set when the share is closed because the game stopped. Cleared when the
/// next game is seen, so a Tab retry cannot open the dialog again.
static REOPEN_BLOCKED: AtomicBool = AtomicBool::new(false);

/// The user cancelled or closed the permission dialog. In memory for this
/// process. A new Overwatch launch clears it.
static SHARE_DECLINED: AtomicBool = AtomicBool::new(false);

/// The one decline warning has been written. Cleared with [`SHARE_DECLINED`].
static DECLINE_LOGGED: AtomicBool = AtomicBool::new(false);

pub fn note_game_running(running: bool) {
    if running {
        REOPEN_BLOCKED.store(false, Ordering::Release);
    }
}

/// Generation of the portal session whose Closed signal we are watching.
static PORTAL_GEN: AtomicU64 = AtomicU64::new(0);
/// Set to [`PORTAL_GEN`] when that session's Closed signal arrives.
static PORTAL_CLOSED_GEN: AtomicU64 = AtomicU64::new(0);

fn arm_portal_watch() -> u64 {
    let watch = PORTAL_GEN.fetch_add(1, Ordering::AcqRel) + 1;
    PORTAL_CLOSED_GEN.store(0, Ordering::Release);
    watch
}

fn mark_portal_closed(watch: u64) {
    PORTAL_CLOSED_GEN.store(watch, Ordering::Release);
}

fn portal_session_closed() -> bool {
    let watch = PORTAL_GEN.load(Ordering::Acquire);
    watch != 0 && PORTAL_CLOSED_GEN.load(Ordering::Acquire) == watch
}

pub fn share_is_declined() -> bool {
    SHARE_DECLINED.load(Ordering::Acquire)
}

/// Record a decline from a failed open. Other failures are left to the
/// caller, which arms the backoff when [`OpenErrorAction::arm_backoff`] is
/// set. The warn is once per decline.
fn apply_share_decline(
    err: &str,
    fallback_safe: bool,
    data_dir: &Path,
    last_fail: &mut Option<Instant>,
    had_token: bool,
) -> OpenErrorAction {
    let action = open_error_action(err, fallback_safe, had_token);
    if !action.declined {
        return action;
    }
    SHARE_DECLINED.store(true, Ordering::Release);
    *last_fail = None;
    if let Some(notice) = action.notice {
        let mut already = DECLINE_LOGGED.swap(true, Ordering::AcqRel);
        warn_share_declined(&mut already, notice);
        crate::ocr_status::publish_share_notice(data_dir, notice);
    }
    action
}

/// `note_game_running`, and clear a decline on a new Overwatch launch.
/// Set on the not-running to running edge. The next [`capture`] applies it
/// while it holds the session lock, so this function does not take that lock.
static STALL_RESET: AtomicBool = AtomicBool::new(false);

fn apply_stall_reset(watch: &mut ReopenWatch) {
    if STALL_RESET.swap(false, Ordering::AcqRel) {
        reset_stall_on_game_start(watch);
    }
}

fn note_game_edge(was_running: bool, running: bool) -> bool {
    note_game_running(running);
    if !clears_share_decline(was_running, running) {
        return false;
    }
    STALL_RESET.store(true, Ordering::Release);
    true
}

pub fn note_game_seen(was_running: bool, running: bool, data_dir: &Path) {
    if !note_game_edge(was_running, running) {
        return;
    }
    if !SHARE_DECLINED.swap(false, Ordering::AcqRel) {
        return;
    }
    DECLINE_LOGGED.store(false, Ordering::Release);
    crate::ocr_status::publish_share_notice(data_dir, "");
}

/// [`stall_fallback_at`] for this desktop and route. A quiet poll that is
/// let through stamps the rate-limit clock.
fn decide_stall(
    state: &mut CastState,
    site: StallSite,
    kind: CaptureKind,
    stall: Option<Stall>,
) -> Option<Stall> {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    let now = Instant::now();
    let quiet_due = quiet_poll_shot_due(state.last_quiet_shot, now);
    let take = stall_fallback_at(
        site,
        kind,
        stall,
        &desktop,
        super::state::stream_was_forced(),
        quiet_due,
    );
    if take == Some(Stall::Quiet) && kind == CaptureKind::Poll {
        state.last_quiet_shot = Some(now);
    }
    take
}

static TAB_MISS_COUNT: AtomicU64 = AtomicU64::new(0);
static TAB_SHOT_COUNT: AtomicU64 = AtomicU64::new(0);

/// `(misses, screenshot fallbacks)` since this process started.
#[cfg_attr(not(test), allow(dead_code))]
pub fn tab_timeout_counts() -> (u64, u64) {
    (
        TAB_MISS_COUNT.load(Ordering::Relaxed),
        TAB_SHOT_COUNT.load(Ordering::Relaxed),
    )
}

fn mark_share(open: bool, data_dir: Option<&Path>) {
    SESSION_OPEN.store(open, Ordering::Release);
    if let Some(dir) = data_dir {
        crate::ocr_status::publish_share_open(dir, open);
    }
}

/// Clears `opening` if the open task panics before it can.
struct ClearOpening {
    generation: u64,
}

impl Drop for ClearOpening {
    fn drop(&mut self) {
        let Ok(mut state) = CAST.try_lock() else {
            return;
        };
        if state.open_gen == self.generation && state.opening {
            state.opening = false;
        }
    }
}

fn fresh_capture(image: DynamicImage) -> super::types::CapturedFrame {
    super::types::CapturedFrame {
        image,
        seq: None,
        stream: None,
        is_new: true,
    }
}

/// Drop a PipeWire session off the async worker. Joining that thread can
/// take the quit timeout, and the session mutex must not be held across it.
async fn release_live(live: Option<Live>) {
    if let Some(live) = live {
        let _ = tokio::task::spawn_blocking(move || drop(live)).await;
    }
}

/// Close the live session when one exists and this process is on the stream
/// route. A no-op otherwise. Does not start the failure backoff. The PipeWire
/// join runs on a blocking thread so a shutdown timer can still fire.
pub async fn close_if_open() {
    if !super::state::stream_route_active() {
        return;
    }
    let mut state = CAST.lock().await;
    let had = state.live.is_some() || state.opening;
    state.open_gen = state.open_gen.wrapping_add(1);
    state.opening = false;
    if let Some(task) = state.open_task.take() {
        task.abort();
    }
    let live = state.live.take();
    state.watch = watch_after_close(std::mem::take(&mut state.watch));
    // A deliberate close does not start the failure backoff. `last_fail`
    // stays whatever it was, which is unset while a session was up, so the
    // next capture opens immediately.
    state.fresh_misses = 0;
    let data_dir = state.data_dir.clone();
    REOPEN_BLOCKED.store(true, Ordering::Release);
    drop(state);
    release_live(live).await;
    mark_share(false, data_dir.as_deref());
    if had {
        tracing::info!("screencast session closed");
    }
}

pub async fn capture(
    output_name: Option<&str>,
    kind: CaptureKind,
    data_dir: &Path,
    debug_frames: bool,
) -> Result<super::types::CapturedFrame, Box<dyn std::error::Error + Send + Sync>> {
    let max_fps = super::state::prefs().max_fps.max(1);
    let mut state = CAST.lock().await;
    apply_stall_reset(&mut state.watch);
    state.data_dir = Some(data_dir.to_path_buf());
    if !share_prompt_allowed(share_is_declined()) {
        let safe = fallback_is_safe();
        drop(state);
        let notice = decline_notice(safe);
        if capture_while_declined(safe) {
            return screenshot(output_name, notice).await.map(fresh_capture);
        }
        // The decline notice is the one log line. Do not add a skip line.
        return Err(notice.into());
    }
    if !may_open_share(
        REOPEN_BLOCKED.load(Ordering::Acquire),
        state.live.is_some(),
        state.opening,
    ) {
        drop(state);
        return Err(SHARE_CLOSED_UNTIL_GAME.into());
    }
    let age = state.last_fail.map(|at| at.elapsed());
    if !should_try_stream(age, START_BACKOFF) {
        drop(state);
        // KDE, Plasma, and KWin stop inside screenshot_if_safe. That path
        // does not call portal::capture.
        return screenshot_if_safe(output_name, "screencast backoff")
            .await
            .map(fresh_capture);
    }
    let restart = match open_permit(state.opening, state.live.is_some()) {
        OpenPermit::Busy => {
            drop(state);
            return while_opening(output_name).await.map(fresh_capture);
        }
        OpenPermit::Reuse => state
            .live
            .as_ref()
            .is_some_and(|live| live.stream.as_ref().is_some_and(|stream| !stream.alive())),
        OpenPermit::Open => true,
    };
    if restart {
        // Drop a dead session before asking for another, so the portal is
        // not holding two screen shares. The dialog runs in its own task.
        // This grab does not wait on it, and a second grab sees `opening`.
        let retired = state.live.take();
        mark_share(false, Some(data_dir));
        state.open_gen = state.open_gen.wrapping_add(1);
        let generation = state.open_gen;
        state.opening = true;
        let data_dir = data_dir.to_path_buf();
        let output = output_name.map(str::to_string);
        // Whether a token is sent, read before the dialog can time out.
        // A token written during the open does not count.
        let had_token = read_token(&data_dir).is_some();
        if let Some(task) = state.open_task.take() {
            task.abort();
        }
        let task = tokio::spawn(async move {
            let _clear = ClearOpening { generation };
            let result = match tokio::time::timeout(
                PERMISSION_DIALOG_TIMEOUT,
                open_session(&data_dir, output.as_deref(), max_fps),
            )
            .await
            {
                Ok(inner) => inner,
                Err(_elapsed) => {
                    // One warn, from `apply_share_decline`, not a second line
                    // here. A token-less timeout is a No. A timeout after a
                    // token was sent is the backoff.
                    Err("screencast permission dialog timed out".into())
                }
            };
            let mut state = CAST.lock().await;
            if state.open_gen != generation {
                drop(state);
                release_live(result.ok()).await;
                return;
            }
            state.opening = false;
            match result {
                Ok(live) => {
                    let extra = if state.live.is_some() {
                        Some(live)
                    } else {
                        if let Some(stream) = live.stream.as_ref() {
                            pause::register(stream);
                        }
                        state.live = Some(live);
                        state.last_fail = None;
                        clear_portal_warn();
                        mark_share(true, Some(data_dir.as_path()));
                        None
                    };
                    drop(state);
                    release_live(extra).await;
                }
                Err(err) => {
                    let safe = fallback_is_safe();
                    let action = apply_share_decline(
                        &err,
                        safe,
                        data_dir.as_path(),
                        &mut state.last_fail,
                        had_token,
                    );
                    let retired = state.live.take();
                    mark_share(false, Some(data_dir.as_path()));
                    if action.arm_backoff {
                        state.last_fail = Some(Instant::now());
                        let line = fallback_line(&format!("screencast session failed ({err})"));
                        note_fallback(&mut state, &line);
                    }
                    drop(state);
                    release_live(retired).await;
                }
            }
        });
        state.open_task = Some(task);
        drop(state);
        release_live(retired).await;
        return while_opening(output_name).await.map(fresh_capture);
    }
    let Some(stream) = state
        .live
        .as_ref()
        .and_then(|live| live.stream.as_ref().map(std::sync::Arc::clone))
    else {
        state.last_fail = Some(Instant::now());
        let line = fallback_line("screencast stream is not running");
        note_fallback(&mut state, &line);
        drop(state);
        return screenshot_if_safe(output_name, "screencast stream is not running")
            .await
            .map(fresh_capture);
    };
    let streamed = stream.has_streamed();
    let health = health_now(&stream);
    let token_saved = restore_token_saved(data_dir);
    // A poll is where a deferred reopen runs. A Tab never reopens here, so
    // the scoreboard it just read is not thrown away while the share opens.
    if kind == CaptureKind::Poll && (streamed || state.watch.pending.is_some()) {
        let step = reopen_step(
            Observe::Poll { health },
            token_saved,
            Instant::now(),
            &mut state.watch,
        );
        match step {
            ReopenStep::Reopen(why) => {
                tracing::warn!("{why}");
                drop(stream);
                let retired = state.live.take();
                state.fresh_misses = 0;
                mark_share(false, Some(data_dir));
                drop(state);
                release_live(retired).await;
                return Box::pin(capture(output_name, kind, data_dir, debug_frames)).await;
            }
            ReopenStep::KeepNoToken => {
                tracing::warn!("{REOPEN_NO_TOKEN}");
            }
            ReopenStep::Keep | ReopenStep::HoldForPoll => {}
        }
    }
    if !stream.alive() && streamed {
        if kind == CaptureKind::Tab {
            let step = reopen_step(
                Observe::Tab {
                    health,
                    waited: false,
                    got_fresh: false,
                    buffer_during_wait: false,
                },
                token_saved,
                Instant::now(),
                &mut state.watch,
            );
            if matches!(step, ReopenStep::KeepNoToken) {
                tracing::warn!("{REOPEN_NO_TOKEN}");
            }
        }
        // The session stays. Dropping it with no restore token would open
        // the share dialog again. Where a screenshot is safe, this grab is
        // one, so a dead stream waiting for its reopen still reads.
        drop(stream);
        let take = decide_stall(&mut state, StallSite::DeadBeforeGrab, kind, None);
        drop(state);
        if let Some(stall) = take {
            return stall_fallback(output_name, stall).await;
        }
        return Err("screencast stream ended".into());
    }
    if !stream.alive() {
        state.last_fail = Some(Instant::now());
        let retired = state.live.take();
        mark_share(false, Some(data_dir));
        let line = fallback_line("screencast stream is not running");
        note_fallback(&mut state, &line);
        drop(stream);
        drop(state);
        release_live(retired).await;
        return screenshot_if_safe(output_name, "screencast stream is not running")
            .await
            .map(fresh_capture);
    }
    if drop_unstarted_stream(streamed, stream.running_for(), NEVER_STREAMING_WAIT) {
        state.last_fail = Some(Instant::now());
        let retired = state.live.take();
        mark_share(false, Some(data_dir));
        let line = fallback_line("screencast never reached streaming");
        note_fallback(&mut state, &line);
        drop(stream);
        drop(state);
        release_live(retired).await;
        return screenshot_if_safe(output_name, "screencast never reached streaming")
            .await
            .map(fresh_capture);
    }
    let tab_press = match kind {
        CaptureKind::Tab => Some(super::tab::tab_press().unwrap_or_else(Instant::now)),
        CaptureKind::Poll => None,
    };
    let fresh = stream.needs_fresh_frame();
    // Resume the stream for this grab. It pauses again when the hold is
    // dropped below, after the post-grab health read, or on any early
    // return. A Tab grab takes over the key hold from the press.
    let (resuming, grab_hold) = pause::prepare_grab(&stream, tab_press, max_fps).await;
    let timeout = if tab_press.is_some() {
        Duration::from_millis(super::tab::TAB_GRAB_TIMEOUT_AFTER_PRESS_MS)
    } else if fresh {
        SCREENCAST_FRAME_WAIT
    } else {
        poll_wait(max_fps, resuming)
    };
    // The mutex stays held so a second caller cannot open another share.
    let grab_started = Instant::now();
    let grabbed = stream.grab(timeout, tab_press).await;
    // Before the grab's verdict: a portal close during an unanswered
    // resume is not a Tab miss or a slow resume.
    mark_if_portal_closed(&stream);
    stream.note_grab_result(&grabbed);
    let timing = stream.timing();
    let alive = stream.alive();
    let stream_id = stream.stream_id();
    let mut keep_session = false;
    let health = health_now(&stream);
    if kind == CaptureKind::Tab {
        let buffer_during_wait = stream.last_buffer_at().is_some_and(|at| at >= grab_started);
        let (waited, got_fresh) = match &grabbed {
            Ok(pw::GrabOutcome::TabMiss { .. }) => (true, false),
            Ok(pw::GrabOutcome::Frame { is_new, .. }) => (true, *is_new),
            Err(_) => (false, false),
        };
        let step = reopen_step(
            Observe::Tab {
                health,
                waited,
                got_fresh,
                buffer_during_wait,
            },
            restore_token_saved(data_dir),
            Instant::now(),
            &mut state.watch,
        );
        // HoldForPoll and KeepNoToken both leave the session up so this
        // grab's frame or miss is what the caller sees.
        keep_session = matches!(step, ReopenStep::HoldForPoll | ReopenStep::KeepNoToken);
        if matches!(step, ReopenStep::KeepNoToken) {
            tracing::warn!("{REOPEN_NO_TOKEN}");
        }
    }
    // Only read when the grab got no new picture. A Tab miss has its own
    // fallback in `tab_timeout`.
    // Active time only: the tracker's own pauses are not quiet time.
    let quiet_for = stream.quiet_for();
    let stall = classify_stall(
        !alive || health_is_dead(health),
        state.watch.pending.is_some(),
        quiet_for,
    );
    drop(grab_hold);
    // A poll that only got the previous frame back from a stalled stream
    // would read nothing. Where a screenshot is safe, take one instead.
    if matches!(grabbed, Ok(pw::GrabOutcome::Frame { is_new: false, .. }))
        && let Some(stall) = decide_stall(&mut state, StallSite::ReusedFrame, kind, stall)
    {
        drop(stream);
        drop(state);
        return stall_fallback(output_name, stall).await;
    }
    match grabbed {
        Ok(pw::GrabOutcome::TabMiss { delay_ms }) => {
            drop(state);
            tab_timeout(output_name, delay_ms).await.map(fresh_capture)
        }
        Ok(pw::GrabOutcome::Frame { frame, is_new }) => {
            state.last_fail = None;
            state.fresh_misses = 0;
            state.warned = false;
            let grab_ms = timing.avg_grab_ms;
            let log_summary = state.last_timing_log.elapsed() >= Duration::from_secs(60);
            if log_summary {
                state.last_timing_log = Instant::now();
            }
            let compare =
                debug_frames && kind == CaptureKind::Tab && state.compare_saved < COMPARE_LIMIT;
            let index = state.compare_saved;
            if compare {
                state.compare_saved += 1;
                if !state.compare_warned {
                    state.compare_warned = true;
                    tracing::warn!(
                        "debug portal frames also takes a screenshot; a voice app sharing the screen can hit the KWin cursor crash while this is on"
                    );
                }
            }
            drop(state);
            tracing::debug!(
                grab_ms,
                frames_dropped = timing.frames_dropped,
                "screencast grab"
            );
            if log_summary {
                tracing::info!("{}", pw::timing_line(&timing));
            }
            let seq = frame.clock.seq;
            // Synchronous, no await inside: thread CPU measures it.
            let convert_cpu_started = crate::perf::thread_cpu();
            let img = framebuf::to_dynamic(&frame).ok_or("screencast frame format is not RGB")?;
            let convert_cpu = crate::perf::thread_cpu().saturating_sub(convert_cpu_started);
            let img = fit_stream(img, output_name).await?;
            // Only a grab that produced an image counts, as on every route.
            crate::perf::record_cpu(crate::perf::Stage::Capture, convert_cpu);
            if compare {
                save_compare(data_dir, index, &img, output_name).await;
            }
            Ok(super::types::CapturedFrame {
                image: img,
                seq: Some(seq),
                stream: Some(stream_id),
                is_new,
            })
        }
        Err(err) => {
            let streamed_now = stream.has_streamed();
            let uptime = stream.running_for();
            drop(stream);
            if err == pw::NO_STREAM_FRAME {
                state.fresh_misses = state.fresh_misses.saturating_add(1);
                if fresh_miss_should_restart(state.fresh_misses, FRESH_MISS_LIMIT) {
                    state.fresh_misses = 0;
                    state.last_fail = Some(Instant::now());
                    let retired = state.live.take();
                    mark_share(false, Some(data_dir));
                    let line = fallback_line("screencast had no usable frame");
                    note_fallback(&mut state, &line);
                    drop(state);
                    release_live(retired).await;
                    return screenshot_if_safe(output_name, "screencast had no usable frame")
                        .await
                        .map(fresh_capture);
                }
            }
            if !alive && !keep_session {
                state.last_fail = Some(Instant::now());
                let retired = state.live.take();
                mark_share(false, Some(data_dir));
                let line = fallback_line(&format!("screencast stream ended ({err})"));
                note_fallback(&mut state, &line);
                drop(state);
                release_live(retired).await;
                return screenshot_if_safe(output_name, "screencast stream ended")
                    .await
                    .map(fresh_capture);
            }
            if !keep_session && drop_unstarted_stream(streamed_now, uptime, NEVER_STREAMING_WAIT) {
                state.last_fail = Some(Instant::now());
                let retired = state.live.take();
                mark_share(false, Some(data_dir));
                let line = fallback_line("screencast never reached streaming");
                note_fallback(&mut state, &line);
                drop(state);
                release_live(retired).await;
                return screenshot_if_safe(output_name, "screencast never reached streaming")
                    .await
                    .map(fresh_capture);
            }
            // A core fault, or a poll wait with nothing, on a stalled stream.
            let site = StallSite::GrabError {
                shutting_down: crate::replace::shutdown_requested(),
            };
            let take = decide_stall(&mut state, site, kind, stall);
            drop(state);
            if let Some(stall) = take {
                return stall_fallback(output_name, stall).await;
            }
            Err(err.into())
        }
    }
}

/// One screenshot for a stalled stream, taken without the session lock.
/// The session stays as it is: the reopen, if one is due, still runs on a
/// poll. The line names the method that took the picture, and is a warn
/// once per [`STALL_WARN_WINDOW`], then debug.
async fn stall_fallback(
    output_name: Option<&str>,
    stall: Stall,
) -> Result<super::types::CapturedFrame, Box<dyn std::error::Error + Send + Sync>> {
    let why = stall.reason();
    let (image, method) = screenshot_with(output_name, why).await?;
    let line = fallback_line_for(why, Some(method));
    let warn = {
        let mut last = STALL_WARN_AT.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        let due = portal_warn_due(*last, now, STALL_WARN_WINDOW);
        if due {
            *last = Some(now);
        }
        due
    };
    if warn {
        tracing::warn!("{line}");
    } else {
        tracing::debug!("{line}");
    }
    Ok(fresh_capture(image))
}

static STALL_WARN_AT: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);

async fn tab_timeout(
    output_name: Option<&str>,
    delay_ms: u64,
) -> Result<DynamicImage, Box<dyn std::error::Error + Send + Sync>> {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    match tab_timeout_action(&desktop, super::state::stream_was_forced()) {
        TabTimeoutAction::Screenshot => {
            let count = TAB_SHOT_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
            tracing::info!(
                delay_ms,
                count,
                "Tab grab: stream timeout, screenshot fallback"
            );
            screenshot(output_name, "Tab stream timeout").await
        }
        TabTimeoutAction::Skip => {
            // A press the open already pushed past the frame budget is
            // discarded and grabbed again. That miss is not a log line.
            note_tab_stream_miss(delay_ms, super::tab::stream_miss_log_suppressed());
            Err(STREAM_MISS.into())
        }
    }
}

/// Log a Tab stream miss unless this is the first Tab discarding a press
/// the share open already pushed past the frame budget. Returns whether
/// the line was written.
fn note_tab_stream_miss(delay_ms: u64, suppressed: bool) -> bool {
    if suppressed {
        return false;
    }
    let count = TAB_MISS_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
    tracing::info!(delay_ms, count, "Tab grab: no fresh stream frame (timeout)");
    true
}

fn note_fallback(state: &mut CastState, message: &str) {
    if state.warned {
        tracing::debug!("{message}");
    } else {
        tracing::warn!("{message}");
        state.warned = true;
    }
}

/// A grab that arrived while the permission dialog is still up. KDE must
/// not open the screenshot portal beside a share that is about to start.
async fn while_opening(
    output_name: Option<&str>,
) -> Result<DynamicImage, Box<dyn std::error::Error + Send + Sync>> {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    if screenshot_fallback_is_safe(&desktop, super::state::stream_was_forced()) {
        return screenshot(output_name, OPEN_IN_PROGRESS).await;
    }
    Err(OPEN_IN_PROGRESS.into())
}

fn fallback_is_safe() -> bool {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    screenshot_fallback_is_safe(&desktop, super::state::stream_was_forced())
}

/// The method [`screenshot`] tries first on this desktop.
fn planned_shot_method() -> ShotMethod {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    first_shot_method(&desktop, super::wayshot::is_available())
}

/// Logged before the screenshot, so it names the method tried first. A
/// wayshot failure that falls through to the portal logs that separately.
fn fallback_line(why: &str) -> String {
    fallback_line_for(why, fallback_is_safe().then(planned_shot_method))
}

async fn screenshot(
    output_name: Option<&str>,
    why: &str,
) -> Result<DynamicImage, Box<dyn std::error::Error + Send + Sync>> {
    screenshot_with(output_name, why)
        .await
        .map(|(image, _)| image)
}

/// wayshot first where the compositor has screencopy, then the portal.
/// Returns which one produced the image.
async fn screenshot_with(
    output_name: Option<&str>,
    why: &str,
) -> Result<(DynamicImage, ShotMethod), Box<dyn std::error::Error + Send + Sync>> {
    if planned_shot_method() == ShotMethod::Wayshot {
        match super::wayshot::capture_with_output(output_name).await {
            Ok(image) => {
                tracing::debug!(why, "screencast skipped; using wayshot");
                return Ok((image, ShotMethod::Wayshot));
            }
            Err(err) => {
                tracing::info!(why, error = %err, "wayshot screenshot failed; using Older screenshot");
            }
        }
    }
    tracing::debug!(why, "screencast skipped; using Older screenshot");
    // `portal::capture` runs the request on its own task, so a dropped grab
    // does not leave the portal's file behind.
    portal::capture(output_name)
        .await
        .map(|image| (image, ShotMethod::Portal))
}

/// Screenshot only on niri, wlroots, and forced GNOME. KDE skips and logs,
/// the same rule as a Tab timeout.
async fn screenshot_if_safe(
    output_name: Option<&str>,
    why: &str,
) -> Result<DynamicImage, Box<dyn std::error::Error + Send + Sync>> {
    if fallback_is_safe() {
        screenshot(output_name, why).await
    } else {
        log_screenshot_skipped(why);
        Err(why.into())
    }
}

static SKIP_LOG_AT: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);

fn log_screenshot_skipped(why: &str) {
    let info = {
        let mut last = SKIP_LOG_AT.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        let (info, next) = screenshot_skip_step(*last, now);
        *last = next;
        info
    };
    if info {
        tracing::info!(why, "screencast screenshot skipped");
    } else {
        tracing::debug!(why, "screencast screenshot skipped");
    }
}

#[cfg(test)]
mod tests {
    use crate::capture::geom::OutputRect;

    use super::fit::*;
    use super::policy::*;
    use super::session::*;
    use super::token::*;
    use super::*;

    fn output(name: &str, w: u32, h: u32) -> OutputRect {
        OutputRect {
            name: name.to_string(),
            x: 0,
            y: 0,
            width: w,
            height: h,
            real_width: w,
            real_height: h,
        }
    }

    #[test]
    fn backoff_skips_a_recent_failure_and_allows_a_later_try() {
        assert!(should_try_stream(None, START_BACKOFF));
        assert!(!should_try_stream(
            Some(Duration::from_secs(1)),
            START_BACKOFF
        ));
        assert!(should_try_stream(Some(START_BACKOFF), START_BACKOFF));
        assert_eq!(SCREENCAST_FRAME_WAIT, Duration::from_secs(2));
    }

    #[test]
    fn a_single_monitor_frame_is_not_cropped() {
        let outputs = [output("eDP-1", 1920, 1200), output("DP-1", 1920, 1080)];
        assert!(!stream_should_crop(
            1920,
            1080,
            outputs[1].area(),
            outputs.len()
        ));
        assert!(stream_should_crop(
            3840,
            1200,
            outputs[1].area(),
            outputs.len()
        ));
        assert!(!stream_should_crop(1920, 1080, outputs[0].area(), 1));
    }

    #[test]
    fn a_desktop_sized_stream_crops_and_a_monitor_sized_one_does_not() {
        let mut dp = output("DP-1", 1920, 1080);
        dp.x = 1920;
        let outputs = [output("eDP-1", 1920, 1080), dp];
        let desktop = image::DynamicImage::ImageRgb8(image::RgbImage::new(3840, 1080));
        let cropped = fit_stream_image(desktop, &outputs, Some("DP-1")).unwrap();
        assert_eq!((cropped.width(), cropped.height()), (1920, 1080));
        let one = image::DynamicImage::ImageRgb8(image::RgbImage::new(1920, 1080));
        let kept = fit_stream_image(one, &outputs, Some("eDP-1")).unwrap();
        assert_eq!((kept.width(), kept.height()), (1920, 1080));
    }

    #[test]
    fn the_share_closes_on_the_running_edge_and_only_on_the_stream() {
        assert!(should_close_share(true, true, false, true));
        assert!(!should_close_share(true, true, false, false));
        assert!(!should_close_share(true, false, false, true));
        assert!(!should_close_share(true, true, true, true));
        assert!(!should_close_share(false, true, false, true));
        assert!(should_try_stream(None, START_BACKOFF));
        assert_eq!(open_permit(true, false), OpenPermit::Busy);
        assert_eq!(open_permit(false, true), OpenPermit::Reuse);
        assert_eq!(open_permit(false, false), OpenPermit::Open);
        let mut placed = output("DP-1", 1920, 1080);
        placed.x = 1920;
        assert!(monitor_matches(
            Some((1920, 0)),
            Some((1920, 1080)),
            &placed
        ));
        assert!(!monitor_matches(Some((0, 0)), Some((1920, 1080)), &placed));
        assert!(!monitor_matches(None, Some((1920, 1080)), &placed));
        assert!(must_close_session(true, false));
        assert!(!must_close_session(true, true));
        assert!(!must_close_session(false, false));
    }

    #[test]
    fn a_cancel_is_not_retried_and_kde_does_not_screenshot_a_late_tab() {
        assert!(is_user_cancel("org.freedesktop.portal.Error.Cancelled"));
        assert!(is_user_cancel("Dialog dismissed by the user"));
        assert!(!is_user_cancel("select_sources failed"));
        assert_eq!(tab_timeout_action("KDE", true), TabTimeoutAction::Skip);
        assert_eq!(tab_timeout_action("KDE", false), TabTimeoutAction::Skip);
        assert_eq!(tab_timeout_action("GNOME", false), TabTimeoutAction::Skip);
        assert_eq!(
            tab_timeout_action("GNOME", true),
            TabTimeoutAction::Screenshot
        );
        assert_eq!(
            tab_timeout_action("niri", true),
            TabTimeoutAction::Screenshot
        );
        assert_eq!(
            tab_timeout_action("Hyprland", false),
            TabTimeoutAction::Screenshot
        );
        assert!(screenshot_fallback_is_safe("sway", true));
        assert!(!screenshot_fallback_is_safe("ubuntu:GNOME", false));
        let _ = tab_timeout_counts();
        assert_eq!(PERMISSION_DIALOG_TIMEOUT, Duration::from_secs(60));
        assert_eq!(NEVER_STREAMING_WAIT, Duration::from_secs(6));
        assert!(!drop_unstarted_stream(
            false,
            Duration::from_secs(2),
            NEVER_STREAMING_WAIT
        ));
        assert!(drop_unstarted_stream(
            false,
            NEVER_STREAMING_WAIT,
            NEVER_STREAMING_WAIT
        ));
        assert!(!drop_unstarted_stream(
            true,
            Duration::from_secs(30),
            NEVER_STREAMING_WAIT
        ));
        assert!(capture_now_blocked_by_daemon_share("KDE", true));
        assert!(capture_now_blocked_by_daemon_share("KDE Plasma", true));
        assert!(capture_now_blocked_by_daemon_share("kwin", true));
        assert!(!capture_now_blocked_by_daemon_share("KDE", false));
        assert!(!capture_now_blocked_by_daemon_share("niri", true));
        assert_eq!(
            fallback_line_for("screencast backoff", Some(ShotMethod::Portal)),
            "screencast backoff; falling back to Older screenshot"
        );
        assert_eq!(
            fallback_line_for("screencast backoff", Some(ShotMethod::Wayshot)),
            "screencast backoff; falling back to a wayshot screenshot"
        );
        assert_eq!(
            fallback_line_for("screencast backoff", None),
            "screencast backoff; screenshot skipped"
        );
        match select_error("org.freedesktop.portal.Error.Cancelled".into(), true) {
            OpenFail::Other(msg) => assert_eq!(msg, "select_sources cancelled"),
            OpenFail::Select(_) => panic!("a decline must keep the restore token"),
        }
        match select_error("portal rejected the request".into(), true) {
            OpenFail::Select(_) => {}
            OpenFail::Other(msg) => panic!("a token failure should retry without it: {msg}"),
        }
        assert!(is_stream_miss(STREAM_MISS));
        assert!(is_share_opening(OPEN_IN_PROGRESS));
        assert!(!is_stream_miss(OPEN_IN_PROGRESS));
        assert!(!may_open_share(true, false, false));
        assert!(may_open_share(true, true, false));
        assert!(may_open_share(false, false, false));
        assert!(fresh_miss_should_restart(
            FRESH_MISS_LIMIT,
            FRESH_MISS_LIMIT
        ));
        assert!(!fresh_miss_should_restart(
            FRESH_MISS_LIMIT - 1,
            FRESH_MISS_LIMIT
        ));
        let mut reads = 0u32;
        assert!(!share_close_due(false, true, false, || {
            reads += 1;
            true
        }));
        assert_eq!(reads, 0);
        assert!(share_close_due(true, true, false, || {
            reads += 1;
            true
        }));
        assert_eq!(reads, 1);
        let now = Instant::now();
        assert!(portal_warn_due(None, now, START_BACKOFF));
        assert!(!portal_warn_due(Some(now), now, START_BACKOFF));
        assert!(portal_warn_due(
            Some(now - START_BACKOFF),
            now,
            START_BACKOFF
        ));
    }

    #[test]
    fn an_unavailable_portal_is_not_a_missing_hidden_cursor() {
        assert_eq!(
            cursor_modes_block(true, false, false),
            CursorModesBlock::PortalUnavailable
        );
        assert_eq!(
            cursor_modes_block(false, true, false),
            CursorModesBlock::PortalUnavailable
        );
        assert_eq!(
            cursor_modes_block(true, true, true),
            CursorModesBlock::PortalUnavailable
        );
        assert_eq!(
            cursor_modes_block(false, false, false),
            CursorModesBlock::NoHiddenCursor
        );
        assert_eq!(cursor_modes_block(false, false, true), CursorModesBlock::Ok);
        assert_eq!(PORTAL_UNAVAILABLE, "screencast portal is unavailable");
        // Backoff on KDE calls screenshot_if_safe, which uses this predicate
        // and does not call portal::capture.
        assert!(!screenshot_fallback_is_safe("KDE", true));
        assert!(!screenshot_fallback_is_safe("KDE", false));
        assert!(!screenshot_fallback_is_safe("plasma", true));
        assert!(!screenshot_fallback_is_safe("kwin", false));
        assert!(!screenshot_fallback_is_safe("KDE:plasma", true));
    }

    #[test]
    fn restore_token_is_replaced_atomically_and_private() {
        let dir = tempfile::tempdir().unwrap();
        write_token(dir.path(), "secret-token");
        let path = token_path(dir.path());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "secret-token");
        assert!(!dir.path().join(format!("{TOKEN_FILE}.tmp")).exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
            let mut perms = std::fs::metadata(&path).unwrap().permissions();
            perms.set_mode(0o644);
            std::fs::set_permissions(&path, perms).unwrap();
        }
        write_token(dir.path(), "next-token");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "next-token");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert!(!dir.path().join(format!("{TOKEN_FILE}.tmp")).exists());
    }

    /// Clears the process-wide decline flags if a test stops early.
    struct DeclineReset;

    impl Drop for DeclineReset {
        fn drop(&mut self) {
            SHARE_DECLINED.store(false, Ordering::Release);
            DECLINE_LOGGED.store(false, Ordering::Release);
            crate::ocr_status::set_share_notice("");
        }
    }

    #[derive(Clone, Default)]
    struct LogBuf(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for LogBuf {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogBuf {
        type Writer = LogBuf;
        fn make_writer(&'a self) -> LogBuf {
            self.clone()
        }
    }

    fn with_log_level(level: tracing::Level, f: impl FnOnce()) -> String {
        let buf = LogBuf::default();
        let sub = tracing_subscriber::fmt()
            .with_writer(buf.clone())
            .with_ansi(false)
            .with_max_level(level)
            .finish();
        tracing::subscriber::with_default(sub, f);
        String::from_utf8(buf.0.lock().unwrap().clone()).unwrap()
    }

    fn with_log(f: impl FnOnce()) -> String {
        with_log_level(tracing::Level::WARN, f)
    }

    #[test]
    fn a_skipped_screenshot_is_info_once_per_backoff_window() {
        let now = Instant::now();
        for last in [None, Some(now), Some(now - START_BACKOFF)] {
            assert_eq!(
                screenshot_skip_log_is_info(last, now),
                portal_warn_due(last, now, START_BACKOFF)
            );
        }
        let (info, next) = screenshot_skip_step(None, now);
        assert!(info);
        assert_eq!(next, Some(now));
        let (info, next) = screenshot_skip_step(Some(now), now);
        assert!(!info);
        assert_eq!(next, Some(now));
        let (info, next) = screenshot_skip_step(Some(now - START_BACKOFF), now);
        assert!(info);
        assert_eq!(next, Some(now));

        *SKIP_LOG_AT.lock().unwrap_or_else(|e| e.into_inner()) = None;
        let first = with_log_level(tracing::Level::INFO, || {
            log_screenshot_skipped("screencast backoff");
        });
        let second = with_log_level(tracing::Level::INFO, || {
            log_screenshot_skipped("screencast backoff");
        });
        assert_eq!(first.matches("screencast screenshot skipped").count(), 1);
        assert!(first.contains("INFO"));
        assert!(first.contains("screencast backoff"));
        assert!(
            !second.contains("screencast screenshot skipped"),
            "a second skip inside the window is debug, and this subscriber stops at info: {second}"
        );
        *SKIP_LOG_AT.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    #[test]
    fn an_unavailable_portal_warns_once_per_backoff_episode() {
        let now = Instant::now();
        assert!(portal_should_warn(None, now, PortalFail::Proxy));
        let again = now + START_BACKOFF;
        assert!(
            !portal_should_warn(Some((now, PortalFail::Proxy)), again, PortalFail::Proxy),
            "a retry one backoff later does not warn again"
        );
        assert!(
            portal_should_warn(
                Some((now, PortalFail::Proxy)),
                again,
                PortalFail::CursorEmpty
            ),
            "a different failure kind warns"
        );
        assert!(portal_should_warn(
            Some((now, PortalFail::Proxy)),
            now + PORTAL_WARN_WINDOW,
            PortalFail::Proxy
        ));
        assert!(!portal_should_warn(
            Some((now, PortalFail::Proxy)),
            now + PORTAL_WARN_WINDOW - Duration::from_secs(1),
            PortalFail::Proxy
        ));

        // A local slot, the same way match-end OCR fallback takes an
        // AtomicBool. The process-wide slot is what a full-suite run shares.
        let slot = std::sync::Mutex::new(None);
        let first = with_log_level(tracing::Level::WARN, || {
            let _ = portal_unavailable_in(&slot, PortalFail::Proxy, &"portal down");
            let _ = portal_unavailable_in(&slot, PortalFail::Proxy, &"portal is gone");
        });
        assert_eq!(first.matches(PORTAL_UNAVAILABLE).count(), 1);
        assert!(first.contains("WARN"));
        assert!(first.contains("portal down"));
        let second = with_log_level(tracing::Level::WARN, || {
            let _ = portal_unavailable_in(&slot, PortalFail::Proxy, &"portal down");
        });
        assert!(
            !second.contains(PORTAL_UNAVAILABLE),
            "the same kind inside the window does not warn again: {second}"
        );
        let other = with_log_level(tracing::Level::WARN, || {
            let _ = portal_unavailable_in(&slot, PortalFail::CursorRead, &"cursor read failed");
        });
        assert_eq!(other.matches(PORTAL_UNAVAILABLE).count(), 1);
        clear_portal_warn_in(&slot);
        let after_clear = with_log_level(tracing::Level::WARN, || {
            let _ = portal_unavailable_in(&slot, PortalFail::Proxy, &"portal down");
        });
        assert_eq!(
            after_clear.matches(PORTAL_UNAVAILABLE).count(),
            1,
            "a successful open clears the episode, so the next outage warns"
        );
    }

    fn tab_obs(waited: bool, got_fresh: bool, buffer_during_wait: bool) -> Observe {
        Observe::Tab {
            health: StreamHealth::Streaming,
            waited,
            got_fresh,
            buffer_during_wait,
        }
    }

    fn quiet_polls(watch: &mut ReopenWatch, now: Instant, n: u32) {
        for _ in 0..n {
            assert_eq!(
                reopen_step(
                    Observe::Poll {
                        health: StreamHealth::Streaming
                    },
                    true,
                    now,
                    watch
                ),
                ReopenStep::Keep
            );
        }
    }

    #[test]
    fn a_quiet_stream_never_reopens_and_a_dead_one_is_reopened_but_not_twice_in_a_row() {
        let now = Instant::now();
        let mut watch = ReopenWatch::default();
        quiet_polls(&mut watch, now + Duration::from_secs(60), 8);
        assert!(watch.pending.is_none());
        assert_eq!(watch.reopens_this_game, 0);
        assert_eq!(watch.soft_misses, 0);

        let mut dead = ReopenWatch::default();
        assert_eq!(
            reopen_step(
                Observe::Poll {
                    health: StreamHealth::Error
                },
                true,
                now,
                &mut dead
            ),
            ReopenStep::Reopen(REOPEN_DEAD)
        );
        assert_eq!(
            reopen_step(
                Observe::Poll {
                    health: StreamHealth::Error
                },
                true,
                now,
                &mut dead
            ),
            ReopenStep::Keep,
            "the same dead stream is not reopened twice in a row"
        );
        let mut unconnected = ReopenWatch::default();
        assert_eq!(
            reopen_step(
                Observe::Poll {
                    health: StreamHealth::Unconnected
                },
                true,
                now,
                &mut unconnected
            ),
            ReopenStep::Reopen(REOPEN_DEAD)
        );
        assert_eq!(
            reopen_step(
                Observe::Poll {
                    health: StreamHealth::Unconnected
                },
                true,
                now,
                &mut unconnected
            ),
            ReopenStep::Keep
        );
        let mut paused = ReopenWatch::default();
        assert_eq!(
            reopen_step(
                Observe::Poll {
                    health: StreamHealth::Paused {
                        paused_for: Duration::from_secs(1)
                    }
                },
                true,
                now,
                &mut paused
            ),
            ReopenStep::Keep
        );
        assert_eq!(
            reopen_step(
                Observe::Poll {
                    health: StreamHealth::Paused {
                        paused_for: PAUSED_PERSIST
                    }
                },
                true,
                now,
                &mut paused
            ),
            ReopenStep::Reopen(REOPEN_DEAD)
        );
    }

    #[test]
    fn two_tab_soft_misses_reopen_on_the_next_poll_and_again_after_the_gap() {
        let now = Instant::now();
        let mut watch = ReopenWatch::default();
        assert_eq!(
            reopen_step(tab_obs(true, false, false), true, now, &mut watch),
            ReopenStep::Keep,
            "one soft miss does not reopen"
        );
        assert_eq!(watch.soft_misses, 1);
        assert_eq!(
            reopen_step(tab_obs(true, false, false), true, now, &mut watch),
            ReopenStep::HoldForPoll,
            "the Tab that trips the stall keeps its result"
        );
        assert!(matches!(watch.pending, Some(Pending::Tabs)));
        assert_eq!(
            reopen_step(
                Observe::Poll {
                    health: StreamHealth::Streaming
                },
                true,
                now,
                &mut watch
            ),
            ReopenStep::Reopen(REOPEN_TABS)
        );
        assert_eq!(watch.reopens_this_game, 1);
        assert_eq!(watch.soft_misses, 0);

        assert_eq!(
            reopen_step(tab_obs(true, false, false), true, now, &mut watch),
            ReopenStep::Keep
        );
        assert_eq!(
            reopen_step(tab_obs(true, false, false), true, now, &mut watch),
            ReopenStep::HoldForPoll,
            "a second pair in the same game asks for another reopen"
        );
        assert_eq!(
            reopen_step(
                Observe::Poll {
                    health: StreamHealth::Streaming
                },
                true,
                now,
                &mut watch
            ),
            ReopenStep::Keep,
            "inside the gap the reopen waits"
        );
        assert!(matches!(watch.pending, Some(Pending::Tabs)));
        assert_eq!(
            reopen_step(
                Observe::Poll {
                    health: StreamHealth::Streaming
                },
                true,
                now + REOPEN_GAP_MIN,
                &mut watch
            ),
            ReopenStep::Reopen(REOPEN_TABS),
            "after the gap the second stall in the same game reopens"
        );

        let mut buffered = ReopenWatch::default();
        assert_eq!(
            reopen_step(tab_obs(true, false, true), true, now, &mut buffered),
            ReopenStep::Keep
        );
        assert_eq!(
            reopen_step(tab_obs(true, false, true), true, now, &mut buffered),
            ReopenStep::Keep,
            "a PipeWire buffer during the wait is not a soft miss"
        );
        assert_eq!(buffered.soft_misses, 0);

        let mut fresh = ReopenWatch::default();
        assert_eq!(
            reopen_step(tab_obs(true, false, false), true, now, &mut fresh),
            ReopenStep::Keep
        );
        assert_eq!(fresh.soft_misses, 1);
        assert_eq!(
            reopen_step(tab_obs(true, true, false), true, now, &mut fresh),
            ReopenStep::Keep
        );
        assert_eq!(fresh.soft_misses, 0, "a fresh Tab frame clears the streak");
    }

    #[test]
    fn a_reopen_waits_out_the_gap_and_a_new_game_keeps_that_clock() {
        let now = Instant::now();
        let mut watch = ReopenWatch::default();
        assert_eq!(
            reopen_step(tab_obs(true, false, false), true, now, &mut watch),
            ReopenStep::Keep
        );
        assert_eq!(
            reopen_step(tab_obs(true, false, false), true, now, &mut watch),
            ReopenStep::HoldForPoll
        );
        assert_eq!(
            reopen_step(
                Observe::Poll {
                    health: StreamHealth::Streaming
                },
                true,
                now,
                &mut watch
            ),
            ReopenStep::Reopen(REOPEN_TABS)
        );
        let opened_at = watch.last_reopen.expect("the reopen stamps the clock");

        reset_stall_on_game_start(&mut watch);
        assert_eq!(watch.reopens_this_game, 0);
        assert_eq!(watch.soft_misses, 0);
        assert_eq!(watch.last_reopen, Some(opened_at));
        assert_eq!(
            reopen_step(tab_obs(true, false, false), true, now, &mut watch),
            ReopenStep::Keep
        );
        assert_eq!(
            reopen_step(tab_obs(true, false, false), true, now, &mut watch),
            ReopenStep::HoldForPoll
        );
        let too_soon = now + REOPEN_GAP_MIN - Duration::from_secs(1);
        assert_eq!(
            reopen_step(
                Observe::Poll {
                    health: StreamHealth::Streaming
                },
                true,
                too_soon,
                &mut watch
            ),
            ReopenStep::Keep,
            "the gap has not elapsed"
        );
        assert!(matches!(watch.pending, Some(Pending::Tabs)));
        assert_eq!(
            reopen_step(
                Observe::Poll {
                    health: StreamHealth::Streaming
                },
                true,
                now + REOPEN_GAP_MIN,
                &mut watch
            ),
            ReopenStep::Reopen(REOPEN_TABS)
        );

        let closed = watch_after_close(watch);
        assert_eq!(closed.last_reopen, Some(now + REOPEN_GAP_MIN));
        assert_eq!(closed.reopens_this_game, 0);
    }

    #[test]
    fn a_relaunch_without_a_token_never_reopens() {
        let now = Instant::now();
        let mut watch = ReopenWatch::default();
        assert_eq!(
            reopen_step(
                Observe::Poll {
                    health: StreamHealth::Error
                },
                false,
                now,
                &mut watch
            ),
            ReopenStep::KeepNoToken
        );
        for _ in 0..4 {
            assert_eq!(
                reopen_step(
                    Observe::Poll {
                        health: StreamHealth::Error
                    },
                    false,
                    now,
                    &mut watch
                ),
                ReopenStep::Keep
            );
        }
        let mut tabs = ReopenWatch::default();
        assert_eq!(
            reopen_step(tab_obs(true, false, false), false, now, &mut tabs),
            ReopenStep::Keep
        );
        assert_eq!(
            reopen_step(tab_obs(true, false, false), false, now, &mut tabs),
            ReopenStep::KeepNoToken
        );
        assert_eq!(
            reopen_step(
                Observe::Poll {
                    health: StreamHealth::Streaming
                },
                false,
                now,
                &mut tabs
            ),
            ReopenStep::Keep
        );
        assert!(tabs.pending.is_none());
        assert_eq!(tabs.reopens_this_game, 0);
    }

    #[test]
    fn a_new_game_resets_the_stall_watch_except_the_gap_clock() {
        let now = Instant::now();
        let mut watch = ReopenWatch {
            soft_misses: 2,
            last_reopen: Some(now),
            pending: Some(Pending::Tabs),
            death_latched: true,
            token_warned: true,
            reopens_this_game: 3,
        };
        STALL_RESET.store(false, Ordering::Release);
        assert!(!note_game_edge(true, true));
        apply_stall_reset(&mut watch);
        assert_eq!(
            watch.reopens_this_game, 3,
            "staying in a game does not reset"
        );
        assert!(note_game_edge(false, true));
        apply_stall_reset(&mut watch);
        assert_eq!(watch.reopens_this_game, 0);
        assert_eq!(watch.soft_misses, 0);
        assert!(watch.pending.is_none());
        assert!(!watch.death_latched);
        assert!(!watch.token_warned);
        assert_eq!(watch.last_reopen, Some(now));
        STALL_RESET.store(false, Ordering::Release);
    }

    fn poll(health: StreamHealth, now: Instant, watch: &mut ReopenWatch) -> ReopenStep {
        reopen_step(Observe::Poll { health }, true, now, watch)
    }

    #[test]
    fn the_reopen_gap_escalates_and_never_closes() {
        assert_eq!(reopen_gap(0), REOPEN_GAP_MIN);
        assert_eq!(reopen_gap(1), REOPEN_GAP_MIN);
        assert_eq!(reopen_gap(2), REOPEN_GAP_MIN * 2);
        assert_eq!(reopen_gap(3), REOPEN_GAP_MIN * 4);
        assert_eq!(reopen_gap(4), REOPEN_GAP_MAX);
        assert_eq!(reopen_gap(u32::MAX), REOPEN_GAP_MAX);
        for n in 0..64 {
            assert!(reopen_gap(n) >= REOPEN_GAP_MIN);
            assert!(reopen_gap(n) <= REOPEN_GAP_MAX);
        }
    }

    #[test]
    fn a_second_dead_stream_in_the_same_game_reopens_after_the_gap() {
        let start = Instant::now();
        let mut watch = ReopenWatch::default();
        assert_eq!(
            poll(StreamHealth::Error, start, &mut watch),
            ReopenStep::Reopen(REOPEN_DEAD)
        );
        // The new stream comes up, then dies again in the same game.
        let up = start + Duration::from_secs(5);
        assert_eq!(
            poll(StreamHealth::Streaming, up, &mut watch),
            ReopenStep::Keep
        );
        let died = start + Duration::from_secs(20);
        for at in [died, died + Duration::from_secs(10)] {
            assert_eq!(
                poll(StreamHealth::Error, at, &mut watch),
                ReopenStep::Keep,
                "a stall inside the gap waits"
            );
        }
        // A Tab that sees the dead stream inside the gap holds for a poll and
        // keeps the reopen wanted.
        assert_eq!(
            reopen_step(
                Observe::Tab {
                    health: StreamHealth::Error,
                    waited: false,
                    got_fresh: false,
                    buffer_during_wait: false,
                },
                true,
                died + Duration::from_secs(15),
                &mut watch
            ),
            ReopenStep::HoldForPoll
        );
        assert_eq!(
            poll(
                StreamHealth::Error,
                start + REOPEN_GAP_MIN - Duration::from_secs(1),
                &mut watch
            ),
            ReopenStep::Keep
        );
        assert!(matches!(watch.pending, Some(Pending::Dead)));
        assert_eq!(
            poll(StreamHealth::Error, start + REOPEN_GAP_MIN, &mut watch),
            ReopenStep::Reopen(REOPEN_DEAD),
            "the second stall in one game is not left dead"
        );
        assert_eq!(watch.reopens_this_game, 2);
        assert!(watch.pending.is_none());
    }

    #[test]
    fn a_stream_that_keeps_dying_is_reopened_without_a_storm() {
        // Polls every 1.5 s for two hours of one game. Each reopened stream
        // streams once and then dies again.
        let start = Instant::now();
        let step = Duration::from_millis(1500);
        let mut watch = ReopenWatch::default();
        let mut reopens: Vec<Instant> = Vec::new();
        let mut fresh_stream = false;
        let mut at = start;
        while at < start + Duration::from_secs(2 * 60 * 60) {
            let health = if fresh_stream {
                fresh_stream = false;
                StreamHealth::Streaming
            } else {
                StreamHealth::Error
            };
            if let ReopenStep::Reopen(_) = poll(health, at, &mut watch) {
                reopens.push(at);
                fresh_stream = true;
            }
            at += step;
        }
        for pair in reopens.windows(2) {
            assert!(
                pair[1] - pair[0] >= REOPEN_GAP_MIN,
                "two reopens closer than the minimum gap"
            );
            assert!(
                pair[1] - pair[0] <= REOPEN_GAP_MAX + step,
                "a dead stream waited longer than the longest gap"
            );
        }
        // Reopens at about 0, 1, 3 and 7 minutes, then every 5 minutes.
        assert!(
            reopens.len() <= 30,
            "{} reopens in two hours is a storm",
            reopens.len()
        );
        let last = *reopens.last().expect("the stream was reopened");
        assert!(
            start + Duration::from_secs(2 * 60 * 60) - last <= REOPEN_GAP_MAX + step,
            "the stream was left dead at the end of the game"
        );
    }

    fn report(
        phase: pw::ReportedPhase,
        paused_for: Option<Duration>,
        self_paused: bool,
    ) -> pw::StreamReport {
        pw::StreamReport {
            phase,
            paused_for,
            self_paused,
            core_fault: false,
            has_streamed: true,
        }
    }

    #[test]
    fn our_own_pause_is_a_healthy_stream_and_a_foreign_pause_still_dies() {
        let ours = health_of(report(
            pw::ReportedPhase::Paused,
            Some(Duration::ZERO),
            true,
        ));
        assert_eq!(ours, StreamHealth::Streaming);
        assert!(!health_is_dead(ours));
        let foreign = health_of(report(
            pw::ReportedPhase::Paused,
            Some(PAUSED_PERSIST),
            false,
        ));
        assert_eq!(
            foreign,
            StreamHealth::Paused {
                paused_for: PAUSED_PERSIST
            }
        );
        assert!(health_is_dead(foreign));
        // A dead foreign pause reopens on the next poll, as before.
        let now = Instant::now();
        let mut watch = ReopenWatch::default();
        assert_eq!(
            reopen_step(Observe::Poll { health: foreign }, true, now, &mut watch),
            ReopenStep::Reopen(REOPEN_DEAD)
        );
        // Our pause keeps a latched death from blocking the next one: it
        // reads as Streaming and clears the latch.
        assert!(watch.death_latched);
        assert_eq!(
            reopen_step(Observe::Poll { health: ours }, true, now, &mut watch),
            ReopenStep::Keep
        );
        assert!(!watch.death_latched);
        // Tab soft misses still count while our pause reads as Streaming.
        let tab = Observe::Tab {
            health: ours,
            waited: true,
            got_fresh: false,
            buffer_during_wait: false,
        };
        reopen_step(tab, true, now, &mut watch);
        assert_eq!(watch.soft_misses, 1);
    }

    #[test]
    fn a_closed_session_marks_the_stream_as_ended_from_outside() {
        let open = pw::test_stream(true);
        mark_external_if(false, &open);
        assert!(!open.external_end_for_test());
        let closed = pw::test_stream(true);
        mark_external_if(true, &closed);
        assert!(closed.external_end_for_test());
    }

    /// Where a closed portal session is told to the pause fail-safe, and
    /// that it comes first: in `health_now` before the stuck check, in
    /// `Live::drop` before the stream (and its retire check) is dropped, and
    /// in `capture` before the grab's verdict.
    #[test]
    fn a_closed_session_is_marked_before_each_pause_verdict() {
        let src = include_str!("mod.rs");
        let body = |start: &str, end: &str| -> String {
            let at = src.find(start).unwrap_or_else(|| panic!("{start}"));
            let rest = &src[at..];
            rest[..rest.find(end).unwrap_or_else(|| panic!("{end}"))].to_string()
        };
        let mark = concat!("mark_if_portal_closed(", "");
        let health = body(concat!("fn health_now(", "stream"), "\n}\n");
        assert!(
            health.find(mark).expect("health_now marks")
                < health
                    .find(concat!("check_stuck_", "resume()"))
                    .expect("stuck check")
        );
        let live = body(concat!("impl Drop for ", "Live {"), "\n}\n");
        assert!(
            live.find(mark).expect("Live::drop marks")
                < live
                    .find(concat!("drop(", "stream);"))
                    .expect("stream dropped")
        );
        let capture = body(concat!("pub async fn ", "capture("), "\n}\n");
        let grab = capture
            .find(concat!("stream.grab(", "timeout"))
            .expect("the grab");
        let after = &capture[grab..];
        assert!(
            after.find(mark).expect("capture marks after the grab")
                < after
                    .find(concat!("note_grab_", "result(&grabbed)"))
                    .expect("the verdict")
        );
    }

    /// The round-1 fix's wiring: `health_now` runs the stuck-resume check
    /// before it reads health, so a stream stuck after the tracker's own
    /// pause trips (and is counted) before the 2 s paused rule reopens it.
    #[test]
    fn health_now_trips_a_stuck_resume_before_reading_health() {
        let stream = pw::test_stream_stuck_after_our_pause();
        assert!(!stream.pause_tripped());
        let health = health_now(&stream);
        assert!(stream.pause_tripped(), "tripped by the health read");
        assert!(stream.wants_active(), "running as rc.6 from here");
        // The pause is still real, so health still reports it.
        assert!(matches!(health, StreamHealth::Paused { .. }));
    }

    #[test]
    fn a_poll_that_resumes_waits_longer_but_under_the_poll() {
        assert_eq!(poll_wait(4, false), Duration::from_millis(350));
        assert_eq!(poll_wait(4, true), Duration::from_millis(750));
        assert_eq!(poll_wait(2, true), Duration::from_millis(1_000));
        for fps in 1..=5 {
            assert!(poll_wait(fps, true) <= crate::config::NEW_READER_POLL_INTERVAL);
        }
    }

    #[test]
    fn a_stall_classifies_as_dead_then_waiting_then_quiet() {
        let short = QUIET_STALL - Duration::from_millis(1);
        assert_eq!(classify_stall(true, true, short), Some(Stall::Dead));
        assert_eq!(
            classify_stall(false, true, Duration::ZERO),
            Some(Stall::ReopenWaiting)
        );
        assert_eq!(
            classify_stall(false, false, QUIET_STALL),
            Some(Stall::Quiet)
        );
        assert_eq!(classify_stall(false, false, short), None);
    }

    const KINDS: [CaptureKind; 2] = [CaptureKind::Tab, CaptureKind::Poll];
    const STALLS: [Stall; 3] = [Stall::Dead, Stall::ReopenWaiting, Stall::Quiet];

    #[test]
    fn a_stalled_stream_screenshots_on_niri_wlroots_and_forced_gnome_never_on_kde() {
        for stall in STALLS {
            for kind in KINDS {
                for forced in [false, true] {
                    for kde in ["KDE", "plasma", "KDE:plasma", "kwin"] {
                        assert!(
                            !stall_screenshot(kde, forced, Some(stall), kind),
                            "{kde} forced={forced} {stall:?} {kind:?} must not screenshot"
                        );
                    }
                    for safe in ["niri", "sway", "Hyprland", "wlroots", "river"] {
                        assert!(
                            stall_screenshot(safe, forced, Some(stall), kind),
                            "{safe} forced={forced} {stall:?} {kind:?} must fall back"
                        );
                    }
                }
                assert!(!stall_screenshot("GNOME", false, Some(stall), kind));
            }
        }
        for desktop in ["niri", "sway", "GNOME", "KDE"] {
            for kind in KINDS {
                assert!(
                    !stall_screenshot(desktop, true, None, kind),
                    "{desktop}: a healthy stream is not replaced by a screenshot"
                );
            }
        }
    }

    #[test]
    fn forced_gnome_never_screenshots_a_quiet_poll() {
        for gnome in ["GNOME", "ubuntu:GNOME"] {
            assert!(
                !stall_screenshot(gnome, true, Some(Stall::Quiet), CaptureKind::Poll),
                "{gnome}: the portal flashes the screen on every quiet poll"
            );
            assert!(stall_screenshot(
                gnome,
                true,
                Some(Stall::Quiet),
                CaptureKind::Tab
            ));
            for stall in [Stall::Dead, Stall::ReopenWaiting] {
                for kind in KINDS {
                    assert!(
                        stall_screenshot(gnome, true, Some(stall), kind),
                        "{gnome} {stall:?} {kind:?} keeps its fallback"
                    );
                }
            }
        }
    }

    #[test]
    fn wayshot_is_tried_first_on_screencopy_desktops_only() {
        for desktop in ["niri", "sway", "Hyprland", "wayfire", "river", "labwc"] {
            assert_eq!(first_shot_method(desktop, true), ShotMethod::Wayshot);
            assert_eq!(
                first_shot_method(desktop, false),
                ShotMethod::Portal,
                "{desktop}: no Wayland socket, no wayshot"
            );
        }
        for desktop in ["GNOME", "ubuntu:GNOME", "KDE", "plasma", ""] {
            assert_eq!(first_shot_method(desktop, true), ShotMethod::Portal);
        }
    }

    #[test]
    fn quiet_poll_screenshots_are_at_most_one_per_gap() {
        let now = Instant::now();
        assert!(quiet_poll_shot_due(None, now));
        assert!(!quiet_poll_shot_due(Some(now), now));
        assert!(!quiet_poll_shot_due(
            Some(now),
            now + QUIET_SHOT_GAP - Duration::from_millis(1)
        ));
        assert!(quiet_poll_shot_due(Some(now), now + QUIET_SHOT_GAP));

        // 1.5 s polls on a still niri screen for one minute: about six shots.
        let mut last = None;
        let mut shots = 0;
        let mut at = now;
        while at < now + Duration::from_secs(60) {
            let due = quiet_poll_shot_due(last, at);
            if stall_fallback_at(
                StallSite::ReusedFrame,
                CaptureKind::Poll,
                Some(Stall::Quiet),
                "niri",
                true,
                due,
            )
            .is_some()
            {
                shots += 1;
                last = Some(at);
            }
            at += Duration::from_millis(1500);
        }
        assert!((5..=6).contains(&shots), "{shots} quiet shots in a minute");
    }

    #[test]
    fn each_capture_site_decides_its_own_stall_fallback() {
        use StallSite::*;
        let running = GrabError {
            shutting_down: false,
        };
        let stopping = GrabError {
            shutting_down: true,
        };
        // A dead stream before the grab is a Dead stall for Tab and poll.
        for kind in KINDS {
            assert_eq!(
                stall_fallback_at(DeadBeforeGrab, kind, None, "niri", true, false),
                Some(Stall::Dead)
            );
            assert_eq!(
                stall_fallback_at(DeadBeforeGrab, kind, None, "KDE", true, true),
                None
            );
        }
        // A reused frame only matters to a poll.
        for stall in STALLS {
            assert_eq!(
                stall_fallback_at(
                    ReusedFrame,
                    CaptureKind::Tab,
                    Some(stall),
                    "niri",
                    true,
                    true
                ),
                None
            );
            assert_eq!(
                stall_fallback_at(
                    ReusedFrame,
                    CaptureKind::Poll,
                    Some(stall),
                    "niri",
                    true,
                    true
                ),
                Some(stall)
            );
        }
        // The quiet rate limit holds back a poll; dead and waiting ones are not limited.
        assert_eq!(
            stall_fallback_at(
                ReusedFrame,
                CaptureKind::Poll,
                Some(Stall::Quiet),
                "niri",
                true,
                false
            ),
            None
        );
        for stall in [Stall::Dead, Stall::ReopenWaiting] {
            for site in [ReusedFrame, running] {
                assert_eq!(
                    stall_fallback_at(site, CaptureKind::Poll, Some(stall), "sway", false, false),
                    Some(stall)
                );
            }
        }
        // A Tab grab error on a quiet stream is not rate limited.
        assert_eq!(
            stall_fallback_at(
                running,
                CaptureKind::Tab,
                Some(Stall::Quiet),
                "niri",
                true,
                false
            ),
            Some(Stall::Quiet)
        );
        // Nothing during shutdown, nothing without a stall, nothing on KDE.
        for kind in KINDS {
            assert_eq!(
                stall_fallback_at(stopping, kind, Some(Stall::Dead), "niri", true, true),
                None
            );
            assert_eq!(
                stall_fallback_at(running, kind, None, "niri", true, true),
                None
            );
            for stall in STALLS {
                for site in [ReusedFrame, running] {
                    assert_eq!(
                        stall_fallback_at(site, kind, Some(stall), "KDE", true, true),
                        None
                    );
                }
            }
        }
        // Forced GNOME: no quiet poll, but a dead stream still falls back.
        assert_eq!(
            stall_fallback_at(
                ReusedFrame,
                CaptureKind::Poll,
                Some(Stall::Quiet),
                "GNOME",
                true,
                true
            ),
            None
        );
        assert_eq!(
            stall_fallback_at(DeadBeforeGrab, CaptureKind::Poll, None, "GNOME", true, true),
            Some(Stall::Dead)
        );
    }

    #[test]
    fn capture_calls_reopen_step_and_a_poll_waits_one_frame_plus_margin() {
        assert_eq!(reuse_wait(2), Duration::from_millis(600));
        assert_eq!(reuse_wait(4), Duration::from_millis(350));
        assert_eq!(REOPEN_GAP_MIN, Duration::from_secs(60));
        assert_eq!(REOPEN_GAP_MAX, Duration::from_secs(5 * 60));
        let src = include_str!("mod.rs");
        let capture_at = src.find("pub async fn capture(").expect("capture");
        let after = &src[capture_at..];
        let end = after.find("\nasync fn tab_timeout(").expect("tab_timeout");
        let body = &after[..end];
        assert!(
            body.contains("reopen_step("),
            "deleting the reopen call from capture must fail"
        );
        assert!(
            body.contains("apply_stall_reset("),
            "a new game must reset the stall watch inside capture"
        );
        assert!(
            body.contains("release_live("),
            "capture must drop the PipeWire thread off the session lock"
        );
        assert!(body.contains("had_token"));
        assert!(body.contains("clear_portal_warn()"));
        let skip = src
            .find("fn note_tab_stream_miss(")
            .expect("note_tab_stream_miss");
        let timeout = &src[src.find("async fn tab_timeout(").unwrap()..skip];
        assert!(
            timeout.contains(
                "note_tab_stream_miss(delay_ms, super::tab::stream_miss_log_suppressed())"
            ),
            "the first-Tab suppress flag must gate the miss log"
        );
        let before = tab_timeout_counts().0;
        assert!(!note_tab_stream_miss(12, true));
        assert_eq!(tab_timeout_counts().0, before);
        assert!(note_tab_stream_miss(12, false));
        assert_eq!(tab_timeout_counts().0, before + 1);
        assert!(capture_now_blocked_message(true).contains("Quit Overwatch"));
        assert!(!capture_now_blocked_message(false).contains("Quit Overwatch"));
        assert!(capture_now_blocked_message(false).contains("Stop the tracker"));
    }

    #[test]
    fn a_declined_share_stops_prompts_and_captures_until_overwatch_starts() {
        let _reset = DeclineReset;
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(decline_notice(false), NO_CAPTURE_DECLINE_NOTICE);
        assert!(!NO_CAPTURE_DECLINE_NOTICE.contains("KDE"));
        assert!(NO_CAPTURE_DECLINE_NOTICE.starts_with("Screen share was declined"));
        assert!(!decline_notice(true).contains("no captures"));
        assert!(!screenshot_fallback_is_safe("KDE", false));
        assert!(!capture_while_declined(screenshot_fallback_is_safe(
            "KDE", false
        )));
        assert!(capture_while_declined(screenshot_fallback_is_safe(
            "niri", false
        )));
        assert!(!share_prompt_allowed(true));
        assert!(share_prompt_allowed(false));
        assert!(is_share_declined("org.freedesktop.portal.Error.Cancelled"));
        assert!(is_share_declined("Dialog dismissed by the user"));
        assert!(is_share_declined("Dialog was closed"));
        assert!(!is_share_declined("screencast session closed"));
        assert!(!is_share_declined("screencast permission dialog timed out"));
        assert!(is_permission_timeout(
            "screencast permission dialog timed out"
        ));
        assert!(is_declined_capture(NO_CAPTURE_DECLINE_NOTICE));
        assert!(is_declined_capture(timeout_notice(false)));

        let declined = open_error_action("select_sources cancelled", false, false);
        assert!(declined.declined);
        assert!(!declined.arm_backoff);
        assert_eq!(declined.notice, Some(NO_CAPTURE_DECLINE_NOTICE));
        let timeout = open_error_action("screencast permission dialog timed out", false, false);
        assert!(
            timeout.declined,
            "a timeout with no token sent is a No for this launch"
        );
        assert!(!timeout.arm_backoff);
        assert_eq!(timeout.notice, Some(timeout_notice(false)));
        let timeout_with_token =
            open_error_action("screencast permission dialog timed out", false, true);
        assert!(
            !timeout_with_token.declined,
            "a timeout after a token was sent is not a No"
        );
        assert!(timeout_with_token.arm_backoff);
        assert!(timeout_with_token.notice.is_none());
        let timeout_with_token_safe =
            open_error_action("screencast permission dialog timed out", true, true);
        assert!(!timeout_with_token_safe.declined);
        assert!(timeout_with_token_safe.arm_backoff);
        assert!(timeout_notice(false).contains("no captures"));
        assert!(!timeout_notice(true).contains("no captures"));
        assert!(timeout_notice(true).contains("Older screenshot"));
        let safe = open_error_action("Dialog was closed", true, false);
        assert_eq!(safe.notice, Some(SAFE_DECLINE_NOTICE));
        assert!(!SAFE_DECLINE_NOTICE.contains("no captures"));
        assert!(SAFE_DECLINE_NOTICE.contains("Older screenshot"));

        match select_error("Dialog was closed".into(), true) {
            OpenFail::Other(msg) => assert_eq!(msg, "select_sources cancelled"),
            OpenFail::Select(_) => panic!("closing the dialog must keep the restore token"),
        }
        match select_error("portal rejected the request".into(), true) {
            OpenFail::Select(_) => {}
            OpenFail::Other(msg) => panic!("a rejected token is still dropped: {msg}"),
        }

        let once = with_log(|| {
            let mut logged = false;
            assert!(warn_share_declined(&mut logged, NO_CAPTURE_DECLINE_NOTICE));
            assert!(!warn_share_declined(&mut logged, NO_CAPTURE_DECLINE_NOTICE));
        });
        assert_eq!(once.matches(NO_CAPTURE_DECLINE_NOTICE).count(), 1);
        assert!(once.contains("WARN"));

        let mut last_fail = Some(Instant::now());
        let first = with_log(|| {
            let action = apply_share_decline(
                "select_sources cancelled",
                false,
                dir.path(),
                &mut last_fail,
                false,
            );
            assert!(action.declined);
            assert!(!action.arm_backoff);
            assert!(last_fail.is_none());
            assert!(should_try_stream(None, START_BACKOFF));
            assert!(!share_prompt_allowed(share_is_declined()));
            assert!(!capture_while_declined(false));
            let mut again = None;
            assert!(
                apply_share_decline(
                    "Dialog dismissed by the user",
                    false,
                    dir.path(),
                    &mut again,
                    false,
                )
                .declined
            );
        });
        assert_eq!(first.matches(NO_CAPTURE_DECLINE_NOTICE).count(), 1);
        let status = crate::ocr_status::read_status(dir.path()).unwrap();
        assert_eq!(status.share_notice, NO_CAPTURE_DECLINE_NOTICE);
        crate::ocr_status::publish_share_open(dir.path(), false);
        assert_eq!(
            crate::ocr_status::read_status(dir.path())
                .unwrap()
                .share_notice,
            NO_CAPTURE_DECLINE_NOTICE
        );

        assert!(!clears_share_decline(true, true));
        assert!(!clears_share_decline(true, false));
        assert!(!clears_share_decline(false, false));
        note_game_seen(true, true, dir.path());
        assert!(share_is_declined(), "staying in a game does not ask again");
        assert!(clears_share_decline(false, true));
        note_game_seen(false, true, dir.path());
        assert!(!share_is_declined());
        assert!(share_prompt_allowed(share_is_declined()));
        assert!(
            crate::ocr_status::read_status(dir.path())
                .unwrap()
                .share_notice
                .is_empty()
        );

        let mut relaunch = Some(Instant::now());
        let second = with_log(|| {
            assert!(
                apply_share_decline(
                    "select_sources cancelled",
                    false,
                    dir.path(),
                    &mut relaunch,
                    false,
                )
                .declined
            );
            let mut poll = None;
            assert!(
                apply_share_decline(
                    "select_sources cancelled",
                    false,
                    dir.path(),
                    &mut poll,
                    false,
                )
                .declined
            );
        });
        assert_eq!(second.matches(NO_CAPTURE_DECLINE_NOTICE).count(), 1);
        assert!(relaunch.is_none());
    }
}
