//! Capture and analysis timing for the daemon log.
//!
//! Each screen grab and each frame analysis records its wall time here. Once
//! a minute has passed since the window opened, the next record logs one info
//! line for that window and starts a new one. Nothing is logged while nothing
//! is recorded, so an idle desktop stays quiet.
//!
//! The line is what a performance report reads: it says whether the screen
//! grab (compositor or X server readback) or the OCR is the slow part, and how
//! often each one ran. A Tab also records cheap sub-stage times. The
//! capture-to-image step after a grab, and each debug-save job, record there
//! too. Those land on a second line (`tab_stages`) for the same window, as
//! p50 milliseconds.

use std::cell::Cell;
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

/// One slice of a Tab frame analysis, recorded beside [`Stage::TabAnalyze`].
///
/// Each analysis slice is its wall time minus RGB copies counted in
/// [`TabStage::ToRgb`], so those p50s add instead of counting the same copy
/// twice. [`TabStage::GrabConvert`] is the capture-to-image step after a
/// grab, and [`TabStage::Save`] is one debug-save job. Those two are not
/// part of the analysis laps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TabStage {
    ToRgb = 0,
    Detect = 1,
    Portraits = 2,
    Digits = 3,
    Map = 4,
    Outcome = 5,
    Shadow = 6,
    GrabConvert = 7,
    Save = 8,
}

pub const TAB_STAGES: usize = 9;

impl TabStage {
    fn index(self) -> usize {
        self as usize
    }
}

/// p50 milliseconds for each [`TabStage`] in one window. A stage that did
/// not run has count 0 and p50 0.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TabStageSummary {
    pub to_rgb: StageStats,
    pub detect: StageStats,
    pub portraits: StageStats,
    pub digits: StageStats,
    pub map: StageStats,
    pub outcome: StageStats,
    pub shadow: StageStats,
    /// Capture-to-image conversion after a grab (poll or Tab).
    pub grab_convert: StageStats,
    /// Debug-save worker, one sample per job that ran.
    pub save: StageStats,
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
    /// Sub-stage p50s for Tabs whose [`Stage::TabAnalyze`] sample is in this
    /// window. Empty (all zeroes) when the window had no Tab analysis.
    pub tab_stages: TabStageSummary,
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
    tab_samples: [Vec<u64>; TAB_STAGES],
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
            tab_samples: [
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ],
            x11_size: None,
            x11_grabs: Vec::new(),
        }
    }

    /// Add one sample at `now`. Returns the previous window's summary when
    /// that window is at least [`WINDOW`] old; the sample then opens the next.
    pub fn record(&mut self, stage: Stage, took: Duration, now: Instant) -> Option<Summary> {
        self.record_with(stage, took, now, None)
    }

    /// [`Self::record`] plus the Tab sub-stage milliseconds that belong with
    /// this sample. `None` entries are stages that did not run. Passed only
    /// for [`Stage::TabAnalyze`], after the window decision, so a Tab that
    /// closes the window keeps its sub-stages with its own total.
    pub fn record_with(
        &mut self,
        stage: Stage,
        took: Duration,
        now: Instant,
        tabs: Option<[Option<u64>; TAB_STAGES]>,
    ) -> Option<Summary> {
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
        if let Some(tabs) = tabs {
            for (i, sample) in tabs.into_iter().enumerate() {
                if let Some(ms) = sample {
                    self.tab_samples[i].push(ms);
                }
            }
        }
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

    /// One [`TabStage`] sample that is not part of a [`Stage::TabAnalyze`]
    /// stash: the post-grab conversion, or one debug-save job. It stays in
    /// the open window until the next record closes it.
    pub fn add_tab_stage(&mut self, stage: TabStage, took: Duration) {
        let ms = u64::try_from(took.as_millis()).unwrap_or(u64::MAX);
        self.tab_samples[stage.index()].push(ms);
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
        let tab_stages = self.take_tab_stages();
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
            tab_stages,
        })
    }

    fn take_tab_stages(&mut self) -> TabStageSummary {
        let mut take = |stage: TabStage| {
            let mut samples = std::mem::take(&mut self.tab_samples[stage.index()]);
            StageStats::from_samples(&mut samples)
        };
        TabStageSummary {
            to_rgb: take(TabStage::ToRgb),
            detect: take(TabStage::Detect),
            portraits: take(TabStage::Portraits),
            digits: take(TabStage::Digits),
            map: take(TabStage::Map),
            outcome: take(TabStage::Outcome),
            shadow: take(TabStage::Shadow),
            grab_convert: take(TabStage::GrabConvert),
            save: take(TabStage::Save),
        }
    }
}

impl Default for Window {
    fn default() -> Self {
        Self::new()
    }
}

static WINDOW_STATE: Mutex<Window> = Mutex::new(Window::new());

std::thread_local! {
    static STASHED_TABS: Cell<Option<[Option<u64>; TAB_STAGES]>> = const { Cell::new(None) };
    static RGB_ON: Cell<bool> = const { Cell::new(false) };
    static RGB_US: Cell<u128> = const { Cell::new(0) };
}

/// Hold this for one Tab analysis. [`to_rgb8`] then adds its copy time here
/// and nowhere else, so a poll tick is unchanged.
pub struct RgbTiming;

impl RgbTiming {
    pub fn enter() -> Self {
        RGB_ON.with(|on| on.set(true));
        RGB_US.with(|us| us.set(0));
        Self
    }

    /// Microseconds of RGB copies since [`Self::enter`].
    pub fn micros(&self) -> u128 {
        RGB_US.with(|us| us.get())
    }
}

impl Drop for RgbTiming {
    fn drop(&mut self) {
        RGB_ON.with(|on| on.set(false));
        RGB_US.with(|us| us.set(0));
    }
}

/// `img.to_rgb8()`, and while an [`RgbTiming`] is active on this thread, add
/// the copy to that scope. One thread-local load when timing is off.
pub fn to_rgb8(img: &image::DynamicImage) -> image::RgbImage {
    if !RGB_ON.with(|on| on.get()) {
        return img.to_rgb8();
    }
    let started = Instant::now();
    let rgb = img.to_rgb8();
    let us = started.elapsed().as_micros();
    RGB_US.with(|total| total.set(total.get().saturating_add(us)));
    rgb
}

/// Sub-stage times for the Tab analysis that is about to call [`record`]
/// with [`Stage::TabAnalyze`]. The next such record takes them.
pub fn stash_tab_stages(samples: [Option<Duration>; TAB_STAGES]) {
    let ms =
        samples.map(|took| took.map(|took| u64::try_from(took.as_millis()).unwrap_or(u64::MAX)));
    STASHED_TABS.with(|stashed| stashed.set(Some(ms)));
}

fn take_stashed_tabs() -> Option<[Option<u64>; TAB_STAGES]> {
    STASHED_TABS.with(|stashed| stashed.take())
}

/// Record one sample in the daemon-wide window and log a summary when one
/// closes. A [`Stage::TabAnalyze`] sample also stores the stages stashed
/// from that same analysis.
pub fn record(stage: Stage, took: Duration) {
    let tabs = (stage == Stage::TabAnalyze)
        .then(take_stashed_tabs)
        .flatten();
    let closed = match WINDOW_STATE.lock() {
        Ok(mut window) => window.record_with(stage, took, Instant::now(), tabs),
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

/// Capture-to-image conversion after a grab. Cheap: one integer into the
/// open window. X11 times `image_to_rgba8`. The portal times the PNG decode.
/// libwayshot returns an image from the screenshot call, so it has no second
/// step to time.
pub fn record_grab_convert(took: Duration) {
    if let Ok(mut window) = WINDOW_STATE.lock() {
        window.add_tab_stage(TabStage::GrabConvert, took);
    }
}

/// One debug-save job finished (ring, report kit, or map crop).
pub fn record_debug_save(took: Duration) {
    if let Ok(mut window) = WINDOW_STATE.lock() {
        window.add_tab_stage(TabStage::Save, took);
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
    let t = &s.tab_stages;
    let ran = t.to_rgb.count
        + t.detect.count
        + t.portraits.count
        + t.digits.count
        + t.map.count
        + t.outcome.count
        + t.shadow.count
        + t.grab_convert.count
        + t.save.count;
    if ran > 0 {
        tracing::info!(
            to_rgb_p50_ms = t.to_rgb.p50_ms,
            detect_p50_ms = t.detect.p50_ms,
            portraits_p50_ms = t.portraits.p50_ms,
            digits_p50_ms = t.digits.p50_ms,
            map_p50_ms = t.map.p50_ms,
            outcome_p50_ms = t.outcome.p50_ms,
            shadow_p50_ms = t.shadow.p50_ms,
            grab_convert_p50_ms = t.grab_convert.p50_ms,
            save_p50_ms = t.save.p50_ms,
            "tab stages"
        );
    }
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

    #[test]
    fn tab_substages_stay_with_the_tab_that_closed_the_window() {
        let t0 = Instant::now();
        let mut w = Window::new();
        // Order matches TabStage. Grab conversion and save are not stashed
        // with the analysis laps.
        let tabs = Some([
            Some(4),
            Some(5),
            Some(6),
            Some(80),
            Some(7),
            Some(9),
            Some(1),
            None,
            None,
        ]);
        assert!(
            w.record_with(Stage::TabAnalyze, ms(230), t0, tabs)
                .is_none()
        );
        let s = w
            .record_with(Stage::TabAnalyze, ms(10), t0 + WINDOW, None)
            .expect("window closes");
        assert_eq!(s.tab_analyze.p50_ms, 230);
        assert_eq!(s.tab_stages.to_rgb.p50_ms, 4);
        assert_eq!(s.tab_stages.detect.p50_ms, 5);
        assert_eq!(s.tab_stages.portraits.p50_ms, 6);
        assert_eq!(s.tab_stages.digits.p50_ms, 80);
        assert_eq!(s.tab_stages.map.p50_ms, 7);
        assert_eq!(s.tab_stages.outcome.p50_ms, 9);
        assert_eq!(s.tab_stages.shadow.p50_ms, 1);
        assert_eq!(s.tab_stages.grab_convert.p50_ms, 0);
        assert_eq!(s.tab_stages.save.p50_ms, 0);
        assert_eq!(s.tab_stages.digits.count, 1);
        let next = w
            .record_with(Stage::Capture, ms(1), t0 + WINDOW * 2, None)
            .expect("next window");
        assert_eq!(next.tab_analyze.p50_ms, 10);
        assert_eq!(next.tab_stages.digits.count, 0);
        assert_eq!(next.tab_stages.to_rgb.p50_ms, 0);
    }

    #[test]
    fn grab_convert_and_save_close_with_the_open_window() {
        let t0 = Instant::now();
        let mut w = Window::new();
        w.record(Stage::Capture, ms(40), t0);
        w.add_tab_stage(TabStage::GrabConvert, ms(40));
        w.add_tab_stage(TabStage::GrabConvert, ms(80));
        w.add_tab_stage(TabStage::Save, ms(12));
        let s = w.record(Stage::Capture, ms(1), t0 + WINDOW).unwrap();
        assert_eq!(s.tab_stages.grab_convert.count, 2);
        assert_eq!(s.tab_stages.grab_convert.p50_ms, 80);
        assert_eq!(s.tab_stages.save.count, 1);
        assert_eq!(s.tab_stages.save.p50_ms, 12);
        assert_eq!(s.tab_analyze.count, 0);
        let next = w.record(Stage::Capture, ms(1), t0 + WINDOW * 2).unwrap();
        assert_eq!(next.tab_stages.grab_convert.count, 0);
        assert_eq!(next.tab_stages.save.count, 0);
    }
}
