//! Backend selection at start: env and Settings, the live probes, and
//! publishing the decision to [`super::state`]. The pure policy is
//! [`super::route`].

use super::state::{
    PORTAL_PATH, PORTAL_STATE, ROUTE_NOTICE, SHOT_ROUTE, STREAM_FORCED, STREAM_ROUTE,
    daemon_share_is_open, prefs, route_flags,
};
use super::stream::portal_backend;
use super::types::{CaptureBackend, PortalState, ShotRoute, perf_backend_label};
use super::{kwin, portal, route, wayshot, x11};

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
pub(super) fn auto_may_use_portal(session: route::SessionKind) -> bool {
    session != route::SessionKind::X11
}

/// The app's notice when the X11 probe failed on a known X11 session and
/// nothing else can capture.
pub(super) fn x11_failure_notice(err: &str) -> String {
    format!("X11 capture is not available ({err}); no capture method works on this session")
}
