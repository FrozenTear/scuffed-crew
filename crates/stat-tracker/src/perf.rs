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
use std::sync::atomic::{AtomicU64, Ordering};
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
    /// Screenshot portal steps, when the portal took a screenshot.
    pub portal: Option<PortalStats>,
    /// PipeWire stream buffers, when the stream delivered or copied any.
    pub stream: Option<StreamStats>,
}

/// One screenshot through the desktop portal, split by step.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PortalShot {
    /// From sending the request to the portal's answer: the compositor
    /// renders the desktop, encodes the PNG and writes the file.
    pub round_trip: Duration,
    /// Size of the PNG file the portal wrote.
    pub png_bytes: u64,
    /// Reading and decoding the PNG in the tracker.
    pub decode: Duration,
    /// Reading the output layout (a Wayland round trip).
    pub geometry: Duration,
    /// Cropping to one output and any resize. Zero with one output.
    pub crop: Duration,
}

/// Screenshot portal steps in one window. Times in ms, sizes in KiB.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalStats {
    pub round_trip: StageStats,
    pub png_kb: StageStats,
    pub decode: StageStats,
    pub geometry: StageStats,
    pub crop: StageStats,
}

/// The buffer PipeWire handed the stream consumer, as last copied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamBuffer {
    /// `memfd`, `memptr`, `dmabuf` or `other`.
    pub kind: &'static str,
    /// Bytes of the chunk (`chunk.size`).
    pub bytes: u32,
    /// SPA video format name (`BGRx`, ...).
    pub format: &'static str,
    pub width: u32,
    pub height: u32,
}

/// The stream's frame-rate cap as `num/den` frames per second.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamCap {
    pub num: u32,
    pub den: u32,
    /// `negotiated` (PipeWire's max-framerate) or `configured`.
    pub source: &'static str,
}

impl StreamCap {
    /// `4` for 4/1, `59.94` for 60000/1001.
    pub fn label(&self) -> String {
        if self.den <= 1 {
            self.num.to_string()
        } else {
            format!("{:.2}", f64::from(self.num) / f64::from(self.den))
        }
    }
}

/// PipeWire stream buffers in one window. `delivered` counts every buffer
/// the stream dequeued; `copied` the ones the tracker copied out (only
/// while a grab waits).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamStats {
    pub delivered: u64,
    pub copied: u64,
    pub buffer: Option<StreamBuffer>,
    pub cap: Option<StreamCap>,
}

impl StreamStats {
    /// Buffers delivered per second over `window_secs`.
    pub fn delivered_fps(&self, window_secs: u64) -> f64 {
        self.delivered as f64 / window_secs.max(1) as f64
    }

    /// Copied over delivered, `None` with nothing delivered.
    pub fn copy_ratio(&self) -> Option<f64> {
        (self.delivered > 0).then(|| self.copied as f64 / self.delivered as f64)
    }
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
    portal_shots: Vec<PortalShot>,
    stream_delivered: u64,
    stream_copied: u64,
    stream_buffer: Option<StreamBuffer>,
    stream_cap: Option<StreamCap>,
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
            portal_shots: Vec::new(),
            stream_delivered: 0,
            stream_copied: 0,
            stream_buffer: None,
            stream_cap: None,
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

    /// One portal screenshot.
    pub fn add_portal_shot(&mut self, shot: PortalShot) {
        self.portal_shots.push(shot);
    }

    /// Stream buffers delivered and copied since the last call.
    pub fn add_stream_counts(&mut self, delivered: u64, copied: u64) {
        self.stream_delivered = self.stream_delivered.saturating_add(delivered);
        self.stream_copied = self.stream_copied.saturating_add(copied);
    }

    /// The buffer a stream copy read.
    pub fn set_stream_buffer(&mut self, buffer: StreamBuffer) {
        self.stream_buffer = Some(buffer);
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
        let shots = std::mem::take(&mut self.portal_shots);
        let portal = (!shots.is_empty()).then(|| {
            let ms = |d: Duration| u64::try_from(d.as_millis()).unwrap_or(u64::MAX);
            let col = |f: &dyn Fn(&PortalShot) -> u64| {
                let mut v: Vec<u64> = shots.iter().map(f).collect();
                StageStats::from_samples(&mut v)
            };
            PortalStats {
                round_trip: col(&|s| ms(s.round_trip)),
                png_kb: col(&|s| s.png_bytes / 1024),
                decode: col(&|s| ms(s.decode)),
                geometry: col(&|s| ms(s.geometry)),
                crop: col(&|s| ms(s.crop)),
            }
        });
        let delivered = std::mem::take(&mut self.stream_delivered);
        let copied = std::mem::take(&mut self.stream_copied);
        // The buffer description is kept: it holds until the stream
        // renegotiates, and a window with no copy still shows it.
        let stream = (delivered > 0 || copied > 0).then_some(StreamStats {
            delivered,
            copied,
            buffer: self.stream_buffer,
            cap: self.stream_cap,
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
            portal,
            stream,
        })
    }
}

impl Default for Window {
    fn default() -> Self {
        Self::new()
    }
}

/// The daemon-wide window plus the stream counters the PipeWire thread
/// bumps without a lock. One static instance backs the free functions
/// below; tests drive their own.
#[derive(Debug)]
pub struct Recorder {
    window: Mutex<Window>,
    /// Stream buffers dequeued and copied since the last [`Recorder::record`].
    stream_delivered: AtomicU64,
    stream_copied: AtomicU64,
}

/// What one [`Recorder::record`] did.
#[derive(Debug, Default)]
pub struct Recorded {
    /// The window this sample closed.
    pub closed: Option<Summary>,
    /// True when this sample opened the first window. Earlier CPU belongs
    /// to no window, so the performance log takes a new baseline.
    pub opened_first: bool,
}

impl Recorder {
    pub const fn new() -> Self {
        Self {
            window: Mutex::new(Window::new()),
            stream_delivered: AtomicU64::new(0),
            stream_copied: AtomicU64::new(0),
        }
    }

    fn with_window(&self, f: impl FnOnce(&mut Window)) {
        if let Ok(mut window) = self.window.lock() {
            f(&mut window);
        }
    }

    /// Record one sample at `now`.
    pub fn record(&self, stage: Stage, took: Duration, now: Instant) -> Recorded {
        let Ok(mut window) = self.window.lock() else {
            return Recorded::default();
        };
        window.add_stream_counts(
            self.stream_delivered.swap(0, Ordering::Relaxed),
            self.stream_copied.swap(0, Ordering::Relaxed),
        );
        let opened_first = window.opened.is_none();
        Recorded {
            closed: window.record(stage, took, now),
            opened_first,
        }
    }

    pub fn note_stream_delivered(&self) {
        self.stream_delivered.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_stream_copy(&self, buffer: Option<StreamBuffer>, copied: bool, cpu: Duration) {
        if copied {
            self.stream_copied.fetch_add(1, Ordering::Relaxed);
        }
        self.with_window(|window| {
            if let Some(buffer) = buffer {
                window.set_stream_buffer(buffer);
            }
            window.add_cpu(Stage::Capture, cpu);
        });
    }
}

impl Default for Recorder {
    fn default() -> Self {
        Self::new()
    }
}

static RECORDER: Recorder = Recorder::new();

/// Record one sample in the daemon-wide window and log a summary when one
/// closes.
pub fn record(stage: Stage, took: Duration) {
    let recorded = RECORDER.record(stage, took, Instant::now());
    if recorded.opened_first {
        crate::perf_log::rebaseline();
    }
    if let Some(summary) = recorded.closed {
        log_summary(&summary);
        crate::perf_log::write_summary(summary);
    }
}

/// Add thread CPU time for one sample of `stage`.
pub fn record_cpu(stage: Stage, cpu: Duration) {
    RECORDER.with_window(|window| window.add_cpu(stage, cpu));
}

/// Note one X11 GetImage grab (no shm).
pub fn record_x11_grab(width: u16, height: u16, full: bool, took: Duration) {
    RECORDER.with_window(|window| window.add_x11_grab(width, height, full, took));
}

/// Note one portal screenshot's step times.
pub fn record_portal_shot(shot: PortalShot) {
    RECORDER.with_window(|window| window.add_portal_shot(shot));
}

/// One buffer the PipeWire stream dequeued. Lock-free: called from the
/// PipeWire thread for every buffer.
pub fn note_stream_delivered() {
    RECORDER.note_stream_delivered();
}

/// One stream buffer the tracker tried to copy: what the buffer was,
/// whether its bytes were copied (a DmaBuf or corrupt chunk is not), and
/// the thread CPU the attempt took.
pub fn record_stream_copy(buffer: Option<StreamBuffer>, copied: bool, cpu: Duration) {
    RECORDER.record_stream_copy(buffer, copied, cpu);
}

/// The stream's frame-rate cap: what PipeWire negotiated, else the
/// configured `screencast_max_fps`. Kept until the next stream sets it.
pub fn set_stream_cap(cap: StreamCap) {
    RECORDER.with_window(|window| window.stream_cap = Some(cap));
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
    RECORDER.with_window(|window| window.set_backend(backend));
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

    #[test]
    fn portal_shots_are_split_by_step() {
        let t0 = Instant::now();
        let mut w = Window::new();
        w.set_backend("portal");
        w.record(Stage::Capture, ms(300), t0);
        for (rt, kb, decode, crop) in [
            (180, 5_000, 90, 0),
            (240, 6_000, 120, 1),
            (200, 5_500, 95, 0),
        ] {
            w.add_portal_shot(PortalShot {
                round_trip: ms(rt),
                png_bytes: kb * 1024 + 512,
                decode: ms(decode),
                geometry: ms(2),
                crop: ms(crop),
            });
        }
        let s = w.record(Stage::Capture, ms(1), t0 + WINDOW).unwrap();
        let p = s.portal.expect("portal stats");
        assert_eq!(p.round_trip.count, 3);
        assert_eq!((p.round_trip.p50_ms, p.round_trip.max_ms), (200, 240));
        assert_eq!((p.png_kb.p50_ms, p.png_kb.max_ms), (5_500, 6_000));
        assert_eq!((p.decode.p50_ms, p.decode.max_ms), (95, 120));
        assert_eq!(p.geometry.max_ms, 2);
        assert_eq!(p.crop.max_ms, 1);
        assert!(s.stream.is_none());
        let next = w.record(Stage::Capture, ms(1), t0 + WINDOW * 2).unwrap();
        assert!(next.portal.is_none());
    }

    /// The free functions' path: buffers noted by the PipeWire thread and
    /// copy attempts land in the window the next `record` closes, and a
    /// tried-but-not-copied buffer (DmaBuf) is not counted as copied.
    #[test]
    fn the_recorder_moves_stream_counts_into_the_window_it_closes() {
        let r = Recorder::new();
        let t0 = Instant::now();
        let memfd = StreamBuffer {
            kind: "memfd",
            bytes: 100,
            format: "BGRx",
            width: 10,
            height: 10,
        };
        let dmabuf = StreamBuffer {
            kind: "dmabuf",
            ..memfd
        };
        let first = r.record(Stage::Capture, ms(5), t0);
        assert!(
            first.opened_first,
            "the first sample opens the first window"
        );
        assert!(first.closed.is_none());
        for _ in 0..3 {
            r.note_stream_delivered();
        }
        r.record_stream_copy(Some(memfd), true, ms(2));
        r.record_stream_copy(Some(dmabuf), false, ms(1));
        let mid = r.record(Stage::Capture, ms(5), t0 + ms(1_000));
        assert!(!mid.opened_first && mid.closed.is_none());
        r.note_stream_delivered();
        r.record_stream_copy(Some(memfd), true, ms(4));
        let s = r
            .record(Stage::Capture, ms(5), t0 + WINDOW)
            .closed
            .expect("window closes");
        let st = s.stream.expect("stream stats");
        assert_eq!((st.delivered, st.copied), (4, 2));
        assert_eq!(st.buffer, Some(memfd));
        assert_eq!(s.capture_cpu_ms, 7, "copy CPU is capture CPU");
        let next = r
            .record(Stage::Capture, ms(5), t0 + WINDOW * 2)
            .closed
            .expect("second window closes");
        assert!(next.stream.is_none(), "counts were moved, not copied");
        assert!(
            !r.record(Stage::Capture, ms(5), t0 + WINDOW * 3)
                .opened_first
        );
    }

    #[test]
    fn a_zero_second_window_still_has_a_finite_rate() {
        let st = StreamStats {
            delivered: 30,
            copied: 0,
            buffer: None,
            cap: None,
        };
        assert_eq!(st.delivered_fps(0), 30.0);
        assert_eq!(st.delivered_fps(10), 3.0);
        assert_eq!(st.copy_ratio(), Some(0.0));
        let none = StreamStats { delivered: 0, ..st };
        assert_eq!(none.copy_ratio(), None);
    }

    #[test]
    fn stream_cap_labels() {
        let cap = |num, den| StreamCap {
            num,
            den,
            source: "negotiated",
        };
        assert_eq!(cap(4, 1).label(), "4");
        assert_eq!(cap(4, 0).label(), "4");
        assert_eq!(cap(60_000, 1_001).label(), "59.94");
        assert_eq!(cap(5, 2).label(), "2.50");
    }

    #[test]
    fn stream_counts_reset_per_window_and_the_buffer_is_kept() {
        let t0 = Instant::now();
        let mut w = Window::new();
        w.record(Stage::Capture, ms(5), t0);
        let buf = StreamBuffer {
            kind: "memfd",
            bytes: 8_294_400,
            format: "BGRx",
            width: 1920,
            height: 1080,
        };
        w.set_stream_buffer(buf);
        let cap = StreamCap {
            num: 4,
            den: 1,
            source: "negotiated",
        };
        w.stream_cap = Some(cap);
        w.add_stream_counts(3_000, 100);
        w.add_stream_counts(600, 20);
        let s = w.record(Stage::Capture, ms(5), t0 + WINDOW).unwrap();
        let st = s.stream.expect("stream stats");
        assert_eq!((st.delivered, st.copied), (3_600, 120));
        assert_eq!(st.buffer, Some(buf));
        assert_eq!(st.cap, Some(cap));
        assert_eq!(st.delivered_fps(s.window_secs), 60.0);
        assert_eq!(st.copy_ratio().map(|r| (r * 300.0).round()), Some(10.0));
        // Nothing delivered or copied: no stream fields.
        let quiet = w.record(Stage::Capture, ms(5), t0 + WINDOW * 2).unwrap();
        assert!(quiet.stream.is_none());
        w.add_stream_counts(4, 0);
        let again = w.record(Stage::Capture, ms(5), t0 + WINDOW * 3).unwrap();
        assert_eq!(again.stream.unwrap().buffer, Some(buf));
    }
}
