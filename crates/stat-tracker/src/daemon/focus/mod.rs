//! The focus gate (`capture_only_on_screen`): poll and Tab reads pause
//! while a good read says a window other than Overwatch is focused.
//!
//! It fails open. When the focused window cannot be read (no source for
//! this desktop, an error, a timeout), reads go on and the status is
//! [`FocusGateStatus::Unreadable`], a warning state. Only a read that names
//! another window, or says no window is focused where that is reliable,
//! pauses.
//!
//! Sources, in order: niri IPC when `NIRI_SOCKET` is set ([`niri`]); a
//! KWin script on KDE Plasma Wayland ([`kwin`]); X11 or XWayland when
//! `DISPLAY` is set ([`x11`]); else none. niri lists its XWayland windows
//! (xwayland-satellite) itself, so it stays first. The X11 source says not
//! focused only on a session that is positively X11 ([`DesktopEnv`]); on
//! any other it gives focused or unreadable (see [`x11`]). niri and the
//! KWin script can say not focused from a good read.
//!
//! A pause only skips a read. It never ends, resets or splits a game or a
//! session; the poll arm's caller ([`poll_pause`]) drops the poll's frame
//! stability history on the edge into a pause and nothing else.

use crate::*;
use std::time::Duration;

pub(crate) mod kwin;
pub(crate) mod niri;
pub(crate) mod x11;

pub(crate) use stat_tracker::ocr_status::FocusGateStatus;

/// Window ids that count as Overwatch, matched exactly and ignoring ASCII
/// case: the Battle.net (Wine) build's app id or class, and the Steam
/// (Proton) build's. Not a `steam_app_` prefix: other Steam games must not
/// count. Taken from window rules with the game closed; the value in a
/// live game is not confirmed yet.
pub(crate) const OVERWATCH_WINDOW_IDS: &[&str] = &["overwatch.exe", "steam_app_2357570"];

/// While the focus stays unreadable, at most one warn line this often.
pub(crate) const UNREADABLE_WARN_EVERY: Duration = Duration::from_secs(5 * 60);

/// Longest wait for one focus read, inside the event loop. A slower read
/// counts as unreadable (reads go on). A niri socket round trip, a few X11
/// requests or the KWin script's last report take far less.
pub(crate) const FOCUS_READ_TIMEOUT: Duration = Duration::from_millis(50);

/// One focus read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FocusRead {
    /// The focused window, by its ids (niri's app id, or both `WM_CLASS`
    /// strings). Empty when the window has none.
    Window(Vec<String>),
    /// The source reliably says no window is focused.
    NoWindow,
    /// The focus could not be read, and why.
    Unreadable(String),
}

/// Where the focused window is read from.
pub(crate) trait FocusSource: Send + Sync {
    /// The source's name, for the startup line.
    fn name(&self) -> &'static str;
    /// Read the focused window. Blocking; the caller bounds it.
    fn read(&self) -> FocusRead;
}

/// True when `id` names Overwatch ([`OVERWATCH_WINDOW_IDS`]).
pub(crate) fn matches_overwatch(id: &str) -> bool {
    OVERWATCH_WINDOW_IDS
        .iter()
        .any(|known| id.eq_ignore_ascii_case(known))
}

/// What the gate lets a read do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GateDecision {
    Read,
    Pause,
}

/// The gate's verdict on one read: the status it gives, the decision, and
/// whether a warn line is due ([`UNREADABLE_WARN_EVERY`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Applied {
    pub(crate) status: FocusGateStatus,
    pub(crate) decision: GateDecision,
    pub(crate) warn: bool,
}

/// The gate's state between reads: the last status and the warn clock.
#[derive(Debug)]
pub(crate) struct FocusGate {
    status: FocusGateStatus,
    warned_at: Option<Instant>,
}

impl FocusGate {
    pub(crate) fn new(on: bool) -> Self {
        Self {
            status: if on {
                FocusGateStatus::Unknown
            } else {
                FocusGateStatus::Off
            },
            warned_at: None,
        }
    }

    pub(crate) fn status(&self) -> FocusGateStatus {
        self.status
    }

    /// Apply one read taken at `now`. Unreadable reads go on and warn on
    /// entry, then at most every [`UNREADABLE_WARN_EVERY`]; a good read
    /// (focused or not) clears the warn clock, so the next entry warns at
    /// once.
    pub(crate) fn apply(&mut self, read: &FocusRead, now: Instant) -> Applied {
        let (status, decision) = match read {
            FocusRead::Window(ids) if ids.iter().any(|id| matches_overwatch(id)) => {
                (FocusGateStatus::Focused, GateDecision::Read)
            }
            FocusRead::Window(_) | FocusRead::NoWindow => {
                (FocusGateStatus::NotFocused, GateDecision::Pause)
            }
            FocusRead::Unreadable(_) => (FocusGateStatus::Unreadable, GateDecision::Read),
        };
        let warn = if status == FocusGateStatus::Unreadable {
            let due = self
                .warned_at
                .is_none_or(|at| now.saturating_duration_since(at) >= UNREADABLE_WARN_EVERY);
            if due {
                self.warned_at = Some(now);
            }
            due
        } else {
            self.warned_at = None;
            false
        };
        self.status = status;
        Applied {
            status,
            decision,
            warn,
        }
    }
}

/// The desktop facts that pick a source, read from the environment.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct DesktopEnv {
    pub(crate) niri_socket: Option<std::path::PathBuf>,
    pub(crate) display: bool,
    /// A Wayland signal: `WAYLAND_DISPLAY` set, or `XDG_SESSION_TYPE` is
    /// `wayland`.
    pub(crate) wayland: bool,
    /// The session is positively X11: `XDG_SESSION_TYPE` is `x11`, or it is
    /// unset, with no Wayland signal either way. Only then may the X11
    /// source say not focused.
    pub(crate) x11_session: bool,
    /// `XDG_CURRENT_DESKTOP` names KDE.
    pub(crate) kde: bool,
}

impl DesktopEnv {
    /// From the process environment ([`Self::from_vars`]).
    pub(crate) fn from_env() -> Self {
        Self::from_vars(|key| std::env::var(key).ok())
    }

    /// `NIRI_SOCKET`, `DISPLAY`, `WAYLAND_DISPLAY`, `XDG_SESSION_TYPE` and
    /// `XDG_CURRENT_DESKTOP` as `var` gives them; an empty value is unset.
    pub(crate) fn from_vars(var: impl Fn(&str) -> Option<String>) -> Self {
        let set = |key: &str| var(key).filter(|value| !value.trim().is_empty());
        let session = set("XDG_SESSION_TYPE").map(|s| s.trim().to_ascii_lowercase());
        let wayland_display = set("WAYLAND_DISPLAY").is_some();
        let wayland = wayland_display || session.as_deref() == Some("wayland");
        let x11_session = !wayland && matches!(session.as_deref(), None | Some("x11"));
        Self {
            niri_socket: set("NIRI_SOCKET").map(Into::into),
            display: set("DISPLAY").is_some(),
            wayland,
            x11_session,
            kde: desktop_is_kde(&set("XDG_CURRENT_DESKTOP").unwrap_or_default()),
        }
    }
}

/// `XDG_CURRENT_DESKTOP` (a colon list) names KDE, ignoring case.
pub(crate) fn desktop_is_kde(desktop: &str) -> bool {
    desktop
        .split(':')
        .any(|part| part.trim().eq_ignore_ascii_case("kde"))
}

/// Which source a desktop gets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SourceKind {
    Niri(std::path::PathBuf),
    /// The KWin script, on KDE Plasma Wayland.
    Kwin,
    /// X11. `wayland` unless the session is positively X11
    /// ([`DesktopEnv::x11_session`]): then it never says not focused.
    X11 {
        wayland: bool,
    },
}

/// niri IPC first, then the KWin script on KDE Wayland, then X11 or
/// XWayland, else none (unreadable).
pub(crate) fn source_kind(env: &DesktopEnv) -> Option<SourceKind> {
    if let Some(socket) = &env.niri_socket {
        return Some(SourceKind::Niri(socket.clone()));
    }
    if env.kde && env.wayland {
        return Some(SourceKind::Kwin);
    }
    env.display.then_some(SourceKind::X11 {
        wayland: !env.x11_session,
    })
}

fn make_source(kind: SourceKind, data_dir: &std::path::Path) -> Arc<dyn FocusSource> {
    match kind {
        SourceKind::Niri(socket) => Arc::new(niri::NiriSource::new(socket)),
        SourceKind::Kwin => Arc::new(kwin::start(data_dir)),
        SourceKind::X11 { wayland } => Arc::new(x11::X11Source::new(wayland)),
    }
}

/// The gate as the daemon runs it: the source, the gate state, and where
/// the status is written. Shared by the poll arm and the Tab captures.
pub(crate) struct FocusGuard {
    on: bool,
    source: Option<Arc<dyn FocusSource>>,
    gate: std::sync::Mutex<FocusGate>,
    /// A read still running (a hung source): the next check does not start
    /// another one and counts as unreadable.
    in_flight: Arc<std::sync::atomic::AtomicBool>,
    data_dir: Option<std::path::PathBuf>,
}

impl FocusGuard {
    /// `on` is `capture_only_on_screen`; off, `source` is never read.
    pub(crate) fn new(
        on: bool,
        source: Option<Arc<dyn FocusSource>>,
        data_dir: Option<std::path::PathBuf>,
    ) -> Self {
        Self {
            on,
            source: source.filter(|_| on),
            gate: std::sync::Mutex::new(FocusGate::new(on)),
            in_flight: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            data_dir,
        }
    }

    pub(crate) fn status(&self) -> FocusGateStatus {
        self.gate
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .status()
    }

    /// One focus check at `now`. Off, it reads nothing and always reads on.
    pub(crate) async fn check(&self, now: Instant) -> GateDecision {
        if !self.on {
            return GateDecision::Read;
        }
        let Some(read) = self.read().await else {
            // Another check's read is still running (the Tab capture and
            // the poll overlap, or a hung source): keep the last verdict.
            return decision_of(self.status());
        };
        let mut gate = self.gate.lock().unwrap_or_else(|err| err.into_inner());
        let before = gate.status();
        let applied = gate.apply(&read, now);
        log_read(before, &applied, &read);
        // Under the gate's lock, so status writes land in read order.
        if before != applied.status
            && let Some(dir) = &self.data_dir
        {
            stat_tracker::ocr_status::publish_focus_gate(dir, applied.status);
        }
        applied.decision
    }

    /// One read, or `None` while another read is still running.
    async fn read(&self) -> Option<FocusRead> {
        let Some(source) = self.source.clone() else {
            return Some(FocusRead::Unreadable(
                "no focus source for this desktop".into(),
            ));
        };
        if self
            .in_flight
            .swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            return None;
        }
        // Cleared when the read ends, a panic in it included.
        let done = InFlight(Arc::clone(&self.in_flight));
        let task = tokio::task::spawn_blocking(move || {
            let _done = done;
            source.read()
        });
        Some(match tokio::time::timeout(FOCUS_READ_TIMEOUT, task).await {
            Ok(Ok(read)) => read,
            Ok(Err(err)) => FocusRead::Unreadable(format!("focus read failed: {err}")),
            Err(_) => FocusRead::Unreadable("focus read timed out".into()),
        })
    }
}

/// Clears the in-flight flag when the read that set it ends.
struct InFlight(Arc<std::sync::atomic::AtomicBool>);

impl Drop for InFlight {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::Release);
    }
}

/// The decision a status stands for: only not focused pauses.
fn decision_of(status: FocusGateStatus) -> GateDecision {
    if status == FocusGateStatus::NotFocused {
        GateDecision::Pause
    } else {
        GateDecision::Read
    }
}

fn log_read(before: FocusGateStatus, applied: &Applied, read: &FocusRead) {
    if applied.warn
        && let FocusRead::Unreadable(why) = read
    {
        tracing::warn!(
            target: "scuffed_stat_tracker",
            reason = %why,
            "focus gate: the focused window cannot be read, reads go on (status unreadable; repeats are logged at most every 5 minutes)"
        );
    }
    if before == applied.status {
        return;
    }
    match applied.status {
        FocusGateStatus::Focused => {
            tracing::info!(target: "scuffed_stat_tracker", "focus gate: Overwatch is focused, reads go on");
        }
        FocusGateStatus::NotFocused => {
            tracing::info!(target: "scuffed_stat_tracker", "focus gate: another window is focused, reads pause");
        }
        _ => {}
    }
}

static GUARD: std::sync::OnceLock<FocusGuard> = std::sync::OnceLock::new();

/// Set up the daemon's gate once at startup, log which source it uses and
/// write its first status. A second call keeps the first gate.
pub(crate) fn install(on: bool, data_dir: &std::path::Path) {
    let kind = on.then(|| source_kind(&DesktopEnv::from_env())).flatten();
    let source = kind.map(|kind| make_source(kind, data_dir));
    match (on, &source) {
        (false, _) => {
            tracing::info!(target: "scuffed_stat_tracker", "focus gate off (capture_only_on_screen = false): reads go on whatever window is focused");
        }
        (true, Some(source)) => {
            tracing::info!(target: "scuffed_stat_tracker", source = source.name(), "focus gate on: reads pause while another window is focused");
        }
        (true, None) => {
            tracing::info!(target: "scuffed_stat_tracker", "focus gate on, but no focus source for this desktop (no NIRI_SOCKET, not KDE Wayland, no DISPLAY): reads go on, status unreadable");
        }
    }
    let guard = FocusGuard::new(on, source, Some(data_dir.to_path_buf()));
    stat_tracker::ocr_status::publish_focus_gate(data_dir, guard.status());
    let _ = GUARD.set(guard);
}

/// Undo what the gate set up outside the daemon (the KWin script).
pub(crate) async fn shutdown() {
    kwin::shutdown().await;
}

/// The daemon's focus check. With no gate set up (tests), reads go on.
pub(crate) async fn check(now: Instant) -> GateDecision {
    check_with(GUARD.get(), now).await
}

/// [`check`] on a given gate, `None` meaning none is set up.
pub(crate) async fn check_with(guard: Option<&FocusGuard>, now: Instant) -> GateDecision {
    match guard {
        Some(guard) => guard.check(now).await,
        None => GateDecision::Read,
    }
}

/// The poll arm's focus step: check, then [`poll_pause`]. True when this
/// tick reads nothing (another window is focused).
pub(crate) async fn poll_skip(paused: &mut bool, st: &mut SessionState, now: Instant) -> bool {
    poll_skip_with(GUARD.get(), paused, st, now).await
}

/// [`poll_skip`] on a given gate.
pub(crate) async fn poll_skip_with(
    guard: Option<&FocusGuard>,
    paused: &mut bool,
    st: &mut SessionState,
    now: Instant,
) -> bool {
    let decision = check_with(guard, now).await;
    poll_pause(paused, decision, st)
}

/// The Tab arm's and each Tab attempt's focus step: true when the press or
/// attempt is skipped (another window is focused).
pub(crate) async fn tab_skip(now: Instant) -> bool {
    check(now).await == GateDecision::Pause
}

/// The poll arm's pause, given the focus `decision` for this tick. True
/// when the tick reads nothing. `paused` is the poll's own edge: on the
/// edge into a pause the frame stability history is dropped, so frames on
/// either side of an alt-tab never count as consecutive. Nothing else in
/// the session is touched: no game, timer, wake, streak or held state.
pub(crate) fn poll_pause(paused: &mut bool, decision: GateDecision, st: &mut SessionState) -> bool {
    match decision {
        GateDecision::Read => {
            *paused = false;
            false
        }
        GateDecision::Pause => {
            if !*paused {
                st.ocr_stability.reset();
                *paused = true;
            }
            true
        }
    }
}
