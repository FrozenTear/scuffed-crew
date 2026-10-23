//! The screen-stream rules, as plain functions: reopen and stall,
//! decline and permission timeout, the screenshot fallback and its method,
//! and the notice and error text the bin and the app match.

use std::time::{Duration, Instant};

use super::pw;
use crate::capture::types::CaptureKind;

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

/// A pause this long, after the stream has reached Streaming, is a dead
/// stream. A shorter pause is negotiation. Silence while the state stays
/// Streaming is not a pause: niri and KWin send buffers only on damage.
pub const PAUSED_PERSIST: Duration = Duration::from_secs(2);

/// Two Tab waits with no PipeWire buffer reopen the stream. One does not.
pub const SOFT_MISS_LIMIT: u32 = 2;

/// The shortest gap between two stream reopens. A new game does not clear
/// the clock, so a reopen is never closer than this to the one before it.
pub const REOPEN_GAP_MIN: Duration = Duration::from_secs(60);

/// The longest gap between two stream reopens. Each reopen in the same game
/// doubles the gap up to this, and a stream that keeps stalling is still
/// reopened at this spacing for as long as the tracker runs.
pub const REOPEN_GAP_MAX: Duration = Duration::from_secs(5 * 60);

/// A stream that is still Streaming but has handed over no PipeWire buffer
/// for this long, while a poll runs, counts as stalled for the screenshot
/// fallback. It is not a reopen: niri and KWin send buffers only on damage.
pub const QUIET_STALL: Duration = Duration::from_secs(8);

/// At most one screenshot per this long for a poll on a quiet stream, on
/// every desktop. A still screen stays quiet, and a poll runs every 1.5 s.
/// Tab grabs and a dead or reopen-waiting stream are not limited.
pub const QUIET_SHOT_GAP: Duration = Duration::from_secs(10);

/// The stall-fallback warning is once per this window, then debug, so a
/// mostly still screen does not warn on every quiet spell.
pub const STALL_WARN_WINDOW: Duration = Duration::from_secs(10 * 60);

pub(super) const REOPEN_DEAD: &str = "screencast stream ended; reopening";
pub(super) const REOPEN_TABS: &str = "screencast stream had no fresh frame on two Tabs; reopening";
pub(super) const REOPEN_NO_TOKEN: &str =
    "screencast stream needs a reopen but no restore token is saved; keeping the stream";

/// What the capture loop can see without holding the PipeWire thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamHealth {
    Streaming,
    Error,
    Unconnected,
    Paused { paused_for: Duration },
    Connecting,
    CoreFault,
    PortalClosed,
}

/// One look at the stream. A poll never counts as a soft miss. A Tab does
/// only when that grab waited for a fresh frame, PipeWire delivered no
/// buffer during the wait, and the stream was still Streaming.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Observe {
    Poll {
        health: StreamHealth,
    },
    Tab {
        health: StreamHealth,
        waited: bool,
        got_fresh: bool,
        buffer_during_wait: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Pending {
    Dead,
    Tabs,
}

#[derive(Debug, Clone, Default)]
pub(super) struct ReopenWatch {
    pub(super) soft_misses: u32,
    pub(super) last_reopen: Option<Instant>,
    pub(super) pending: Option<Pending>,
    pub(super) death_latched: bool,
    pub(super) token_warned: bool,
    /// Reopens in this game. Each one doubles the gap before the next, from
    /// [`REOPEN_GAP_MIN`] up to [`REOPEN_GAP_MAX`]. A new game clears it.
    pub(super) reopens_this_game: u32,
}

/// A new game clears the miss streak and the gap escalation. The gap clock
/// stays, so a new game cannot reopen inside [`REOPEN_GAP_MIN`] of the last
/// reopen.
pub(super) fn reset_stall_on_game_start(watch: &mut ReopenWatch) {
    let last_reopen = watch.last_reopen;
    *watch = ReopenWatch {
        last_reopen,
        ..ReopenWatch::default()
    };
}

/// A deliberate close keeps the gap clock and drops the rest.
pub(super) fn watch_after_close(watch: ReopenWatch) -> ReopenWatch {
    ReopenWatch {
        last_reopen: watch.last_reopen,
        ..ReopenWatch::default()
    }
}

/// What to do with the live session. A Tab never gets [`ReopenStep::Reopen`]:
/// the grab's result is returned, and the next poll performs the reopen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReopenStep {
    Keep,
    HoldForPoll,
    Reopen(&'static str),
    KeepNoToken,
}

pub(super) fn health_is_dead(health: StreamHealth) -> bool {
    match health {
        StreamHealth::Error
        | StreamHealth::Unconnected
        | StreamHealth::CoreFault
        | StreamHealth::PortalClosed => true,
        StreamHealth::Paused { paused_for } => paused_for >= PAUSED_PERSIST,
        StreamHealth::Streaming | StreamHealth::Connecting => false,
    }
}

/// The gap a reopen must keep after the previous one, given how many
/// reopens this game has already done: [`REOPEN_GAP_MIN`] for the first two,
/// then doubling, capped at [`REOPEN_GAP_MAX`]. It never stops allowing one.
pub fn reopen_gap(reopens_this_game: u32) -> Duration {
    let doublings = reopens_this_game.saturating_sub(1).min(16);
    REOPEN_GAP_MIN
        .saturating_mul(1u32 << doublings)
        .min(REOPEN_GAP_MAX)
}

fn reopen_rate_ok(last: Option<Instant>, reopens_this_game: u32, now: Instant) -> bool {
    last.is_none_or(|at| now.saturating_duration_since(at) >= reopen_gap(reopens_this_game))
}

fn keep_without_token(watch: &mut ReopenWatch, dead: bool) -> ReopenStep {
    watch.pending = None;
    if dead {
        watch.death_latched = true;
    }
    if watch.token_warned {
        ReopenStep::Keep
    } else {
        watch.token_warned = true;
        ReopenStep::KeepNoToken
    }
}

/// Open the stream again, or refuse because the gap has not elapsed. A
/// refusal leaves `pending` for the caller to put back, so the reopen runs
/// on the first poll after the gap.
fn try_reopen(watch: &mut ReopenWatch, now: Instant, why: &'static str, dead: bool) -> ReopenStep {
    if !reopen_rate_ok(watch.last_reopen, watch.reopens_this_game, now) {
        return ReopenStep::Keep;
    }
    watch.reopens_this_game = watch.reopens_this_game.saturating_add(1);
    watch.last_reopen = Some(now);
    watch.soft_misses = 0;
    watch.pending = None;
    if dead {
        watch.death_latched = true;
    }
    ReopenStep::Reopen(why)
}

/// Silence while Streaming is [`ReopenStep::Keep`]. Polls never count as a
/// soft miss. A dead stream, or two Tab waits with no PipeWire buffer,
/// reopens on a poll, at least [`reopen_gap`] after the previous reopen. A
/// stall inside that gap waits for it and then reopens; a later stall in
/// the same game reopens again after a longer gap. A Tab never returns
/// [`ReopenStep::Reopen`]. A new game clears the miss streak and the gap
/// escalation, and leaves the gap clock. Without a saved restore token the
/// stream stays up.
pub(super) fn reopen_step(
    obs: Observe,
    has_token: bool,
    now: Instant,
    watch: &mut ReopenWatch,
) -> ReopenStep {
    let health = match obs {
        Observe::Poll { health } | Observe::Tab { health, .. } => health,
    };
    let tab = matches!(obs, Observe::Tab { .. });
    if matches!(health, StreamHealth::Streaming) {
        watch.death_latched = false;
    }
    if !tab {
        if let Some(cause) = watch.pending.take() {
            if !has_token {
                return keep_without_token(
                    watch,
                    matches!(cause, Pending::Dead) || health_is_dead(health),
                );
            }
            let (why, dead) = match cause {
                Pending::Dead => (REOPEN_DEAD, true),
                Pending::Tabs => (REOPEN_TABS, false),
            };
            let step = try_reopen(watch, now, why, dead);
            if matches!(step, ReopenStep::Keep) {
                watch.pending = Some(cause);
            }
            return step;
        }
        if health_is_dead(health) && !watch.death_latched {
            if !has_token {
                return keep_without_token(watch, true);
            }
            return try_reopen(watch, now, REOPEN_DEAD, true);
        }
        return ReopenStep::Keep;
    }
    if watch.pending.is_some() {
        return ReopenStep::HoldForPoll;
    }
    if health_is_dead(health) && !watch.death_latched {
        if !has_token {
            return keep_without_token(watch, true);
        }
        watch.pending = Some(Pending::Dead);
        return ReopenStep::HoldForPoll;
    }
    if let Observe::Tab {
        waited,
        got_fresh,
        buffer_during_wait,
        health,
    } = obs
    {
        if matches!(health, StreamHealth::Streaming) && got_fresh {
            watch.soft_misses = 0;
        }
        let soft = matches!(health, StreamHealth::Streaming)
            && waited
            && !got_fresh
            && !buffer_during_wait;
        if soft {
            watch.soft_misses = watch.soft_misses.saturating_add(1);
            if watch.soft_misses >= SOFT_MISS_LIMIT {
                if !has_token {
                    return keep_without_token(watch, false);
                }
                watch.pending = Some(Pending::Tabs);
                return ReopenStep::HoldForPoll;
            }
        }
    }
    ReopenStep::Keep
}

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

/// A grab while the share is closed because the game stopped, and may
/// only reopen once it runs again.
pub const SHARE_CLOSED_UNTIL_GAME: &str = "screencast share stays closed until the game is running";

pub fn is_share_closed_until_game(err: &str) -> bool {
    err.contains(SHARE_CLOSED_UNTIL_GAME)
}

pub fn is_share_opening(err: &str) -> bool {
    err.contains(OPEN_IN_PROGRESS)
}

/// The sentence for a declined share. A desktop where a screenshot is not
/// safe says captures stop. A desktop that already screenshots does not.
pub const NO_CAPTURE_DECLINE_NOTICE: &str =
    "Screen share was declined: no captures until Overwatch is restarted";

pub const SAFE_DECLINE_NOTICE: &str =
    "Screen share was declined: Older screenshot continues until Overwatch is restarted";

pub fn decline_notice(fallback_safe: bool) -> &'static str {
    if fallback_safe {
        SAFE_DECLINE_NOTICE
    } else {
        NO_CAPTURE_DECLINE_NOTICE
    }
}

/// A timed-out permission dialog is a No for this launch when no restore
/// token was sent. A timeout after a token was sent is not a No. Flip this
/// if a token-less timeout should ask again after the backoff instead.
const PERMISSION_TIMEOUT_IS_NO: bool = true;

pub fn is_permission_timeout(err: &str) -> bool {
    err.to_ascii_lowercase()
        .contains("permission dialog timed out")
}

pub fn timeout_notice(fallback_safe: bool) -> &'static str {
    if fallback_safe {
        "Screen share permission timed out: Older screenshot continues until Overwatch is restarted"
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
/// when [`PERMISSION_TIMEOUT_IS_NO`] is set and no restore token was sent.
/// That check lives in [`open_error_action`].
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

pub fn open_error_action(err: &str, fallback_safe: bool, had_token: bool) -> OpenErrorAction {
    if PERMISSION_TIMEOUT_IS_NO && is_permission_timeout(err) && !had_token {
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
    daemon_share_open && is_kde(desktop)
}

/// Capture now's error when the daemon share is already open. Quitting
/// Overwatch closes that share only after a permission token was saved.
pub fn capture_now_blocked_message(token_saved: bool) -> &'static str {
    if token_saved {
        "The tracker is sharing the screen, so this screenshot was skipped. Quit Overwatch, or stop the tracker, so the share closes, then try again."
    } else {
        "The tracker is sharing the screen, so this screenshot was skipped. Stop the tracker so the share closes, then try again."
    }
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
/// converted frame. One frame period plus 100 ms, so the default 4 fps
/// stream (frames 250 ms apart) waits 350 ms, and 2 fps waits 600 ms, and
/// neither reuses the one before it. The new-reader poll is 1.5 s, and this stays under that.
pub fn reuse_wait(max_fps: u32) -> Duration {
    Duration::from_millis(pw::throttle_gap_ms(max_fps).saturating_add(100))
}

/// Extra wait for a poll that had to resume a paused stream: the
/// compositor has to start producing again before the first buffer.
pub const RESUME_POLL_ALLOWANCE: Duration = Duration::from_millis(400);

/// A poll's wait. A poll that resumes a paused stream waits
/// [`RESUME_POLL_ALLOWANCE`] longer, 750 ms at 4 fps and 1 s at 2 fps, still
/// under the 1.5 s poll. A buffer ends the wait as soon as it arrives.
pub fn poll_wait(max_fps: u32, resuming: bool) -> Duration {
    let wait = reuse_wait(max_fps);
    if resuming {
        wait + RESUME_POLL_ALLOWANCE
    } else {
        wait
    }
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

/// Compositors with wlr-screencopy, niri included. libwayshot works there.
const SCREENCOPY_DESKTOPS: &[&str] = &[
    "niri", "sway", "hyprland", "wayfire", "river", "labwc", "wlroots",
];

fn is_kde(desktop: &str) -> bool {
    crate::capture::route::is_kde(desktop)
}

pub fn screenshot_fallback_is_safe(desktop: &str, stream_forced: bool) -> bool {
    if is_kde(desktop) {
        return false;
    }
    if desktop_has(desktop, SCREENCOPY_DESKTOPS) {
        return true;
    }
    desktop_has(desktop, &["gnome"]) && stream_forced
}

/// How a fallback screenshot is taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShotMethod {
    /// wlr-screencopy through libwayshot: one output, no file, tens of ms.
    Wayshot,
    /// The xdg-desktop-portal Screenshot ("Older screenshot"). On niri that
    /// renders every output and writes a PNG, about a second per shot.
    Portal,
}

impl ShotMethod {
    fn label(self) -> &'static str {
        match self {
            ShotMethod::Wayshot => "a wayshot screenshot",
            ShotMethod::Portal => "Older screenshot",
        }
    }
}

/// The method a fallback screenshot tries first. Where the compositor has
/// screencopy and a Wayland socket is set, that is wayshot, and the portal
/// is used only when wayshot fails. Everywhere else it is the portal.
pub fn first_shot_method(desktop: &str, wayland: bool) -> ShotMethod {
    if wayland && !is_kde(desktop) && desktop_has(desktop, SCREENCOPY_DESKTOPS) {
        ShotMethod::Wayshot
    } else {
        ShotMethod::Portal
    }
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

/// Why a grab on the live stream got no new picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stall {
    /// PipeWire error, unconnected after streaming, a long pause, a
    /// connection fault, or a closed portal session.
    Dead,
    /// A reopen is wanted and is waiting for the next poll or its gap.
    ReopenWaiting,
    /// Still Streaming, with no PipeWire buffer for [`QUIET_STALL`].
    Quiet,
}

impl Stall {
    pub(super) fn reason(self) -> &'static str {
        match self {
            Stall::Dead => "screencast stream ended",
            Stall::ReopenWaiting => "screencast stream is waiting to reopen",
            Stall::Quiet => "screencast stream stalled with no new frame",
        }
    }
}

/// The stall, if any, behind a grab that got no new picture. `quiet_for` is
/// how long the stream has gone without a PipeWire buffer.
pub fn classify_stall(dead: bool, reopen_waiting: bool, quiet_for: Duration) -> Option<Stall> {
    if dead {
        Some(Stall::Dead)
    } else if reopen_waiting {
        Some(Stall::ReopenWaiting)
    } else if quiet_for >= QUIET_STALL {
        Some(Stall::Quiet)
    } else {
        None
    }
}

/// True when a stalled grab takes one screenshot instead of returning no
/// picture. That is niri, wlroots, and GNOME when the stream was forced, for
/// Tab and poll grabs. KDE never does: a screenshot beside a live share is
/// the KWin crash. GNOME does not for a poll on a quiet stream: its portal
/// screenshot flashes the screen, and mutter sends no buffers while the
/// screen is still. A grab with no stall uses the stream.
pub fn stall_screenshot(
    desktop: &str,
    stream_forced: bool,
    stall: Option<Stall>,
    kind: CaptureKind,
) -> bool {
    let Some(stall) = stall else {
        return false;
    };
    if !screenshot_fallback_is_safe(desktop, stream_forced) {
        return false;
    }
    let quiet_poll = stall == Stall::Quiet && kind == CaptureKind::Poll;
    !(quiet_poll && desktop_has(desktop, &["gnome"]))
}

/// True when a poll on a quiet stream may take its screenshot now.
pub fn quiet_poll_shot_due(last: Option<Instant>, now: Instant) -> bool {
    last.is_none_or(|at| now.saturating_duration_since(at) >= QUIET_SHOT_GAP)
}

/// Where in [`capture`] a stalled stream can be noticed.
///
/// [`capture`]: super::capture
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StallSite {
    /// The stream had streamed and is no longer alive. No grab runs.
    DeadBeforeGrab,
    /// The grab handed back the previous frame, which a poll ignores.
    ReusedFrame,
    /// The grab ended with an error (a fault, or a poll wait with nothing).
    GrabError { shutting_down: bool },
}

/// The fallback decision [`capture`] makes at each [`StallSite`]: the stall
/// to take a screenshot for, or `None` to keep the stream's answer. A
/// reused frame only matters to a poll. Nothing is taken while shutting
/// down. A poll on a quiet stream also waits out [`QUIET_SHOT_GAP`]
/// (`quiet_due`).
///
/// [`capture`]: super::capture
pub(super) fn stall_fallback_at(
    site: StallSite,
    kind: CaptureKind,
    stall: Option<Stall>,
    desktop: &str,
    stream_forced: bool,
    quiet_due: bool,
) -> Option<Stall> {
    let stall = match site {
        StallSite::DeadBeforeGrab => Some(Stall::Dead),
        StallSite::ReusedFrame if kind == CaptureKind::Poll => stall,
        StallSite::ReusedFrame => None,
        StallSite::GrabError { shutting_down } if !shutting_down => stall,
        StallSite::GrabError { .. } => None,
    }?;
    if !stall_screenshot(desktop, stream_forced, Some(stall), kind) {
        return None;
    }
    if stall == Stall::Quiet && kind == CaptureKind::Poll && !quiet_due {
        return None;
    }
    Some(stall)
}

/// `method` is the screenshot that is (or was) used, `None` when skipped.
pub(super) fn fallback_line_for(why: &str, method: Option<ShotMethod>) -> String {
    match method {
        Some(method) => format!("{why}; falling back to {}", method.label()),
        None => format!("{why}; screenshot skipped"),
    }
}

/// Info once per backoff window, then debug, the same window as the
/// portal-unavailable warning.
pub(super) fn screenshot_skip_log_is_info(last: Option<Instant>, now: Instant) -> bool {
    portal_warn_due(last, now, START_BACKOFF)
}

pub(super) fn screenshot_skip_step(last: Option<Instant>, now: Instant) -> (bool, Option<Instant>) {
    if screenshot_skip_log_is_info(last, now) {
        (true, Some(now))
    } else {
        (false, last)
    }
}

pub(super) fn portal_warn_due(last: Option<Instant>, now: Instant, window: Duration) -> bool {
    match last {
        None => true,
        Some(at) => now.saturating_duration_since(at) >= window,
    }
}
