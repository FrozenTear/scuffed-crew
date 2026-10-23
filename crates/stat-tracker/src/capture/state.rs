//! Capture's process-wide state: prefs, the route the last
//! [`detect_backend`](super::detect_backend) chose, the notice the app
//! shows, and the running daemon's share status.

use super::route;
use super::types::{CaptureBackend, CapturePrefs, PortalState, ShotRoute};

pub(super) fn prefs() -> CapturePrefs {
    PREFS
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone()
        .unwrap_or_else(|| CapturePrefs {
            data_dir: dirs::data_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join("scuffed-stat-tracker"),
            route: crate::config::CaptureRoute::Auto,
            debug_frames: false,
            max_fps: crate::config::SCREENCAST_MAX_FPS_DEFAULT,
        })
}

/// `share_open` from a status file counts only when that file's pid is the
/// live daemon. A stale pid file, or a status left by a daemon that has
/// exited, is a closed share.
pub fn live_share_open(share_open: bool, status_pid: u32, live_pid: Option<u32>) -> bool {
    share_open && live_pid == Some(status_pid)
}

pub(super) fn live_daemon_pid(data_dir: &std::path::Path) -> Option<u32> {
    let text = std::fs::read_to_string(data_dir.join("daemon.pid")).ok()?;
    let rec = crate::proc_id::parse_pid_record(&text)?;
    crate::proc_id::pid_is_live_tracker_started(rec.pid, rec.start_ticks).then_some(rec.pid)
}

pub fn set_prefs(prefs: CapturePrefs) {
    *PREFS.lock().unwrap_or_else(|err| err.into_inner()) = Some(prefs);
}

static PREFS: std::sync::Mutex<Option<CapturePrefs>> = std::sync::Mutex::new(None);

/// Portal path chosen by the last [`detect_backend`](super::detect_backend) call.
pub(super) fn portal_path() -> route::PortalPath {
    match PORTAL_PATH.load(std::sync::atomic::Ordering::Acquire) {
        1 => route::PortalPath::Screenshot,
        _ => route::PortalPath::Stream,
    }
}

pub(super) static PORTAL_PATH: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

pub(super) static STREAM_ROUTE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
pub(super) static STREAM_FORCED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// This process is capturing through the long-lived screen stream.
/// Wayshot, X11, KDE direct and the screenshot portal are not.
pub fn stream_route_active() -> bool {
    STREAM_ROUTE.load(std::sync::atomic::Ordering::Acquire)
}

pub(super) static SHOT_ROUTE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Which per-grab screenshot route this process uses, if any.
pub fn shot_route() -> ShotRoute {
    match SHOT_ROUTE.load(std::sync::atomic::Ordering::Acquire) {
        1 => ShotRoute::Portal,
        2 => ShotRoute::Kwin,
        _ => ShotRoute::None,
    }
}

pub(super) static ROUTE_NOTICE: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// The warning from the last [`detect_backend`](super::detect_backend) the app should show: KDE
/// direct was asked for and could not be used, a saved Wayland-only route
/// was ignored off Wayland, or the X11 probe failed and nothing else can
/// capture. `None` when there is none.
pub fn route_notice() -> Option<String> {
    let notice = ROUTE_NOTICE
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone();
    (!notice.is_empty()).then_some(notice)
}

pub(super) static PORTAL_STATE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

fn portal_state() -> PortalState {
    match PORTAL_STATE.load(std::sync::atomic::Ordering::Acquire) {
        1 => PortalState::MayBePrompt,
        2 => PortalState::Stalled,
        _ => PortalState::Answering,
    }
}

/// Where this process publishes [`capture_notice`]. Only the daemon sets
/// it (the status file); the app's own grabs leave the daemon's file alone.
type NoticeSink = Box<dyn Fn(&str) + Send + Sync>;
static NOTICE_SINK: std::sync::OnceLock<NoticeSink> = std::sync::OnceLock::new();

/// The daemon's sink for [`capture_notice`]. Set once; later calls are
/// ignored.
pub fn set_notice_sink(sink: impl Fn(&str) + Send + Sync + 'static) {
    let _ = NOTICE_SINK.set(Box::new(sink));
}

/// The sentence the app shows under the capture method: the route notice
/// ([`route_notice`]) and what the screenshot portal is up to. Empty when
/// there is nothing to say.
pub fn capture_notice() -> String {
    notice_text(&route_notice().unwrap_or_default(), portal_state())
}

/// The prompt notice: a request waits on a permission dialog, perhaps.
pub const PORTAL_PROMPT_NOTICE: &str = "A screenshot permission prompt may be waiting, possibly behind the game. Screenshots wait for it.";

/// The stall notice: permission is granted and the portal did not answer.
pub const PORTAL_STALL_NOTICE: &str = "No answer from the screenshot portal. It may have stopped; restarting the tracker or the portal can help.";

/// [`capture_notice`] from its parts.
pub(super) fn notice_text(route: &str, portal: PortalState) -> String {
    let portal = match portal {
        PortalState::Answering => None,
        PortalState::MayBePrompt => Some(PORTAL_PROMPT_NOTICE.to_string()),
        PortalState::Stalled => Some(PORTAL_STALL_NOTICE.to_string()),
    };
    match (route.trim(), portal) {
        ("", None) => String::new(),
        ("", Some(stall)) => stall,
        (route, None) => route.to_string(),
        (route, Some(stall)) => format!("{route}. {stall}"),
    }
}

/// A grab worked, on any route: a stalled-portal notice no longer holds.
/// A possible permission prompt stays until its request ends.
pub(super) fn note_grab_worked() {
    if PORTAL_STATE
        .compare_exchange(
            PortalState::Stalled as u8,
            PortalState::Answering as u8,
            std::sync::atomic::Ordering::AcqRel,
            std::sync::atomic::Ordering::Acquire,
        )
        .is_ok()
        && let Some(sink) = NOTICE_SINK.get()
    {
        sink(&capture_notice());
    }
}

/// What the screenshot portal's last request came to. A change is
/// published through the daemon's sink. A successful grab on any route
/// and a new route decision set [`PortalState::Answering`].
pub(crate) fn note_portal_state(state: PortalState) {
    let was = PORTAL_STATE.swap(state as u8, std::sync::atomic::Ordering::AcqRel);
    if was != state as u8
        && let Some(sink) = NOTICE_SINK.get()
    {
        sink(&capture_notice());
    }
}

/// The stream was asked for by the environment or by Settings, not chosen
/// because the desktop is KDE or GNOME.
pub fn stream_was_forced() -> bool {
    STREAM_FORCED.load(std::sync::atomic::Ordering::Acquire)
}

/// The process-wide route flags for one decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RouteFlags {
    /// The screen stream is this process's route.
    pub(super) on_stream: bool,
    /// ... and it was asked for, not chosen because of the desktop.
    pub(super) stream_forced: bool,
    pub(super) shot: ShotRoute,
}

pub(super) fn route_flags(
    backend: CaptureBackend,
    portal: route::PortalPath,
    forced: bool,
) -> RouteFlags {
    let on_stream = backend == CaptureBackend::Portal && portal == route::PortalPath::Stream;
    RouteFlags {
        on_stream,
        stream_forced: on_stream && forced,
        shot: match (backend, portal) {
            (CaptureBackend::Portal, route::PortalPath::Screenshot) => ShotRoute::Portal,
            (CaptureBackend::Kwin, _) => ShotRoute::Kwin,
            _ => ShotRoute::None,
        },
    }
}

/// The running tracker's screen share is open, from its status file.
pub fn daemon_share_is_open(data_dir: &std::path::Path) -> bool {
    let live_pid = live_daemon_pid(data_dir);
    crate::ocr_status::read_status(data_dir)
        .is_some_and(|status| live_share_open(status.share_open, status.pid, live_pid))
}
