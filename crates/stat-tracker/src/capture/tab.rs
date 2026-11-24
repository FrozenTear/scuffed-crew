//! The Tab and poll grab context: the task-locals a grab reads (press
//! instant, lock wait, miss-log suppression) and all Tab timing.

use super::flight;

tokio::task_local! {
    static TAB_PRESS: std::time::Instant;
}

tokio::task_local! {
    pub(super) static SUPPRESS_STREAM_MISS_LOG: ();
}

/// True while the first Tab is discarding a miss from a press that the
/// share open already pushed past the frame budget. That miss is not logged.
pub(crate) fn stream_miss_log_suppressed() -> bool {
    SUPPRESS_STREAM_MISS_LOG.try_with(|_| ()).is_ok()
}

/// The Tab key instant for the current capture task, when this is a Tab grab.
pub fn tab_press() -> Option<std::time::Instant> {
    TAB_PRESS.try_with(|press| *press).ok()
}

tokio::task_local! {
    static POLL_LOCK_WAIT: std::time::Duration;
}

/// How long the end-reel outcome-only poll waits for the screenshot lock
/// while a Tab grab holds it. The VICTORY banner lasts about 3 s and, with
/// the new reader, polls come every 1.5 s (4 s with classic OCR), so a poll
/// turned away at once could lose the banner while the player held Tab
/// over it. One second keeps the poll arm short.
pub const OUTCOME_POLL_LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(1);

/// The lock wait for a poll: [`OUTCOME_POLL_LOCK_WAIT`] for the end-reel
/// outcome-only poll, nothing for an ordinary one.
pub fn poll_lock_wait(outcome_only: bool) -> std::time::Duration {
    if outcome_only {
        OUTCOME_POLL_LOCK_WAIT
    } else {
        std::time::Duration::ZERO
    }
}

/// Run a poll grab that may wait `wait` for the screenshot lock. Not an
/// async fn: it returns the scope future itself, so the poll future does
/// not carry a second copy of `fut`.
pub fn with_poll_lock_wait<F: std::future::Future>(
    wait: std::time::Duration,
    fut: F,
) -> impl std::future::Future<Output = F::Output> {
    POLL_LOCK_WAIT.scope(wait, fut)
}

/// The lock wait for the grab running in this task: `tab_wait` in a Tab
/// capture, the poll's wait ([`with_poll_lock_wait`]) otherwise, zero by
/// default.
pub(crate) fn grab_lock_wait(tab_wait: std::time::Duration) -> std::time::Duration {
    flight::lock_wait(
        tab_press().is_some(),
        tab_wait,
        POLL_LOCK_WAIT
            .try_with(|wait| *wait)
            .unwrap_or(std::time::Duration::ZERO),
    )
}

/// Run `fut` as a Tab grab whose press time is `press`. Direct capture still
/// sleeps before calling this. The stream path reads the press and does not.
pub async fn with_tab_press<T>(
    press: std::time::Instant,
    fut: impl std::future::Future<Output = T>,
) -> T {
    TAB_PRESS.scope(press, fut).await
}

/// The first attempt uses the key instant. A retry starts its own
/// press-relative deadline at `now`, so it is not already past the first
/// attempt's hard stop.
pub fn tab_attempt_press(
    original: std::time::Instant,
    attempt: usize,
    now: std::time::Instant,
) -> std::time::Instant {
    if attempt == 0 { original } else { now }
}

/// How long the first Tab waits for a fresh frame from a share that is
/// still opening. The frame budget stays 1200 ms.
pub const FIRST_TAB_OPEN_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// What the first Tab does while the share is still opening.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpeningTab {
    /// The wait for a fresh frame has not passed.
    Wait,
    /// [`FIRST_TAB_OPEN_WAIT`] has passed. The attempt returns
    /// open-in-progress so a later retry can try again. It is not a
    /// stream miss, and it does not log one.
    Miss,
}

pub fn first_tab_while_opening(
    elapsed: std::time::Duration,
    bound: std::time::Duration,
) -> OpeningTab {
    if elapsed < bound {
        OpeningTab::Wait
    } else {
        OpeningTab::Miss
    }
}

/// After a late open, the original frame budget may already be gone. The
/// grab then starts again from `now`, while this Tab is still held.
pub fn tab_press_after_late_open(
    original: std::time::Instant,
    now: std::time::Instant,
    frame_budget: std::time::Duration,
    waited_for_open: bool,
) -> std::time::Instant {
    if waited_for_open && now >= original + frame_budget {
        now
    } else {
        original
    }
}

/// Milliseconds after a Tab press before a stream frame can show the
/// scoreboard overlay. The stream path waits for a buffer instead of
/// sleeping a fixed 400 ms.
pub const TAB_OVERLAY_AFTER_PRESS_MS: u64 = 300;

/// Hard stop for that wait, measured from the same press. This is a
/// constant, not a config key. A 2 fps cap (the default is 4, a member can
/// set 2) can leave about half a second between buffers, so the stop sits past one of those gaps. A later check
/// still accepts a buffer that arrived in the gap once its arrival is at or
/// after the overlay delay.
pub const TAB_GRAB_TIMEOUT_AFTER_PRESS_MS: u64 = 1200;

/// The press a Tab retry hands the stream grab, given the instant the retry
/// starts. A retry is not a new key press: the overlay delay already passed
/// for the real press, so the grab takes the next buffer that arrives from
/// `now` on instead of waiting another 300 ms on top. The hard stop is then
/// 900 ms after `now`, still past one 2 fps gap.
pub fn tab_retry_press(now: std::time::Instant) -> std::time::Instant {
    now.checked_sub(std::time::Duration::from_millis(TAB_OVERLAY_AFTER_PRESS_MS))
        .unwrap_or(now)
}

/// The press the first Tab grab hands the stream, given the key-event time
/// and the instant the grab starts. Normally the key time. A grab that
/// starts late (a press queued behind a running capture) would already be
/// past the key's 1200 ms hard stop and miss at once, counting as a soft
/// miss; it then takes the next buffer from `now` on, as a retry does.
pub fn first_grab_press(key: std::time::Instant, now: std::time::Instant) -> std::time::Instant {
    key.max(tab_retry_press(now))
}
