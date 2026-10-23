//! The pause fail-safe: the active clock, resume bookkeeping and the trip
//! rules. Plain data, no PipeWire.

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use super::ffi::{PW_STREAM_STATE_PAUSED, PW_STREAM_STATE_STREAMING};
use super::stream_state_is_dead;

/// Active time a resume has to reach Streaming, or bring a buffer. PipeWire
/// itself answers a resume in a few ms (Start to the follower, then the
/// driver); KWin 6.8 then waits at most one frame period (250 ms at 4 fps).
/// It is above the 1200 ms Tab window, so one slow Tab alone does not
/// count.
pub const RESUME_BUFFER_BOUND: Duration = Duration::from_millis(1500);

/// Consecutive unanswered resumes ([`RESUME_BUFFER_BOUND`] each) that turn
/// pausing off for one stream. A resume that reaches Streaming or brings a
/// buffer resets the count.
pub const RESUME_FAILS_TO_TRIP: u32 = 2;

/// Streams whose fail-safe has tripped since this process started. A new
/// stream may pause again while this is below [`PAUSE_TRIPS_LIMIT`].
pub(super) static PAUSE_TRIPS: AtomicU32 = AtomicU32::new(0);

/// After this many tripped streams the tracker stops pausing for the rest
/// of the process: the first trip lets one new share or reopen try again.
pub const PAUSE_TRIPS_LIMIT: u32 = 2;

pub fn pause_trips() -> u32 {
    PAUSE_TRIPS.load(Ordering::Acquire)
}

pub const TRIP_SLOW_RESUMES: &str = "resumes stayed unanswered (no Streaming, no frame) for 1.5 s of active time twice, with no answer in between";
pub const TRIP_TAB_MISS: &str =
    "a Tab missed its frame and the resumed stream had not reached Streaming";
pub const TRIP_STUCK: &str =
    "the stream stayed paused (or failed) 1.5 s after the tracker resumed it";
pub const TRIP_RETIRED: &str = "the stream was closed while a resume was still unanswered";
pub const TRIP_FAILED_PAUSED: &str = "the stream ended within 2 s of a pause the tracker asked for";

/// A stream that ends this soon after the Paused the tracker asked for is
/// counted as a failed pause ([`TRIP_FAILED_PAUSED`]). While a game is
/// played the stream pauses again about every 1.5 s, so this window is
/// nearly always open then; it only filters ends on a share that sits
/// idle (slow poll mode, or Overwatch not running). What keeps external
/// ends from counting are the exemptions: a lost PipeWire connection
/// ([`state_is_core_fault`], so a PipeWire restart) and a closed portal
/// session (on KDE a stopped share or an output change ends the KWin
/// stream and xdg-desktop-portal-kde closes the session).
///
/// [`state_is_core_fault`]: super::state_is_core_fault
pub const FAILED_AFTER_PAUSE_WINDOW: Duration = Duration::from_secs(2);

/// The one warning a trip logs. A stream that ended right after a pause
/// may have ended for another reason, so that line does not blame pausing.
pub fn trip_warning(why: &str) -> &'static str {
    if why == TRIP_FAILED_PAUSED {
        "screencast stream ended right after the tracker paused it; it can also be an output or monitor change, or the compositor ending the share, but pause between grabs is now off for this stream"
    } else {
        "screencast pause between grabs turned off for this stream; it stays running"
    }
}

/// Time during which the tracker wanted the stream running. It stops while
/// the tracker keeps the stream paused and starts again on resume, so
/// health timers measured on it ignore the tracker's own pauses. With
/// pausing off it never stops and equals the stream's age.
#[derive(Debug, Clone, Copy)]
pub(super) struct ActiveClock {
    banked: Duration,
    since: Option<std::time::Instant>,
}

impl ActiveClock {
    pub(super) fn running(at: std::time::Instant) -> Self {
        Self {
            banked: Duration::ZERO,
            since: Some(at),
        }
    }

    pub(super) fn reading(&self, now: std::time::Instant) -> Duration {
        self.banked
            + self
                .since
                .map_or(Duration::ZERO, |at| now.saturating_duration_since(at))
    }

    pub(super) fn start(&mut self, now: std::time::Instant) {
        if self.since.is_none() {
            self.since = Some(now);
        }
    }

    pub(super) fn stop(&mut self, now: std::time::Instant) {
        if let Some(at) = self.since.take() {
            self.banked += now.saturating_duration_since(at);
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct ResumeLatency {
    pub(super) count: u64,
    pub(super) total_ms: u64,
    pub(super) max_ms: u64,
    pub(super) unanswered: u64,
}

/// The PipeWire state a fail-safe check needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PwPhase {
    Streaming,
    Paused,
    /// Error, or Unconnected after it had streamed.
    Failed,
    Other,
}

pub(super) fn pw_phase(state: i32, seen_streaming: bool) -> PwPhase {
    match state {
        PW_STREAM_STATE_STREAMING => PwPhase::Streaming,
        PW_STREAM_STATE_PAUSED => PwPhase::Paused,
        _ if stream_state_is_dead(state, seen_streaming) => PwPhase::Failed,
        _ => PwPhase::Other,
    }
}

/// Pause bookkeeping for one stream. Plain data so it can be tested
/// without PipeWire. All `Duration` stamps are active-clock readings.
///
/// A resume is *answered* when a buffer arrives. On niri, which draws a
/// cast only on damage (a still screen sends nothing after a resume),
/// reaching Streaming again answers it too. Everywhere else a resume that
/// reaches Streaming and sends no buffer is a broken resume: KWin 6.1+
/// emits a frame on resume and Mutter queues a redraw.
#[derive(Debug)]
pub(super) struct PauseState {
    /// The tracker wants the stream running. Always true unless pausing is
    /// on for this stream, it has streamed, and no grab holds it.
    pub(super) want: bool,
    /// Pausing is allowed on this stream (decided at start, symbols present,
    /// and the process has not run out of fail-safe trips).
    pub(super) managed: bool,
    /// Why the fail-safe turned pausing off for this stream.
    pub(super) tripped: Option<&'static str>,
    pub(super) clock: ActiveClock,
    /// When the stream entered Paused, after it had streamed.
    pub(super) paused_at: Option<Duration>,
    /// That Paused began while the tracker did not want the stream running,
    /// or answered a pause the tracker had asked for: it started as the
    /// tracker's own pause.
    pub(super) paused_by_us: bool,
    /// A pause was requested and its Paused has not been seen yet. A
    /// resume that arrives first still leaves that Paused marked ours.
    self_pause_pending: bool,
    /// Reaching Streaming answers a resume (niri only).
    pub(super) streaming_answers: bool,
    /// The stream failed (Error, or Unconnected after streaming) within
    /// [`FAILED_AFTER_PAUSE_WINDOW`] of the Paused the tracker asked for,
    /// with no resume outstanding.
    pub(super) failed_while_paused: bool,
    /// Wall time of the Paused that answered the tracker's own pause
    /// request. Cleared when the stream streams again.
    pub(super) our_paused_wall: Option<std::time::Instant>,
    /// The stream ended for a reason outside the pause: the portal session
    /// closed. A core fault is passed in separately.
    pub(super) external_end: bool,
    /// When the last PipeWire buffer was dequeued.
    last_buffer_at: Option<Duration>,
    /// Start of the oldest resume not answered yet. It survives a pause, so
    /// short grabs add up, and only an answer clears it.
    pub(super) unanswered_since: Option<Duration>,
    /// Start of the unanswered resume still to be counted toward
    /// [`RESUME_FAILS_TO_TRIP`]. Counting it ends it; the next resume starts
    /// another.
    episode_from: Option<Duration>,
    /// Wall time of the latest resume request, until its first buffer.
    resume_requested: Option<std::time::Instant>,
    /// Consecutive unanswered resumes.
    pub(super) resume_fails: u32,
    pub(super) latency: ResumeLatency,
}

impl PauseState {
    pub(super) fn new(now: std::time::Instant) -> Self {
        Self {
            want: true,
            managed: false,
            tripped: None,
            clock: ActiveClock::running(now),
            paused_at: None,
            paused_by_us: false,
            self_pause_pending: false,
            streaming_answers: false,
            failed_while_paused: false,
            our_paused_wall: None,
            external_end: false,
            last_buffer_at: None,
            unanswered_since: None,
            episode_from: None,
            resume_requested: None,
            resume_fails: 0,
            latency: ResumeLatency::default(),
        }
    }

    /// Pausing is on and has not tripped.
    pub(super) fn pausing(&self) -> bool {
        self.managed && self.tripped.is_none()
    }

    /// Record a run or pause request. `streaming_now` is the stream's
    /// current state: a resume of a stream that never left Streaming (the
    /// pause had not been applied yet) has nothing to answer. Returns true
    /// when the wanted state changed and the loop thread has to apply it.
    pub(super) fn request(
        &mut self,
        want: bool,
        streamed: bool,
        streaming_now: bool,
        now: std::time::Instant,
    ) -> bool {
        let want = want || !self.pausing() || !streamed;
        if want == self.want {
            return false;
        }
        if want {
            self.clock.start(now);
            self.resume_requested = Some(now);
            if streaming_now {
                // The pause was never applied: no Paused will come.
                self.self_pause_pending = false;
            } else {
                let reading = self.clock.reading(now);
                self.unanswered_since.get_or_insert(reading);
                self.episode_from.get_or_insert(reading);
            }
        } else {
            if self.resume_requested.take().is_some() {
                self.latency.unanswered = self.latency.unanswered.saturating_add(1);
            }
            self.clock.stop(now);
            self.self_pause_pending = true;
        }
        self.want = want;
        true
    }

    /// The resume was answered: Streaming again, or a buffer.
    fn answered(&mut self) {
        self.unanswered_since = None;
        self.episode_from = None;
        self.resume_fails = 0;
    }

    /// Count an unanswered resume that has run [`RESUME_BUFFER_BOUND`] of
    /// active time. Returns the trip reason when the count reaches
    /// [`RESUME_FAILS_TO_TRIP`].
    pub(super) fn check(&mut self, now: std::time::Instant) -> Option<&'static str> {
        if !self.pausing() {
            return None;
        }
        let from = self.episode_from?;
        if self.clock.reading(now).saturating_sub(from) < RESUME_BUFFER_BOUND {
            return None;
        }
        self.episode_from = None;
        self.resume_fails = self.resume_fails.saturating_add(1);
        (self.resume_fails >= RESUME_FAILS_TO_TRIP).then_some(TRIP_SLOW_RESUMES)
    }

    /// A grab ended while the stream was still held. A Tab miss while the
    /// resume is unanswered trips at once.
    pub(super) fn grab_end(
        &mut self,
        tab_missed: bool,
        now: std::time::Instant,
    ) -> Option<&'static str> {
        // A closed portal session ended the grab, not the resume.
        if !self.pausing() || self.external_end {
            return None;
        }
        if tab_missed && self.unanswered_since.is_some() {
            return Some(TRIP_TAB_MISS);
        }
        self.check(now)
    }

    /// Checked before every health read. A pause that began as the
    /// tracker's own and has not lifted [`RESUME_BUFFER_BOUND`] after the
    /// resume, or a stream that failed while a resume was unanswered, trips
    /// here, before the 2 s paused rule can call it dead and reopen it with
    /// pausing still on.
    pub(super) fn stuck(
        &self,
        phase: PwPhase,
        external: bool,
        now: std::time::Instant,
    ) -> Option<&'static str> {
        if !self.pausing() {
            return None;
        }
        // A closed portal session or a lost PipeWire connection is not the
        // pause failing.
        if phase == PwPhase::Failed && (external || self.external_end) {
            return None;
        }
        if phase == PwPhase::Failed && self.failed_while_paused {
            return Some(TRIP_FAILED_PAUSED);
        }
        let since = self.unanswered_since?;
        match phase {
            PwPhase::Failed => Some(TRIP_STUCK),
            PwPhase::Paused
                if self.paused_by_us
                    && self.clock.reading(now).saturating_sub(since) >= RESUME_BUFFER_BOUND =>
            {
                Some(TRIP_STUCK)
            }
            _ => None,
        }
    }

    /// The stream is being closed. An unanswered resume counts as a trip.
    pub(super) fn retire(&self, external: bool) -> Option<&'static str> {
        if !self.pausing() || external || self.external_end {
            return None;
        }
        if self.failed_while_paused {
            return Some(TRIP_FAILED_PAUSED);
        }
        self.unanswered_since.is_some().then_some(TRIP_RETIRED)
    }

    /// Turn pausing off for this stream. Returns true the first time.
    pub(super) fn trip(&mut self, why: &'static str, now: std::time::Instant) -> bool {
        if self.tripped.is_some() {
            return false;
        }
        self.tripped = Some(why);
        self.unanswered_since = None;
        self.episode_from = None;
        if !self.want {
            if self.resume_requested.take().is_some() {
                self.latency.unanswered = self.latency.unanswered.saturating_add(1);
            }
            self.clock.start(now);
            self.want = true;
        }
        true
    }

    /// A buffer was dequeued. Returns the resume latency in ms when this is
    /// the first buffer after a resume request.
    pub(super) fn on_buffer(&mut self, now: std::time::Instant) -> Option<u64> {
        self.last_buffer_at = Some(self.clock.reading(now));
        self.answered();
        let at = self.resume_requested.take()?;
        let ms = now.saturating_duration_since(at).as_millis() as u64;
        let latency = &mut self.latency;
        latency.count = latency.count.saturating_add(1);
        latency.total_ms = latency.total_ms.saturating_add(ms);
        latency.max_ms = latency.max_ms.max(ms);
        Some(ms)
    }

    pub(super) fn on_streaming(&mut self) {
        self.paused_at = None;
        self.paused_by_us = false;
        self.self_pause_pending = false;
        self.our_paused_wall = None;
        if self.streaming_answers {
            self.answered();
        }
    }

    pub(super) fn on_paused(&mut self, now: std::time::Instant) {
        if self.paused_at.is_none() {
            self.paused_at = Some(self.clock.reading(now));
            self.paused_by_us = !self.want || self.self_pause_pending;
            if self.paused_by_us {
                self.our_paused_wall = Some(now);
            }
        }
        self.self_pause_pending = false;
    }

    /// Error, or Unconnected after streaming.
    /// `now` is a monotonic `Instant` (suspend is not counted). Counted
    /// only within [`FAILED_AFTER_PAUSE_WINDOW`] of the Paused that
    /// answered the tracker's own pause.
    pub(super) fn on_failed(&mut self, now: std::time::Instant) {
        let soon_after_our_pause = self
            .our_paused_wall
            .is_some_and(|at| now.saturating_duration_since(at) <= FAILED_AFTER_PAUSE_WINDOW);
        if self.pausing() && !self.want && self.unanswered_since.is_none() && soon_after_our_pause {
            self.failed_while_paused = true;
        }
    }

    pub(super) fn on_unpaused(&mut self) {
        self.paused_at = None;
        self.paused_by_us = false;
    }

    pub(super) fn paused_for(&self, now: std::time::Instant) -> Option<Duration> {
        let at = self.paused_at?;
        Some(self.clock.reading(now).saturating_sub(at))
    }

    /// Paused by the tracker's own request: it does not want the stream
    /// running, and the pause began after the clock stopped. A pause that
    /// began, or lasted, while the tracker wanted the stream running has
    /// active time on it and is not this.
    pub(super) fn self_paused(&self, now: std::time::Instant) -> bool {
        !self.want && self.paused_for(now) == Some(Duration::ZERO)
    }

    pub(super) fn quiet_for(&self, now: std::time::Instant) -> Duration {
        self.clock
            .reading(now)
            .saturating_sub(self.last_buffer_at.unwrap_or(Duration::ZERO))
    }
}
