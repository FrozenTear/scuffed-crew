//! Poll cadence: slow mode mid-match, the full-cadence wakes, the per-grab
//! route's idle floor, and runs of failed poll grabs.

use crate::*;

/// PR-B (fleet::tracker-fps): mid-match the poller performs its screencopy
/// only on every Nth interval tick. Each compositor screencopy is a GPU
/// readback on the gaming output, the frametime hitch behind the "fuzzy"
/// reports, and mid-match ticks carry no signal worth that cost at full
/// cadence. 2 (not 3): phase/end screens must still land two poll ticks
/// inside their lifetime, the OCR stability gate defers the first sighting,
/// and the shortest screen (map vote, ~15s) only fits two ticks at the 8s
/// effective cadence this yields under the default 4s interval.
pub(crate) const SLOW_POLL_DIVISOR: u32 = 2;

/// Out of a match, a per-grab screenshot route (Older screenshot, KDE
/// direct) skips poll ticks so it grabs about this often, and never less
/// often (the tick count rounds down). Each grab there is a compositor
/// render plus readback (and, for the portal, a dialog and a PNG) on the
/// gaming machine, and in menus and queue the poll mostly sees nothing.
///
/// Timing constraints it keeps:
/// - The map vote (~15 s, the shortest start screen) needs two consecutive
///   ticks because of the OCR stability gate: 3 s or less gives it five.
///   Hero ban and hero select are longer.
/// - The result is read from words: the end title, the accolade screen and
///   the rank screen, two agreeing reads inside the 60 s confirm window. A
///   3 s cadence lands several ticks on each. The classic reader polls
///   every 4 s, so this is never slower than a cadence those screens
///   already work with. The ~3 s colour banner gives no result
///   (0.5.0-rc.9, [`detect::match_end::OutcomeSource`]).
/// - Any end-reel / POTG wake or fresh result-word read returns to full
///   cadence at once ([`cadence_wake_active`]).
///
/// The stream, wayshot and X11 keep their cadence. At the classic 4 s
/// interval this skips nothing.
pub(crate) const IDLE_SHOT_POLL_FLOOR: std::time::Duration = std::time::Duration::from_secs(3);

/// How recently a game must have opened for the poller to stay at full
/// cadence: start screens (map vote / ban / hero select) and early corrective
/// evidence cluster in the first stretch of a session, and matches don't end
/// this early, after it, mid-match slow cadence applies until end evidence
/// shows up (outcome recorded, fresh word-OCR streak, POTG / end-reel wake,
/// or a new game opening).
pub(crate) const SLOW_AFTER_GAME_OPEN: std::time::Duration = std::time::Duration::from_secs(120);

/// How long a Play of the Game / end-reel sighting holds the poller at full
/// cadence. POTG is ~15–20s; the Victory/Defeat banner is ~3s and the
/// accolade screen follows immediately (see `read_result_word`). 45s from
/// first sighting covers remaining reel + the short V/D window with margin,
/// then slow mode resumes if no outcome/streak has landed. Mid-point of the
/// 30–60s range, long enough to not miss Victory, short enough that a
/// false letterbox does not pin 4s screencopy for a full minute.
pub(crate) const END_REEL_WAKE: std::time::Duration = std::time::Duration::from_secs(45);

/// Unexpired POTG / end-reel deadline. Exclusive at the instant (`now < until`)
/// so expiry drops back to mid-match skip / slow cadence on the same tick.
pub(crate) fn end_reel_wake_active(st: &SessionState, now: Instant) -> bool {
    st.end_reel_wake_until.is_some_and(|until| now < until)
}

/// Fresh word-OCR streak or an unexpired end-reel wake, both force full
/// cadence. One helper so the two wake sources cannot drift.
pub(crate) fn cadence_wake_active(st: &SessionState, now: Instant) -> bool {
    st.word_outcome_streak
        .as_ref()
        .is_some_and(|streak| now.duration_since(streak.seen_at) <= OUTCOME_CONFIRM_WINDOW)
        || end_reel_wake_active(st, now)
}

/// Mid-match, skip the auto-detect poll while Tab scoreboard OCR saturates
/// the pool. During an active `end_reel_wake_until` the short Victory/Defeat
/// banner (~3s) can land in that same window, skipping starves the only
/// outcome path (2026-09-06: Tab held through end-reel, no poll
/// `result word: VICTORY`, no `poll_confirm_victory_*` dump). Outcome-signal
/// OCR is small-crop, not the Tab cell pool; see
/// [`poll_outcome_only_while_tab_busy`].
pub(crate) fn skip_poll_for_tab_in_flight(
    tab_in_flight: bool,
    st: &SessionState,
    now: Instant,
) -> bool {
    tab_in_flight && !end_reel_wake_active(st, now)
}

/// Cheap outcome-only poll: Tab OCR is in flight **and** end-reel wake is
/// still hot. Screenshot + `detect_outcome_signal_*` + `detect_end_reel` +
/// existing confirm/streak / `poll_debug_hit` dumps. Phase and accolade-map
/// OCR stay off so we do not compete with the Tab cell pool. Mid-match
/// (no wake) never takes this path, those ticks still skip.
pub(crate) fn poll_outcome_only_while_tab_busy(
    tab_in_flight: bool,
    st: &SessionState,
    now: Instant,
) -> bool {
    tab_in_flight && end_reel_wake_active(st, now)
}

pub(crate) fn clear_cadence_wakes(st: &mut SessionState) {
    st.word_outcome_streak = None;
    st.end_reel_wake_until = None;
}

/// PR-B: whether the poller sits mid-match with nothing imminent, a mature
/// open game, outcome still unknown, and no cadence wake (fresh word-OCR
/// streak or POTG / end-reel deadline). Every end-of-match signal path
/// already drops this back to full cadence through existing state: a confirmed
/// result records the outcome (`finished()`), a word read sets `word_outcome_streak`,
/// an end-reel / POTG hit sets `end_reel_wake_until`, and a detected start
/// phase opens a new game (resetting `last_game_open`).
pub(crate) fn poll_slow_mode(st: &SessionState, now: Instant) -> bool {
    st.active_game.as_ref().is_some_and(|g| !g.finished())
        && st
            .last_game_open
            .is_none_or(|t| now.duration_since(t) >= SLOW_AFTER_GAME_OPEN)
        && !cadence_wake_active(st, now)
        && !boundary_hint_open(st)
}

/// A game boundary or an unconfirmed result is waiting on the poller.
pub(crate) fn boundary_hint_open(st: &SessionState) -> bool {
    st.active_game.as_ref().is_some_and(|g| {
        g.pending_boundary
            || g.result_mark
                .is_some_and(|mark| !mark.confirmed && mark.outcome.is_decided())
    })
}

/// No match in progress as far as the poller knows: no open game, or the
/// open one has its result, and nothing is waiting on a poll.
pub(crate) fn no_match_in_progress(st: &SessionState, now: Instant) -> bool {
    st.active_game.as_ref().is_none_or(|g| g.finished())
        && !cadence_wake_active(st, now)
        && !boundary_hint_open(st)
}

/// Poll ticks per screen grab: mid-match slow mode ([`SLOW_POLL_DIVISOR`]),
/// else out of a match on a per-grab screenshot route the whole ticks that
/// fit in [`IDLE_SHOT_POLL_FLOOR`], else every tick.
pub(crate) fn poll_divisor(
    st: &SessionState,
    now: Instant,
    route: capture::ShotRoute,
    interval: std::time::Duration,
) -> u32 {
    if poll_slow_mode(st, now) {
        return SLOW_POLL_DIVISOR;
    }
    if route.polls_less_out_of_a_match() && no_match_in_progress(st, now) {
        return idle_shot_divisor(interval);
    }
    1
}

/// Whether a finished poll grab restarts the slow-cadence count. A grab
/// turned away because a Tab held the screenshot lock did not look at the
/// screen, so the next tick must poll.
pub(crate) fn poll_resets_cadence<T, E: std::fmt::Display, S>(
    grabbed: &Result<Result<T, E>, S>,
) -> bool {
    !matches!(grabbed, Ok(Err(err)) if capture::is_capture_busy(&err.to_string()))
}

/// While poll grabs keep failing, a WARN repeats at most this often.
pub(crate) const POLL_FAILURE_WARN_EVERY: std::time::Duration =
    std::time::Duration::from_secs(5 * 60);

/// A run of failed poll grabs while the game runs (S5-CAP-7). The first
/// failure of a run is a WARN, then one at most every
/// [`POLL_FAILURE_WARN_EVERY`] while the run lasts; the rest are TRACE.
/// The clock is not reset by a success, so grabs that fail and work in
/// turn do not warn on every tick. A known state ([`capture::is_expected_poll_miss`]:
/// a held capture lock, a share still opening, declined or closed until the
/// game runs, no fresh stream frame) is neither a failure nor a success.
/// The run ends when the game stops.
#[derive(Debug, Default)]
pub(crate) struct PollFailureStreak {
    /// Failures in this run.
    pub(crate) count: u32,
    started: Option<Instant>,
    /// This run logged a WARN.
    warned: bool,
    last_warn: Option<Instant>,
}

impl PollFailureStreak {
    /// Counts one failure; true when this one is logged at WARN.
    pub(crate) fn failed(&mut self, now: Instant) -> bool {
        self.count = self.count.saturating_add(1);
        self.started.get_or_insert(now);
        let warn = self
            .last_warn
            .is_none_or(|at| now.saturating_duration_since(at) >= POLL_FAILURE_WARN_EVERY);
        if warn {
            self.last_warn = Some(now);
            self.warned = true;
        }
        warn
    }

    /// How to log one poll grab error, counting it when it is a failure.
    pub(crate) fn on_error(&mut self, err: &str, now: Instant) -> PollErrorLog {
        if capture::is_expected_poll_miss(err) {
            PollErrorLog::Expected
        } else if self.failed(now) {
            PollErrorLog::Warn
        } else {
            PollErrorLog::Trace
        }
    }

    /// The game stopped: counts and times do not carry into the next game.
    pub(crate) fn game_stopped(&mut self) {
        *self = Self::default();
    }

    /// A grab worked: the run ends. Returns how many failed and for how
    /// long when the run logged a WARN, so its end is logged too.
    pub(crate) fn recovered(&mut self, now: Instant) -> Option<(u32, std::time::Duration)> {
        let started = self.started.take()?;
        let count = std::mem::take(&mut self.count);
        std::mem::take(&mut self.warned).then(|| (count, now.saturating_duration_since(started)))
    }
}

/// How one poll grab error is logged ([`PollFailureStreak::on_error`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PollErrorLog {
    /// A known state, not a failure: TRACE, the run untouched.
    Expected,
    /// A failure logged at WARN.
    Warn,
    /// A failure inside the WARN gap: TRACE.
    Trace,
}

/// Whole ticks of `interval` that fit in [`IDLE_SHOT_POLL_FLOOR`], at
/// least 1. Rounding down keeps the cadence at or under the floor.
pub(crate) fn idle_shot_divisor(interval: std::time::Duration) -> u32 {
    let interval = interval.as_millis().max(1);
    let floor = IDLE_SHOT_POLL_FLOOR.as_millis();
    u32::try_from(floor / interval).unwrap_or(1).max(1)
}
