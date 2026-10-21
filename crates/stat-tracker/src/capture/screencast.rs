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
//! (niri, wlroots, forced GNOME). KDE skips it and logs. A failed start is
//! not retried for [`START_BACKOFF`]. Cancelling or closing the permission
//! dialog is a decline: asking stops until the next Overwatch launch, KDE
//! takes no captures, and the restore token stays. A permission dialog that
//! times out is the same No until the next Overwatch launch. Shutdown aborts
//! the dialog.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use ashpd::desktop::screencast::{
    CursorMode, Screencast, SelectSourcesOptions, SourceType, Stream,
};
use ashpd::desktop::{PersistMode, Session};
use image::DynamicImage;

use super::CaptureKind;
use super::framebuf;
use super::portal::{self, OutputRect};
use super::portal_backend;
use super::pw;

/// How long a grab waits after the session is up. The permission dialog
/// is not on this timer.
pub const SCREENCAST_FRAME_WAIT: Duration = Duration::from_secs(2);

/// After a failed start or a grab with no frame, wait this long before
/// opening the stream again. A screenshot in that window is only taken
/// where [`screenshot_fallback_is_safe`] is true.
pub const START_BACKOFF: Duration = Duration::from_secs(30);

/// A session that never reaches STREAMING is closed after this, then
/// [`START_BACKOFF`] applies. Grabs before the bound still wait for a frame.
pub const NEVER_STREAMING_WAIT: Duration = Duration::from_secs(6);

/// The permission dialog is closed after this. Shutdown aborts the open
/// sooner, and dropping that task closes the portal session.
pub const PERMISSION_DIALOG_TIMEOUT: Duration = Duration::from_secs(60);

/// A stream that has delivered a frame and then gone this long without
/// another is closed and opened again, while Overwatch is running.
pub const STREAM_STALL: Duration = Duration::from_secs(8);

/// True when a stream that already produced a frame has gone `limit`
/// without another, and the game is running. A stream that has not yet
/// produced a frame is the never-streaming path, not this one.
pub fn stream_stall_reopen(
    last_new_frame: Option<Instant>,
    now: Instant,
    game_running: bool,
    limit: Duration,
) -> bool {
    game_running && last_new_frame.is_some_and(|at| now.saturating_duration_since(at) >= limit)
}

static STALL_WARNED: AtomicBool = AtomicBool::new(false);

fn warn_stream_stall() {
    if STALL_WARNED.swap(true, Ordering::AcqRel) {
        tracing::debug!("screencast stream stalled with no new frame; reopening");
    } else {
        tracing::warn!("screencast stream stalled with no new frame; reopening");
    }
}

const TOKEN_FILE: &str = "screencast-restore-token";
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
        // consumer is gone before the portal session ends.
        drop(self.stream.take());
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
    /// When the live stream last handed out a new frame. `None` until the
    /// first one, and after the session is dropped.
    last_new_frame: Option<Instant>,
    /// Data dir for the status file. Set on each capture.
    data_dir: Option<PathBuf>,
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
            last_new_frame: None,
            data_dir: None,
        }
    }
}

static CAST: LazyLock<tokio::sync::Mutex<CastState>> =
    LazyLock::new(|| tokio::sync::Mutex::new(CastState::default()));

/// True when a new session attempt is allowed. `age` is how long ago the
/// last failure was.
pub fn should_try_stream(last_fail_age: Option<Duration>, backoff: Duration) -> bool {
    match last_fail_age {
        None => true,
        Some(age) => age >= backoff,
    }
}

/// True on the running to not-running edge, and only when this process is
/// on the screen stream and a restore token can reopen it without a prompt.
/// That includes a forced `screencast` or Settings stream on niri, not only
/// the KDE or GNOME default. Without a token the share stays up, so the
/// next game does not ask again.
pub fn should_close_share(
    stream_route: bool,
    was_running: bool,
    running: bool,
    restorable: bool,
) -> bool {
    stream_route && was_running && !running && restorable
}

/// True when a restore token is on disk. The bytes are not logged.
pub fn restore_token_saved(data_dir: &Path) -> bool {
    read_token(data_dir).is_some()
}

/// A deliberate close is not a failed start. The failure clock is left as
/// How many grabs with no usable buffer close a session that is streaming.
pub const FRESH_MISS_LIMIT: u32 = 3;

pub fn fresh_miss_should_restart(misses: u32, limit: u32) -> bool {
    misses >= limit
}

/// No new stream frame. Retried, and never written into debug/rejected.
pub const STREAM_MISS: &str = "no fresh stream frame";

/// The share dialog is still up. A Tab waits this out on the original press.
pub const OPEN_IN_PROGRESS: &str = "screencast open in progress";

pub fn is_stream_miss(err: &str) -> bool {
    err.contains(STREAM_MISS)
}

pub fn is_share_opening(err: &str) -> bool {
    err.contains(OPEN_IN_PROGRESS)
}

/// Set when the share is closed because the game stopped. Cleared when the
/// next game is seen, so a Tab retry cannot open the dialog again.
static REOPEN_BLOCKED: AtomicBool = AtomicBool::new(false);

/// The user cancelled or closed the permission dialog. In memory for this
/// process. A new Overwatch launch clears it.
static SHARE_DECLINED: AtomicBool = AtomicBool::new(false);

/// The one decline warning has been written. Cleared with [`SHARE_DECLINED`].
static DECLINE_LOGGED: AtomicBool = AtomicBool::new(false);

static GAME_RUNNING: AtomicBool = AtomicBool::new(false);

pub fn note_game_running(running: bool) {
    GAME_RUNNING.store(running, Ordering::Release);
    if running {
        REOPEN_BLOCKED.store(false, Ordering::Release);
    }
}

fn game_is_running() -> bool {
    GAME_RUNNING.load(Ordering::Acquire)
}

/// The sentence for a declined share. A desktop where a screenshot is not
/// safe says captures stop. A desktop that already screenshots does not.
pub const NO_CAPTURE_DECLINE_NOTICE: &str =
    "Screen share was declined: no captures until Overwatch is restarted";

pub const SAFE_DECLINE_NOTICE: &str =
    "Screen share was declined: screenshots continue until Overwatch is restarted";

pub fn decline_notice(fallback_safe: bool) -> &'static str {
    if fallback_safe {
        SAFE_DECLINE_NOTICE
    } else {
        NO_CAPTURE_DECLINE_NOTICE
    }
}

/// A timed-out permission dialog is a No for this launch. Flip this if a
/// timeout should ask again after the backoff instead.
const PERMISSION_TIMEOUT_IS_NO: bool = true;

pub fn is_permission_timeout(err: &str) -> bool {
    err.to_ascii_lowercase()
        .contains("permission dialog timed out")
}

pub fn timeout_notice(fallback_safe: bool) -> &'static str {
    if fallback_safe {
        "Screen share permission timed out: screenshots continue until Overwatch is restarted"
    } else {
        "Screen share permission timed out: no captures until Overwatch is restarted"
    }
}

/// The error from a declined share or a timed-out dialog. A later Tab must
/// not log it again. The one warn was written when the dialog ended.
pub fn is_declined_capture(err: &str) -> bool {
    err.contains("Screen share was declined") || err.contains("Screen share permission timed out")
}

/// A declined share does not open the portal again.
pub fn share_prompt_allowed(declined: bool) -> bool {
    !declined
}

/// While a share is declined, screenshot only where that fallback is already
/// safe. KDE does not.
pub fn capture_while_declined(fallback_safe: bool) -> bool {
    fallback_safe
}

/// A decline clears only when the game goes from not running to running.
/// That is the process-start edge the loop already tracks. Staying running
/// does not ask again.
pub fn clears_share_decline(was_running: bool, running: bool) -> bool {
    !was_running && running
}

/// The user cancelled or closed the permission dialog. Closing the dialog
/// is the portal Cancelled or Dismissed response, or a line that says the
/// dialog was closed. A session close is not this. A timeout is a No only
/// when [`PERMISSION_TIMEOUT_IS_NO`] is set, and that check lives in
/// [`open_error_action`].
pub fn is_share_declined(err: &str) -> bool {
    if is_user_cancel(err) {
        return true;
    }
    let lower = err.to_ascii_lowercase();
    lower.contains("dialog") && lower.contains("close")
}

/// Logs `notice` once. A later call with the flag set does not log again.
pub fn warn_share_declined(already_logged: &mut bool, notice: &str) -> bool {
    if *already_logged {
        return false;
    }
    tracing::warn!("{notice}");
    *already_logged = true;
    true
}

/// What a failed open does. A decline does not start the backoff clock, so
/// the next launch is not stuck waiting, and it does not ask again until
/// that launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenErrorAction {
    pub declined: bool,
    pub arm_backoff: bool,
    pub notice: Option<&'static str>,
}

pub fn open_error_action(err: &str, fallback_safe: bool) -> OpenErrorAction {
    if PERMISSION_TIMEOUT_IS_NO && is_permission_timeout(err) {
        OpenErrorAction {
            declined: true,
            arm_backoff: false,
            notice: Some(timeout_notice(fallback_safe)),
        }
    } else if is_share_declined(err) {
        OpenErrorAction {
            declined: true,
            arm_backoff: false,
            notice: Some(decline_notice(fallback_safe)),
        }
    } else {
        OpenErrorAction {
            declined: false,
            arm_backoff: true,
            notice: None,
        }
    }
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
) -> OpenErrorAction {
    let action = open_error_action(err, fallback_safe);
    if !action.declined {
        return action;
    }
    SHARE_DECLINED.store(true, Ordering::Release);
    *last_fail = None;
    if let Some(notice) = action.notice {
        let mut already = DECLINE_LOGGED.swap(true, Ordering::AcqRel);
        warn_share_declined(&mut already, notice);
        crate::ocr_switch::publish_share_notice(data_dir, notice);
    }
    action
}

/// `note_game_running`, and clear a decline on a new Overwatch launch.
pub fn note_game_seen(was_running: bool, running: bool, data_dir: &Path) {
    note_game_running(running);
    if !clears_share_decline(was_running, running) {
        return;
    }
    if !SHARE_DECLINED.swap(false, Ordering::AcqRel) {
        return;
    }
    DECLINE_LOGGED.store(false, Ordering::Release);
    crate::ocr_switch::publish_share_notice(data_dir, "");
}

/// A new session may start when the game-exit close is not holding the
/// share shut, or when one is already up.
pub fn may_open_share(reopen_blocked: bool, live: bool, opening: bool) -> bool {
    !reopen_blocked || live || opening
}

/// The token file is read only on the running to not-running edge.
pub fn share_close_due(
    stream_route: bool,
    was_running: bool,
    running: bool,
    restorable: impl FnOnce() -> bool,
) -> bool {
    if !should_close_share(stream_route, was_running, running, true) {
        return false;
    }
    restorable()
}

/// Capture now on KDE must not open the screenshot portal while the daemon
/// says its share is open.
pub fn capture_now_blocked_by_daemon_share(desktop: &str, daemon_share_open: bool) -> bool {
    daemon_share_open && desktop_has(desktop, &["kde", "plasma", "kwin"])
}

/// What a second caller may do while a session open is in progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenPermit {
    Open,
    Reuse,
    Busy,
}

pub fn open_permit(opening: bool, live: bool) -> OpenPermit {
    if opening {
        OpenPermit::Busy
    } else if live {
        OpenPermit::Reuse
    } else {
        OpenPermit::Open
    }
}

/// A portal session that was created and then failed or timed out must be
/// closed. A success hands the session to the hold task.
pub fn must_close_session(created: bool, succeeded: bool) -> bool {
    created && !succeeded
}

/// How long a later grab waits for a new buffer before reusing the last
/// converted frame. One poll converts at most one frame. The new-reader
/// poll is 1.5 s, so this stays well under that.
pub fn reuse_wait(max_fps: u32) -> Duration {
    Duration::from_millis(pw::throttle_gap_ms(max_fps).min(400))
}

/// What a Tab stream timeout does. KDE/KWin must not open the screenshot
/// portal while a share is up. niri and other wlroots compositors can, and
/// so can GNOME when the stream was forced rather than chosen automatically.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabTimeoutAction {
    Skip,
    Screenshot,
}

pub fn is_user_cancel(err: &str) -> bool {
    let lower = err.to_ascii_lowercase();
    lower.contains("cancel") || lower.contains("dismiss")
}

fn desktop_has(desktop: &str, names: &[&str]) -> bool {
    desktop.split(':').any(|token| {
        let token = token.trim().to_ascii_lowercase();
        names
            .iter()
            .any(|name| token == *name || token.contains(name))
    })
}

pub fn screenshot_fallback_is_safe(desktop: &str, stream_forced: bool) -> bool {
    if desktop_has(desktop, &["kde", "plasma", "kwin"]) {
        return false;
    }
    if desktop_has(
        desktop,
        &[
            "niri", "sway", "hyprland", "wayfire", "river", "labwc", "wlroots",
        ],
    ) {
        return true;
    }
    desktop_has(desktop, &["gnome"]) && stream_forced
}

/// True when a session that has never reached STREAMING should be closed.
pub fn drop_unstarted_stream(has_streamed: bool, uptime: Duration, bound: Duration) -> bool {
    !has_streamed && uptime >= bound
}

pub fn tab_timeout_action(desktop: &str, stream_forced: bool) -> TabTimeoutAction {
    if screenshot_fallback_is_safe(desktop, stream_forced) {
        TabTimeoutAction::Screenshot
    } else {
        TabTimeoutAction::Skip
    }
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
        crate::ocr_switch::publish_share_open(dir, open);
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

fn fresh_capture(image: DynamicImage) -> super::CapturedFrame {
    super::CapturedFrame {
        image,
        seq: None,
        stream: None,
        is_new: true,
    }
}

/// Close the live session when one exists and this process is on the stream
/// route. A no-op otherwise. Does not start the failure backoff. The PipeWire
/// join runs on a blocking thread so a shutdown timer can still fire.
pub async fn close_if_open() {
    if !super::stream_route_active() {
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
    state.last_new_frame = None;
    // A deliberate close does not start the failure backoff. `last_fail`
    // stays whatever it was, which is unset while a session was up, so the
    // next capture opens immediately.
    state.fresh_misses = 0;
    let data_dir = state.data_dir.clone();
    REOPEN_BLOCKED.store(true, Ordering::Release);
    drop(state);
    if let Some(live) = live {
        let _ = tokio::task::spawn_blocking(move || drop(live)).await;
    }
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
) -> Result<super::CapturedFrame, Box<dyn std::error::Error + Send + Sync>> {
    let max_fps = super::prefs().max_fps.max(1);
    let mut state = CAST.lock().await;
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
        return Err("screencast share stays closed until the game is running".into());
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
        state.live.take();
        state.last_new_frame = None;
        mark_share(false, Some(data_dir));
        state.open_gen = state.open_gen.wrapping_add(1);
        let generation = state.open_gen;
        state.opening = true;
        let data_dir = data_dir.to_path_buf();
        let output = output_name.map(str::to_string);
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
                    // here. `PERMISSION_TIMEOUT_IS_NO` decides what it does.
                    Err("screencast permission dialog timed out".into())
                }
            };
            let mut state = CAST.lock().await;
            if state.open_gen != generation {
                drop(result);
                return;
            }
            state.opening = false;
            match result {
                Ok(live) => {
                    if state.live.is_some() {
                        drop(live);
                    } else {
                        state.live = Some(live);
                        state.last_fail = None;
                        mark_share(true, Some(data_dir.as_path()));
                    }
                }
                Err(err) => {
                    let safe = fallback_is_safe();
                    let action =
                        apply_share_decline(&err, safe, data_dir.as_path(), &mut state.last_fail);
                    state.live = None;
                    state.last_new_frame = None;
                    mark_share(false, Some(data_dir.as_path()));
                    if action.arm_backoff {
                        state.last_fail = Some(Instant::now());
                        let line = fallback_line(&format!("screencast session failed ({err})"));
                        note_fallback(&mut state, &line);
                    }
                }
            }
        });
        state.open_task = Some(task);
        drop(state);
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
    if !stream.alive() {
        state.last_fail = Some(Instant::now());
        state.live = None;
        state.last_new_frame = None;
        mark_share(false, Some(data_dir));
        let line = fallback_line("screencast stream is not running");
        note_fallback(&mut state, &line);
        drop(state);
        return screenshot_if_safe(output_name, "screencast stream is not running")
            .await
            .map(fresh_capture);
    }
    if drop_unstarted_stream(
        stream.has_streamed(),
        stream.running_for(),
        NEVER_STREAMING_WAIT,
    ) {
        state.last_fail = Some(Instant::now());
        state.live = None;
        state.last_new_frame = None;
        mark_share(false, Some(data_dir));
        let line = fallback_line("screencast never reached streaming");
        note_fallback(&mut state, &line);
        drop(state);
        return screenshot_if_safe(output_name, "screencast never reached streaming")
            .await
            .map(fresh_capture);
    }
    if stream_stall_reopen(
        state.last_new_frame,
        Instant::now(),
        game_is_running(),
        STREAM_STALL,
    ) {
        warn_stream_stall();
        drop(stream);
        state.last_new_frame = None;
        state.fresh_misses = 0;
        state.live = None;
        mark_share(false, Some(data_dir));
        drop(state);
        return Box::pin(capture(output_name, kind, data_dir, debug_frames)).await;
    }
    let tab_press = match kind {
        CaptureKind::Tab => Some(super::tab_press().unwrap_or_else(Instant::now)),
        CaptureKind::Poll => None,
    };
    let fresh = stream.needs_fresh_frame();
    let timeout = if tab_press.is_some() {
        Duration::from_millis(framebuf::TAB_GRAB_TIMEOUT_AFTER_PRESS_MS)
    } else if fresh {
        SCREENCAST_FRAME_WAIT
    } else {
        reuse_wait(max_fps)
    };
    // The mutex stays held so a second caller cannot open another share.
    let grabbed = stream.grab(timeout, tab_press).await;
    let timing = stream.timing();
    let alive = stream.alive();
    let stream_id = stream.stream_id();
    match grabbed {
        Ok(pw::GrabOutcome::TabMiss { delay_ms }) => {
            drop(state);
            tab_timeout(output_name, delay_ms).await.map(fresh_capture)
        }
        Ok(pw::GrabOutcome::Frame { frame, is_new }) => {
            state.last_fail = None;
            state.fresh_misses = 0;
            state.warned = false;
            if is_new {
                state.last_new_frame = Some(Instant::now());
                STALL_WARNED.store(false, Ordering::Release);
            }
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
            let img = framebuf::to_dynamic(&frame).ok_or("screencast frame format is not RGB")?;
            let img = fit_stream(img, output_name).await?;
            if compare {
                save_compare(data_dir, index, &img, output_name).await;
            }
            Ok(super::CapturedFrame {
                image: img,
                seq: Some(seq),
                stream: Some(stream_id),
                is_new,
            })
        }
        Err(err) => {
            if err == pw::NO_STREAM_FRAME {
                state.fresh_misses = state.fresh_misses.saturating_add(1);
                if fresh_miss_should_restart(state.fresh_misses, FRESH_MISS_LIMIT) {
                    state.fresh_misses = 0;
                    state.last_fail = Some(Instant::now());
                    state.live = None;
                    state.last_new_frame = None;
                    mark_share(false, Some(data_dir));
                    let line = fallback_line("screencast had no usable frame");
                    note_fallback(&mut state, &line);
                    drop(state);
                    return screenshot_if_safe(output_name, "screencast had no usable frame")
                        .await
                        .map(fresh_capture);
                }
            }
            if !alive {
                state.last_fail = Some(Instant::now());
                state.live = None;
                state.last_new_frame = None;
                mark_share(false, Some(data_dir));
                let line = fallback_line(&format!("screencast stream ended ({err})"));
                note_fallback(&mut state, &line);
                drop(state);
                return screenshot_if_safe(output_name, "screencast stream ended")
                    .await
                    .map(fresh_capture);
            }
            if drop_unstarted_stream(
                stream.has_streamed(),
                stream.running_for(),
                NEVER_STREAMING_WAIT,
            ) {
                state.last_fail = Some(Instant::now());
                state.live = None;
                state.last_new_frame = None;
                mark_share(false, Some(data_dir));
                let line = fallback_line("screencast never reached streaming");
                note_fallback(&mut state, &line);
                drop(state);
                return screenshot_if_safe(output_name, "screencast never reached streaming")
                    .await
                    .map(fresh_capture);
            }
            drop(state);
            Err(err.into())
        }
    }
}

async fn tab_timeout(
    output_name: Option<&str>,
    delay_ms: u64,
) -> Result<DynamicImage, Box<dyn std::error::Error + Send + Sync>> {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    match tab_timeout_action(&desktop, super::stream_was_forced()) {
        TabTimeoutAction::Screenshot => {
            let count = TAB_SHOT_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
            tracing::info!(
                delay_ms,
                count,
                "Tab grab: stream timeout, screenshot fallback"
            );
            portal::capture(output_name).await
        }
        TabTimeoutAction::Skip => {
            // A press the open already pushed past the frame budget is
            // discarded and grabbed again. That miss is not a log line.
            if !super::stream_miss_log_suppressed() {
                let count = TAB_MISS_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
                tracing::info!(delay_ms, count, "Tab grab: no fresh stream frame (timeout)");
            }
            Err(STREAM_MISS.into())
        }
    }
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
    if screenshot_fallback_is_safe(&desktop, super::stream_was_forced()) {
        return screenshot(output_name, OPEN_IN_PROGRESS).await;
    }
    Err(OPEN_IN_PROGRESS.into())
}

fn fallback_is_safe() -> bool {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    screenshot_fallback_is_safe(&desktop, super::stream_was_forced())
}

fn fallback_line_for(why: &str, safe: bool) -> String {
    if safe {
        format!("{why}; falling back to the screenshot portal")
    } else {
        format!("{why}; screenshot skipped")
    }
}

fn fallback_line(why: &str) -> String {
    fallback_line_for(why, fallback_is_safe())
}

async fn screenshot(
    output_name: Option<&str>,
    why: &str,
) -> Result<DynamicImage, Box<dyn std::error::Error + Send + Sync>> {
    tracing::debug!(why, "screencast skipped; using the screenshot portal");
    portal::capture(output_name).await
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

/// Info once per backoff window, then debug, the same window as the
/// portal-unavailable warning.
fn screenshot_skip_log_is_info(last: Option<Instant>, now: Instant) -> bool {
    portal_warn_due(last, now, START_BACKOFF)
}

fn screenshot_skip_step(last: Option<Instant>, now: Instant) -> (bool, Option<Instant>) {
    if screenshot_skip_log_is_info(last, now) {
        (true, Some(now))
    } else {
        (false, last)
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

enum OpenFail {
    Select(String),
    Other(String),
}

impl std::fmt::Display for OpenFail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenFail::Select(msg) | OpenFail::Other(msg) => f.write_str(msg),
        }
    }
}

const PORTAL_UNAVAILABLE: &str = "screencast portal is unavailable";

/// How an `AvailableCursorModes` read decides the open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CursorModesBlock {
    /// Hidden is offered. Continue into select and start.
    Ok,
    /// The portal is down, or the property could not be read, or the bit
    /// set was empty. Retry later. Do not drop the restore token.
    PortalUnavailable,
    /// A compositor answered with modes and Hidden was not one of them.
    NoHiddenCursor,
}

/// Empty flags are a dead backend, not a compositor that lacks Hidden.
/// A failed read is the same. Only a non-empty set without Hidden is
/// [`CursorModesBlock::NoHiddenCursor`].
fn cursor_modes_block(read_failed: bool, modes_empty: bool, has_hidden: bool) -> CursorModesBlock {
    if read_failed || modes_empty {
        CursorModesBlock::PortalUnavailable
    } else if has_hidden {
        CursorModesBlock::Ok
    } else {
        CursorModesBlock::NoHiddenCursor
    }
}

fn portal_warn_due(last: Option<Instant>, now: Instant, window: Duration) -> bool {
    match last {
        None => true,
        Some(at) => now.saturating_duration_since(at) >= window,
    }
}

/// How long a portal-unavailable warning stays quiet. The open itself
/// retries every [`START_BACKOFF`], so the warning window has to be longer
/// than that gap or every retry warns again.
const PORTAL_WARN_WINDOW: Duration = Duration::from_secs(10 * 60);

/// Warn on the first failure, again when the failure text changes, and
/// again after [`PORTAL_WARN_WINDOW`]. A retry of the same failure inside
/// the window does not warn.
fn portal_should_warn(last: Option<(Instant, &str)>, now: Instant, err: &str) -> bool {
    match last {
        None => true,
        Some((at, prev)) => prev != err || now.saturating_duration_since(at) >= PORTAL_WARN_WINDOW,
    }
}

struct PortalWarn {
    at: Instant,
    err: String,
}

static PORTAL_WARN_AT: std::sync::Mutex<Option<PortalWarn>> = std::sync::Mutex::new(None);

fn portal_unavailable(err: &impl std::fmt::Display) -> OpenFail {
    let text = err.to_string();
    let mut last = PORTAL_WARN_AT.lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    let warn = {
        let prev = last.as_ref().map(|warn| (warn.at, warn.err.as_str()));
        portal_should_warn(prev, now, &text)
    };
    if warn {
        *last = Some(PortalWarn { at: now, err: text });
        tracing::warn!(error = %err, "{PORTAL_UNAVAILABLE}");
    } else {
        tracing::debug!(error = %err, "{PORTAL_UNAVAILABLE}");
    }
    OpenFail::Other(PORTAL_UNAVAILABLE.into())
}

async fn open_session(
    data_dir: &Path,
    output_name: Option<&str>,
    max_fps: u32,
) -> Result<Live, String> {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    portal_backend::log_answered(&desktop);
    let token = read_token(data_dir);
    match open_once(
        data_dir,
        output_name,
        token.as_deref(),
        PersistMode::ExplicitlyRevoked,
        max_fps,
    )
    .await
    {
        Ok(live) => Ok(live),
        Err(OpenFail::Select(_)) if token.is_some() => {
            tracing::info!(
                "portal backend did not accept a persistent restore token; asking again without it"
            );
            let _ = std::fs::remove_file(token_path(data_dir));
            match open_once(
                data_dir,
                output_name,
                None,
                PersistMode::ExplicitlyRevoked,
                max_fps,
            )
            .await
            {
                Ok(live) => Ok(live),
                Err(err) => Err(err.to_string()),
            }
        }
        Err(err) => Err(err.to_string()),
    }
}

struct SessionClose {
    session: Option<Session<Screencast>>,
}

impl SessionClose {
    fn arm(session: Session<Screencast>) -> Self {
        Self {
            session: Some(session),
        }
    }

    fn session(&self) -> &Session<Screencast> {
        self.session.as_ref().expect("portal session")
    }

    async fn fail(mut self, err: OpenFail) -> OpenFail {
        if must_close_session(self.session.is_some(), false)
            && let Some(session) = self.session.take()
        {
            let _ = tokio::time::timeout(Duration::from_secs(2), session.close()).await;
        }
        err
    }

    fn disarm(mut self) -> Session<Screencast> {
        self.session.take().expect("portal session")
    }
}

impl Drop for SessionClose {
    fn drop(&mut self) {
        if let Some(session) = self.session.take() {
            tokio::spawn(async move {
                let _ = tokio::time::timeout(Duration::from_secs(2), session.close()).await;
            });
        }
    }
}

async fn open_once(
    data_dir: &Path,
    output_name: Option<&str>,
    token: Option<&str>,
    persist: PersistMode,
    max_fps: u32,
) -> Result<Live, OpenFail> {
    let proxy = match Screencast::new().await {
        Ok(proxy) => proxy,
        Err(err) => return Err(portal_unavailable(&err)),
    };
    let modes = match proxy.available_cursor_modes().await {
        Ok(modes) => {
            let block =
                cursor_modes_block(false, modes.is_empty(), modes.contains(CursorMode::Hidden));
            if block == CursorModesBlock::PortalUnavailable {
                let _ = portal_unavailable(&PORTAL_UNAVAILABLE);
            }
            block
        }
        Err(err) => {
            let _ = portal_unavailable(&err);
            cursor_modes_block(true, true, false)
        }
    };
    match modes {
        CursorModesBlock::Ok => {}
        CursorModesBlock::PortalUnavailable => {
            return Err(OpenFail::Other(PORTAL_UNAVAILABLE.into()));
        }
        CursorModesBlock::NoHiddenCursor => {
            return Err(OpenFail::Other(
                "portal does not offer a hidden cursor, so cursor metadata was not requested"
                    .into(),
            ));
        }
    }
    let session = match proxy.create_session(Default::default()).await {
        Ok(session) => session,
        Err(err) => return Err(OpenFail::Other(err.to_string())),
    };
    let guard = SessionClose::arm(session);
    let mut options = SelectSourcesOptions::default()
        .set_sources(enumflags2::BitFlags::from(SourceType::Monitor))
        .set_multiple(false)
        .set_cursor_mode(CursorMode::Hidden)
        .set_persist_mode(persist);
    if let Some(token) = token {
        options = options.set_restore_token(Some(token));
    }
    let selected = match proxy.select_sources(guard.session(), options).await {
        Ok(selected) => selected,
        Err(err) => {
            return Err(guard
                .fail(select_error(err.to_string(), token.is_some()))
                .await);
        }
    };
    if let Err(err) = selected.response() {
        return Err(guard
            .fail(select_error(err.to_string(), token.is_some()))
            .await);
    }
    let started = match proxy.start(guard.session(), None, Default::default()).await {
        Ok(started) => started,
        Err(err) => return Err(guard.fail(OpenFail::Other(err.to_string())).await),
    };
    let streams = match started.response() {
        Ok(streams) => streams,
        Err(err) => return Err(guard.fail(OpenFail::Other(err.to_string())).await),
    };
    if let Some(restore) = streams.restore_token() {
        write_token(data_dir, restore);
    }
    let outputs = portal::output_geometry().await;
    let Some(chosen) = pick_stream(streams.streams(), output_name, &outputs) else {
        return Err(guard
            .fail(OpenFail::Other(
                "screencast start returned no stream".into(),
            ))
            .await);
    };
    let node = chosen.pipe_wire_node_id();
    tracing::info!(node, "screencast stream ready");
    let fd = match proxy
        .open_pipe_wire_remote(guard.session(), Default::default())
        .await
    {
        Ok(fd) => fd,
        Err(err) => return Err(guard.fail(OpenFail::Other(err.to_string())).await),
    };
    let handle = match pw::open_stream(std::os::fd::AsRawFd::as_raw_fd(&fd), node, max_fps) {
        Ok(handle) => handle,
        Err(err) => return Err(guard.fail(OpenFail::Other(err)).await),
    };
    let session = guard.disarm();
    let (hold, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let _proxy = proxy;
        let _ = rx.await;
        let _ = tokio::time::timeout(Duration::from_secs(2), session.close()).await;
    });
    Ok(Live {
        _fd: fd,
        stream: Some(std::sync::Arc::new(handle)),
        close_session: Some(hold),
    })
}

fn select_error(err: String, had_token: bool) -> OpenFail {
    // A cancel or a closed dialog is a decline. The token stays, so a later
    // launch can restore it. KDE invalidates a token by rejecting the select,
    // which is the branch below. The one warn is logged where the decline is
    // recorded, not here.
    if is_share_declined(&err) {
        return OpenFail::Other("select_sources cancelled".into());
    }
    if had_token {
        OpenFail::Select("select_sources failed".into())
    } else {
        OpenFail::Other(format!("select_sources failed: {err}"))
    }
}

fn pick_stream<'a>(
    streams: &'a [Stream],
    output_name: Option<&str>,
    outputs: &[OutputRect],
) -> Option<&'a Stream> {
    let monitors: Vec<&Stream> = streams
        .iter()
        .filter(|stream| stream.source_type() == Some(SourceType::Monitor))
        .collect();
    let pool: Vec<&Stream> = if monitors.is_empty() {
        streams.iter().collect()
    } else {
        monitors
    };
    if let Some(name) = output_name
        && let Some(output) = outputs.iter().find(|output| output.name == name)
    {
        if let Some(found) = pool
            .iter()
            .copied()
            .find(|stream| stream_covers_output(stream, output))
        {
            return Some(found);
        }
        tracing::warn!(
            output = name,
            "configured monitor was not the shared stream; using the stream the portal returned"
        );
    }
    if let Some(name) = output_name
        && let Some(found) = pool
            .iter()
            .copied()
            .find(|stream| stream.id() == Some(name) || stream.mapping_id() == Some(name))
    {
        return Some(found);
    }
    pool.first().copied().or_else(|| streams.first())
}

/// Position and size in compositor space, against the monitor the user set.
pub fn stream_covers_output(stream: &Stream, output: &OutputRect) -> bool {
    monitor_matches(stream.position(), stream.size(), output)
}

pub fn monitor_matches(
    position: Option<(i32, i32)>,
    size: Option<(i32, i32)>,
    output: &OutputRect,
) -> bool {
    match (position, size) {
        (Some((x, y)), Some((w, h))) => {
            x == output.x
                && y == output.y
                && w == i32::try_from(output.width).unwrap_or(i32::MAX)
                && h == i32::try_from(output.height).unwrap_or(i32::MAX)
        }
        _ => false,
    }
}

fn token_path(data_dir: &Path) -> PathBuf {
    data_dir.join(TOKEN_FILE)
}

fn read_token(data_dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(token_path(data_dir)).ok()?;
    let token = text.trim();
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

fn write_token(data_dir: &Path, token: &str) {
    if token.is_empty() {
        return;
    }
    let path = token_path(data_dir);
    let tmp = data_dir.join(format!("{TOKEN_FILE}.tmp"));
    let _ = std::fs::remove_file(&tmp);
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let Ok(mut file) = options.open(&tmp) else {
        return;
    };
    if file.write_all(token.as_bytes()).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return;
    }
    let _ = file.sync_all();
    drop(file);
    if std::fs::rename(&tmp, &path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

async fn fit_stream(
    img: DynamicImage,
    output_name: Option<&str>,
) -> Result<DynamicImage, Box<dyn std::error::Error + Send + Sync>> {
    let outputs = portal::output_geometry().await;
    fit_stream_image(img, &outputs, output_name)
}

pub fn fit_stream_image(
    img: DynamicImage,
    outputs: &[OutputRect],
    output_name: Option<&str>,
) -> Result<DynamicImage, Box<dyn std::error::Error + Send + Sync>> {
    if outputs.is_empty() {
        return Ok(img);
    }
    let index = match output_name {
        Some(name) => outputs
            .iter()
            .position(|output| output.name == name)
            .ok_or_else(|| format!("output '{name}' not found"))?,
        None => super::auto_output_index(
            outputs
                .iter()
                .map(|output| (output.name.as_str(), output.area())),
        )
        .unwrap_or(0),
    };
    let chosen = &outputs[index];
    if stream_should_crop(img.width(), img.height(), chosen.area(), outputs.len()) {
        let Some((_, rect)) = portal::crop_rect(outputs, output_name, img.width(), img.height())?
        else {
            return Ok(img);
        };
        return Ok(img.crop_imm(rect.x, rect.y, rect.width, rect.height));
    }
    Ok(img)
}

/// Crop only when the image is clearly the whole desktop: at least two
/// outputs, and the image area is more than 1.2 times the chosen output.
pub fn stream_should_crop(img_w: u32, img_h: u32, chosen_area: u64, outputs: usize) -> bool {
    outputs >= 2
        && chosen_area > 0
        && u64::from(img_w).saturating_mul(u64::from(img_h)) > chosen_area.saturating_mul(6) / 5
}

async fn save_compare(
    data_dir: &Path,
    index: u32,
    stream: &DynamicImage,
    output_name: Option<&str>,
) {
    let dir = data_dir.join("debug").join("portal-compare");
    if std::fs::create_dir_all(&dir).is_err() {
        tracing::warn!("could not create debug/portal-compare");
        return;
    }
    crate::fs_mode::tighten_private_dir(&dir);
    let n = index + 1;
    let stream_path = dir.join(format!("{n:04}-stream.png"));
    let shot_path = dir.join(format!("{n:04}-screenshot.png"));
    if stream.save(&stream_path).is_err() {
        tracing::warn!("could not save a screencast debug frame");
        return;
    }
    crate::fs_mode::tighten_private_file(&stream_path);
    match portal::capture(output_name).await {
        Ok(shot) => {
            if shot.save(&shot_path).is_ok() {
                crate::fs_mode::tighten_private_file(&shot_path);
                tracing::info!(
                    pair = n,
                    "saved a portal compare pair under debug/portal-compare"
                );
            }
        }
        Err(err) => {
            tracing::warn!(error = %err, "screenshot half of the portal compare pair failed");
        }
    }
}

#[cfg(test)]
mod tests {
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
            fallback_line_for("screencast backoff", true),
            "screencast backoff; falling back to the screenshot portal"
        );
        assert_eq!(
            fallback_line_for("screencast backoff", false),
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
            crate::ocr_switch::set_share_notice("");
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
        assert!(portal_should_warn(None, now, "portal down"));
        let again = now + START_BACKOFF;
        assert!(
            !portal_should_warn(Some((now, "portal down")), again, "portal down"),
            "a retry one backoff later does not warn again"
        );
        assert!(
            portal_should_warn(Some((now, "portal down")), again, "cursor modes empty"),
            "a different failure warns"
        );
        assert!(portal_should_warn(
            Some((now, "portal down")),
            now + PORTAL_WARN_WINDOW,
            "portal down"
        ));
        assert!(!portal_should_warn(
            Some((now, "portal down")),
            now + PORTAL_WARN_WINDOW - Duration::from_secs(1),
            "portal down"
        ));

        *PORTAL_WARN_AT.lock().unwrap_or_else(|e| e.into_inner()) = None;
        let first = with_log_level(tracing::Level::WARN, || {
            let _ = portal_unavailable(&"portal down");
            let _ = portal_unavailable(&"portal down");
        });
        let second = with_log_level(tracing::Level::WARN, || {
            let _ = portal_unavailable(&"portal down");
        });
        assert_eq!(first.matches(PORTAL_UNAVAILABLE).count(), 1);
        assert!(first.contains("WARN"));
        assert!(
            !second.contains(PORTAL_UNAVAILABLE),
            "consecutive retries do not warn again: {second}"
        );
        *PORTAL_WARN_AT.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    #[test]
    fn a_stalled_stream_reopens_while_the_game_is_running() {
        let now = Instant::now();
        assert!(!stream_stall_reopen(None, now, true, STREAM_STALL));
        assert!(!stream_stall_reopen(Some(now), now, true, STREAM_STALL));
        assert!(!stream_stall_reopen(
            Some(now - STREAM_STALL + Duration::from_millis(1)),
            now,
            true,
            STREAM_STALL
        ));
        assert!(!stream_stall_reopen(
            Some(now - STREAM_STALL),
            now,
            false,
            STREAM_STALL
        ));
        assert!(stream_stall_reopen(
            Some(now - STREAM_STALL),
            now,
            true,
            STREAM_STALL
        ));
        STALL_WARNED.store(false, Ordering::Release);
        let first = with_log(warn_stream_stall);
        let second = with_log(warn_stream_stall);
        assert_eq!(first.matches("screencast stream stalled").count(), 1);
        assert!(first.contains("WARN"));
        assert!(
            !second.contains("screencast stream stalled"),
            "the same stall episode does not warn again: {second}"
        );
        STALL_WARNED.store(false, Ordering::Release);
        let third = with_log(warn_stream_stall);
        assert_eq!(third.matches("screencast stream stalled").count(), 1);
        STALL_WARNED.store(false, Ordering::Release);
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

        let declined = open_error_action("select_sources cancelled", false);
        assert!(declined.declined);
        assert!(!declined.arm_backoff);
        assert_eq!(declined.notice, Some(NO_CAPTURE_DECLINE_NOTICE));
        let timeout = open_error_action("screencast permission dialog timed out", false);
        assert!(timeout.declined, "a timeout is a No for this launch");
        assert!(!timeout.arm_backoff);
        assert_eq!(timeout.notice, Some(timeout_notice(false)));
        assert!(timeout_notice(false).contains("no captures"));
        assert!(!timeout_notice(true).contains("no captures"));
        let safe = open_error_action("Dialog was closed", true);
        assert_eq!(safe.notice, Some(SAFE_DECLINE_NOTICE));
        assert!(!SAFE_DECLINE_NOTICE.contains("no captures"));

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
                )
                .declined
            );
        });
        assert_eq!(first.matches(NO_CAPTURE_DECLINE_NOTICE).count(), 1);
        let status = crate::ocr_switch::read_status(dir.path()).unwrap();
        assert_eq!(status.share_notice, NO_CAPTURE_DECLINE_NOTICE);
        crate::ocr_switch::publish_share_open(dir.path(), false);
        assert_eq!(
            crate::ocr_switch::read_status(dir.path())
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
            crate::ocr_switch::read_status(dir.path())
                .unwrap()
                .share_notice
                .is_empty()
        );

        let mut relaunch = Some(Instant::now());
        let second = with_log(|| {
            assert!(
                apply_share_decline("select_sources cancelled", false, dir.path(), &mut relaunch,)
                    .declined
            );
            let mut poll = None;
            assert!(
                apply_share_decline("select_sources cancelled", false, dir.path(), &mut poll,)
                    .declined
            );
        });
        assert_eq!(second.matches(NO_CAPTURE_DECLINE_NOTICE).count(), 1);
        assert!(relaunch.is_none());
    }
}
