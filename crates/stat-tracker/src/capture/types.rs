//! Capture's shared data types and the pure rules on them (frame reuse,
//! backend labels). No process-wide state lives here.

/// Why this grab is happening. Tab captures may save a debug pair. Poll
/// captures, and the detectors that read them, stay on the same image type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureKind {
    Poll,
    Tab,
}

/// Prefs the daemon and the app set before
/// [`detect_backend`](super::select::detect_backend). The env var still wins over
/// `route` when it is non-empty.
#[derive(Debug, Clone)]
pub struct CapturePrefs {
    pub data_dir: std::path::PathBuf,
    pub route: crate::config::CaptureRoute,
    pub debug_frames: bool,
    /// Screen-stream cap in frames per second. Already clamped.
    pub max_fps: u32,
}

/// A route that takes one fresh screenshot per grab, as
/// [`detect_backend`](super::select::detect_backend) chose it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShotRoute {
    /// The stream, wayshot or X11.
    None,
    /// The screenshot portal ("Older screenshot"): on KDE about 450 ms of
    /// compositor and portal work per grab, plus a PNG.
    Portal,
    /// KDE direct: KWin renders the output into a pipe.
    Kwin,
}

impl ShotRoute {
    /// Out of a match this route polls less (each grab is a compositor
    /// render, and for the portal a dialog and a PNG).
    pub fn polls_less_out_of_a_match(self) -> bool {
        self != ShotRoute::None
    }

    /// A rejected Tab is retried once here instead of twice: each portal
    /// grab is a full KDE capture of about 450 ms.
    pub fn one_tab_retry(self) -> bool {
        self == ShotRoute::Portal
    }
}

/// What the screenshot portal's last request came to, for the app's
/// notice (S5-CAP-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PortalState {
    /// Nothing to say: it answered, or nothing asked it.
    Answering = 0,
    /// A request waits and permission is not granted: a permission prompt
    /// may be up, perhaps behind the game.
    MayBePrompt = 1,
    /// Permission is granted and a request got no answer: it was given up.
    Stalled = 2,
}

/// One captured image plus whether the stream had not already handed it out.
/// Screenshots are always new and have no sequence.
pub struct CapturedFrame {
    pub image: image::DynamicImage,
    pub seq: Option<u64>,
    /// Which stream produced `seq`. A new stream starts its numbers again.
    pub stream: Option<u64>,
    pub is_new: bool,
}

/// The last stream frame a poll accepted. Sequence numbers restart at 0
/// on every new stream, so the id has to travel with the sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeenStream {
    pub id: u64,
    pub seq: u64,
}

/// A still screen reuses one frame on every poll. The skip is debug once
/// per this window, then trace.
pub const REUSED_FRAME_LOG_WINDOW: std::time::Duration = std::time::Duration::from_secs(30);

/// True when the reused-frame skip should be logged at debug.
pub fn reused_frame_skip_is_debug(
    last: Option<std::time::Instant>,
    now: std::time::Instant,
    window: std::time::Duration,
) -> bool {
    match last {
        None => true,
        Some(at) => now.saturating_duration_since(at) >= window,
    }
}

/// A poll may update streaks only for a frame it has not already used.
/// A reused frame is no read. A new frame whose sequence was already seen
/// on the same stream is no read. A different stream is a read. A
/// screenshot (`seq` none, `is_new`) is a read.
pub fn poll_uses_frame(
    is_new: bool,
    seq: Option<u64>,
    stream: Option<u64>,
    seen: Option<SeenStream>,
) -> bool {
    if !is_new {
        return false;
    }
    match (seq, stream, seen) {
        (Some(seq), Some(id), Some(seen)) if id == seen.id && seq <= seen.seq => false,
        (Some(seq), None, Some(seen)) if seq <= seen.seq => false,
        _ => true,
    }
}

/// True when the poll loop must skip the read. The daemon `continue`s on
/// this, so a reused frame never confirms Victory or Defeat.
pub fn reused_frame_skips_the_read(
    is_new: bool,
    seq: Option<u64>,
    stream: Option<u64>,
    seen: Option<SeenStream>,
) -> bool {
    !poll_uses_frame(is_new, seq, stream, seen)
}

pub(super) fn as_new_capture(image: image::DynamicImage) -> CapturedFrame {
    CapturedFrame {
        image,
        seq: None,
        stream: None,
        is_new: true,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureBackend {
    Wayshot,
    X11,
    Portal,
    /// KDE direct: KWin's ScreenShot2 interface, no portal and no file.
    Kwin,
    None,
}

/// `system.capture_backend` in problem reports (`report::bundle`).
pub const fn capture_backend_label(backend: CaptureBackend) -> &'static str {
    match backend {
        CaptureBackend::X11 => "x11",
        CaptureBackend::Wayshot => "wayshot",
        // KDE direct goes out as `portal`: the site's validator
        // (`scuffed_types::stat_report`) knows no other word yet, and a
        // report it refuses is lost. The log in the bundle names the route.
        CaptureBackend::Portal | CaptureBackend::Kwin => "portal",
        CaptureBackend::None => "none",
    }
}

/// The name a backend's captures carry in the perf summary
/// (`backend = ...`, with `capture_p50_ms`). Every backend that can capture
/// has one, so X11, Wayland and the portal report the same timing line.
pub fn perf_backend_label(backend: &CaptureBackend) -> Option<&'static str> {
    match backend {
        CaptureBackend::Wayshot => Some("wayshot"),
        CaptureBackend::X11 => Some("x11"),
        CaptureBackend::Portal => Some("portal"),
        CaptureBackend::Kwin => Some("kwin"),
        CaptureBackend::None => None,
    }
}
