mod framebuf;
mod pod;
pub mod portal;
mod portal_backend;
mod pw;
mod route;
mod screencast;
mod stream_pause;
pub mod wayshot;
/// Native X11 capture (pure X11 sessions). Wired into selection by Lane C.
pub mod x11;

/// Why this grab is happening. Tab captures may save a debug pair. Poll
/// captures, and the detectors that read them, stay on the same image type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureKind {
    Poll,
    Tab,
}

/// Prefs the daemon and the app set before [`detect_backend`]. The env var
/// still wins over `route` when it is non-empty.
#[derive(Debug, Clone)]
pub struct CapturePrefs {
    pub data_dir: std::path::PathBuf,
    pub route: crate::config::CaptureRoute,
    pub debug_frames: bool,
    /// Screen-stream cap in frames per second. Already clamped.
    pub max_fps: u32,
    /// Pause the screen stream between grabs. Config and env already applied.
    pub pause_between_grabs: bool,
}

fn prefs() -> CapturePrefs {
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
            pause_between_grabs: true,
        })
}

tokio::task_local! {
    static TAB_PRESS: std::time::Instant;
}

tokio::task_local! {
    static SUPPRESS_STREAM_MISS_LOG: ();
}

/// True while the first Tab is discarding a miss from a press that the
/// share open already pushed past the frame budget. That miss is not logged.
pub(crate) fn stream_miss_log_suppressed() -> bool {
    SUPPRESS_STREAM_MISS_LOG.try_with(|_| ()).is_ok()
}

/// The Tab key instant for the current capture task, when this is a Tab grab.
pub fn tab_press() -> Option<std::time::Instant> {
    TAB_PRESS.try_with(|press| *press).ok()
}

/// Run `fut` as a Tab grab whose press time is `press`. Direct capture still
/// sleeps before calling this. The stream path reads the press and does not.
pub async fn with_tab_press<T>(
    press: std::time::Instant,
    fut: impl std::future::Future<Output = T>,
) -> T {
    TAB_PRESS.scope(press, fut).await
}

/// The first attempt uses the key instant. A retry starts its own
/// press-relative deadline at `now`, so it is not already past the first
/// attempt's hard stop.
pub fn tab_attempt_press(
    original: std::time::Instant,
    attempt: usize,
    now: std::time::Instant,
) -> std::time::Instant {
    if attempt == 0 { original } else { now }
}

/// How long the first Tab waits for a fresh frame from a share that is
/// still opening. The frame budget stays 1200 ms.
pub const FIRST_TAB_OPEN_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// What the first Tab does while the share is still opening.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpeningTab {
    /// The wait for a fresh frame has not passed.
    Wait,
    /// [`FIRST_TAB_OPEN_WAIT`] has passed. The attempt returns
    /// open-in-progress so a later retry can try again. It is not a
    /// stream miss, and it does not log one.
    Miss,
}

pub fn first_tab_while_opening(
    elapsed: std::time::Duration,
    bound: std::time::Duration,
) -> OpeningTab {
    if elapsed < bound {
        OpeningTab::Wait
    } else {
        OpeningTab::Miss
    }
}

/// After a late open, the original frame budget may already be gone. The
/// grab then starts again from `now`, while this Tab is still held.
pub fn tab_press_after_late_open(
    original: std::time::Instant,
    now: std::time::Instant,
    frame_budget: std::time::Duration,
    waited_for_open: bool,
) -> std::time::Instant {
    if waited_for_open && now >= original + frame_budget {
        now
    } else {
        original
    }
}

/// The first Tab's wait while a share is still opening.
///
/// `elapsed` is how long `press` has been held. `now` is read when a grab
/// misses, and again for the fresh press if that miss is past the frame
/// budget. `grab` is the frame source. `pause` is the gap between looks
/// (the daemon sleeps 50 ms). The daemon's first Tab calls this.
pub async fn first_tab_grab<G, GF, P, PF>(
    press: std::time::Instant,
    mut elapsed: impl FnMut() -> std::time::Duration,
    mut now: impl FnMut() -> std::time::Instant,
    mut grab: G,
    mut pause: P,
) -> Result<CapturedFrame, Box<dyn std::error::Error + Send + Sync>>
where
    G: FnMut() -> GF,
    GF: std::future::Future<
            Output = Result<CapturedFrame, Box<dyn std::error::Error + Send + Sync>>,
        > + Send,
    P: FnMut() -> PF,
    PF: std::future::Future<Output = ()> + Send,
{
    let frame_budget = std::time::Duration::from_millis(TAB_GRAB_TIMEOUT_AFTER_PRESS_MS);
    let mut waited_for_open = false;
    loop {
        // A press the open already pushed past the frame budget would log a
        // miss before this loop can start a fresh grab. Suppress that log.
        let suppress =
            waited_for_open && tab_press_after_late_open(press, now(), frame_budget, true) != press;
        let grabbed = if suppress {
            SUPPRESS_STREAM_MISS_LOG.scope((), grab()).await
        } else {
            grab().await
        };
        match grabbed {
            Err(err) if is_share_opening(&err.to_string()) => {
                waited_for_open = true;
                match first_tab_while_opening(elapsed(), FIRST_TAB_OPEN_WAIT) {
                    OpeningTab::Wait => pause().await,
                    OpeningTab::Miss => return Err(screencast::OPEN_IN_PROGRESS.into()),
                }
            }
            Err(err) if suppress && is_stream_miss(&err.to_string()) => {
                let fresh = now();
                return with_tab_press(fresh, grab()).await;
            }
            other => return other,
        }
    }
}

/// `share_open` from a status file counts only when that file's pid is the
/// live daemon. A stale pid file, or a status left by a daemon that has
/// exited, is a closed share.
pub fn live_share_open(share_open: bool, status_pid: u32, live_pid: Option<u32>) -> bool {
    share_open && live_pid == Some(status_pid)
}

fn live_daemon_pid(data_dir: &std::path::Path) -> Option<u32> {
    let text = std::fs::read_to_string(data_dir.join("daemon.pid")).ok()?;
    let rec = crate::proc_id::parse_pid_record(&text)?;
    crate::proc_id::pid_is_live_tracker_started(rec.pid, rec.start_ticks).then_some(rec.pid)
}

pub fn set_prefs(prefs: CapturePrefs) {
    stream_pause::set_enabled(prefs.pause_between_grabs);
    *PREFS.lock().unwrap_or_else(|err| err.into_inner()) = Some(prefs);
}

static PREFS: std::sync::Mutex<Option<CapturePrefs>> = std::sync::Mutex::new(None);

/// Portal path chosen by the last [`detect_backend`] call.
fn portal_path() -> route::PortalPath {
    match PORTAL_PATH.load(std::sync::atomic::Ordering::Acquire) {
        1 => route::PortalPath::Screenshot,
        _ => route::PortalPath::Stream,
    }
}

static PORTAL_PATH: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

static STREAM_ROUTE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static STREAM_FORCED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// This process is capturing through the long-lived screen stream.
/// Wayshot, X11, and the screenshot portal are not.
pub fn stream_route_active() -> bool {
    STREAM_ROUTE.load(std::sync::atomic::Ordering::Acquire)
}

/// The stream was asked for by the environment or by Settings, not chosen
/// because the desktop is KDE or GNOME.
pub fn stream_was_forced() -> bool {
    STREAM_FORCED.load(std::sync::atomic::Ordering::Acquire)
}

pub use screencast::{
    STREAM_MISS, close_if_open, is_declined_capture, is_share_opening, is_stream_miss,
    note_game_running, note_game_seen, session_is_open, share_close_due, should_close_share,
};

pub use framebuf::{TAB_GRAB_TIMEOUT_AFTER_PRESS_MS, first_grab_press, tab_retry_press};
pub use stream_pause::{KeyHold, key_hold as stream_key_hold};

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

fn as_new_capture(image: image::DynamicImage) -> CapturedFrame {
    CapturedFrame {
        image,
        seq: None,
        stream: None,
        is_new: true,
    }
}

/// A saved restore token can reopen the share without a prompt.
pub fn share_restorable(data_dir: &std::path::Path) -> bool {
    screencast::restore_token_saved(data_dir)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureBackend {
    Wayshot,
    X11,
    Portal,
    None,
}

/// Normalized force target parsed from `STAT_TRACKER_CAPTURE` (pure; unit-tested).
/// The accepted keyword set (and its case-folding) lives here and nowhere else,
/// both `select_backend` and `detect_backend` route through `force_target`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ForceTarget {
    Wayshot,
    X11,
    Portal,
    Unknown,
}

fn force_target(raw: &str) -> ForceTarget {
    match route::classify(raw) {
        route::ForceWord::Wayshot => ForceTarget::Wayshot,
        route::ForceWord::X11 => ForceTarget::X11,
        // `portal` and `screencast` are the stream. `portal-screenshot` is
        // the older portal. Both are the portal backend; `route` picks which.
        route::ForceWord::Stream | route::ForceWord::Screenshot => ForceTarget::Portal,
        route::ForceWord::Unknown => ForceTarget::Unknown,
    }
}

/// Pure backend-selection policy (R1-6). Every env read and live probe happens
/// in `detect_backend`; this takes their already-normalized results so the exact
/// production decision is unit-testable without touching process env or an X
/// server. `detect_backend` routes through this, the tests below exercise the
/// real logic, not a parallel copy.
///
/// `force` is the raw `STAT_TRACKER_CAPTURE` value (or `None`), normalized here
/// via `force_target` so the keyword set exists once. A forced backend never
/// silently falls back (R1-3): unavailable/unknown force → `None`.
///
/// `wayland_session` is true when the desktop itself is Wayland. There the
/// X11 probe can pass through XWayland (KDE, GNOME) even though X11 cannot
/// see the real screen, so the portal goes ahead of X11 and X11 is only a
/// last resort.
fn select_backend(
    force: Option<&str>,
    wayland_session: bool,
    wayshot_ok: bool,
    x11_ok: bool,
    portal_ok: bool,
) -> CaptureBackend {
    match force {
        Some(raw) => match force_target(raw) {
            ForceTarget::Wayshot => {
                if wayshot_ok {
                    CaptureBackend::Wayshot
                } else {
                    CaptureBackend::None
                }
            }
            ForceTarget::X11 => {
                if x11_ok {
                    CaptureBackend::X11
                } else {
                    CaptureBackend::None
                }
            }
            ForceTarget::Portal => {
                if portal_ok {
                    CaptureBackend::Portal
                } else {
                    CaptureBackend::None
                }
            }
            // Unknown forced value, fail closed, no fallback.
            ForceTarget::Unknown => CaptureBackend::None,
        },
        // Automatic priority: usable Wayshot, then X11, then Portal. On a
        // Wayland session: Wayshot, then Portal, then X11.
        None => {
            if wayshot_ok {
                CaptureBackend::Wayshot
            } else if wayland_session && portal_ok {
                CaptureBackend::Portal
            } else if x11_ok {
                CaptureBackend::X11
            } else if portal_ok {
                CaptureBackend::Portal
            } else {
                CaptureBackend::None
            }
        }
    }
}

/// True when the desktop session is Wayland, from `XDG_SESSION_TYPE`, or from
/// `WAYLAND_DISPLAY` when the session type is missing. A session that says
/// `x11` stays X11 even if a stray `WAYLAND_DISPLAY` is set.
fn is_wayland_session(session_type: Option<&str>, wayland_display: bool) -> bool {
    match session_type.map(str::to_ascii_lowercase).as_deref() {
        Some("wayland") => true,
        Some("x11") => false,
        _ => wayland_display,
    }
}

pub async fn detect_backend() -> CaptureBackend {
    // STAT_TRACKER_CAPTURE override (R1-3, fail-closed). A non-empty value
    // wins over Settings. Otherwise `capture_route` wins when it is not
    // auto. Probing is lazy per force so a forced backend never warms an
    // unrelated connection. The route log is `route::resolve`, which the
    // unit tests call with the same probe results.
    let prefs = prefs();
    let env = std::env::var("STAT_TRACKER_CAPTURE").ok();
    let env_set = env.as_deref().map(str::trim).is_some_and(|s| !s.is_empty());
    let settings = match prefs.route {
        crate::config::CaptureRoute::Screencast => Some(route::ForceWord::Stream),
        crate::config::CaptureRoute::Screenshot => Some(route::ForceWord::Screenshot),
        crate::config::CaptureRoute::Auto => None,
    };
    let session_type = std::env::var("XDG_SESSION_TYPE").ok();
    let wayland_display = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let wayland = is_wayland_session(session_type.as_deref(), wayland_display);
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    let chosen = route::effective(env.as_deref(), settings);

    let (wayshot_ok, x11_ok, portal_ok) = match chosen {
        route::Effective::Forced(route::ForceWord::Wayshot) => {
            (wayshot::probe().await.is_ok(), false, false)
        }
        route::Effective::Forced(route::ForceWord::X11) => {
            (false, x11::probe().await.is_ok(), false)
        }
        route::Effective::Forced(route::ForceWord::Stream | route::ForceWord::Screenshot) => {
            (false, false, portal::is_available().await)
        }
        route::Effective::Forced(route::ForceWord::Unknown) => (false, false, false),
        route::Effective::Auto => {
            let wayshot_ok = wayshot::probe().await.is_ok();
            let mut portal_checked = false;
            let mut portal_ok = false;
            if !wayshot_ok && wayland {
                portal_ok = portal::is_available().await;
                portal_checked = true;
            }
            let x11_ok = !wayshot_ok && !portal_ok && x11::probe().await.is_ok();
            if !wayshot_ok && !x11_ok && !portal_checked {
                portal_ok = portal::is_available().await;
            }
            (wayshot_ok, x11_ok, portal_ok)
        }
    };

    let decision = route::resolve(
        env.as_deref(),
        settings,
        wayland,
        wayshot_ok,
        x11_ok,
        portal_ok,
        &desktop,
    );
    PORTAL_PATH.store(
        match decision.portal {
            route::PortalPath::Screenshot => 1,
            route::PortalPath::Stream => 0,
        },
        std::sync::atomic::Ordering::Release,
    );
    let on_stream =
        decision.backend == CaptureBackend::Portal && decision.portal == route::PortalPath::Stream;
    STREAM_ROUTE.store(on_stream, std::sync::atomic::Ordering::Release);
    STREAM_FORCED.store(
        on_stream && decision.forced,
        std::sync::atomic::Ordering::Release,
    );
    tracing::info!("{}", decision.info);
    if let Some(line) = &decision.forced_line {
        tracing::info!("{line}");
    }
    if let Some(line) = &decision.mismatch {
        tracing::warn!("{line}");
    }
    if env_set
        && matches!(
            route::classify(env.as_deref().unwrap_or("")),
            route::ForceWord::Unknown
        )
    {
        tracing::warn!(
            raw = env.as_deref().unwrap_or(""),
            "unknown STAT_TRACKER_CAPTURE value"
        );
    }
    tracing::info!(
        backend = perf_backend_label(&decision.backend).unwrap_or("none"),
        session_type = session_type.as_deref().unwrap_or("unset"),
        wayland_display,
        "capture method chosen"
    );
    if decision.backend == CaptureBackend::None {
        tracing::warn!("no capture backend available");
    } else if decision.backend == CaptureBackend::Portal
        && decision.portal == route::PortalPath::Stream
    {
        portal_backend::log_configured(&desktop);
    }
    decision.backend
}

/// Built-in panels (laptop screens). On Auto these lose to an external
/// monitor, since a player with a monitor plugged in plays on it.
fn is_builtin_panel(name: &str) -> bool {
    ["eDP", "LVDS", "DSI"]
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

/// Which output Auto (no `capture_output`) captures, given each output's
/// name and real pixel area, in compositor order. One output is always that
/// output. With several, external monitors win over built-in panels (eDP,
/// LVDS, DSI), then the largest area wins, and a tie keeps list order.
/// `None` only for an empty list.
pub fn auto_output_index<'a>(outputs: impl IntoIterator<Item = (&'a str, u64)>) -> Option<usize> {
    let outputs: Vec<(&str, u64)> = outputs.into_iter().collect();
    if outputs.len() <= 1 {
        return if outputs.is_empty() { None } else { Some(0) };
    }
    let any_external = outputs.iter().any(|(name, _)| !is_builtin_panel(name));
    let mut best: Option<(usize, u64)> = None;
    for (i, (name, area)) in outputs.iter().enumerate() {
        if any_external && is_builtin_panel(name) {
            continue;
        }
        if best.is_none_or(|(_, best_area)| *area > best_area) {
            best = Some((i, *area));
        }
    }
    best.map(|(i, _)| i)
}

/// The name a backend's captures carry in the perf summary
/// (`backend = ...`, with `capture_p50_ms`). Every backend that can capture
/// has one, so X11, Wayland and the portal report the same timing line.
pub fn perf_backend_label(backend: &CaptureBackend) -> Option<&'static str> {
    match backend {
        CaptureBackend::Wayshot => Some("wayshot"),
        CaptureBackend::X11 => Some("x11"),
        CaptureBackend::Portal => Some("portal"),
        CaptureBackend::None => None,
    }
}

pub async fn capture_screen_output(
    backend: &CaptureBackend,
    output_name: Option<&str>,
) -> Result<image::DynamicImage, Box<dyn std::error::Error + Send + Sync>> {
    capture_screen_output_for(backend, output_name, CaptureKind::Poll)
        .await
        .map(|frame| frame.image)
}

/// One frame for the app's Capture now and the setup guide. A portal
/// backend uses the screenshot portal and does not open the long-lived
/// stream or write the restore token. On KDE it reads the daemon status
/// file and skips the screenshot while that share is open.
pub async fn capture_screen_oneshot(
    backend: &CaptureBackend,
    output_name: Option<&str>,
    data_dir: &std::path::Path,
) -> Result<image::DynamicImage, Box<dyn std::error::Error + Send + Sync>> {
    match backend {
        CaptureBackend::Portal => {
            let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
            let live_pid = live_daemon_pid(data_dir);
            let share_open = crate::ocr_switch::read_status(data_dir)
                .is_some_and(|status| live_share_open(status.share_open, status.pid, live_pid));
            if screencast::capture_now_blocked_by_daemon_share(&desktop, share_open) {
                tracing::warn!(
                    "Capture now skipped: the tracker is sharing the screen, and a screenshot beside it can crash KWin"
                );
                let token_saved = screencast::restore_token_saved(data_dir);
                return Err(screencast::capture_now_blocked_message(token_saved).into());
            }
            portal::capture(output_name).await
        }
        _ => capture_screen_output(backend, output_name).await,
    }
}

/// Same image as [`capture_screen_output`]. `kind` only changes whether a
/// Tab grab may write a local debug pair. Detection still receives a
/// [`image::DynamicImage`].
pub async fn capture_screen_output_for(
    backend: &CaptureBackend,
    output_name: Option<&str>,
    kind: CaptureKind,
) -> Result<CapturedFrame, Box<dyn std::error::Error + Send + Sync>> {
    let started = std::time::Instant::now();
    let Some(name) = perf_backend_label(backend) else {
        return Err("no capture backend available".into());
    };
    let result = match backend {
        CaptureBackend::Wayshot => wayshot::capture_with_output(output_name)
            .await
            .map(as_new_capture),
        CaptureBackend::X11 => x11::capture_with_output(output_name)
            .await
            .map(as_new_capture),
        CaptureBackend::Portal => match portal_path() {
            route::PortalPath::Screenshot => portal::capture(output_name).await.map(as_new_capture),
            route::PortalPath::Stream => {
                let prefs = prefs();
                screencast::capture(output_name, kind, &prefs.data_dir, prefs.debug_frames).await
            }
        },
        CaptureBackend::None => return Err("no capture backend available".into()),
    };
    if result.is_ok() {
        crate::perf::set_backend(name);
        crate::perf::record(crate::perf::Stage::Capture, started.elapsed());
    }
    result
}

/// List capture targets for the backend already selected by detect_backend.
/// Connection work must stay off the async/UI thread.
pub async fn list_outputs(
    backend: CaptureBackend,
) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    match backend {
        CaptureBackend::Wayshot => {
            let result =
                tokio::task::spawn_blocking(|| wayshot::list_outputs().map_err(|e| e.to_string()))
                    .await?;
            result.map_err(Into::into)
        }
        // x11::list_outputs is already async + Send (spawn_blocking internally).
        CaptureBackend::X11 => x11::list_outputs().await,
        // The portal shoots the whole desktop and is cropped to one output
        // afterwards, so the picker lists the Wayland outputs it can crop to.
        CaptureBackend::Portal => Ok(portal::list_outputs().await),
        CaptureBackend::None => Ok(Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// alpha.10 check: the capture p50 in the perf summary is recorded for
    /// X11 exactly as for Wayland, labelled by backend.
    #[test]
    fn every_capturing_backend_reports_capture_timing() {
        assert_eq!(perf_backend_label(&CaptureBackend::X11), Some("x11"));
        assert_eq!(
            perf_backend_label(&CaptureBackend::Wayshot),
            Some("wayshot")
        );
        assert_eq!(perf_backend_label(&CaptureBackend::Portal), Some("portal"));
        assert_eq!(perf_backend_label(&CaptureBackend::None), None);
        let key = std::time::Instant::now();
        let later = key + std::time::Duration::from_millis(800);
        assert_eq!(tab_attempt_press(key, 0, later), key);
        assert_eq!(tab_attempt_press(key, 1, later), later);
        assert_eq!(tab_attempt_press(key, 2, later), later);
        assert_eq!(
            first_tab_while_opening(std::time::Duration::from_millis(1200), FIRST_TAB_OPEN_WAIT),
            OpeningTab::Wait
        );
        assert_eq!(
            first_tab_while_opening(std::time::Duration::from_millis(1500), FIRST_TAB_OPEN_WAIT),
            OpeningTab::Wait
        );
        assert_eq!(
            first_tab_while_opening(FIRST_TAB_OPEN_WAIT, FIRST_TAB_OPEN_WAIT),
            OpeningTab::Miss
        );
        assert_eq!(
            first_tab_while_opening(std::time::Duration::from_secs(4), FIRST_TAB_OPEN_WAIT),
            OpeningTab::Wait
        );
        let t0 = std::time::Instant::now();
        assert!(reused_frame_skip_is_debug(
            None,
            t0,
            REUSED_FRAME_LOG_WINDOW
        ));
        assert!(!reused_frame_skip_is_debug(
            Some(t0),
            t0 + std::time::Duration::from_secs(1),
            REUSED_FRAME_LOG_WINDOW
        ));
        assert!(reused_frame_skip_is_debug(
            Some(t0),
            t0 + REUSED_FRAME_LOG_WINDOW,
            REUSED_FRAME_LOG_WINDOW
        ));
        let budget = std::time::Duration::from_millis(TAB_GRAB_TIMEOUT_AFTER_PRESS_MS);
        let late = key + std::time::Duration::from_millis(2000);
        assert_eq!(tab_press_after_late_open(key, later, budget, true), key);
        assert_eq!(tab_press_after_late_open(key, late, budget, true), late);
        assert_eq!(tab_press_after_late_open(key, late, budget, false), key);
        assert!(!live_share_open(true, 41, None));
        assert!(!live_share_open(true, 41, Some(42)));
        assert!(live_share_open(true, 41, Some(41)));
        assert!(!live_share_open(false, 41, Some(41)));
        let dir = tempfile::tempdir().unwrap();
        let status = crate::ocr_switch::Status {
            pid: 999_999,
            share_open: true,
            ..crate::ocr_switch::Status::default()
        };
        crate::ocr_switch::write_status(dir.path(), &status).unwrap();
        std::fs::write(dir.path().join("daemon.pid"), "999999\n").unwrap();
        assert_eq!(live_daemon_pid(dir.path()), None);
        assert!(!live_share_open(
            status.share_open,
            status.pid,
            live_daemon_pid(dir.path())
        ));
    }

    /// QA measured the share taking 3.4 to 3.9 seconds. This drives
    /// [`first_tab_grab`], the same loop the daemon uses. The clock and the
    /// frame source are injected, so the 4 seconds are not a real sleep.
    /// A 4 second open still gets a frame, and the re-grab uses a fresh
    /// press because the 1200 ms frame budget is already gone. That stale
    /// miss is not logged. An open that is still going at 5 seconds stays
    /// open-in-progress, so a later retry can try again.
    #[tokio::test]
    async fn a_four_second_share_open_keeps_the_first_tab() {
        assert_eq!(FIRST_TAB_OPEN_WAIT, std::time::Duration::from_secs(5));
        let opened = script_opening_share(std::time::Duration::from_secs(4)).await;
        assert!(opened.started.elapsed() < std::time::Duration::from_secs(1));
        assert!(opened.frame.is_new);
        assert!(opened.fresh_press);
        assert!(
            opened.miss_suppressed,
            "the stale miss from a press the open pushed past the frame budget is not logged"
        );
        assert_eq!(opened.elapsed, std::time::Duration::from_secs(4));

        let still = script_opening_share(std::time::Duration::from_secs(6)).await;
        assert!(still.started.elapsed() < std::time::Duration::from_secs(1));
        assert!(
            is_share_opening(&still.miss),
            "an open that is still going is retried, not a missed frame: {}",
            still.miss
        );
        assert!(!still.fresh_press);
        assert_eq!(still.elapsed, FIRST_TAB_OPEN_WAIT);
    }

    struct ScriptedOpen {
        started: std::time::Instant,
        elapsed: std::time::Duration,
        fresh_press: bool,
        miss_suppressed: bool,
        frame: CapturedFrame,
        miss: String,
    }

    async fn script_opening_share(open_for: std::time::Duration) -> ScriptedOpen {
        let started = std::time::Instant::now();
        let origin = std::time::Instant::now();
        let at = std::sync::atomic::AtomicU64::new(0);
        let after_open = std::sync::atomic::AtomicU8::new(0);
        let fresh_press = std::sync::atomic::AtomicBool::new(false);
        let miss_suppressed = std::sync::atomic::AtomicBool::new(false);
        let step_ms = 50u64;
        let open_ms = u64::try_from(open_for.as_millis()).unwrap_or(u64::MAX);
        let result = first_tab_grab(
            origin,
            || std::time::Duration::from_millis(at.load(std::sync::atomic::Ordering::Relaxed)),
            || {
                origin
                    + std::time::Duration::from_millis(
                        at.load(std::sync::atomic::Ordering::Relaxed),
                    )
            },
            || async {
                let elapsed_ms = at.load(std::sync::atomic::Ordering::Relaxed);
                if elapsed_ms < open_ms {
                    return Err(super::screencast::OPEN_IN_PROGRESS.into());
                }
                let n = after_open.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if n == 0 {
                    miss_suppressed.store(
                        stream_miss_log_suppressed(),
                        std::sync::atomic::Ordering::Relaxed,
                    );
                    return Err(STREAM_MISS.into());
                }
                fresh_press.store(
                    tab_press().is_some_and(|press| press != origin),
                    std::sync::atomic::Ordering::Relaxed,
                );
                Ok(CapturedFrame {
                    image: image::DynamicImage::ImageRgb8(image::RgbImage::new(1, 1)),
                    seq: Some(1),
                    stream: Some(1),
                    is_new: true,
                })
            },
            || async {
                at.fetch_add(step_ms, std::sync::atomic::Ordering::Relaxed);
            },
        )
        .await;
        let elapsed =
            std::time::Duration::from_millis(at.load(std::sync::atomic::Ordering::Relaxed));
        let fresh_press = fresh_press.load(std::sync::atomic::Ordering::Relaxed);
        let miss_suppressed = miss_suppressed.load(std::sync::atomic::Ordering::Relaxed);
        match result {
            Ok(frame) => ScriptedOpen {
                started,
                elapsed,
                fresh_press,
                miss_suppressed,
                frame,
                miss: String::new(),
            },
            Err(err) => ScriptedOpen {
                started,
                elapsed,
                fresh_press,
                miss_suppressed,
                frame: CapturedFrame {
                    image: image::DynamicImage::ImageRgb8(image::RgbImage::new(1, 1)),
                    seq: None,
                    stream: None,
                    is_new: false,
                },
                miss: err.to_string(),
            },
        }
    }

    // Pure selection-policy tests (R1-6). No `std::env::set_var`, process-global
    // env mutation races parallel test threads; we feed `select_backend` explicit
    // normalized inputs, which is exactly what `detect_backend` routes through.

    #[test]
    fn auto_priority_wayshot_first() {
        // Wayshot wins whenever usable, regardless of the others.
        assert_eq!(
            select_backend(None, false, true, true, true),
            CaptureBackend::Wayshot
        );
        assert_eq!(
            select_backend(None, false, true, false, false),
            CaptureBackend::Wayshot
        );
    }

    #[test]
    fn auto_failed_wayshot_falls_to_x11() {
        assert_eq!(
            select_backend(None, false, false, true, true),
            CaptureBackend::X11
        );
        assert_eq!(
            select_backend(None, false, false, true, false),
            CaptureBackend::X11
        );
    }

    #[test]
    fn auto_failed_x11_falls_to_portal() {
        assert_eq!(
            select_backend(None, false, false, false, true),
            CaptureBackend::Portal
        );
    }

    #[test]
    fn auto_none_available() {
        assert_eq!(
            select_backend(None, false, false, false, false),
            CaptureBackend::None
        );
    }

    #[test]
    fn forced_wayshot_available() {
        assert_eq!(
            select_backend(Some("wayshot"), false, true, false, false),
            CaptureBackend::Wayshot
        );
        // `wayland` is an accepted alias for the Wayshot force.
        assert_eq!(
            select_backend(Some("wayland"), false, true, false, false),
            CaptureBackend::Wayshot
        );
    }

    #[test]
    fn forced_x11_available() {
        assert_eq!(
            select_backend(Some("x11"), false, false, true, false),
            CaptureBackend::X11
        );
    }

    #[test]
    fn forced_portal_available() {
        assert_eq!(
            select_backend(Some("portal"), false, false, false, true),
            CaptureBackend::Portal
        );
    }

    #[test]
    fn forced_unavailable_never_falls_back() {
        // Each of the four force values, unavailable, must return None even when
        // other backends are usable, force means force (R1-3).
        assert_eq!(
            select_backend(Some("wayshot"), false, false, true, true),
            CaptureBackend::None
        );
        assert_eq!(
            select_backend(Some("wayland"), false, false, true, true),
            CaptureBackend::None
        );
        assert_eq!(
            select_backend(Some("x11"), false, true, false, true),
            CaptureBackend::None
        );
        assert_eq!(
            select_backend(Some("portal"), false, true, true, false),
            CaptureBackend::None
        );
    }

    #[test]
    fn unknown_force_is_none() {
        // Unknown value → None even with every backend available.
        assert_eq!(
            select_backend(Some("garbage"), false, true, true, true),
            CaptureBackend::None
        );
        assert_eq!(
            select_backend(Some(""), false, true, true, true),
            CaptureBackend::None
        );
    }

    // --- Normalizer (single keyword-set home) ---

    #[test]
    fn force_target_normalizes_case_and_aliases() {
        // Case-insensitive: uppercase env values map to the same targets.
        assert_eq!(force_target("X11"), ForceTarget::X11);
        assert_eq!(force_target("WAYSHOT"), ForceTarget::Wayshot);
        // `wayland` is an accepted alias for the Wayshot force, any case.
        assert_eq!(force_target("Wayland"), ForceTarget::Wayshot);
        assert_eq!(force_target("wayland"), ForceTarget::Wayshot);
        // Canonical lowercase spellings.
        assert_eq!(force_target("wayshot"), ForceTarget::Wayshot);
        assert_eq!(force_target("x11"), ForceTarget::X11);
        assert_eq!(force_target("portal"), ForceTarget::Portal);
        assert_eq!(force_target("screencast"), ForceTarget::Portal);
        assert_eq!(force_target("portal-screenshot"), ForceTarget::Portal);
        // Anything else is Unknown (fail-closed at the decision layer).
        assert_eq!(force_target("garbage"), ForceTarget::Unknown);
        assert_eq!(force_target(""), ForceTarget::Unknown);
    }

    // --- MUTANT_A: a forced backend wins even when a higher-priority,
    // non-forced backend is also usable. Force means force, the auto priority
    // order (Wayshot > X11 > Portal) must not leak into a forced decision. ---

    #[test]
    fn forced_backend_beats_higher_priority_available() {
        // Forced x11 with the higher-priority Wayshot ALSO usable → still X11.
        assert_eq!(
            select_backend(Some("x11"), false, true, true, false),
            CaptureBackend::X11
        );
        // Forced portal with both higher-priority backends usable → still Portal.
        assert_eq!(
            select_backend(Some("portal"), false, true, true, true),
            CaptureBackend::Portal
        );
    }

    // --- Wayland sessions (KDE, GNOME): X11 passes through XWayland but cannot
    // see the screen, so the portal must win over X11 there. ---

    #[test]
    fn wayland_session_prefers_portal_over_xwayland() {
        assert_eq!(
            select_backend(None, true, false, true, true),
            CaptureBackend::Portal
        );
    }

    #[test]
    fn wayland_session_still_prefers_wayshot() {
        assert_eq!(
            select_backend(None, true, true, true, true),
            CaptureBackend::Wayshot
        );
    }

    #[test]
    fn wayland_session_without_portal_keeps_x11_as_last_resort() {
        assert_eq!(
            select_backend(None, true, false, true, false),
            CaptureBackend::X11
        );
    }

    #[test]
    fn x11_session_keeps_x11_before_portal() {
        // Mint X11: unchanged order.
        assert_eq!(
            select_backend(None, false, false, true, true),
            CaptureBackend::X11
        );
    }

    #[test]
    fn force_ignores_session_type() {
        assert_eq!(
            select_backend(Some("x11"), true, false, true, true),
            CaptureBackend::X11
        );
    }

    #[test]
    fn session_type_detection() {
        assert!(is_wayland_session(Some("wayland"), false));
        assert!(is_wayland_session(Some("Wayland"), true));
        assert!(!is_wayland_session(Some("x11"), true));
        assert!(is_wayland_session(None, true));
        assert!(!is_wayland_session(None, false));
        assert!(is_wayland_session(Some("tty"), true));
        assert!(!is_wayland_session(Some(""), false));
    }

    #[test]
    fn auto_output_prefers_the_external_monitor() {
        // eDP-1 left, DP-1 right: DP-1, even when the panel is bigger.
        assert_eq!(
            auto_output_index([("eDP-1", 2560 * 1600), ("DP-1", 1920 * 1080)]),
            Some(1)
        );
        assert_eq!(
            auto_output_index([("DP-1", 1920 * 1080), ("LVDS-1", 1366 * 768)]),
            Some(0)
        );
    }

    #[test]
    fn auto_output_with_one_screen_is_that_screen() {
        assert_eq!(auto_output_index([("eDP-1", 1920 * 1200)]), Some(0));
        assert_eq!(auto_output_index([("DP-1", 2560 * 1440)]), Some(0));
        assert_eq!(auto_output_index(std::iter::empty::<(&str, u64)>()), None);
    }

    #[test]
    fn auto_output_picks_the_bigger_external_monitor() {
        assert_eq!(
            auto_output_index([("DP-1", 1920 * 1080), ("HDMI-A-1", 2560 * 1440)]),
            Some(1)
        );
        // A tie keeps list order.
        assert_eq!(
            auto_output_index([
                ("eDP-1", 1920 * 1080),
                ("DP-2", 1920 * 1080),
                ("DP-1", 1920 * 1080)
            ]),
            Some(1)
        );
        // Only built-in panels: the largest of them.
        assert_eq!(
            auto_output_index([("eDP-1", 1920 * 1080), ("eDP-2", 2560 * 1600)]),
            Some(1)
        );
    }
}
