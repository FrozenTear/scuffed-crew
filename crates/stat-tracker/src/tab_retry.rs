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

/// After an accepted board, Tab presses this soon after its press are
/// ignored. A rejected capture sets no debounce.
pub const TAB_DEBOUNCE_AFTER_ACCEPT: Duration = Duration::from_secs(3);

/// A capture with no accepted board whose Tab was released sooner than this
/// after the press logs the hold-Tab hint.
pub const TAB_HOLD_HINT_BELOW: Duration = Duration::from_secs(1);

/// The hint for a tap that ended before the board could be captured.
pub const TAB_HOLD_HINT: &str =
    "Tab was released before the scoreboard could be captured: hold Tab for about a second";

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

    /// Record how the capture ended: `accepted` when a frame passed the
    /// scoreboard check. A short tap with no accepted frame logs the
    /// hold-Tab hint.
    pub fn finish(&self, accepted: bool) {
        if short_tap_hint(accepted, self.press, self.released_at()) {
            tracing::warn!("{TAB_HOLD_HINT}");
        }
        self.set_phase(if accepted {
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

/// Whether a finished capture should log [`TAB_HOLD_HINT`]: no frame passed
/// the scoreboard check, and the key was released under
/// [`TAB_HOLD_HINT_BELOW`] after the press.
pub fn short_tap_hint(accepted: bool, press: Instant, released_at: Option<Instant>) -> bool {
    !accepted
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
/// `retry` says the best result so far should be tried again. `why` names
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
/// Tab is held, at most `waits.len()` of them.
pub async fn with_retries<T, E, A, AF, S, SF>(
    flight: &TabFlight,
    waits: &[Duration],
    retry: impl Fn(&T) -> bool,
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
    Ok((best, retries))
}

#[cfg(test)]
mod tests {
    use super::*;
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
            |p: &ScoreboardPreflight| *p == ScoreboardPreflight::NotAScoreboard,
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
    fn a_later_rejected_grab_never_replaces_a_better_result() {
        // 0 = rejected (retry), 1 = useful but not retried, 2 = accepted.
        let (best, retries) = block_on(with_retries(
            &TabFlight::new(Instant::now()),
            &TAB_RETRY_WAITS,
            |v: &u8| *v == 0,
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
            |_: &u8| true,
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
        let buf = LogWriter::default();
        let sub = tracing_subscriber::fmt()
            .with_writer(buf.clone())
            .with_ansi(false)
            .with_max_level(tracing::Level::INFO)
            .finish();
        tracing::subscriber::with_default(sub, || {
            let _ = block_on(with_retries(
                &TabFlight::new(Instant::now()),
                &TAB_RETRY_WAITS,
                |_: &u8| true,
                |v: &u8| *v,
                |_| "no fresh stream frame arrived",
                |_| async { Ok::<_, ()>(0u8) },
                |_| async {},
            ));
            let _ = block_on(with_retries(
                &TabFlight::new(Instant::now()),
                &[Duration::from_millis(1)],
                |v: &u8| *v == 0,
                |v: &u8| *v,
                |_| "the share is still opening",
                |i| async move { Ok::<_, ()>(if i == 0 { 0 } else { 1 }) },
                |_| async {},
            ));
        });
        let text = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
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
            |v: &u8| *v == 0,
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
            |v: &u8| *v == 0,
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
            |v: &u8| *v == 0,
            |v: &u8| *v,
            |_| "rejected by the scoreboard check",
            |i| async move { Ok::<_, ()>(if i == 2 { 2u8 } else { 0 }) },
            |_| async {},
        ))
        .unwrap();
        assert_eq!((best, retries), (2, 2));
        assert_eq!(flight.phase(), TabFlightPhase::Retrying);
        flight.finish(true);
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
        rejected.finish(false);
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
        accepted.finish(true);
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
        flight.finish(false);
        let mut last_accepted = None;
        assert_eq!(
            next_press_after_report(&flight, Some(queued), &mut last_accepted),
            Some(queued)
        );
        assert_eq!(last_accepted, None);

        // Behind an accepted board it would read the same board: dropped.
        let flight = TabFlight::new(t0);
        flight.finish(true);
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
            |v: &u8| *v == 0,
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
            |v: &u8| *v == 0,
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
            |v: &u8| *v == 0,
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
        assert!(short_tap_hint(false, press, at(400)));
        assert!(!short_tap_hint(false, press, at(1_000)), "held a second");
        assert!(!short_tap_hint(true, press, at(400)), "board accepted");
        assert!(!short_tap_hint(false, press, None), "still held");
    }

    #[derive(Clone, Default)]
    struct LogWriter(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for LogWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogWriter {
        type Writer = LogWriter;
        fn make_writer(&'a self) -> LogWriter {
            self.clone()
        }
    }
}
