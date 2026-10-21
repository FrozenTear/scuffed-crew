mod framebuf;
mod pod;
pub mod portal;
mod portal_backend;
mod pw;
mod route;
mod screencast;
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
        })
}

tokio::task_local! {
    static TAB_PRESS: std::time::Instant;
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

pub fn set_prefs(prefs: CapturePrefs) {
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

pub use screencast::{close_if_open, session_is_open, should_close_share};

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
    capture_screen_output_for(backend, output_name, CaptureKind::Poll).await
}

/// One frame for the app's Capture now and the setup guide. A portal
/// backend uses the screenshot portal and does not open the long-lived
/// stream or write the restore token.
pub async fn capture_screen_oneshot(
    backend: &CaptureBackend,
    output_name: Option<&str>,
) -> Result<image::DynamicImage, Box<dyn std::error::Error + Send + Sync>> {
    match backend {
        CaptureBackend::Portal => {
            let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
            if screencast::oneshot_blocked_by_share(
                &desktop,
                stream_was_forced(),
                screencast::session_is_open(),
            ) {
                tracing::warn!(
                    "Capture now skipped: a screen share is open, and a screenshot beside it can crash KWin"
                );
                return Err("capture now skipped while a screen share is open".into());
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
) -> Result<image::DynamicImage, Box<dyn std::error::Error + Send + Sync>> {
    let started = std::time::Instant::now();
    let Some(name) = perf_backend_label(backend) else {
        return Err("no capture backend available".into());
    };
    let result = match backend {
        CaptureBackend::Wayshot => wayshot::capture_with_output(output_name).await,
        CaptureBackend::X11 => x11::capture_with_output(output_name).await,
        CaptureBackend::Portal => match portal_path() {
            route::PortalPath::Screenshot => portal::capture(output_name).await,
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
