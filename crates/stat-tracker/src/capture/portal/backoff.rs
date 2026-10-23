//! The pause after a stalled screenshot request: its steps, the hold while
//! the request's Close is unanswered, and the process-wide state.

use super::lock;

/// Polls' pauses after successive stalled requests; the last repeats. The
/// first is short: ordinary polls are what wake the end-reel outcome poll,
/// so while they pause the match result screens go unread, and a stall
/// that passes should cost at most half a minute of them. That is a floor:
/// while the Close of the request given up is unanswered, the pause is at
/// least [`SHOT_UNCLOSED_HOLD`] (2 minutes).
pub const SHOT_BACKOFF_STEPS: [std::time::Duration; 4] = [
    std::time::Duration::from_secs(30),
    std::time::Duration::from_secs(2 * 60),
    std::time::Duration::from_secs(5 * 60),
    std::time::Duration::from_secs(10 * 60),
];

/// While the Close of a request given up has not been answered, no new
/// request is made for at least this long after the give-up, whatever the
/// pause step (the first step is shorter).
pub const SHOT_UNCLOSED_HOLD: std::time::Duration = std::time::Duration::from_secs(2 * 60);

/// The error for a grab turned away while polls back off.
pub const SHOT_BACKOFF: &str =
    "the screenshot portal stopped answering: no new request until its pause ends";

/// After a stalled request ([`Unanswered::Stalled`]) ordinary polls make no
/// portal request for a growing pause ([`SHOT_BACKOFF_STEPS`]: 30 s, then
/// 2, 5 and 10 minutes). Ordinary polls also wake the end-reel outcome
/// poll, so during a pause the match result screens are not read either;
/// only an outcome poll whose wake was already set before the pause, while
/// a Tab capture runs, can still run. A Tab (and such an outcome poll, the
/// urgent grabs) may try during a pause once the Close of the request
/// given up was answered.
///
/// While that Close is unanswered the request may still be open at the
/// portal, so no grab, urgent or not, makes a request for the whole pause,
/// and for at least [`SHOT_UNCLOSED_HOLD`] (2 minutes) after the give-up
/// even past a 30 s first step: never two of ours outstanding while the
/// pause lasts. Once it is over, a new request may run beside a request
/// whose Close never came back (the portal's problem by then; waiting
/// longer would stop capture for good). An answered request ends the
/// pause.
///
/// [`Unanswered::Stalled`]: super::answer::Unanswered::Stalled
#[derive(Debug, Default)]
pub(crate) struct ShotBackoff {
    pub(super) until: Option<tokio::time::Instant>,
    pub(super) gave_up_at: Option<tokio::time::Instant>,
    pub(super) stalls: usize,
    pub(super) abandoned_open: bool,
    /// A request of this process got a picture (Response code 0) at
    /// least once. Kept for the life of the process.
    pub(super) answered_ok: bool,
}

impl ShotBackoff {
    /// A request was given up stalled at `now`; returns the pause.
    pub(crate) fn gave_up(&mut self, now: tokio::time::Instant) -> std::time::Duration {
        let pause = SHOT_BACKOFF_STEPS[self.stalls.min(SHOT_BACKOFF_STEPS.len() - 1)];
        self.stalls += 1;
        self.until = Some(now + pause);
        self.gave_up_at = Some(now);
        self.abandoned_open = true;
        pause
    }

    /// The portal answered the Close of the request given up.
    pub(crate) fn closed(&mut self) {
        self.abandoned_open = false;
    }

    /// The portal answered a request (`picture`: with a file): no pause.
    pub(crate) fn answered(&mut self, picture: bool) {
        let answered_ok = self.answered_ok || picture;
        *self = Self {
            answered_ok,
            ..Self::default()
        };
    }

    /// Why a grab may not make a request now, if it may not. `urgent` is a
    /// Tab or the end-reel outcome poll.
    pub(crate) fn refuses(&self, urgent: bool, now: tokio::time::Instant) -> Option<&'static str> {
        let paused = self.until.is_some_and(|until| now < until);
        let unclosed = self.abandoned_open
            && (paused
                || self
                    .gave_up_at
                    .is_some_and(|at| now < at + SHOT_UNCLOSED_HOLD));
        ((paused && !urgent) || unclosed).then_some(SHOT_BACKOFF)
    }
}

pub(super) static BACKOFF: std::sync::Mutex<ShotBackoff> = std::sync::Mutex::new(ShotBackoff {
    until: None,
    gave_up_at: None,
    stalls: 0,
    abandoned_open: false,
    answered_ok: false,
});

pub(super) fn lock_backoff() -> std::sync::MutexGuard<'static, ShotBackoff> {
    lock(&BACKOFF)
}
