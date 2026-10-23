//! What one closed perf window holds: the per-stage timings and the
//! X11, portal and stream details. `perf_log` formats these; `perf` fills
//! them. Kept in its own file so `perf_log` depends on this data, not on
//! the recorder.

use std::time::Duration;

/// Timings for one stage inside one window.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StageStats {
    pub count: u32,
    pub p50_ms: u64,
    pub max_ms: u64,
    pub total_ms: u64,
}

impl StageStats {
    pub(super) fn from_samples(samples: &mut [u64]) -> Self {
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
    /// Reading the PNG file and decoding it in the tracker, both on one
    /// blocking thread since rc.7 (#432): this includes the file read.
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
