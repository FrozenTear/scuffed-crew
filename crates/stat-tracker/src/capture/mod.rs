mod flight;
mod framebuf;
mod geom;
pub mod kwin;
mod pod;
pub mod portal;
mod portal_backend;
mod pw;
mod route;
pub use flight::is_busy as is_capture_busy;
pub use route::{KWIN_DIRECT_DEFAULT_ON_KDE, KwinProbe, SessionKind, is_gnome, is_kde};
mod screencast;
mod stream_pause;
mod tab;
mod types;
pub mod wayshot;
/// Native X11 capture (pure X11 sessions). Wired into selection by Lane C.
pub mod x11;

pub use geom::auto_output_index;
use tab::SUPPRESS_STREAM_MISS_LOG;
pub use tab::{
    FIRST_TAB_OPEN_WAIT, OUTCOME_POLL_LOCK_WAIT, OpeningTab, first_tab_while_opening,
    poll_lock_wait, tab_attempt_press, tab_press, tab_press_after_late_open, with_poll_lock_wait,
    with_tab_press,
};
pub(crate) use tab::{grab_lock_wait, stream_miss_log_suppressed};
use types::as_new_capture;
pub use types::{
    CaptureBackend, CaptureKind, CapturePrefs, CapturedFrame, PortalState, REUSED_FRAME_LOG_WINDOW,
    SeenStream, ShotRoute, capture_backend_label, perf_backend_label, poll_uses_frame,
    reused_frame_skip_is_debug, reused_frame_skips_the_read,
};

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

/// The daemon's poll grab: [`capture_screen_output_for`] with
/// [`CaptureKind::Poll`], waiting for the screenshot lock as
/// [`poll_lock_wait`] says (the end-reel outcome-only poll waits up to
/// [`OUTCOME_POLL_LOCK_WAIT`], an ordinary poll not at all).
pub fn poll_grab<'a>(
    backend: &'a CaptureBackend,
    output_name: Option<&'a str>,
    outcome_only: bool,
) -> impl std::future::Future<Output = Result<CapturedFrame, Box<dyn std::error::Error + Send + Sync>>>
+ 'a {
    with_poll_lock_wait(
        poll_lock_wait(outcome_only),
        capture_screen_output_for(backend, output_name, CaptureKind::Poll),
    )
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
/// Wayshot, X11, KDE direct and the screenshot portal are not.
pub fn stream_route_active() -> bool {
    STREAM_ROUTE.load(std::sync::atomic::Ordering::Acquire)
}

static SHOT_ROUTE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Which per-grab screenshot route this process uses, if any.
pub fn shot_route() -> ShotRoute {
    match SHOT_ROUTE.load(std::sync::atomic::Ordering::Acquire) {
        1 => ShotRoute::Portal,
        2 => ShotRoute::Kwin,
        _ => ShotRoute::None,
    }
}

static ROUTE_NOTICE: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// The warning from the last [`detect_backend`] the app should show: KDE
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

/// A poll grab error that is a known state, not a capture failure: the
/// capture lock was held, the share is still opening, was declined, or
/// stays closed until the game runs, or the stream had no fresh frame for
/// this poll. These have their own log lines (or none on purpose, like the
/// one decline notice); the poll logs them at TRACE and they neither start
/// nor end a failure run (S5-CAP-7).
pub fn is_expected_poll_miss(err: &str) -> bool {
    is_capture_busy(err)
        || screencast::is_share_opening(err)
        || screencast::is_declined_capture(err)
        || screencast::is_share_closed_until_game(err)
        || screencast::is_stream_miss(err)
        || err.contains(pw::NO_STREAM_FRAME)
        || err.contains(portal::SHOT_BACKOFF)
}

static PORTAL_STATE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

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
fn notice_text(route: &str, portal: PortalState) -> String {
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
fn note_grab_worked() {
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

/// A grab error from a screenshot portal that is not answering: a request
/// given up ([`portal::SHOT_UNANSWERED`]), or a grab turned away while new
/// requests are paused or held after one ([`portal::SHOT_BACKOFF`]). The
/// portal code logs its own WARN for these.
pub fn is_portal_unanswered(err: &str) -> bool {
    err.contains(portal::SHOT_UNANSWERED)
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

pub use stream_pause::{
    KeyHold, bridge_hold as stream_bridge_hold, decide_and_apply as decide_stream_pause,
    end_tab_stream_hold, key_hold as stream_key_hold, with_tab_stream_hold,
};
pub use tab::{TAB_GRAB_TIMEOUT_AFTER_PRESS_MS, first_grab_press, tab_retry_press};

/// A saved restore token can reopen the share without a prompt.
pub fn share_restorable(data_dir: &std::path::Path) -> bool {
    screencast::restore_token_saved(data_dir)
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
        // KDE direct is decided in `route::resolve` from its probe and
        // never reaches `select_backend`.
        route::ForceWord::Kwin | route::ForceWord::Unknown => ForceTarget::Unknown,
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
        // Wayland session: Wayshot, then Portal, then X11. On a known X11
        // session `detect_backend` never reports the portal usable on
        // automatic ([`auto_may_use_portal`], S5-CAP-5).
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
        crate::config::CaptureRoute::Kwin => Some(route::ForceWord::Kwin),
        crate::config::CaptureRoute::Auto => None,
    };
    let session_type = std::env::var("XDG_SESSION_TYPE").ok();
    let wayland_display = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let session = route::SessionKind::from_env(session_type.as_deref(), wayland_display);
    let wayland = session == route::SessionKind::Wayland;
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    // A saved Wayland-only route on a known X11 session is not probed:
    // `resolve_on_session` uses automatic for it and says why.
    let chosen = route::effective(env.as_deref(), route::saved_on_session(settings, session));

    // KDE direct is probed when asked for, or on automatic on KDE once
    // `KWIN_DIRECT_DEFAULT_ON_KDE` is flipped. No screen stream is open
    // yet at this point, so the probe's 1x1 render cannot meet it.
    let wants_kwin = match chosen {
        route::Effective::Forced(route::ForceWord::Kwin) => true,
        route::Effective::Auto => {
            route::KWIN_DIRECT_DEFAULT_ON_KDE && wayland && route::is_kde(&desktop)
        }
        route::Effective::Forced(_) => false,
    };
    let kwin_probe = if wants_kwin {
        // The app runs this too, perhaps beside a tracker whose stream is
        // open (a different STAT_TRACKER_CAPTURE): no render there before
        // Plasma 6.8.1. On KDE a probe that can pass (KWin not on the bus
        // yet at login) is tried again for up to 15 s.
        kwin::probe_at_startup(
            || daemon_share_is_open(&prefs.data_dir),
            route::is_kde(&desktop),
        )
        .await
    } else {
        route::KwinProbe::NotProbed
    };
    if chosen == route::Effective::Auto
        && let route::KwinProbe::Unavailable(reason) | route::KwinProbe::NotChecked(reason) =
            &kwin_probe
    {
        tracing::info!("automatic keeps the screen stream: KDE direct is not available ({reason})");
    }
    // A KDE direct that is there needs no other probe. One that is not
    // falls back to what automatic would choose, so probe for that.
    let probe_as = match chosen {
        route::Effective::Forced(route::ForceWord::Kwin)
            if kwin_probe == route::KwinProbe::Available =>
        {
            None
        }
        route::Effective::Forced(route::ForceWord::Kwin) => Some(route::Effective::Auto),
        other => Some(other),
    };

    // Why the X11 probe failed, for the notice when nothing else works.
    let mut x11_failure: Option<String> = None;
    let (wayshot_ok, x11_ok, portal_ok) = match probe_as {
        None => (false, false, false),
        Some(chosen) => match chosen {
            route::Effective::Forced(route::ForceWord::Wayshot) => {
                (wayshot::probe().await.is_ok(), false, false)
            }
            route::Effective::Forced(route::ForceWord::X11) => {
                (false, probe_x11_logged(&mut x11_failure).await, false)
            }
            route::Effective::Forced(route::ForceWord::Stream | route::ForceWord::Screenshot) => {
                (false, false, portal::is_available().await)
            }
            // KDE direct never gets here: `probe_as` maps it to none or auto.
            route::Effective::Forced(route::ForceWord::Unknown | route::ForceWord::Kwin) => {
                (false, false, false)
            }
            route::Effective::Auto => {
                let wayshot_ok = wayshot::probe().await.is_ok();
                if wayshot_ok {
                    (true, false, false)
                } else if wayland {
                    let portal_ok = portal::is_available().await;
                    let x11_ok = !portal_ok && probe_x11_logged(&mut x11_failure).await;
                    (false, x11_ok, portal_ok)
                } else {
                    let x11_ok = probe_x11_logged(&mut x11_failure).await;
                    // S5-CAP-5: on a known X11 session there is no screen
                    // stream to fall back to; a failed X11 probe means no
                    // capture. An unknown session keeps rc.7's fallback.
                    let portal_ok =
                        !x11_ok && auto_may_use_portal(session) && portal::is_available().await;
                    (false, x11_ok, portal_ok)
                }
            }
        },
    };

    let decision = route::resolve_on_session(
        env.as_deref(),
        settings,
        session,
        wayshot_ok,
        x11_ok,
        portal_ok,
        &desktop,
        &kwin_probe,
        route::KWIN_DIRECT_DEFAULT_ON_KDE,
    );
    PORTAL_PATH.store(
        match decision.portal {
            route::PortalPath::Screenshot => 1,
            route::PortalPath::Stream => 0,
        },
        std::sync::atomic::Ordering::Release,
    );
    let flags = route_flags(decision.backend, decision.portal, decision.forced);
    STREAM_ROUTE.store(flags.on_stream, std::sync::atomic::Ordering::Release);
    STREAM_FORCED.store(flags.stream_forced, std::sync::atomic::Ordering::Release);
    SHOT_ROUTE.store(
        match flags.shot {
            ShotRoute::None => 0,
            ShotRoute::Portal => 1,
            ShotRoute::Kwin => 2,
        },
        std::sync::atomic::Ordering::Release,
    );
    let notice = decision.fallback.clone().or_else(|| {
        x11_failure
            .filter(|_| {
                session == route::SessionKind::X11 && decision.backend == CaptureBackend::None
            })
            .map(|err| x11_failure_notice(&err))
    });
    *ROUTE_NOTICE.lock().unwrap_or_else(|err| err.into_inner()) = notice.unwrap_or_default();
    // A new route decision: an old portal notice no longer holds.
    PORTAL_STATE.store(
        PortalState::Answering as u8,
        std::sync::atomic::Ordering::Release,
    );
    tracing::info!("{}", decision.info);
    if let Some(line) = &decision.forced_line {
        tracing::info!("{line}");
    }
    if let Some(line) = &decision.mismatch {
        tracing::warn!("{line}");
    }
    if let Some(line) = &decision.fallback {
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

/// The X11 probe, its error logged at WARN and kept in `failure`.
async fn probe_x11_logged(failure: &mut Option<String>) -> bool {
    match x11::probe().await {
        Ok(()) => true,
        Err(err) => {
            tracing::warn!(error = %err, "X11 capture probe failed");
            *failure = Some(err.to_string());
            false
        }
    }
}

/// Whether automatic may fall back to the portal's screen stream when no
/// direct capture works: not on a known X11 session, where that stream
/// does not exist (S5-CAP-5).
fn auto_may_use_portal(session: route::SessionKind) -> bool {
    session != route::SessionKind::X11
}

/// The app's notice when the X11 probe failed on a known X11 session and
/// nothing else can capture.
fn x11_failure_notice(err: &str) -> String {
    format!("X11 capture is not available ({err}); no capture method works on this session")
}

/// The process-wide route flags for one decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RouteFlags {
    /// The screen stream is this process's route.
    on_stream: bool,
    /// ... and it was asked for, not chosen because of the desktop.
    stream_forced: bool,
    shot: ShotRoute,
}

fn route_flags(backend: CaptureBackend, portal: route::PortalPath, forced: bool) -> RouteFlags {
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
/// file and skips the screenshot while that share is open. KDE direct
/// goes through KWin's same screenshot code, so it skips there too.
pub async fn capture_screen_oneshot(
    backend: &CaptureBackend,
    output_name: Option<&str>,
    data_dir: &std::path::Path,
) -> Result<image::DynamicImage, Box<dyn std::error::Error + Send + Sync>> {
    match backend {
        CaptureBackend::Portal | CaptureBackend::Kwin => {
            let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
            if oneshot_blocked(*backend, &desktop, daemon_share_is_open(data_dir)) {
                tracing::warn!(
                    "Capture now skipped: the tracker is sharing the screen, and a screenshot beside it can crash KWin"
                );
                let token_saved = screencast::restore_token_saved(data_dir);
                return Err(screencast::capture_now_blocked_message(token_saved).into());
            }
            if *backend == CaptureBackend::Kwin {
                kwin::capture(output_name).await
            } else {
                portal::capture(output_name).await
            }
        }
        _ => capture_screen_output(backend, output_name).await,
    }
}

/// Whether Capture now must skip its screenshot: the portal and KDE direct
/// both reach KWin's screenshot code, which must not run beside the
/// tracker's open share on KDE. Wayshot and X11 never do.
pub fn oneshot_blocked(backend: CaptureBackend, desktop: &str, share_open: bool) -> bool {
    matches!(backend, CaptureBackend::Portal | CaptureBackend::Kwin)
        && screencast::capture_now_blocked_by_daemon_share(desktop, share_open)
}

/// The running tracker's screen share is open, from its status file.
pub fn daemon_share_is_open(data_dir: &std::path::Path) -> bool {
    let live_pid = live_daemon_pid(data_dir);
    crate::ocr_status::read_status(data_dir)
        .is_some_and(|status| live_share_open(status.share_open, status.pid, live_pid))
}

/// KDE direct for the app's Settings: `None` off KDE. With `render` (the
/// saved route is KDE direct, or its chip was just chosen) it runs the 1x1
/// probe, never on NVIDIA with a Plasma before the full KWin crash fix,
/// and never beside the tracker's open share on a Plasma not known to have
/// it ([`kwin::render_gate`]). Without, it renders nothing: the last probe
/// result, or what the version read says.
pub async fn kwin_direct_status(data_dir: std::path::PathBuf, render: bool) -> Option<KwinProbe> {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    if !route::is_kde(&desktop) {
        return None;
    }
    Some(if render {
        kwin::probe_beside_share(daemon_share_is_open(&data_dir)).await
    } else {
        kwin::status_without_render().await
    })
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
            // Boxed: these leaf futures are large, and the Tab capture
            // future that awaits this one would carry them inline.
            route::PortalPath::Screenshot => Box::pin(portal::capture(output_name))
                .await
                .map(as_new_capture),
            route::PortalPath::Stream => {
                let prefs = prefs();
                screencast::capture(output_name, kind, &prefs.data_dir, prefs.debug_frames).await
            }
        },
        // Never the stream: this route and the tracker's own screen share
        // are not run together (KDE bug 524840).
        CaptureBackend::Kwin => Box::pin(kwin::capture(output_name))
            .await
            .map(as_new_capture),
        CaptureBackend::None => return Err("no capture backend available".into()),
    };
    if result.is_ok() {
        crate::perf::set_backend(name);
        crate::perf::record(crate::perf::Stage::Capture, started.elapsed());
        note_grab_worked();
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
        // KWin names its outputs as Wayland does, for KDE direct.
        CaptureBackend::Portal | CaptureBackend::Kwin => Ok(portal::list_outputs().await),
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
        assert_eq!(perf_backend_label(&CaptureBackend::Kwin), Some("kwin"));
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
        let status = crate::ocr_status::Status {
            pid: 999_999,
            share_open: true,
            ..crate::ocr_status::Status::default()
        };
        crate::ocr_status::write_status(dir.path(), &status).unwrap();
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

    /// KDE direct is never the stream: the screen share is not opened, not
    /// closed on game exit, and no stall fallback runs beside it (KDE bug
    /// 524840). The `PortalPath` a KDE direct decision carries is ignored.
    #[test]
    fn kde_direct_never_counts_as_the_stream() {
        for portal in [route::PortalPath::Stream, route::PortalPath::Screenshot] {
            for forced in [false, true] {
                let flags = route_flags(CaptureBackend::Kwin, portal, forced);
                assert!(!flags.on_stream);
                assert!(!flags.stream_forced);
                assert_eq!(flags.shot, ShotRoute::Kwin);
            }
        }
        let stream = route_flags(CaptureBackend::Portal, route::PortalPath::Stream, true);
        assert_eq!(
            stream,
            RouteFlags {
                on_stream: true,
                stream_forced: true,
                shot: ShotRoute::None
            }
        );
        let shot = route_flags(CaptureBackend::Portal, route::PortalPath::Screenshot, true);
        assert!(!shot.on_stream);
        assert_eq!(shot.shot, ShotRoute::Portal);
        assert_eq!(
            route_flags(CaptureBackend::Wayshot, route::PortalPath::Stream, true).shot,
            ShotRoute::None
        );
        // Reports keep a word the site's validator accepts.
        assert_eq!(capture_backend_label(CaptureBackend::Kwin), "portal");
    }

    /// Capture now on KDE: the portal and KDE direct are both blocked beside
    /// the tracker's open share; wayshot and X11 are not; nothing is
    /// blocked off KDE or with the share closed.
    #[test]
    fn capture_now_keeps_kwin_calls_away_from_the_share() {
        for backend in [CaptureBackend::Portal, CaptureBackend::Kwin] {
            assert!(oneshot_blocked(backend, "KDE", true), "{backend:?}");
            assert!(!oneshot_blocked(backend, "KDE", false), "{backend:?}");
            assert!(!oneshot_blocked(backend, "niri", true), "{backend:?}");
        }
        assert!(oneshot_blocked(CaptureBackend::Kwin, "KWin", true));
        for backend in [
            CaptureBackend::Wayshot,
            CaptureBackend::X11,
            CaptureBackend::None,
        ] {
            assert!(!oneshot_blocked(backend, "KDE", true), "{backend:?}");
        }
    }

    #[tokio::test]
    async fn a_grab_takes_its_lock_wait_from_its_task() {
        let tab = std::time::Duration::from_secs(4);
        assert_eq!(
            grab_lock_wait(tab),
            std::time::Duration::ZERO,
            "a plain poll"
        );
        let reel = with_poll_lock_wait(poll_lock_wait(true), async { grab_lock_wait(tab) }).await;
        assert_eq!(reel, OUTCOME_POLL_LOCK_WAIT);
        let plain = with_poll_lock_wait(poll_lock_wait(false), async { grab_lock_wait(tab) }).await;
        assert_eq!(plain, std::time::Duration::ZERO);
        let in_tab = with_tab_press(std::time::Instant::now(), async { grab_lock_wait(tab) }).await;
        assert_eq!(in_tab, tab);
        assert_eq!(OUTCOME_POLL_LOCK_WAIT, std::time::Duration::from_secs(1));
    }

    /// A function body from `src` without its tests, whitespace removed:
    /// from `sig` to the end of that item.
    fn squashed_body(src: &'static str, sig: &str) -> String {
        let code = src.split("\nmod tests {").next().unwrap();
        let at = code.find(sig).unwrap_or_else(|| panic!("{sig}"));
        code[at..]
            .split("\n}\n")
            .next()
            .unwrap()
            .split_whitespace()
            .collect()
    }

    // Call sites the unit tests cannot reach without KWin or a portal
    // (#433 review), one test each.

    #[test]
    fn the_poll_grab_passes_the_outcome_only_wait_and_grabs_read_the_task() {
        let module = include_str!("mod.rs");
        let poll = squashed_body(module, concat!("pub fn ", "poll_grab<"));
        assert!(poll.contains(
            "with_poll_lock_wait(poll_lock_wait(outcome_only),capture_screen_output_for(backend,output_name,CaptureKind::Poll)"
        ));
        let wait = squashed_body(
            include_str!("tab.rs"),
            concat!("pub(crate) fn ", "grab_lock_wait("),
        );
        assert!(wait.contains(
            "flight::lock_wait(tab_press().is_some(),tab_wait,POLL_LOCK_WAIT.try_with(|wait|*wait).unwrap_or(std::time::Duration::ZERO)"
        ));
    }

    #[test]
    fn the_portal_grab_reads_its_lock_wait_from_the_task() {
        let shot = squashed_body(
            include_str!("portal.rs"),
            concat!("pub async fn ", "capture("),
        );
        assert!(shot.contains("letwait=super::grab_lock_wait(SHOT_LOCK_WAIT_TAB);"));
        assert!(shot.contains("run_shot(&SHOT_FLIGHT,wait,"));
    }

    #[test]
    fn the_kwin_grab_reads_its_lock_wait_from_the_task() {
        let grab = squashed_body(
            include_str!("kwin.rs"),
            concat!("pub async fn ", "capture("),
        );
        assert!(grab.contains("letwait=super::grab_lock_wait(LOCK_WAIT_TAB);"));
        assert!(grab.contains("grab_in_flight(target,true,wait)"));
    }

    #[test]
    fn the_kwin_grab_respects_the_reread_limit() {
        let grab = squashed_body(
            include_str!("kwin.rs"),
            concat!("pub async fn ", "capture("),
        );
        assert!(grab.contains("letdue=native_reread_due("));
        assert!(grab.contains("check_native_size(size,real,due,"));
    }

    #[test]
    fn the_kwin_reread_counts_as_capture_cpu() {
        let grab = squashed_body(
            include_str!("kwin.rs"),
            concat!("pub async fn ", "capture("),
        );
        assert!(grab.contains("crate::perf::record_cpu(crate::perf::Stage::Capture,reread_cpu);"));
    }

    #[test]
    fn the_compare_half_runs_only_past_its_gate() {
        let compare = squashed_body(
            include_str!("kwin.rs"),
            concat!("pub(crate) async fn ", "save_compare_half("),
        );
        assert!(compare.contains("letprobe=probe_beside_share(true).await;"));
        assert!(compare.contains("ifletErr(why)=compare_half_allowed(&versions().await,&probe){"));
        let gate_at = compare.find("compare_half_allowed(").unwrap();
        let capture_at = compare.find("capture(output_name)").unwrap();
        assert!(gate_at < capture_at, "the gate comes before the grab");
    }

    #[test]
    fn the_startup_probe_gets_a_share_check_for_every_attempt() {
        let detect = squashed_body(
            include_str!("mod.rs"),
            concat!("pub async fn ", "detect_backend("),
        );
        assert!(detect.contains("kwin::probe_at_startup(||daemon_share_is_open(&prefs.data_dir),"));
    }

    #[test]
    fn route_gating_by_shot_route() {
        assert!(!ShotRoute::None.polls_less_out_of_a_match());
        assert!(ShotRoute::Portal.polls_less_out_of_a_match());
        assert!(ShotRoute::Kwin.polls_less_out_of_a_match());
        assert!(!ShotRoute::None.one_tab_retry());
        assert!(ShotRoute::Portal.one_tab_retry());
        assert!(
            !ShotRoute::Kwin.one_tab_retry(),
            "KDE direct grabs are cheap"
        );
    }

    #[test]
    fn the_capture_notice_adds_the_portal_stall_to_the_route_notice() {
        use PortalState::*;
        assert_eq!(notice_text("", Answering), "");
        assert_eq!(notice_text("route reason", Answering), "route reason");
        assert_eq!(notice_text("", Stalled), PORTAL_STALL_NOTICE);
        assert_eq!(notice_text("", MayBePrompt), PORTAL_PROMPT_NOTICE);
        // A pending prompt is never told to restart anything.
        assert!(!PORTAL_PROMPT_NOTICE.contains("restart"));
        let both = notice_text("route reason", Stalled);
        assert!(
            both.starts_with("route reason. No answer from the screenshot portal"),
            "{both}"
        );
    }

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
        // S5-CAP-5: a known X11 session never reports the portal usable on
        // automatic; Wayland and an unknown session may.
        assert!(!auto_may_use_portal(route::SessionKind::X11));
        assert!(auto_may_use_portal(route::SessionKind::Wayland));
        assert!(auto_may_use_portal(route::SessionKind::Unknown));
        assert_eq!(
            select_backend(None, false, false, false, false),
            CaptureBackend::None
        );
    }

    #[test]
    fn known_poll_states_are_not_capture_failures() {
        for err in [
            format!("portal screenshot: {}", flight::BUSY),
            screencast::OPEN_IN_PROGRESS.to_string(),
            screencast::NO_CAPTURE_DECLINE_NOTICE.to_string(),
            screencast::timeout_notice(false).to_string(),
            screencast::SHARE_CLOSED_UNTIL_GAME.to_string(),
            screencast::STREAM_MISS.to_string(),
            pw::NO_STREAM_FRAME.to_string(),
            format!("portal screenshot: {}", portal::SHOT_BACKOFF),
        ] {
            assert!(is_expected_poll_miss(&err), "{err}");
        }
        for err in [
            "screencast stream ended",
            "X11 capture failed: BadMatch",
            "KDE direct capture: KWin did not answer in 2 s",
        ] {
            assert!(!is_expected_poll_miss(err), "{err}");
        }
    }

    #[test]
    fn the_x11_failure_notice_names_the_error() {
        let notice = x11_failure_notice("cannot open display :0");
        assert!(notice.contains("X11 capture is not available"), "{notice}");
        assert!(notice.contains("cannot open display :0"), "{notice}");
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
        // KDE direct is never a `select_backend` force.
        assert_eq!(force_target("kwin"), ForceTarget::Unknown);
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
        use route::SessionKind::{self, *};
        let kind = SessionKind::from_env;
        assert_eq!(kind(Some("wayland"), false), Wayland);
        assert_eq!(kind(Some("Wayland"), true), Wayland);
        assert_eq!(kind(Some("x11"), true), Unknown, "a stray WAYLAND_DISPLAY");
        assert_eq!(kind(Some("x11"), false), X11);
        assert_eq!(kind(None, true), Wayland);
        assert_eq!(kind(None, false), Unknown);
        assert_eq!(kind(Some("tty"), true), Wayland);
        assert_eq!(kind(Some(""), false), Unknown);
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
