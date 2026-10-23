//! Whether KDE direct may be used: the Plasma and interface versions, the
//! NVIDIA check and KDE bug 524840 render gate, and the 1x1 render probe
//! with its retries and kept result.

use std::time::Duration;

use ashpd::zvariant::OwnedValue;

use super::protocol::{INTERFACE, KwinError, PATH, SERVICE, from_zbus, to_rgb_image, value_u32};
use super::{
    GRAB_TIMEOUT, LOCK_WAIT_TAB, bounded, connection, forget_connection, grab_in_flight,
    probe_target,
};
use crate::capture::route::KwinProbe;

/// Plasma version with the full fix for KDE bug 524840.
pub const CRASH_FIX_FULL: (u32, u32, u32) = (6, 8, 1);

/// What the probe's result means for the route.
pub fn probe_outcome(result: &Result<(), KwinError>) -> KwinProbe {
    match result {
        Ok(()) => KwinProbe::Available,
        Err(err) => KwinProbe::Unavailable(err.reason()),
    }
}

// ------------------------------------------------------------------- probe

/// `KWin version: 6.8.1` from KWin's support information.
pub fn parse_kwin_version(text: &str) -> Option<(u32, u32, u32)> {
    let line = text
        .lines()
        .find_map(|line| line.trim().strip_prefix("KWin version:"))?;
    let mut parts = line.trim().split(['.', ' ', '-']);
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    Some((major, minor, patch))
}

/// How far a Plasma version has the KDE bug 524840 fix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrashFix {
    /// Before 6.6.7, 6.7.0 to 6.7.5.
    Missing,
    /// 6.6.7+, 6.7.6+ (not verified), 6.8.0: the main fix, not the 6.8.1
    /// follow-up.
    Partial,
    /// 6.8.1 and later.
    Full,
}

pub fn crash_fix((major, minor, patch): (u32, u32, u32)) -> CrashFix {
    let v = (major, minor, patch);
    if v >= CRASH_FIX_FULL {
        CrashFix::Full
    } else if v >= (6, 8, 0)
        || (major == 6 && minor == 7 && patch >= 6)
        || (major == 6 && minor == 6 && patch >= 7)
    {
        CrashFix::Partial
    } else {
        CrashFix::Missing
    }
}

/// What KWin says about itself, read without rendering anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Versions {
    /// The interface's `Version` property, or why it could not be read.
    pub interface: Result<u32, KwinError>,
    /// The Plasma (KWin) version, when KWin's support information has it.
    pub plasma: Option<(u32, u32, u32)>,
    /// The NVIDIA kernel driver is loaded ([`nvidia_loaded`]): KDE bug
    /// 524840 crashes KWin only there.
    pub nvidia: bool,
}

/// Whether the NVIDIA kernel driver is loaded, from `/sys/module`.
pub fn nvidia_loaded() -> bool {
    nvidia_loaded_in(std::path::Path::new("/sys/module"))
}

/// [`nvidia_loaded`] under another `/sys/module`, for tests.
pub fn nvidia_loaded_in(modules: &std::path::Path) -> bool {
    ["nvidia", "nvidia_drm"]
        .iter()
        .any(|name| modules.join(name).exists())
}

fn version_label(v: Option<(u32, u32, u32)>) -> String {
    v.map_or_else(|| "unknown".to_string(), |(a, b, c)| format!("{a}.{b}.{c}"))
}

/// Why KDE direct is unavailable with NVIDIA on a Plasma without the full
/// KDE bug 524840 fix (before 6.8.1). The app and the route line put
/// "KDE direct is not available" in front of it.
pub const NEEDS_FULL_FIX: &str = "it needs Plasma 6.8.1 or newer on NVIDIA";

/// What a render probe may do now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderGate {
    /// A probe result is cached: use it, render nothing.
    Cached,
    /// The interface did not answer, or the NVIDIA driver is loaded and this
    /// Plasma lacks the full KDE bug 524840 fix: unavailable, render
    /// nothing.
    Refused(String),
    /// A screen share may be open and this Plasma is not known to have the
    /// full fix: not checked, render nothing.
    NotChecked(String),
    /// Render the 1x1 probe.
    Render,
}

/// The one rule for a render probe. With the NVIDIA driver loaded, a
/// Plasma known to be older than 6.8.1 is refused whatever is open: KWin
/// there can crash when a screenshot meets any app's screen share
/// (S5-CAP-3). `share_open` is whether the tracker's screen share may be
/// open beside this call; with it open, only a Plasma known to be 6.8.1 or
/// later renders (the rc.7 rule).
pub fn render_gate(share_open: bool, found: &Versions, probe_done: bool) -> RenderGate {
    if probe_done {
        return RenderGate::Cached;
    }
    if let Err(err) = &found.interface {
        return RenderGate::Refused(err.reason());
    }
    let plasma = found.plasma;
    if found.nvidia
        && let Some(v) = plasma
        && crash_fix(v) != CrashFix::Full
    {
        return RenderGate::Refused(format!(
            "{NEEDS_FULL_FIX} (found {}, KDE bug 524840)",
            version_label(plasma)
        ));
    }
    if share_open && plasma.is_none_or(|v| crash_fix(v) != CrashFix::Full) {
        return RenderGate::NotChecked(format!(
            "it is not tested while the tracker shares the screen on Plasma {} (needs 6.8.1, KDE bug 524840)",
            version_label(plasma)
        ));
    }
    RenderGate::Render
}

/// A probe failure that holds for the life of the process: KWin refused
/// the caller. It is kept for [`FAILURE_TTL`] only, all the same. Other
/// failures (a timeout, no bus, KWin not on the bus yet at login) are not
/// kept at all.
const FAILURE_TTL: Duration = Duration::from_secs(60);

/// Whether a probe result may be reused at `now`.
pub fn reusable(
    result: &Result<(), KwinError>,
    at: tokio::time::Instant,
    now: tokio::time::Instant,
) -> bool {
    match result {
        Ok(()) => true,
        Err(err) => err.is_definitive() && now.saturating_duration_since(at) < FAILURE_TTL,
    }
}

/// What a probe needs from KWin; a fake in tests.
pub trait ProbeBus {
    fn versions(&self) -> impl std::future::Future<Output = Versions> + Send;
    fn render(&self) -> impl std::future::Future<Output = Result<(), KwinError>> + Send;
}

/// A probe's answer and whether trying again soon may change it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeAnswer {
    pub probe: KwinProbe,
    pub transient: bool,
}

/// One probe at a time, with its last result.
pub struct Prober {
    last: tokio::sync::Mutex<Option<(Result<(), KwinError>, tokio::time::Instant)>>,
}

impl Prober {
    pub const fn new() -> Self {
        Self {
            last: tokio::sync::Mutex::const_new(None),
        }
    }

    /// The probe through [`render_gate`]: renders only when allowed, and
    /// keeps the result as [`reusable`] says.
    pub async fn probe<B: ProbeBus>(&self, bus: &B, share_open: bool) -> ProbeAnswer {
        let mut last = self.last.lock().await;
        let now = tokio::time::Instant::now();
        let cached = last
            .as_ref()
            .filter(|(result, at)| reusable(result, *at, now))
            .map(|(result, _)| result.clone());
        let found = bus.versions().await;
        match render_gate(share_open, &found, cached.is_some()) {
            RenderGate::Cached => ProbeAnswer {
                probe: match &cached {
                    Some(result) => probe_outcome(result),
                    None => KwinProbe::NotProbed,
                },
                transient: false,
            },
            RenderGate::Refused(reason) => ProbeAnswer {
                probe: KwinProbe::Unavailable(reason),
                transient: found.interface.as_ref().is_err_and(|e| !e.is_definitive()),
            },
            RenderGate::NotChecked(reason) => ProbeAnswer {
                probe: KwinProbe::NotChecked(reason),
                transient: false,
            },
            RenderGate::Render => {
                let result = bus.render().await;
                log_probe(&result, &found);
                let transient = result.as_ref().is_err_and(|e| !e.is_definitive());
                let probe = probe_outcome(&result);
                *last = Some((result, now));
                ProbeAnswer { probe, transient }
            }
        }
    }

    /// The last result while it may be reused, without asking KWin.
    pub async fn last(&self) -> Option<KwinProbe> {
        let last = self.last.lock().await;
        let now = tokio::time::Instant::now();
        last.as_ref()
            .filter(|(result, at)| reusable(result, *at, now))
            .map(|(result, _)| probe_outcome(result))
    }
}

impl Default for Prober {
    fn default() -> Self {
        Self::new()
    }
}

fn log_probe(result: &Result<(), KwinError>, found: &Versions) {
    let plasma = version_label(found.plasma);
    match result {
        Ok(()) => {
            tracing::info!(
                interface_version = found.interface.as_ref().ok().copied().unwrap_or(0),
                plasma = %plasma,
                "KDE direct capture available (KWin ScreenShot2)"
            );
            // With NVIDIA a known Plasma before 6.8.1 never renders
            // ([`render_gate`]); an unread version is warned about.
            if found.nvidia && found.plasma.is_none() {
                tracing::warn!(
                    "could not read the Plasma version; KWin before 6.8.1 can crash on NVIDIA when a screenshot meets another app's screen share (KDE bug 524840)"
                );
            }
        }
        // The route logs the one warning a user sees (`resolve`'s
        // fallback line); this is for a debug log.
        Err(err) => {
            tracing::debug!(plasma = %plasma, "KDE direct probe failed: {}", err.reason());
        }
    }
}

/// Waits before each startup retry of a probe that failed in a way that
/// can pass (KWin not on the bus yet at login, a timeout): 15 s in all.
pub const STARTUP_RETRY_WAITS: [Duration; 4] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
];

/// [`Prober::probe`], tried again after each of `waits` while the answer
/// is transient.
pub async fn probe_with_retries<B: ProbeBus>(
    prober: &Prober,
    bus: &B,
    share_open: impl Fn() -> bool,
    waits: &[Duration],
) -> KwinProbe {
    // The share is looked at again before every attempt, not once before
    // the retries: a share that opened during a wait is seen.
    let mut answer = prober.probe(bus, share_open()).await;
    for wait in waits {
        if !answer.transient {
            break;
        }
        tracing::debug!(
            wait_ms = wait.as_millis() as u64,
            "KDE direct probe will try again"
        );
        tokio::time::sleep(*wait).await;
        answer = prober.probe(bus, share_open()).await;
    }
    answer.probe
}

/// The real bus. Versions are kept once the interface answered.
struct RealBus;

static VERSIONS: tokio::sync::Mutex<Option<Versions>> = tokio::sync::Mutex::const_new(None);

impl ProbeBus for RealBus {
    async fn versions(&self) -> Versions {
        let mut slot = VERSIONS.lock().await;
        if let Some(found) = slot.as_ref() {
            return found.clone();
        }
        let found = read_versions().await;
        if let Err(err) = &found.interface {
            forget_connection(err).await;
        } else {
            *slot = Some(found.clone());
        }
        found
    }

    async fn render(&self) -> Result<(), KwinError> {
        let target = probe_target(&crate::capture::outputs::output_geometry().await);
        let (meta, bytes) = grab_in_flight(target, false, LOCK_WAIT_TAB).await?;
        to_rgb_image(&meta, &bytes).map(|_| ())
    }
}

static PROBER: Prober = Prober::new();

async fn read_versions() -> Versions {
    let conn = match connection().await {
        Ok(conn) => conn,
        Err(err) => {
            return Versions {
                interface: Err(err),
                plasma: None,
                nvidia: nvidia_loaded(),
            };
        }
    };
    let interface = bounded(async {
        let reply = conn
            .call_method(
                Some(SERVICE),
                PATH,
                Some("org.freedesktop.DBus.Properties"),
                "Get",
                &(INTERFACE, "Version"),
            )
            .await
            .map_err(from_zbus)?;
        let value: OwnedValue = reply
            .body()
            .deserialize()
            .map_err(|err| KwinError::BadReply(err.to_string()))?;
        value_u32(&value).ok_or_else(|| KwinError::BadReply("Version is not a number".into()))
    })
    .await;
    // KWin has no version property. Its support information (the text
    // `qdbus org.kde.KWin /KWin supportInformation` prints) starts with
    // `KWin version: X.Y.Z`. Best effort.
    let plasma = if interface.is_ok() {
        let info = async {
            let reply = conn
                .call_method(
                    Some("org.kde.KWin"),
                    "/KWin",
                    Some("org.kde.KWin"),
                    "supportInformation",
                    &(),
                )
                .await
                .ok()?;
            let text: String = reply.body().deserialize().ok()?;
            parse_kwin_version(&text)
        };
        tokio::time::timeout(GRAB_TIMEOUT, info)
            .await
            .ok()
            .flatten()
    } else {
        None
    };
    Versions {
        interface,
        plasma,
        nvidia: nvidia_loaded(),
    }
}

/// What KWin says about itself, without rendering anything.
pub async fn versions() -> Versions {
    RealBus.versions().await
}

/// The probe for a process that may run beside the tracker's screen
/// stream: with `share_open`, the 1x1 render runs only on Plasma 6.8.1 or
/// later ([`render_gate`]).
pub async fn probe_beside_share(share_open: bool) -> KwinProbe {
    PROBER.probe(&RealBus, share_open).await.probe
}

/// The probe at daemon or app startup. On KDE, a failure that can pass
/// (KWin not on the bus yet at login, a timeout, no bus) is tried again
/// for up to 15 s ([`STARTUP_RETRY_WAITS`]).
pub async fn probe_at_startup(share_open: impl Fn() -> bool, on_kde: bool) -> KwinProbe {
    probe_at_startup_with(&PROBER, &RealBus, share_open, on_kde).await
}

/// [`probe_at_startup`] with its prober and bus given. Off KDE nothing is
/// tried again: KWin is not coming.
pub async fn probe_at_startup_with<B: ProbeBus>(
    prober: &Prober,
    bus: &B,
    share_open: impl Fn() -> bool,
    on_kde: bool,
) -> KwinProbe {
    let waits: &[Duration] = if on_kde { &STARTUP_RETRY_WAITS } else { &[] };
    probe_with_retries(prober, bus, share_open, waits).await
}

/// KDE direct for Settings without rendering anything: the last probe
/// result while it holds, else what the version read says (not available,
/// or not checked yet).
pub async fn status_without_render() -> KwinProbe {
    if let Some(last) = PROBER.last().await {
        return last;
    }
    status_from_versions(&versions().await)
}

/// [`status_without_render`] when no probe result is kept: unavailable
/// when the interface did not answer or the Plasma is before 6.8.1, else
/// not probed yet.
pub fn status_from_versions(found: &Versions) -> KwinProbe {
    match render_gate(false, found, false) {
        RenderGate::Refused(reason) => KwinProbe::Unavailable(reason),
        RenderGate::Cached | RenderGate::NotChecked(_) | RenderGate::Render => KwinProbe::NotProbed,
    }
}

/// Whether the compare half may grab beside the open stream. It must pass
/// the render gate as a fresh render beside a share would
/// ([`render_gate`] with `share_open` and no kept result): a kept
/// "available" from a probe made with no share open says nothing about
/// grabbing beside one. And the probe must have found KDE direct working.
pub fn compare_half_allowed(found: &Versions, probe: &KwinProbe) -> Result<(), String> {
    match render_gate(true, found, false) {
        RenderGate::Render => {}
        RenderGate::Refused(why) | RenderGate::NotChecked(why) => return Err(why),
        RenderGate::Cached => return Err("no kept result was offered".into()),
    }
    match probe {
        KwinProbe::Available => Ok(()),
        other => Err(format!("KDE direct is not available ({other:?})")),
    }
}
