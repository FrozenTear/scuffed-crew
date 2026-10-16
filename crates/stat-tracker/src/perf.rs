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
    /// Thread CPU time spent in screen grabs during the window.
    pub capture_cpu_ms: u64,
    /// Thread CPU time spent in scoreboard OCR during the window.
    pub tab_cpu_ms: u64,
    /// X11 grab details, when X11 grabbed in the window.
    pub x11: Option<X11Stats>,
}

/// X11 GetImage grabs in one window. X11 capture never uses shm: every
/// grab is a full GetImage readback of the selected output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct X11Stats {
    pub width: u16,
    pub height: u16,
    /// `full` when the output covers the whole X screen, else `output`.
    pub scope: &'static str,
    pub grabs: StageStats,
}

/// Samples gathered since `opened`.
#[derive(Debug)]
pub struct Window {
    opened: Option<Instant>,
    backend: &'static str,
    samples: [Vec<u64>; STAGES],
    cpu_us: [u64; STAGES],
    x11_size: Option<(u16, u16, &'static str)>,
    x11_grabs: Vec<u64>,
}

impl Window {
    pub const fn new() -> Self {
        Self {
            opened: None,
            backend: "unknown",
            samples: [Vec::new(), Vec::new(), Vec::new()],
            cpu_us: [0; STAGES],
            x11_size: None,
            x11_grabs: Vec::new(),
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

    /// Thread CPU time used by one sample of `stage`.
    pub fn add_cpu(&mut self, stage: Stage, cpu: Duration) {
        let us = u64::try_from(cpu.as_micros()).unwrap_or(u64::MAX);
        self.cpu_us[stage.index()] = self.cpu_us[stage.index()].saturating_add(us);
    }

    /// One X11 GetImage grab of `width` x `height`.
    pub fn add_x11_grab(&mut self, width: u16, height: u16, full: bool, took: Duration) {
        self.x11_size = Some((width, height, if full { "full" } else { "output" }));
        self.x11_grabs
            .push(u64::try_from(took.as_millis()).unwrap_or(u64::MAX));
    }

    fn summarize(&mut self, elapsed: Duration) -> Option<Summary> {
        if self.samples.iter().all(Vec::is_empty) {
            return None;
        }
        let mut take = |stage: Stage| {
            let mut samples = std::mem::take(&mut self.samples[stage.index()]);
            StageStats::from_samples(&mut samples)
        };
        let capture = take(Stage::Capture);
        let poll_analyze = take(Stage::PollAnalyze);
        let tab_analyze = take(Stage::TabAnalyze);
        let cpu = std::mem::take(&mut self.cpu_us);
        let mut grabs = std::mem::take(&mut self.x11_grabs);
        let x11 =
            (!grabs.is_empty())
                .then_some(self.x11_size)
                .flatten()
                .map(|(width, height, scope)| X11Stats {
                    width,
                    height,
                    scope,
                    grabs: StageStats::from_samples(&mut grabs),
                });
        Some(Summary {
            window_secs: elapsed.as_secs(),
            backend: self.backend,
            capture,
            poll_analyze,
            tab_analyze,
            capture_cpu_ms: cpu[Stage::Capture.index()] / 1000,
            tab_cpu_ms: cpu[Stage::TabAnalyze.index()] / 1000,
            x11,
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
        crate::perf_log::write_summary(&summary);
    }
}

/// Add thread CPU time for one sample of `stage`.
pub fn record_cpu(stage: Stage, cpu: Duration) {
    if let Ok(mut window) = WINDOW_STATE.lock() {
        window.add_cpu(stage, cpu);
    }
}

/// Note one X11 GetImage grab (no shm).
pub fn record_x11_grab(width: u16, height: u16, full: bool, took: Duration) {
    if let Ok(mut window) = WINDOW_STATE.lock() {
        window.add_x11_grab(width, height, full, took);
    }
}

/// CPU time of the calling thread so far.
pub fn thread_cpu() -> Duration {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `ts` is a valid out pointer for clock_gettime.
    if unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts) } != 0 {
        return Duration::ZERO;
    }
    Duration::new(
        ts.tv_sec.max(0) as u64,
        ts.tv_nsec.clamp(0, 999_999_999) as u32,
    )
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
        // Read when the line is written: the limit OCR ran with and how
        // many OS threads the daemon holds. Under the systemd unit this line
        // is in the journal. A problem report only carries it when the app
        // started the daemon itself, because reports read `daemon.log`.
        omp_thread_limit = %crate::omp::env_limit_label(),
        os_threads = %crate::omp::os_threads_label(),
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

    #[test]
    fn x11_grabs_and_cpu_land_in_the_summary() {
        let t0 = Instant::now();
        let mut w = Window::new();
        w.set_backend("x11");
        w.record(Stage::Capture, ms(40), t0);
        w.add_x11_grab(2560, 1440, false, ms(33));
        w.add_x11_grab(2560, 1440, false, ms(35));
        w.add_cpu(Stage::Capture, ms(12));
        w.add_cpu(Stage::TabAnalyze, ms(800));
        let s = w.record(Stage::Capture, ms(1), t0 + WINDOW).unwrap();
        let x = s.x11.expect("x11 stats");
        assert_eq!((x.width, x.height, x.scope), (2560, 1440, "output"));
        assert_eq!(x.grabs.count, 2);
        assert_eq!(x.grabs.max_ms, 35);
        assert_eq!((s.capture_cpu_ms, s.tab_cpu_ms), (12, 800));
        let next = w.record(Stage::Capture, ms(1), t0 + WINDOW * 2).unwrap();
        assert!(next.x11.is_none());
        assert_eq!(next.capture_cpu_ms, 0);
    }
}
