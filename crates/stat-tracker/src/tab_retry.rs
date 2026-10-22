//! Retries for Tab captures that the scoreboard preflight rejected, and
//! what a new Tab press does while a capture is still running.
//!
//! A rejected Tab capture captures again after a short wait, at about
//! 300 ms and about 700 ms after the first rejection, before giving up. Each
//! retry is one screen grab plus the cheap preflight, so CPU stays light.
//! On the screen stream a retry takes the next buffer that arrives, with no
//! second overlay delay (`capture::tab_retry_press`).
//!
//! The rejected frames saved so far (2026-10-15 to 2026-10-21) showed
//! gameplay, a killcam, the VICTORY banner, chat or the desktop: the board
//! was already closed or not open, not half drawn. The player's own next
//! Tab is then the best chance of a board, so a new press stops the
//! running capture's retries and runs as soon as that capture reports
//! ([`tab_press_action`], [`next_press_after_report`]). The 3 s debounce
//! applies only after a board was accepted.
//!
//! Retries run only while Tab is held: a frame taken after the key was
//! released shows the game, not the board. A tap released before any frame
//! passed the check logs a hint to hold Tab for about a second
//! ([`short_tap_hint`]).

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::{Duration, Instant};

/// Waits before each retry, counted from the previous attempt. The retries
/// land about 300 ms and about 700 ms after the first rejection.
pub const TAB_RETRY_WAITS: [Duration; 2] = [Duration::from_millis(300), Duration::from_millis(400)];

/// The retry waits for a capture route. The screenshot portal gets one
/// retry: each of its grabs is a full KDE capture of about 450 ms plus a
/// PNG, so a second retry lands well after the board and costs the game a
/// third full capture. The stream, wayshot, X11 and KDE direct keep both.
pub fn tab_retry_waits(route: crate::capture::ShotRoute) -> &'static [Duration] {
    if route.one_tab_retry() {
        &TAB_RETRY_WAITS[..1]
    } else {
        &TAB_RETRY_WAITS
    }
}

/// After an accepted board, Tab presses this soon after its press are
/// ignored. A rejected capture sets no debounce.
pub const TAB_DEBOUNCE_AFTER_ACCEPT: Duration = Duration::from_secs(3);

/// A capture with no accepted board whose Tab was released sooner than this
/// after the press logs the hold-Tab hint.
pub const TAB_HOLD_HINT_BELOW: Duration = Duration::from_secs(1);

/// The hint for a tap that ended before the board could be captured.
pub const TAB_HOLD_HINT: &str =
    "Tab was released before the scoreboard could be captured: hold Tab for about a second";

/// How one Tab grab ended, for the retry loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabResult {
    /// A frame passed the scoreboard check.
    Accepted,
    /// The frame did not show a full scoreboard. Retried while Tab is held.
    NotAScoreboard,
    /// No frame yet (no fresh stream buffer, share still opening). Retried
    /// while Tab is held.
    NoFrameYet,
    /// Nothing a retry can fix (share declined, team size unsure).
    Final,
}

impl TabResult {
    /// Whether this result is worth another grab.
    pub fn retries(self) -> bool {
        matches!(self, Self::NotAScoreboard | Self::NoFrameYet)
    }
}

/// The hold-Tab hint logs at WARN at most this often, at INFO in between.
pub const TAB_HOLD_HINT_WARN_EVERY: Duration = Duration::from_secs(600);

/// Whether the hold-Tab hint logged now is a WARN, given the last WARN.
pub fn hold_hint_is_warn(last_warn: Option<Instant>, now: Instant) -> bool {
    last_warn.is_none_or(|at| now.saturating_duration_since(at) >= TAB_HOLD_HINT_WARN_EVERY)
}

fn log_hold_hint() {
    static LAST_WARN: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
    let now = Instant::now();
    let warn = {
        let mut last = LAST_WARN.lock().unwrap_or_else(|err| err.into_inner());
        let warn = hold_hint_is_warn(*last, now);
        if warn {
            *last = Some(now);
        }
        warn
    };
    if warn {
        tracing::warn!("{TAB_HOLD_HINT}");
    } else {
        tracing::info!("{TAB_HOLD_HINT}");
    }
}

/// Why a capture's retries stopped early.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryStop {
    /// A newer Tab press takes over.
    NewPress,
    /// The key was released: a later frame shows the game, not the board.
    Released,
}

/// Where one Tab capture is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TabFlightPhase {
    /// The first grab, before any verdict.
    Grabbing = 0,
    /// The first grab was rejected and retries are pending or running.
    Retrying = 1,
    /// A frame passed the scoreboard check and is being read.
    Accepted = 2,
    /// The capture ended without an accepted board.
    Rejected = 3,
}

impl TabFlightPhase {
    fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Grabbing,
            1 => Self::Retrying,
            2 => Self::Accepted,
            _ => Self::Rejected,
        }
    }
}

/// Shared between the daemon loop and one Tab capture task.
#[derive(Debug)]
pub struct TabFlight {
    /// The key press this capture is for.
    pub press: Instant,
    phase: AtomicU8,
    cancel: AtomicBool,
    released_at: std::sync::Mutex<Option<Instant>>,
    wake: tokio::sync::Notify,
}

impl TabFlight {
    pub fn new(press: Instant) -> Arc<Self> {
        Arc::new(Self {
            press,
            phase: AtomicU8::new(TabFlightPhase::Grabbing as u8),
            cancel: AtomicBool::new(false),
            released_at: std::sync::Mutex::new(None),
            wake: tokio::sync::Notify::new(),
        })
    }

    pub fn phase(&self) -> TabFlightPhase {
        TabFlightPhase::from_u8(self.phase.load(Ordering::Acquire))
    }

    pub fn set_phase(&self, phase: TabFlightPhase) {
        self.phase.store(phase as u8, Ordering::Release);
    }

    /// Record how the capture ended. A short tap whose frame showed no
    /// scoreboard logs the hold-Tab hint. [`with_retries`] calls this.
    pub fn finish(&self, result: TabResult) {
        if short_tap_hint(result, self.press, self.released_at()) {
            log_hold_hint();
        }
        self.set_phase(if result == TabResult::Accepted {
            TabFlightPhase::Accepted
        } else {
            TabFlightPhase::Rejected
        });
    }

    /// A newer Tab press takes over: no further retry starts, and a retry
    /// wait that is running ends now. A grab already running finishes.
    pub fn cancel_retries(&self) {
        self.cancel.store(true, Ordering::Release);
        // `notify_one` keeps a permit when nothing waits yet.
        self.wake.notify_one();
    }

    pub fn retries_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }

    /// The Tab key went up at `at`. A release from before this press (an
    /// earlier tap) is ignored. No retry starts after it.
    pub fn note_release(&self, at: Instant) {
        if at < self.press {
            return;
        }
        let mut released = self
            .released_at
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        if released.is_none() {
            *released = Some(at);
        }
        drop(released);
        self.wake.notify_one();
    }

    /// When the key was released, if it was.
    pub fn released_at(&self) -> Option<Instant> {
        *self
            .released_at
            .lock()
            .unwrap_or_else(|err| err.into_inner())
    }

    /// Whether retries must stop, and why. A newer press wins over a release.
    pub fn retry_stop(&self) -> Option<RetryStop> {
        if self.retries_cancelled() {
            Some(RetryStop::NewPress)
        } else if self.released_at().is_some() {
            Some(RetryStop::Released)
        } else {
            None
        }
    }

    async fn stopped(&self) {
        while self.retry_stop().is_none() {
            self.wake.notified().await;
        }
    }
}

/// Whether a finished capture should log [`TAB_HOLD_HINT`]: the frame it
/// kept showed no scoreboard, and the key was released under
/// [`TAB_HOLD_HINT_BELOW`] after the press. A missing stream frame, a share
/// still opening or declined, and an accepted board get no hint.
pub fn short_tap_hint(result: TabResult, press: Instant, released_at: Option<Instant>) -> bool {
    result == TabResult::NotAScoreboard
        && released_at.is_some_and(|at| at.saturating_duration_since(press) < TAB_HOLD_HINT_BELOW)
}

tokio::task_local! {
    static TAB_FLIGHT: Arc<TabFlight>;
}

/// Run `fut` as the capture task of `flight`.
pub async fn with_flight<T>(flight: Arc<TabFlight>, fut: impl Future<Output = T>) -> T {
    TAB_FLIGHT.scope(flight, fut).await
}

/// The flight of the current capture task. Outside one (a test harness
/// calling the capture directly) this is a fresh flight nobody cancels.
pub fn current_flight() -> Arc<TabFlight> {
    TAB_FLIGHT
        .try_with(Arc::clone)
        .unwrap_or_else(|_| TabFlight::new(Instant::now()))
}

/// What the daemon does with one Tab press.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabPressAction {
    /// Start a capture for this press now.
    Start,
    /// Within [`TAB_DEBOUNCE_AFTER_ACCEPT`] of an accepted board's press.
    Debounced,
    /// A capture is running and has no accepted board: stop its retries
    /// and run this press as soon as it reports.
    Queue,
    /// The running capture already accepted a board.
    Ignore,
}

/// `last_accepted` is the press of the last accepted board. `in_flight` is
/// the phase of the capture still running (its report not yet handled).
pub fn tab_press_action(
    press: Instant,
    last_accepted: Option<Instant>,
    in_flight: Option<TabFlightPhase>,
) -> TabPressAction {
    if last_accepted
        .is_some_and(|at| press.saturating_duration_since(at) < TAB_DEBOUNCE_AFTER_ACCEPT)
    {
        return TabPressAction::Debounced;
    }
    match in_flight {
        None => TabPressAction::Start,
        Some(TabFlightPhase::Accepted) => TabPressAction::Ignore,
        Some(_) => TabPressAction::Queue,
    }
}

/// A capture reported. An accepted board starts the debounce from its
/// press, and a press queued behind it is dropped (it would read the same
/// board). Otherwise the queued press, if any, runs now.
pub fn next_press_after_report(
    flight: &TabFlight,
    queued: Option<Instant>,
    last_accepted: &mut Option<Instant>,
) -> Option<Instant> {
    if flight.phase() == TabFlightPhase::Accepted {
        *last_accepted = Some(flight.press);
        return None;
    }
    queued
}

/// Runs `attempt` once, and again after each wait in `waits` for as long as
/// `result` says the best result so far is worth another grab
/// ([`TabResult::retries`]). `why` names
/// that result for the log: a frame with no scoreboard, a missing stream
/// frame, and a share that is still opening are not the same line.
/// A later result replaces the kept one only when `rank` scores it higher,
/// so a rejected grab never replaces a more useful one. Returns the best
/// result and how many retries ran. An error from `attempt` (a failed
/// screen grab) ends the loop at once.
///
/// `flight` moves to [`TabFlightPhase::Retrying`] before the first retry.
/// Once [`TabFlight::cancel_retries`] or [`TabFlight::note_release`] is
/// called, no further retry starts and a wait that is running ends, and
/// the best result so far is returned. Retries therefore run only while
/// Tab is held, at most `waits.len()` of them. When the loop ends without an
/// error, `flight` is finished with the best result ([`TabFlight::finish`]).
pub async fn with_retries<T, E, A, AF, S, SF>(
    flight: &TabFlight,
    waits: &[Duration],
    result: impl Fn(&T) -> TabResult,
    rank: impl Fn(&T) -> u8,
    why: impl Fn(&T) -> &'static str,
    mut attempt: A,
    mut sleep: S,
) -> Result<(T, usize), E>
where
    A: FnMut(usize) -> AF,
    AF: Future<Output = Result<T, E>>,
    S: FnMut(Duration) -> SF,
    SF: Future<Output = ()>,
{
    let retry = |t: &T| result(t).retries();
    let mut best = attempt(0).await?;
    let mut retries = 0;
    let mut stopped = None;
    for &wait in waits {
        if !retry(&best) {
            break;
        }
        if let Some(why) = flight.retry_stop() {
            stopped = Some(why);
            break;
        }
        flight.set_phase(TabFlightPhase::Retrying);
        retries += 1;
        let reason = why(&best);
        tracing::info!(
            retry = retries,
            of = waits.len(),
            wait_ms = wait.as_millis() as u64,
            "Tab capture will try again: {reason}"
        );
        tokio::select! {
            biased;
            _ = flight.stopped() => {}
            _ = sleep(wait) => {}
        }
        if let Some(why) = flight.retry_stop() {
            stopped = Some(why);
            break;
        }
        let next = attempt(retries).await?;
        if rank(&next) > rank(&best) {
            best = next;
        }
    }
    if let Some(why) = stopped {
        let line = match why {
            RetryStop::NewPress => "a newer Tab press takes over",
            RetryStop::Released => "Tab was released (a later frame shows the game, not the board)",
        };
        tracing::info!(retries, "Tab capture retries stopped: {line}");
    } else if retries > 0 {
        if retry(&best) {
            let reason = why(&best);
            tracing::warn!(retries, "Tab capture gave up after retries: {reason}");
        } else {
            tracing::info!(retries, "Tab capture accepted on retry");
        }
    }
    flight.finish(result(&best));
    Ok((best, retries))
}

/// One Tab key event source (the merged keyboards in the daemon).
pub trait TabKeySource {
    fn next_key(
        &mut self,
    ) -> impl Future<Output = Result<crate::detect::TabKey, Box<dyn std::error::Error>>>;
}

impl TabKeySource for crate::detect::MultiKeyboardStream {
    fn next_key(
        &mut self,
    ) -> impl Future<Output = Result<crate::detect::TabKey, Box<dyn std::error::Error>>> {
        self.wait_tab()
    }
}

/// The next Tab press from `source`. A release on the way is recorded in
/// `tabs` and does not end the wait, so a poll raced against this is
/// dropped for a press only, never for a release.
pub async fn next_press<S: TabKeySource>(
    source: &mut S,
    tabs: &mut TabKeys,
) -> Result<crate::detect::TabKey, Box<dyn std::error::Error>> {
    loop {
        let key = source.next_key().await?;
        if key.down {
            return Ok(key);
        }
        tabs.on_release(key.at);
    }
}

/// The daemon loop's Tab state between key events and capture reports:
/// the running capture, a press queued behind it, and the last release.
#[derive(Debug, Default)]
pub struct TabKeys {
    flight: Option<Arc<TabFlight>>,
    queued: Option<Instant>,
    last_release: Option<Instant>,
}

impl TabKeys {
    /// The Tab key went up at `at`: the running capture stops retrying, and
    /// a press queued behind it gets the release when it starts.
    pub fn on_release(&mut self, at: Instant) {
        self.last_release = Some(at);
        if let Some(flight) = &self.flight {
            flight.note_release(at);
        }
    }

    /// A Tab press at `press` (the key-event time). Returns the press to
    /// start a capture for now, if any. `capture_gone` is true when the
    /// running capture's task ended without a report waiting (a panic), so
    /// it no longer holds presses.
    pub fn on_press(
        &mut self,
        press: Instant,
        last_accepted: Option<Instant>,
        capture_gone: bool,
    ) -> Option<Instant> {
        if self.flight.is_some() && capture_gone {
            self.flight = None;
            self.queued = None;
        }
        let in_flight = self.flight.as_ref().map(|f| f.phase());
        match tab_press_action(press, last_accepted, in_flight) {
            TabPressAction::Debounced => {
                tracing::debug!("Tab debounced: a board was accepted under 3 s ago");
                None
            }
            TabPressAction::Ignore => {
                tracing::debug!("Tab ignored: the capture in progress already has a board");
                None
            }
            TabPressAction::Queue => {
                if let Some(flight) = &self.flight {
                    flight.cancel_retries();
                }
                self.queued = Some(press);
                tracing::info!(
                    "Tab pressed again before the capture found a board: its retries stop and this press runs next"
                );
                None
            }
            TabPressAction::Start => {
                self.queued = None;
                Some(press)
            }
        }
    }

    /// A capture was started for `flight`. A release already seen after its
    /// press (a press queued and released meanwhile) is passed on.
    pub fn started(&mut self, flight: Arc<TabFlight>) {
        if let Some(at) = self.last_release {
            flight.note_release(at);
        }
        self.flight = Some(flight);
    }

    /// The press waiting for the running capture to report.
    pub fn queued_press(&self) -> Option<Instant> {
        self.queued
    }

    /// The running capture reported. Returns a queued press to start now
    /// ([`next_press_after_report`]); an accepted board sets `last_accepted`.
    pub fn on_report(&mut self, last_accepted: &mut Option<Instant>) -> Option<Instant> {
        let flight = self.flight.take()?;
        next_press_after_report(&flight, self.queued.take(), last_accepted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 0 = no scoreboard (retried), 1 = useful but final, else accepted.
    fn u8_result(v: u8) -> TabResult {
        match v {
            0 => TabResult::NotAScoreboard,
            1 => TabResult::Final,
            _ => TabResult::Accepted,
        }
    }
    use crate::detect::hero_portrait::{ScoreboardPreflight, preflight_scoreboard};
    use crate::ocr::preprocess::{crop_scoreboard, crop_scoreboard_rect};
    use crate::report::blank::synth::{self, Layout};
    use image::{DynamicImage, Rgb, RgbImage};
    use std::cell::RefCell;

    /// The synthetic 6v6 frame (`report::blank::synth`, no member pixels).
    fn full_6v6() -> RgbImage {
        synth::frame(Layout::L1, &synth::VALUES).to_rgb8()
    }

    /// Minimal synthetic 5v5 frame: the synth board's header, colours and
    /// name strokes with five rows per team at the 5v5 pitch (six 75 px
    /// rows' height over five rows, 90 px). Stands in until Scuffed Vision's
    /// 5v5 board generator lands on main.
    // Kept instead of `synth::tab_frame(Teams::Five, w, drawn)` (#285): on
    // that generator the preflight accepts most part-drawn boards (header
    // only reads Ready(5), two rows Ready(6)), so it cannot stage the
    // reject-then-accept case this test needs. See the alpha.10 cleanup PR.
    fn full_5v5() -> RgbImage {
        let mut f = RgbImage::from_fn(2560, 1440, |x, y| {
            Rgb([(30 + x * 40 / 2560) as u8, (45 + y * 40 / 1440) as u8, 60])
        });
        let (bx, by, _, _) = crop_scoreboard_rect(2560, 1440);
        let (l, r) = Layout::L1.edges();
        let mut b = RgbImage::from_fn(synth::W, synth::H, |x, y| {
            Rgb([
                (40 + (x * 60 / synth::W) as u8),
                (55 + (y * 50 / synth::H) as u8),
                (70 + ((x + y) * 40 / (synth::W + synth::H)) as u8),
            ])
        });
        let fill = |img: &mut RgbImage, x0: u32, y0: u32, x1: u32, y1: u32, c: Rgb<u8>| {
            for y in y0..y1.min(img.height()) {
                for x in x0..x1.min(img.width()) {
                    img.put_pixel(x, y, c);
                }
            }
        };
        fill(&mut b, l, 0, r, synth::T1, Rgb([215, 219, 226]));
        let pitch = 90;
        let row_h = pitch - 2;
        for row in 0..10u32 {
            let team2 = row >= 5;
            let top = if team2 { synth::T2 } else { synth::T1 } + pitch * (row % 5);
            let bot = (top + row_h).min(synth::H);
            let (bar, tint) = if team2 {
                (synth::BAR_T2, synth::TINT_T2)
            } else {
                (synth::BAR_T1, synth::TINT_T1)
            };
            fill(&mut b, l + 104, top, r, bot, tint);
            fill(&mut b, l, top, l + 31, bot, bar);
            for j in 0..7u32 {
                let x = l + 120 + j * 14;
                fill(&mut b, x, top + 28, x + 3, top + 50, Rgb([238, 238, 238]));
                fill(&mut b, x, top + 28, x + 10, top + 31, Rgb([238, 238, 238]));
            }
        }
        image::imageops::replace(&mut f, &b, i64::from(bx), i64::from(by));
        f
    }

    /// `full` as the game shows it mid-draw: only the top `rows_drawn` rows
    /// of team 1 are on screen, the rest of the board area still shows the
    /// world behind it.
    fn half_drawn(full: &RgbImage, rows_drawn: u32, pitch: u32) -> RgbImage {
        let mut f = full.clone();
        let (bx, by, bw, bh) = crop_scoreboard_rect(full.width(), full.height());
        let cut = by + synth::T1 + pitch * rows_drawn;
        for y in cut..by + bh {
            for x in bx..bx + bw {
                f.put_pixel(
                    x,
                    y,
                    Rgb([(30 + x * 40 / 2560) as u8, (45 + y * 40 / 1440) as u8, 60]),
                );
            }
        }
        f
    }

    fn preflight(f: &RgbImage) -> ScoreboardPreflight {
        preflight_scoreboard(&crop_scoreboard(&DynamicImage::ImageRgb8(f.clone()))).0
    }

    /// Feeds `frames` to [`with_retries`] as successive screen grabs and
    /// returns the final verdict, the retry count and the waits slept.
    fn run(frames: Vec<RgbImage>) -> (ScoreboardPreflight, usize, Vec<Duration>, usize) {
        let frames = RefCell::new(frames.into_iter());
        let grabs = RefCell::new(0usize);
        let slept = RefCell::new(Vec::new());
        let (verdict, retries) = block_on(with_retries(
            &TabFlight::new(Instant::now()),
            &TAB_RETRY_WAITS,
            |p: &ScoreboardPreflight| match p {
                ScoreboardPreflight::NotAScoreboard => TabResult::NotAScoreboard,
                ScoreboardPreflight::TeamSizeUncertain => TabResult::Final,
                ScoreboardPreflight::Ready(_) => TabResult::Accepted,
            },
            |p: &ScoreboardPreflight| match p {
                ScoreboardPreflight::NotAScoreboard => 0,
                ScoreboardPreflight::TeamSizeUncertain => 1,
                ScoreboardPreflight::Ready(_) => 2,
            },
            |_| "rejected by the scoreboard check",
            |_| {
                *grabs.borrow_mut() += 1;
                let f = frames.borrow_mut().next().expect("a frame per grab");
                async move { Ok::<_, ()>(preflight(&f)) }
            },
            |d| {
                slept.borrow_mut().push(d);
                async {}
            },
        ))
        .unwrap();
        (verdict, retries, slept.into_inner(), grabs.into_inner())
    }

    fn block_on<F: Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(f)
    }

    fn check(full: RgbImage, pitch: u32, size: usize) {
        assert_eq!(
            preflight(&full),
            ScoreboardPreflight::Ready(size),
            "full board"
        );
        let half = half_drawn(&full, 2, pitch);
        assert_eq!(
            preflight(&half),
            ScoreboardPreflight::NotAScoreboard,
            "half-drawn board"
        );

        // Rejected, then the first retry sees the finished board.
        let (v, retries, slept, grabs) = run(vec![half.clone(), full.clone()]);
        assert_eq!(v, ScoreboardPreflight::Ready(size));
        assert_eq!((retries, grabs), (1, 2));
        assert_eq!(slept, vec![Duration::from_millis(300)]);

        // Still drawing at 300 ms, done by 700 ms.
        let (v, retries, slept, _) = run(vec![half.clone(), half.clone(), full.clone()]);
        assert_eq!(v, ScoreboardPreflight::Ready(size));
        assert_eq!(retries, 2);
        assert_eq!(slept.iter().sum::<Duration>(), Duration::from_millis(700));

        // Never finished: gives up after two retries.
        let (v, retries, _, grabs) = run(vec![half.clone(), half.clone(), half]);
        assert_eq!(v, ScoreboardPreflight::NotAScoreboard);
        assert_eq!((retries, grabs), (2, 3));

        // An accepted first grab never retries.
        let (_, retries, slept, grabs) = run(vec![full]);
        assert_eq!((retries, grabs), (0, 1));
        assert!(slept.is_empty());
    }

    #[test]
    fn half_drawn_6v6_board_is_rejected_then_accepted_on_retry() {
        check(full_6v6(), synth::PITCH, 6);
    }

    #[test]
    fn half_drawn_5v5_board_is_rejected_then_accepted_on_retry() {
        check(full_5v5(), 90, 5);
    }

    #[test]
    fn the_screenshot_portal_retries_a_rejected_tab_once() {
        use crate::capture::ShotRoute;
        assert_eq!(tab_retry_waits(ShotRoute::Portal), &TAB_RETRY_WAITS[..1]);
        assert_eq!(tab_retry_waits(ShotRoute::None), &TAB_RETRY_WAITS[..]);
        assert_eq!(tab_retry_waits(ShotRoute::Kwin), &TAB_RETRY_WAITS[..]);
        for (route, want) in [
            (ShotRoute::Portal, 1usize),
            (ShotRoute::None, 2),
            (ShotRoute::Kwin, 2),
        ] {
            let grabs = RefCell::new(0usize);
            let (_, retries) = block_on(with_retries(
                &TabFlight::new(Instant::now()),
                tab_retry_waits(route),
                |_: &u8| TabResult::NotAScoreboard,
                |v: &u8| *v,
                |_| "rejected by the scoreboard check",
                |_| {
                    *grabs.borrow_mut() += 1;
                    async { Ok::<_, ()>(0u8) }
                },
                |_| async {},
            ))
            .unwrap();
            assert_eq!(retries, want, "{route:?}");
            assert_eq!(*grabs.borrow(), want + 1, "grabs including the first");
        }
    }

    #[test]
    fn a_later_rejected_grab_never_replaces_a_better_result() {
        // 0 = rejected (retry), 1 = useful but not retried, 2 = accepted.
        let (best, retries) = block_on(with_retries(
            &TabFlight::new(Instant::now()),
            &TAB_RETRY_WAITS,
            |v: &u8| u8_result(*v),
            |v: &u8| *v,
            |_| "rejected by the scoreboard check",
            |i| async move { Ok::<_, ()>(if i == 0 { 0 } else { 1 }) },
            |_| async {},
        ))
        .unwrap();
        assert_eq!((best, retries), (1, 1), "stops at the useful result");

        // Even if the predicate kept retrying, rank keeps the best result.
        let seq = RefCell::new(vec![1u8, 0, 0].into_iter());
        let (best, retries) = block_on(with_retries(
            &TabFlight::new(Instant::now()),
            &TAB_RETRY_WAITS,
            |_: &u8| TabResult::NoFrameYet,
            |v: &u8| *v,
            |_| "no fresh stream frame arrived",
            |_| {
                let v = seq.borrow_mut().next().unwrap();
                async move { Ok::<_, ()>(v) }
            },
            |_| async {},
        ))
        .unwrap();
        assert_eq!(
            (best, retries),
            (1, 2),
            "a rejected grab never replaces the kept one"
        );
    }

    #[test]
    fn a_retry_log_names_the_reason_it_was_given() {
        let text = capture_thread_logs(|| {
            let _ = block_on(with_retries(
                &TabFlight::new(Instant::now()),
                &TAB_RETRY_WAITS,
                |_: &u8| TabResult::NoFrameYet,
                |v: &u8| *v,
                |_| "no fresh stream frame arrived",
                |_| async { Ok::<_, ()>(0u8) },
                |_| async {},
            ));
            let _ = block_on(with_retries(
                &TabFlight::new(Instant::now()),
                &[Duration::from_millis(1)],
                |v: &u8| u8_result(*v),
                |v: &u8| *v,
                |_| "the share is still opening",
                |i| async move { Ok::<_, ()>(if i == 0 { 0 } else { 1 }) },
                |_| async {},
            ));
        });
        assert!(text.contains("Tab capture will try again: no fresh stream frame arrived"));
        assert!(text.contains("Tab capture gave up after retries: no fresh stream frame arrived"));
        assert!(text.contains("Tab capture will try again: the share is still opening"));
        assert!(text.contains("Tab capture accepted on retry"));
        assert!(!text.contains("rejected by the scoreboard check: capturing again"));
        assert!(!text.contains("still rejected"));
    }

    /// 2026-10-21: the player's second Tab came while the first capture was
    /// still retrying and was dropped as "a capture is already in
    /// progress". A new press now stops the pending retries.
    #[test]
    fn a_new_press_before_a_retry_stops_it() {
        let flight = TabFlight::new(Instant::now());
        let grabs = RefCell::new(0usize);
        let slept = RefCell::new(0usize);
        let (best, retries) = block_on(with_retries(
            &flight,
            &TAB_RETRY_WAITS,
            |v: &u8| u8_result(*v),
            |v: &u8| *v,
            |_| "rejected by the scoreboard check",
            |_| {
                *grabs.borrow_mut() += 1;
                // The new press lands while the first grab runs.
                flight.cancel_retries();
                async { Ok::<_, ()>(0u8) }
            },
            |_| {
                *slept.borrow_mut() += 1;
                async {}
            },
        ))
        .unwrap();
        assert_eq!((best, retries), (0, 0));
        assert_eq!((grabs.into_inner(), slept.into_inner()), (1, 0));
    }

    #[test]
    fn a_new_press_during_a_retry_wait_ends_the_wait_and_takes_no_frame() {
        let flight = TabFlight::new(Instant::now());
        let grabs = RefCell::new(0usize);
        let (best, retries) = block_on(with_retries(
            &flight,
            &TAB_RETRY_WAITS,
            |v: &u8| u8_result(*v),
            |v: &u8| *v,
            |_| "rejected by the scoreboard check",
            |_| {
                *grabs.borrow_mut() += 1;
                async { Ok::<_, ()>(0u8) }
            },
            |_| {
                // The press lands mid-wait. The wait itself would never end.
                assert_eq!(flight.phase(), TabFlightPhase::Retrying);
                flight.cancel_retries();
                std::future::pending::<()>()
            },
        ))
        .unwrap();
        assert_eq!((best, retries), (0, 1));
        assert_eq!(grabs.into_inner(), 1, "no retry grab after the new press");
    }

    #[test]
    fn retries_still_run_when_no_new_press_comes() {
        let flight = TabFlight::new(Instant::now());
        let (best, retries) = block_on(with_retries(
            &flight,
            &TAB_RETRY_WAITS,
            |v: &u8| u8_result(*v),
            |v: &u8| *v,
            |_| "rejected by the scoreboard check",
            |i| async move { Ok::<_, ()>(if i == 2 { 2u8 } else { 0 }) },
            |_| async {},
        ))
        .unwrap();
        assert_eq!((best, retries), (2, 2));
        // The loop finishes the flight with its best result.
        assert_eq!(flight.phase(), TabFlightPhase::Accepted);
    }

    /// The 3 s debounce used to start at every press, so a rejected
    /// capture blocked the player's next Tab for 3 s. It now starts only
    /// at an accepted board.
    #[test]
    fn debounce_applies_only_after_an_accepted_board() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        // Nothing running, nothing accepted.
        assert_eq!(tab_press_action(at(0), None, None), TabPressAction::Start);
        // A rejected capture finished at once: the next press runs.
        let rejected = TabFlight::new(at(0));
        rejected.finish(TabResult::NotAScoreboard);
        let mut last_accepted = None;
        assert_eq!(
            next_press_after_report(&rejected, None, &mut last_accepted),
            None
        );
        assert_eq!(last_accepted, None);
        assert_eq!(
            tab_press_action(at(1_000), last_accepted, None),
            TabPressAction::Start
        );
        // An accepted board debounces presses within 3 s of its press.
        let accepted = TabFlight::new(at(1_000));
        accepted.finish(TabResult::Accepted);
        assert_eq!(
            next_press_after_report(&accepted, None, &mut last_accepted),
            None
        );
        assert_eq!(last_accepted, Some(at(1_000)));
        assert_eq!(
            tab_press_action(at(3_999), last_accepted, None),
            TabPressAction::Debounced
        );
        assert_eq!(
            tab_press_action(at(4_000), last_accepted, None),
            TabPressAction::Start
        );
    }

    #[test]
    fn a_press_during_a_capture_queues_unless_a_board_was_accepted() {
        let t0 = Instant::now();
        for phase in [
            TabFlightPhase::Grabbing,
            TabFlightPhase::Retrying,
            TabFlightPhase::Rejected,
        ] {
            assert_eq!(
                tab_press_action(t0, None, Some(phase)),
                TabPressAction::Queue,
                "{phase:?}"
            );
        }
        assert_eq!(
            tab_press_action(t0, None, Some(TabFlightPhase::Accepted)),
            TabPressAction::Ignore
        );

        // The queued press runs when a capture without a board reports.
        let queued = t0 + Duration::from_millis(1_500);
        let flight = TabFlight::new(t0);
        flight.set_phase(TabFlightPhase::Retrying);
        flight.cancel_retries();
        flight.finish(TabResult::NotAScoreboard);
        let mut last_accepted = None;
        assert_eq!(
            next_press_after_report(&flight, Some(queued), &mut last_accepted),
            Some(queued)
        );
        assert_eq!(last_accepted, None);

        // Behind an accepted board it would read the same board: dropped.
        let flight = TabFlight::new(t0);
        flight.finish(TabResult::Accepted);
        assert_eq!(
            next_press_after_report(&flight, Some(queued), &mut last_accepted),
            None
        );
        assert_eq!(last_accepted, Some(t0));
    }

    /// A frame after the key went up shows the game, not the board: once
    /// Tab is released no retry starts, and a running retry wait ends.
    #[test]
    fn no_retry_after_tab_is_released() {
        let press = Instant::now();
        let flight = TabFlight::new(press);
        let grabs = RefCell::new(0usize);
        let (_, retries) = block_on(with_retries(
            &flight,
            &TAB_RETRY_WAITS,
            |v: &u8| u8_result(*v),
            |v: &u8| *v,
            |_| "rejected by the scoreboard check",
            |_| {
                *grabs.borrow_mut() += 1;
                flight.note_release(press + Duration::from_millis(200));
                async { Ok::<_, ()>(0u8) }
            },
            |_| async {},
        ))
        .unwrap();
        assert_eq!((retries, grabs.into_inner()), (0, 1));
        assert_eq!(flight.retry_stop(), Some(RetryStop::Released));

        // Released during the first retry wait: the wait ends, no grab.
        let flight = TabFlight::new(press);
        let grabs = RefCell::new(0usize);
        let (_, retries) = block_on(with_retries(
            &flight,
            &TAB_RETRY_WAITS,
            |v: &u8| u8_result(*v),
            |v: &u8| *v,
            |_| "rejected by the scoreboard check",
            |_| {
                *grabs.borrow_mut() += 1;
                async { Ok::<_, ()>(0u8) }
            },
            |_| {
                flight.note_release(press + Duration::from_millis(500));
                std::future::pending::<()>()
            },
        ))
        .unwrap();
        assert_eq!((retries, grabs.into_inner()), (1, 1));
    }

    #[test]
    fn a_release_from_an_earlier_tap_does_not_stop_this_capture() {
        let press = Instant::now() + Duration::from_secs(5);
        let flight = TabFlight::new(press);
        flight.note_release(press - Duration::from_millis(100));
        assert_eq!(flight.retry_stop(), None);
        assert_eq!(flight.released_at(), None);
        // While held, retries still run up to the cap.
        let (best, retries) = block_on(with_retries(
            &flight,
            &TAB_RETRY_WAITS,
            |v: &u8| u8_result(*v),
            |v: &u8| *v,
            |_| "rejected by the scoreboard check",
            |_| async { Ok::<_, ()>(0u8) },
            |_| async {},
        ))
        .unwrap();
        assert_eq!((best, retries), (0, TAB_RETRY_WAITS.len()));
    }

    #[test]
    fn a_short_tap_without_a_board_gets_the_hold_hint() {
        let press = Instant::now();
        let at = |ms: u64| Some(press + Duration::from_millis(ms));
        assert!(short_tap_hint(TabResult::NotAScoreboard, press, at(400)));
        assert!(
            !short_tap_hint(TabResult::NotAScoreboard, press, at(1_000)),
            "held a second"
        );
        assert!(
            !short_tap_hint(TabResult::Accepted, press, at(400)),
            "board accepted"
        );
        assert!(
            !short_tap_hint(TabResult::NotAScoreboard, press, None),
            "still held"
        );
        // Only a frame with no scoreboard: a missing stream frame or a
        // share that is opening or declined is not the player's tap.
        assert!(!short_tap_hint(TabResult::NoFrameYet, press, at(400)));
        assert!(!short_tap_hint(TabResult::Final, press, at(400)));
    }

    #[test]
    fn the_hold_hint_warns_at_most_every_ten_minutes() {
        let t0 = Instant::now();
        assert!(hold_hint_is_warn(None, t0));
        assert!(!hold_hint_is_warn(Some(t0), t0 + Duration::from_secs(599)));
        assert!(hold_hint_is_warn(Some(t0), t0 + TAB_HOLD_HINT_WARN_EVERY));
    }

    /// Key events from a list; then nothing more ever arrives.
    pub(crate) struct FakeKeys(pub std::collections::VecDeque<crate::detect::TabKey>);

    impl TabKeySource for FakeKeys {
        fn next_key(
            &mut self,
        ) -> impl Future<Output = Result<crate::detect::TabKey, Box<dyn std::error::Error>>>
        {
            let key = self.0.pop_front();
            async move {
                match key {
                    Some(key) => Ok(key),
                    None => std::future::pending().await,
                }
            }
        }
    }

    fn key(down: bool, at: Instant) -> crate::detect::TabKey {
        crate::detect::TabKey { down, at }
    }

    /// The loop's Tab wiring: a second press during a capture is queued
    /// and cancels its retries, the release of that second press reaches
    /// the capture started for it, and an accepted board debounces.
    #[test]
    fn tab_keys_queue_a_second_press_and_hand_it_its_release() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut tabs = TabKeys::default();
        let mut last_accepted = None;

        assert_eq!(tabs.on_press(at(0), last_accepted, false), Some(at(0)));
        let first = TabFlight::new(at(0));
        tabs.started(Arc::clone(&first));

        // Second press while the first capture has no board.
        assert_eq!(tabs.on_press(at(900), last_accepted, false), None);
        assert!(first.retries_cancelled());
        tabs.on_release(at(1_100));

        first.finish(TabResult::NotAScoreboard);
        assert_eq!(tabs.on_report(&mut last_accepted), Some(at(900)));
        assert_eq!(last_accepted, None, "a rejected capture sets no debounce");
        let second = TabFlight::new(at(900));
        tabs.started(Arc::clone(&second));
        assert_eq!(
            second.released_at(),
            Some(at(1_100)),
            "the queued press got its release"
        );

        second.finish(TabResult::Accepted);
        assert_eq!(tabs.on_report(&mut last_accepted), None);
        assert_eq!(last_accepted, Some(at(900)));
        assert_eq!(tabs.on_press(at(2_000), last_accepted, false), None);
        assert_eq!(
            tabs.on_press(at(3_900), last_accepted, false),
            Some(at(3_900))
        );
        // No report pending: nothing to start.
        assert_eq!(tabs.on_report(&mut last_accepted), None);
    }

    #[test]
    fn a_capture_that_died_without_a_report_does_not_hold_presses() {
        let t0 = Instant::now();
        let mut tabs = TabKeys::default();
        tabs.started(TabFlight::new(t0));
        let later = t0 + Duration::from_millis(500);
        assert_eq!(tabs.on_press(later, None, false), None, "queued");
        assert_eq!(tabs.on_press(later, None, true), Some(later));
    }

    #[test]
    fn an_older_release_does_not_reach_a_new_capture() {
        let t0 = Instant::now() + Duration::from_secs(5);
        let mut tabs = TabKeys::default();
        tabs.on_release(t0 - Duration::from_millis(50));
        assert_eq!(tabs.on_press(t0, None, false), Some(t0));
        let flight = TabFlight::new(t0);
        tabs.started(Arc::clone(&flight));
        assert_eq!(flight.released_at(), None);
    }

    /// A poll raced against the keyboard must be dropped for a press only:
    /// a release on the way is recorded and the wait goes on.
    #[test]
    fn next_press_records_releases_and_returns_only_a_press() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut tabs = TabKeys::default();
        let flight = TabFlight::new(at(0));
        tabs.started(Arc::clone(&flight));
        let mut keys = FakeKeys(
            [key(false, at(300)), key(true, at(800))]
                .into_iter()
                .collect(),
        );
        let got = block_on(next_press(&mut keys, &mut tabs)).unwrap();
        assert_eq!(got, key(true, at(800)));
        assert_eq!(flight.released_at(), Some(at(300)));
        assert_eq!(tabs.last_release, Some(at(300)));
    }

    thread_local! {
        static THREAD_LOG: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    }

    struct ThreadLog;

    impl std::io::Write for ThreadLog {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            THREAD_LOG.with(|log| log.borrow_mut().extend_from_slice(buf));
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// The log lines `f` writes on this thread. A process-wide subscriber
    /// writes each thread's lines to that thread's buffer. A scoped
    /// `with_default` subscriber raced other test threads over tracing's
    /// cached callsite interest and lost lines when tests ran in parallel
    /// (10 of 12 runs on 2fa4a28); a global one keeps every INFO callsite
    /// enabled.
    fn capture_thread_logs(f: impl FnOnce()) -> String {
        static INSTALL: std::sync::Once = std::sync::Once::new();
        INSTALL.call_once(|| {
            let sub = tracing_subscriber::fmt()
                .with_writer(|| ThreadLog)
                .with_ansi(false)
                .with_max_level(tracing::Level::INFO)
                .finish();
            let _ = tracing::subscriber::set_global_default(sub);
        });
        THREAD_LOG.with(|log| log.borrow_mut().clear());
        f();
        THREAD_LOG.with(|log| String::from_utf8(std::mem::take(&mut *log.borrow_mut())).unwrap())
    }
}
