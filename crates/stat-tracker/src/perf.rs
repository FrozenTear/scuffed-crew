//! Capture and analysis timing for the daemon log.
//!
//! Each screen grab and each frame analysis records its wall time here. Once
//! a minute has passed since the window opened, the next record logs one info
//! line for that window and starts a new one. Nothing is logged while nothing
//! is recorded, so an idle desktop stays quiet.
//!
//! The line is what a performance report reads: it says whether the screen
//! grab (compositor or X server readback) or the OCR is the slow part, and how
//! often each one ran.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long one window covers before its summary is logged.
pub const WINDOW: Duration = Duration::from_secs(60);

/// What was timed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// One full-screen grab, from either the poll or a Tab capture.
    Capture,
    /// Poll-tick detectors on one grabbed frame.
    PollAnalyze,
    /// Scoreboard OCR on one Tab frame.
    TabAnalyze,
}

const STAGES: usize = 3;

impl Stage {
    fn index(self) -> usize {
        match self {
            Stage::Capture => 0,
            Stage::PollAnalyze => 1,
            Stage::TabAnalyze => 2,
        }
    }
}

/// Timings for one stage inside one window.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StageStats {
    pub count: u32,
    pub p50_ms: u64,
    pub max_ms: u64,
    pub total_ms: u64,
}

impl StageStats {
    fn from_samples(samples: &mut [u64]) -> Self {
        if samples.is_empty() {
            return Self::default();
        }
        samples.sort_unstable();
        Self {
            count: samples.len() as u32,
            p50_ms: samples[samples.len() / 2],
            max_ms: samples[samples.len() - 1],
            total_ms: samples.iter().sum(),
        }
    }
}

/// One closed window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    pub window_secs: u64,
    pub backend: &'static str,
    pub capture: StageStats,
    pub poll_analyze: StageStats,
    pub tab_analyze: StageStats,
}

/// Samples gathered since `opened`.
#[derive(Debug)]
pub struct Window {
    opened: Option<Instant>,
    backend: &'static str,
    samples: [Vec<u64>; STAGES],
}

impl Window {
    pub const fn new() -> Self {
        Self {
            opened: None,
            backend: "unknown",
            samples: [Vec::new(), Vec::new(), Vec::new()],
        }
    }

    /// Add one sample at `now`. Returns the previous window's summary when
    /// that window is at least [`WINDOW`] old; the sample then opens the next.
    pub fn record(&mut self, stage: Stage, took: Duration, now: Instant) -> Option<Summary> {
        let closed = match self.opened {
            Some(opened) if now.saturating_duration_since(opened) >= WINDOW => {
                let summary = self.summarize(now.saturating_duration_since(opened));
                self.opened = Some(now);
                summary
            }
            Some(_) => None,
            None => {
                self.opened = Some(now);
                None
            }
        };
        let ms = u64::try_from(took.as_millis()).unwrap_or(u64::MAX);
        self.samples[stage.index()].push(ms);
        closed
    }

    pub fn set_backend(&mut self, backend: &'static str) {
        self.backend = backend;
    }

    fn summarize(&mut self, elapsed: Duration) -> Option<Summary> {
        if self.samples.iter().all(Vec::is_empty) {
            return None;
        }
        let mut take = |stage: Stage| {
            let mut samples = std::mem::take(&mut self.samples[stage.index()]);
            StageStats::from_samples(&mut samples)
        };
        Some(Summary {
            window_secs: elapsed.as_secs(),
            backend: self.backend,
            capture: take(Stage::Capture),
            poll_analyze: take(Stage::PollAnalyze),
            tab_analyze: take(Stage::TabAnalyze),
        })
    }
}

impl Default for Window {
    fn default() -> Self {
        Self::new()
    }
}

static WINDOW_STATE: Mutex<Window> = Mutex::new(Window::new());

/// Record one sample in the daemon-wide window and log a summary when one
/// closes.
pub fn record(stage: Stage, took: Duration) {
    let closed = match WINDOW_STATE.lock() {
        Ok(mut window) => window.record(stage, took, Instant::now()),
        Err(_) => return,
    };
    if let Some(summary) = closed {
        log_summary(&summary);
    }
}

/// Name the capture backend on the next summary.
pub fn set_backend(backend: &'static str) {
    if let Ok(mut window) = WINDOW_STATE.lock() {
        window.set_backend(backend);
    }
}

fn log_summary(s: &Summary) {
    tracing::info!(
        window_secs = s.window_secs,
        backend = s.backend,
        captures = s.capture.count,
        capture_p50_ms = s.capture.p50_ms,
        capture_max_ms = s.capture.max_ms,
        capture_total_ms = s.capture.total_ms,
        polls = s.poll_analyze.count,
        poll_p50_ms = s.poll_analyze.p50_ms,
        poll_max_ms = s.poll_analyze.max_ms,
        poll_total_ms = s.poll_analyze.total_ms,
        tabs = s.tab_analyze.count,
        tab_p50_ms = s.tab_analyze.p50_ms,
        tab_max_ms = s.tab_analyze.max_ms,
        tab_total_ms = s.tab_analyze.total_ms,
        "capture timing"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn first_window_closes_on_the_record_after_a_minute() {
        let t0 = Instant::now();
        let mut w = Window::new();
        w.set_backend("x11");
        assert_eq!(w.record(Stage::Capture, ms(40), t0), None);
        assert_eq!(w.record(Stage::Capture, ms(60), t0 + ms(4_000)), None);
        assert_eq!(w.record(Stage::PollAnalyze, ms(12), t0 + ms(4_100)), None);
        assert_eq!(w.record(Stage::TabAnalyze, ms(900), t0 + ms(30_000)), None);

        let s = w
            .record(Stage::Capture, ms(5), t0 + WINDOW + ms(1))
            .expect("window closes");
        assert_eq!(s.window_secs, 60);
        assert_eq!(s.backend, "x11");
        assert_eq!(
            s.capture,
            StageStats {
                count: 2,
                p50_ms: 60,
                max_ms: 60,
                total_ms: 100
            }
        );
        assert_eq!(s.poll_analyze.count, 1);
        assert_eq!(s.tab_analyze.max_ms, 900);
    }

    #[test]
    fn the_closing_sample_belongs_to_the_next_window() {
        let t0 = Instant::now();
        let mut w = Window::new();
        w.record(Stage::Capture, ms(10), t0);
        w.record(Stage::Capture, ms(77), t0 + WINDOW)
            .expect("closes");
        let next = w
            .record(Stage::Capture, ms(1), t0 + WINDOW * 2)
            .expect("second window closes");
        assert_eq!(next.capture.count, 1);
        assert_eq!(next.capture.max_ms, 77);
    }

    #[test]
    fn p50_is_the_upper_middle_sample() {
        let mut samples = vec![30, 10, 20, 40];
        let s = StageStats::from_samples(&mut samples);
        assert_eq!(s.p50_ms, 30);
        assert_eq!(s.max_ms, 40);
        assert_eq!(s.count, 4);
    }

    #[test]
    fn an_empty_stage_reports_zeroes() {
        let t0 = Instant::now();
        let mut w = Window::new();
        w.record(Stage::TabAnalyze, ms(500), t0);
        let s = w.record(Stage::TabAnalyze, ms(1), t0 + WINDOW).unwrap();
        assert_eq!(s.capture, StageStats::default());
        assert_eq!(s.poll_analyze, StageStats::default());
        assert_eq!(s.tab_analyze.count, 1);
    }
}
