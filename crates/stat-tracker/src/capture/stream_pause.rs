//! Pause the screen stream between grabs.
//!
//! The portal stream runs only while something holds it: a grab in
//! [`super::screencast`], or a Tab press from its key event until its retry
//! sequence ends. With no hold it is set inactive, so the compositor stops
//! rendering frames the tracker would only throw away. Polls and Tabs do not
//! grab while Overwatch is not running, so an open share then stays paused.
//!
//! A Tab press takes a key hold before the session choice and the 300 ms
//! overlay delay, so the compositor has that time to resume. The Tab task
//! keeps it through its retries ([`with_tab_stream_hold`],
//! [`end_tab_stream_hold`]): each resume costs the compositor a rebuild (on
//! KWin 6.8 the layer and the filtered scene view), so a retry 300 ms later
//! does not pause and resume in between. A Tab that preempts a poll takes a
//! bridge hold before the poll's grab is dropped ([`bridge_hold`]).
//!
//! The resume itself starts at [`tab_resume_at`]: `300 ms - one frame
//! period` after the press (50 ms at 4 fps, at once at 3 fps or less).
//! niri and KWin before 6.8 send a frame right after a resume and the next
//! one a frame period later. Resumed at the press, that second frame would
//! land at about 260 ms, inside the overlay delay, and the Tab would wait for
//! the third (about 510 ms). Resumed 50 ms later the second frame lands just
//! after 300 ms. KWin 6.8 paces on a fixed clock grid, so the start does not
//! move its frames. A stream that is already running resumes nothing, and
//! the hold starts at once.
//!
//! Where pausing is on by default: KWin 6.1 or newer, and niri
//! ([`decide`]). Not on GNOME (Mutter connects a `monitors-changed-internal`
//! handler on every resume and never disconnects it), Plasma 5 or KWin 6.0
//! (KWin connects `outputChange` on every Streaming transition), wlroots
//! portals, or anything not recognised. `screencast_pause_between_grabs =
//! false` turns it off everywhere; `STAT_TRACKER_SCREENCAST_PAUSE=1` forces
//! it on (for testing) and `=0` off. The per-stream fail-safe in
//! [`super::pw`] turns it off for one stream when a resume is not answered,
//! and after [`pw::PAUSE_TRIPS_LIMIT`] tripped streams for the rest of the
//! process. With pausing off for a stream, no hold starts a timer or calls
//! PipeWire.

use std::cell::RefCell;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use crate::config::PauseOverride;

use super::framebuf::TAB_OVERLAY_AFTER_PRESS_MS;
use super::pw;

#[derive(Debug, Default)]
struct Ctl {
    /// The startup decision ([`decide`]).
    enabled: bool,
    /// The compositor is niri: Streaming again answers a resume.
    streaming_answers: bool,
    holds: usize,
    current: Option<Weak<pw::Handle>>,
    limit_logged: bool,
}

impl Ctl {
    /// The stream should run.
    fn wanted(&self) -> bool {
        !self.enabled || self.holds > 0
    }

    /// The live stream, when it is paused between grabs. Holds on
    /// anything else do nothing. The caller drops the reference after
    /// unlocking (see [`Self::apply`]).
    #[must_use]
    fn pausing_stream(&self) -> Option<Arc<pw::Handle>> {
        if !self.enabled {
            return None;
        }
        let handle = self.current.as_ref().and_then(Weak::upgrade)?;
        handle.pausing().then_some(handle)
    }

    /// Pass the wanted state to the live stream. Returns the strong
    /// reference so the caller drops it after unlocking: if a retired
    /// stream's last reference ends here, its Drop joins the PipeWire
    /// thread, which must not happen under this lock.
    #[must_use]
    fn apply(&self) -> Option<Arc<pw::Handle>> {
        let handle = self.current.as_ref().and_then(Weak::upgrade)?;
        handle.set_active(self.wanted());
        Some(handle)
    }

    /// `trips` is how many streams have tripped the fail-safe so far.
    fn register(&mut self, handle: &Arc<pw::Handle>, trips: u32) {
        if self.enabled {
            if trips < pw::PAUSE_TRIPS_LIMIT {
                if !handle.enable_pausing(self.streaming_answers) && !self.limit_logged {
                    self.limit_logged = true;
                    tracing::info!(
                        "screencast pause between grabs is off: it needs PipeWire 1.4 or newer"
                    );
                }
            } else if !self.limit_logged {
                self.limit_logged = true;
                tracing::info!(
                    "screencast pause between grabs stays off: the fail-safe tripped on two streams"
                );
            }
        }
        self.current = Some(Arc::downgrade(handle));
        // The caller holds `handle`, so this is never the last reference.
        drop(self.apply());
    }
}

static CTL: Mutex<Ctl> = Mutex::new(Ctl {
    enabled: false,
    streaming_answers: false,
    holds: 0,
    current: None,
    limit_logged: false,
});

fn ctl(slot: &Mutex<Ctl>) -> std::sync::MutexGuard<'_, Ctl> {
    slot.lock().unwrap_or_else(|err| err.into_inner())
}

/// Keeps the stream running while it lives.
#[must_use = "the stream pauses again when the hold is dropped"]
pub struct StreamHold {
    slot: &'static Mutex<Ctl>,
}

impl Drop for StreamHold {
    fn drop(&mut self) {
        let handle = {
            let mut ctl = ctl(self.slot);
            ctl.holds = ctl.holds.saturating_sub(1);
            ctl.apply()
        };
        drop(handle);
    }
}

fn hold_in(slot: &'static Mutex<Ctl>) -> StreamHold {
    let handle = {
        let mut ctl = ctl(slot);
        ctl.holds += 1;
        ctl.apply()
    };
    drop(handle);
    StreamHold { slot }
}

/// The startup decision. Read when a stream is registered and by holds.
/// `streaming_answers`: the compositor is niri.
fn set_enabled_in(slot: &Mutex<Ctl>, enabled: bool, streaming_answers: bool) {
    let handle = {
        let mut ctl = ctl(slot);
        ctl.enabled = enabled;
        ctl.streaming_answers = streaming_answers;
        ctl.apply()
    };
    drop(handle);
}

/// When a Tab press resumes a paused stream: one frame period before the
/// 300 ms overlay delay ends, and never before the press.
pub fn tab_resume_at(press: Instant, max_fps: u32) -> Instant {
    let lead = TAB_OVERLAY_AFTER_PRESS_MS.saturating_sub(pw::throttle_gap_ms(max_fps));
    press + Duration::from_millis(lead)
}

/// A hold for a Tab press. It starts at [`tab_resume_at`], or at once when
/// the stream is already running. Dropping it before then cancels it. On a
/// route or stream that does not pause it is inert: no timer, no PipeWire.
#[must_use = "the stream pauses again when the hold is dropped"]
pub struct KeyHold {
    slot: Option<Arc<Mutex<KeySlot>>>,
    timer: Option<tokio::task::JoinHandle<()>>,
}

#[derive(Default)]
struct KeySlot {
    dropped: bool,
    hold: Option<StreamHold>,
}

impl KeyHold {
    fn inert() -> Self {
        Self {
            slot: None,
            timer: None,
        }
    }

    #[cfg(test)]
    fn is_inert(&self) -> bool {
        self.slot.is_none()
    }
}

impl Drop for KeyHold {
    fn drop(&mut self) {
        if let Some(timer) = self.timer.take() {
            timer.abort();
        }
        let Some(slot) = self.slot.take() else {
            return;
        };
        let hold = {
            let mut slot = slot.lock().unwrap_or_else(|err| err.into_inner());
            slot.dropped = true;
            slot.hold.take()
        };
        drop(hold);
    }
}

fn key_hold_in(slot: &'static Mutex<Ctl>, at: Instant, now: Instant) -> KeyHold {
    let (stream, running) = {
        let ctl = ctl(slot);
        (ctl.pausing_stream(), ctl.holds > 0)
    };
    // Dropped here, outside the lock.
    let pausing = stream.is_some();
    drop(stream);
    if !pausing {
        return KeyHold::inert();
    }
    let key = Arc::new(Mutex::new(KeySlot::default()));
    if at <= now || running {
        key.lock().unwrap_or_else(|err| err.into_inner()).hold = Some(hold_in(slot));
        return KeyHold {
            slot: Some(key),
            timer: None,
        };
    }
    let later = Arc::clone(&key);
    let timer = tokio::spawn(async move {
        tokio::time::sleep_until(tokio::time::Instant::from_std(at)).await;
        let mut key = later.lock().unwrap_or_else(|err| err.into_inner());
        if !key.dropped {
            key.hold = Some(hold_in(slot));
        }
    });
    KeyHold {
        slot: Some(key),
        timer: Some(timer),
    }
}

/// A Tab key went down while Overwatch runs: resume the stream at
/// [`tab_resume_at`]. The Tab task keeps it until its retries end. Needs a
/// Tokio runtime when the stream pauses.
pub fn key_hold(press: Instant) -> KeyHold {
    let at = tab_resume_at(press, super::prefs().max_fps);
    key_hold_in(&CTL, at, Instant::now())
}

/// Taken as a key event preempts a poll, before the poll's grab (and its
/// hold) is dropped, so the stream does not pause and resume in between.
/// Drop it once the press has its own [`KeyHold`].
pub fn bridge_hold() -> KeyHold {
    key_hold_in(&CTL, Instant::now(), Instant::now())
}

tokio::task_local! {
    static TAB_STREAM_HOLD: RefCell<Option<KeyHold>>;
}

/// Run a Tab capture task with its press's hold. The hold ends at
/// [`end_tab_stream_hold`], or when `fut` ends.
pub async fn with_tab_stream_hold<T>(
    hold: KeyHold,
    fut: impl std::future::Future<Output = T>,
) -> T {
    TAB_STREAM_HOLD.scope(RefCell::new(Some(hold)), fut).await
}

/// The Tab retry sequence is over: let the stream pause. Returns whether a
/// hold was released (none outside a Tab task, or the second time).
pub fn end_tab_stream_hold() -> bool {
    let hold = TAB_STREAM_HOLD
        .try_with(|cell| cell.borrow_mut().take())
        .ok()
        .flatten();
    let released = hold.is_some();
    drop(hold);
    released
}

/// Resume the stream for one grab. A Tab grab whose stream is paused waits
/// for [`tab_resume_at`] first (at most one frame period short of 300 ms,
/// inside the wait the grab does anyway). Returns whether the grab resumed a
/// paused stream, and the hold, which the caller drops after its post-grab
/// health read.
pub(super) async fn prepare_grab(
    stream: &pw::Handle,
    tab_press: Option<Instant>,
    max_fps: u32,
) -> (bool, StreamHold) {
    prepare_grab_in(&CTL, stream, tab_press, max_fps).await
}

/// The instant a grab has to wait for before it resumes the stream, if any.
fn grab_resume_wait(
    resuming: bool,
    tab_press: Option<Instant>,
    max_fps: u32,
    now: Instant,
) -> Option<Instant> {
    let at = tab_resume_at(tab_press?, max_fps);
    (resuming && at > now).then_some(at)
}

async fn prepare_grab_in(
    slot: &'static Mutex<Ctl>,
    stream: &pw::Handle,
    tab_press: Option<Instant>,
    max_fps: u32,
) -> (bool, StreamHold) {
    let mut resuming = !stream.wants_active();
    if let Some(at) = grab_resume_wait(resuming, tab_press, max_fps, Instant::now()) {
        tokio::time::sleep_until(tokio::time::Instant::from_std(at)).await;
        resuming = !stream.wants_active();
    }
    (resuming, hold_in(slot))
}

/// The live stream changed. It starts running, and pauses at once unless
/// something holds it.
pub(super) fn register(handle: &Arc<pw::Handle>) {
    ctl(&CTL).register(handle, pw::pause_trips());
}

// ---------------------------------------------------------------- decision

/// The compositor behind the portal, as far as the session tells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compositor {
    KWin,
    Niri,
    Gnome,
    Other,
}

/// What [`decide`] reads. Gathered once at startup ([`detect_facts`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DesktopFacts {
    /// `XDG_CURRENT_DESKTOP`.
    pub desktop: String,
    /// `KDE_SESSION_VERSION`.
    pub kde_session_version: Option<String>,
    /// `NIRI_SOCKET` is set.
    pub niri_socket: bool,
    /// Compositor processes this user runs.
    pub niri_running: bool,
    pub kwin_running: bool,
    pub gnome_shell_running: bool,
    /// KWin's own version, when it could be read.
    pub kwin_version: Option<(u32, u32)>,
}

fn desktop_has(desktop: &str, names: &[&str]) -> bool {
    desktop.split(':').any(|token| {
        let token = token.trim().to_ascii_lowercase();
        names.iter().any(|name| token.contains(name))
    })
}

/// The desktop name first; the running compositor only when the name says
/// nothing this module knows.
pub fn compositor(facts: &DesktopFacts) -> Compositor {
    let desktop = facts.desktop.as_str();
    if desktop_has(desktop, &["kde", "plasma", "kwin"]) {
        return Compositor::KWin;
    }
    if desktop_has(desktop, &["niri"]) {
        return Compositor::Niri;
    }
    if desktop_has(desktop, &["gnome"]) {
        return Compositor::Gnome;
    }
    if desktop_has(
        desktop,
        &[
            "sway", "hyprland", "wayfire", "river", "labwc", "wlroots", "cosmic",
        ],
    ) {
        return Compositor::Other;
    }
    if facts.niri_socket || facts.niri_running {
        Compositor::Niri
    } else if facts.kwin_running {
        Compositor::KWin
    } else if facts.gnome_shell_running {
        Compositor::Gnome
    } else {
        Compositor::Other
    }
}

/// KWin older than this connects `outputChange` again on every Streaming
/// transition (5.27 and 6.0), or does not stop rendering on Paused.
pub const KWIN_MIN: (u32, u32) = (6, 1);

/// Whether the stream pauses between grabs, and why, for the one startup
/// line.
pub fn decide(over: PauseOverride, facts: &DesktopFacts) -> (bool, &'static str) {
    match over {
        PauseOverride::EnvOn => return (true, "forced on by STAT_TRACKER_SCREENCAST_PAUSE"),
        PauseOverride::EnvOff => return (false, "turned off by STAT_TRACKER_SCREENCAST_PAUSE"),
        PauseOverride::ConfigOff => {
            return (
                false,
                "turned off by screencast_pause_between_grabs = false",
            );
        }
        PauseOverride::Auto => {}
    }
    match compositor(facts) {
        Compositor::KWin => {
            if facts.kde_session_version.as_deref().map(str::trim) == Some("5") {
                return (
                    false,
                    "off on Plasma 5: KWin 5 leaks a connection per resume",
                );
            }
            match facts.kwin_version {
                Some(version) if version >= KWIN_MIN => (true, "on: KWin 6.1 or newer"),
                Some(_) => (false, "off: KWin is older than 6.1"),
                None => (false, "off: the KWin version could not be read"),
            }
        }
        Compositor::Niri => (true, "on: niri"),
        Compositor::Gnome => (false, "off on GNOME: Mutter leaks a handler per resume"),
        Compositor::Other => (false, "off: compositor not recognised"),
    }
}

/// `KWin version: 6.8.1` from KWin's support information, or `kwin 6.8.1`
/// from `kwin_wayland --version`.
pub fn parse_kwin_version(text: &str) -> Option<(u32, u32)> {
    let rest = text.lines().find_map(|line| {
        let line = line.trim();
        line.strip_prefix("KWin version:")
            .or_else(|| line.strip_prefix("kwin_wayland "))
            .or_else(|| line.strip_prefix("kwin "))
    })?;
    let mut parts = rest.trim().split(['.', ' ', '-']);
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    Some((major, minor))
}

/// Compositor processes in `proc_root` (comm names), at most `limit`
/// entries looked at.
fn scan_processes(proc_root: &std::path::Path, limit: usize) -> (bool, bool, bool) {
    let (mut niri, mut kwin, mut gnome) = (false, false, false);
    let Ok(dir) = std::fs::read_dir(proc_root) else {
        return (niri, kwin, gnome);
    };
    for entry in dir.flatten().take(limit) {
        let name = entry.file_name();
        if !name.to_string_lossy().bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let Ok(comm) = std::fs::read_to_string(entry.path().join("comm")) else {
            continue;
        };
        match comm.trim() {
            "niri" => niri = true,
            "kwin_wayland" | "kwin_wayland_wr" | "kwin_x11" => kwin = true,
            "gnome-shell" => gnome = true,
            _ => {}
        }
    }
    (niri, kwin, gnome)
}

async fn kwin_version_over_dbus() -> Option<(u32, u32)> {
    let conn = ashpd::zbus::Connection::session().await.ok()?;
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
}

async fn kwin_version_from_binary() -> Option<(u32, u32)> {
    let out = tokio::process::Command::new("kwin_wayland")
        .arg("--version")
        .kill_on_drop(true)
        .output()
        .await
        .ok()?;
    parse_kwin_version(&String::from_utf8_lossy(&out.stdout))
}

/// Read the session once. KWin's version is asked only when the compositor
/// is KWin ([`kwin_version`]).
pub async fn detect_facts() -> DesktopFacts {
    let mut facts = detect_facts_without_kwin().await;
    if compositor(&facts) == Compositor::KWin {
        facts.kwin_version = kwin_version(facts.kde_session_version.as_deref()).await;
    }
    facts
}

/// The env vars and the compositor process scan.
async fn detect_facts_without_kwin() -> DesktopFacts {
    let var = |key: &str| std::env::var(key).ok().filter(|v| !v.trim().is_empty());
    let (niri_running, kwin_running, gnome_shell_running) =
        tokio::task::spawn_blocking(|| scan_processes(std::path::Path::new("/proc"), 100_000))
            .await
            .unwrap_or_default();
    DesktopFacts {
        desktop: var("XDG_CURRENT_DESKTOP").unwrap_or_default(),
        kde_session_version: var("KDE_SESSION_VERSION"),
        niri_socket: var("NIRI_SOCKET").is_some(),
        niri_running,
        kwin_running,
        gnome_shell_running,
        kwin_version: None,
    }
}

/// The running KWin's version. D-Bus first; on Plasma 6, where KWin may
/// still be starting, D-Bus once more after [`KWIN_DBUS_RETRY`]. Only then
/// `kwin_wayland --version`, which reads the installed binary: after an
/// upgrade without a new login that can be newer than the running KWin.
async fn kwin_version(kde_session_version: Option<&str>) -> Option<(u32, u32)> {
    let limit = Duration::from_secs(2);
    kwin_version_with(
        kde_session_version,
        KWIN_DBUS_RETRY,
        || async {
            tokio::time::timeout(limit, kwin_version_over_dbus())
                .await
                .ok()
                .flatten()
        },
        || async {
            tokio::time::timeout(limit, kwin_version_from_binary())
                .await
                .ok()
                .flatten()
        },
    )
    .await
}

/// [`kwin_version`] with its two sources passed in.
async fn kwin_version_with<D, DF, B, BF>(
    kde_session_version: Option<&str>,
    retry: Duration,
    mut dbus: D,
    binary: B,
) -> Option<(u32, u32)>
where
    D: FnMut() -> DF,
    DF: std::future::Future<Output = Option<(u32, u32)>>,
    B: FnOnce() -> BF,
    BF: std::future::Future<Output = Option<(u32, u32)>>,
{
    if let Some(version) = dbus().await {
        return Some(version);
    }
    if kde_session_version.map(str::trim) == Some("6") {
        tokio::time::sleep(retry).await;
        if let Some(version) = dbus().await {
            return Some(version);
        }
    }
    binary().await
}

/// Wait before the second D-Bus read of KWin's version.
const KWIN_DBUS_RETRY: Duration = Duration::from_secs(3);

/// Decide once at startup, log one line, and apply it. Returns the
/// decision.
pub async fn decide_and_apply(over: PauseOverride) -> bool {
    decide_and_apply_in(&CTL, over, detect_facts, detect_facts_without_kwin).await
}

/// What the startup decision applies: pausing on or off, and whether
/// Streaming answers a resume (niri, whether pausing is automatic or
/// forced on).
pub fn applied_decision(over: PauseOverride, facts: &DesktopFacts) -> (bool, bool, &'static str) {
    let (on, why) = decide(over, facts);
    (on, compositor(facts) == Compositor::Niri, why)
}

/// [`decide_and_apply`] with the control slot and the two session readers
/// passed in. The session is read even when the env var or config decides,
/// so a forced-on niri still counts Streaming as an answer; KWin's version
/// (`full`) is read only for Auto.
async fn decide_and_apply_in<F, FF, B, BF>(
    slot: &Mutex<Ctl>,
    over: PauseOverride,
    full: F,
    basic: B,
) -> bool
where
    F: FnOnce() -> FF,
    FF: std::future::Future<Output = DesktopFacts>,
    B: FnOnce() -> BF,
    BF: std::future::Future<Output = DesktopFacts>,
{
    let facts = if over == PauseOverride::Auto {
        full().await
    } else {
        basic().await
    };
    let (on, streaming_answers, why) = applied_decision(over, &facts);
    tracing::info!(
        pause_between_grabs = on,
        compositor = ?compositor(&facts),
        kwin_version = ?facts.kwin_version,
        "screencast stream pause between grabs: {why}"
    );
    set_enabled_in(slot, on, streaming_answers);
    on
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leak_ctl(enabled: bool) -> &'static Mutex<Ctl> {
        Box::leak(Box::new(Mutex::new(Ctl {
            enabled,
            ..Ctl::default()
        })))
    }

    fn live(slot: &'static Mutex<Ctl>) -> Arc<pw::Handle> {
        let handle = Arc::new(pw::test_stream(true));
        ctl(slot).register(&handle, 0);
        handle
    }

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
    }

    #[test]
    fn a_registered_stream_pauses_with_no_hold_and_runs_while_held() {
        let slot = leak_ctl(true);
        let stream = live(slot);
        assert!(!stream.wants_active(), "nothing holds it: paused");
        let poll = hold_in(slot);
        assert!(stream.wants_active());
        drop(poll);
        assert!(!stream.wants_active(), "the grab is done: paused again");
    }

    #[test]
    fn a_tab_that_preempts_a_poll_never_pauses_in_between() {
        let slot = leak_ctl(true);
        let stream = live(slot);
        let poll = hold_in(slot);
        // The key future takes the bridge before the poll's grab is dropped.
        let bridge = key_hold_in(slot, Instant::now(), Instant::now());
        drop(poll);
        assert!(stream.wants_active(), "the bridge still runs it");
        // The keyboard arm then takes the press's hold. The stream is running,
        // so it starts at once instead of at tab_resume_at.
        let press = Instant::now();
        let key = key_hold_in(slot, press + Duration::from_secs(60), press);
        assert!(!key.is_inert() && key.timer.is_none());
        drop(bridge);
        assert!(stream.wants_active());
        drop(key);
        assert!(!stream.wants_active());
    }

    #[test]
    fn a_tab_keeps_the_stream_running_through_its_retries() {
        let slot = leak_ctl(true);
        let stream = live(slot);
        let key = key_hold_in(slot, Instant::now(), Instant::now());
        // First attempt, then a retry 300 ms later: two grabs.
        for _ in 0..2 {
            let grab = hold_in(slot);
            drop(grab);
            assert!(stream.wants_active(), "no pause between Tab attempts");
        }
        drop(key);
        assert!(!stream.wants_active(), "the retry sequence is over");
    }

    #[test]
    fn the_tab_hold_ends_with_the_retry_sequence_not_the_task() {
        let slot = leak_ctl(true);
        let stream = live(slot);
        rt().block_on(async {
            let key = key_hold_in(slot, Instant::now(), Instant::now());
            assert!(!end_tab_stream_hold(), "outside a Tab task: nothing");
            with_tab_stream_hold(key, async {
                assert!(stream.wants_active());
                assert!(end_tab_stream_hold());
                assert!(!stream.wants_active(), "paused while OCR goes on");
                assert!(!end_tab_stream_hold(), "only once");
            })
            .await;
            // A task that ends without the call still releases it.
            let key = key_hold_in(slot, Instant::now(), Instant::now());
            with_tab_stream_hold(key, async {
                assert!(stream.wants_active());
            })
            .await;
            assert!(!stream.wants_active());
        });
    }

    #[test]
    fn a_grab_that_gives_up_releases_its_hold() {
        let slot = leak_ctl(true);
        let stream = live(slot);
        let outcome: Result<(), &str> = {
            let _grab = hold_in(slot);
            Err("Tab grab: no fresh stream frame")
        };
        assert!(outcome.is_err());
        assert!(!stream.wants_active(), "a miss pauses like a frame does");
    }

    #[test]
    fn with_the_game_gone_nothing_holds_and_the_stream_stays_paused() {
        let slot = leak_ctl(true);
        let stream = live(slot);
        {
            let _poll = hold_in(slot);
        }
        // Polls and Tabs do not grab while Overwatch is not running, so no
        // hold is taken and the open share stays paused.
        assert!(!stream.wants_active());
        assert_eq!(ctl(slot).holds, 0);
    }

    #[test]
    fn switched_off_the_stream_always_runs_and_key_holds_are_inert() {
        let slot = leak_ctl(false);
        let stream = live(slot);
        assert!(stream.wants_active());
        drop(hold_in(slot));
        assert!(stream.wants_active(), "pausing off: rc.6 behaviour");
        let now = Instant::now();
        let key = key_hold_in(slot, now + Duration::from_secs(1), now);
        assert!(key.is_inert() && key.timer.is_none(), "no timer task");
    }

    #[test]
    fn key_holds_are_inert_without_a_pausing_stream() {
        let now = Instant::now();
        // No stream registered (another route, or the share is opening).
        let empty = leak_ctl(true);
        assert!(key_hold_in(empty, now + Duration::from_secs(1), now).is_inert());
        // A stream whose fail-safe tripped.
        let slot = leak_ctl(true);
        let stream = live(slot);
        let trip = test_trip(&stream);
        assert!(trip);
        assert!(key_hold_in(slot, now + Duration::from_secs(1), now).is_inert());
        // A stream on PipeWire older than 1.4 is never managed.
        let old = leak_ctl(true);
        let handle = Arc::new(pw::test_stream_without_invoke(true));
        ctl(old).register(&handle, 0);
        assert!(key_hold_in(old, now + Duration::from_secs(1), now).is_inert());
        assert!(handle.wants_active());
    }

    fn test_trip(stream: &pw::Handle) -> bool {
        stream.trip_for_test();
        stream.pause_tripped()
    }

    #[test]
    fn a_stream_that_has_not_streamed_is_not_paused() {
        let slot = leak_ctl(true);
        let handle = Arc::new(pw::test_stream(false));
        ctl(slot).register(&handle, 0);
        assert!(handle.wants_active(), "a pause before Streaming is ignored");
    }

    #[test]
    fn a_new_stream_replaces_the_old_one() {
        let slot = leak_ctl(true);
        let old = live(slot);
        let new = live(slot);
        let poll = hold_in(slot);
        assert!(new.wants_active());
        assert!(!old.wants_active(), "the retired stream is not touched");
        drop(poll);
        assert!(!new.wants_active());
    }

    #[test]
    fn after_two_tripped_streams_a_new_stream_is_not_paused() {
        let slot = leak_ctl(true);
        let once = Arc::new(pw::test_stream(true));
        ctl(slot).register(&once, 1);
        assert!(
            !once.wants_active(),
            "one trip: the next stream tries again"
        );
        let twice = Arc::new(pw::test_stream(true));
        ctl(slot).register(&twice, pw::PAUSE_TRIPS_LIMIT);
        assert!(twice.wants_active(), "two trips: pausing stays off");
        drop(hold_in(slot));
        assert!(twice.wants_active());
    }

    #[test]
    fn a_tab_resumes_a_frame_period_before_the_overlay_delay() {
        let press = Instant::now();
        assert_eq!(tab_resume_at(press, 4), press + Duration::from_millis(50));
        assert_eq!(tab_resume_at(press, 5), press + Duration::from_millis(100));
        assert_eq!(tab_resume_at(press, 3), press + Duration::from_millis(0));
        assert_eq!(tab_resume_at(press, 2), press);
        assert_eq!(tab_resume_at(press, 1), press);
        // A compositor that sends a frame at once on resume and then one
        // per frame period: resumed at the press, the second frame is inside
        // the 300 ms overlay delay; resumed at tab_resume_at it is not.
        for fps in 1..=5 {
            let gap = pw::throttle_gap_ms(fps);
            let lead = tab_resume_at(press, fps).duration_since(press).as_millis() as u64;
            let first_ready = |start: u64| {
                (0..10)
                    .map(|k| start + 5 + k * gap)
                    .find(|at| *at >= TAB_OVERLAY_AFTER_PRESS_MS)
                    .unwrap()
            };
            assert!(first_ready(lead) <= first_ready(0), "{fps} fps");
            assert!(first_ready(lead) < 300 + gap, "{fps} fps");
        }
        let at_press = 5 + 2 * pw::throttle_gap_ms(4);
        assert_eq!(at_press, 505, "4 fps resumed at the press: the third frame");
    }

    #[test]
    fn a_grab_waits_for_tab_resume_at_only_when_it_resumes_a_tab() {
        let press = Instant::now();
        let at = press + Duration::from_millis(50);
        assert_eq!(grab_resume_wait(true, Some(press), 4, press), Some(at));
        assert_eq!(
            grab_resume_wait(false, Some(press), 4, press),
            None,
            "running"
        );
        assert_eq!(grab_resume_wait(true, None, 4, press), None, "a poll");
        assert_eq!(
            grab_resume_wait(true, Some(press), 4, at),
            None,
            "already due"
        );
        assert_eq!(grab_resume_wait(true, Some(press), 2, press), None, "2 fps");
    }

    #[test]
    fn a_tab_grab_on_a_paused_stream_resumes_at_tab_resume_at() {
        let slot = leak_ctl(true);
        let stream = live(slot);
        rt().block_on(async {
            let press = Instant::now();
            let (resuming, hold) = prepare_grab_in(slot, &stream, Some(press), 4).await;
            assert!(resuming);
            assert!(
                press.elapsed() >= Duration::from_millis(50),
                "waited for tab_resume_at"
            );
            assert!(stream.wants_active());
            drop(hold);
            assert!(!stream.wants_active());
            // A poll resumes at once.
            let started = Instant::now();
            let (resuming, hold) = prepare_grab_in(slot, &stream, None, 4).await;
            assert!(resuming && started.elapsed() < Duration::from_millis(40));
            // A Tab on a stream already running does not wait.
            let started = Instant::now();
            let (resuming, again) = prepare_grab_in(slot, &stream, Some(started), 4).await;
            assert!(!resuming && started.elapsed() < Duration::from_millis(40));
            drop((hold, again));
        });
    }

    #[test]
    fn a_key_hold_starts_at_its_resume_time_and_not_before() {
        let slot = leak_ctl(true);
        let stream = live(slot);
        rt().block_on(async {
            let now = Instant::now();
            let key = key_hold_in(slot, now + Duration::from_millis(60), now);
            tokio::time::sleep(Duration::from_millis(20)).await;
            assert!(!stream.wants_active(), "not yet");
            tokio::time::sleep(Duration::from_millis(120)).await;
            assert!(stream.wants_active(), "resumed at its time");
            drop(key);
            assert!(!stream.wants_active());
        });
    }

    #[test]
    fn a_key_hold_dropped_before_its_time_never_starts() {
        let slot = leak_ctl(true);
        let stream = live(slot);
        rt().block_on(async {
            let now = Instant::now();
            let key = key_hold_in(slot, now + Duration::from_millis(40), now);
            drop(key);
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert!(
                !stream.wants_active(),
                "a dropped hold does not start later"
            );
            assert_eq!(ctl(slot).holds, 0);
        });
    }

    fn facts(desktop: &str) -> DesktopFacts {
        DesktopFacts {
            desktop: desktop.into(),
            ..DesktopFacts::default()
        }
    }

    #[test]
    fn the_auto_decision_table() {
        let kde = |session: Option<&str>, version: Option<(u32, u32)>| DesktopFacts {
            kde_session_version: session.map(str::to_string),
            kwin_version: version,
            ..facts("KDE")
        };
        let auto = |f: &DesktopFacts| decide(PauseOverride::Auto, f).0;
        // KWin: 6.1 and newer only, Plasma 5 never, unknown off.
        assert!(auto(&kde(Some("6"), Some((6, 1)))));
        assert!(auto(&kde(Some("6"), Some((6, 8)))));
        assert!(auto(&kde(None, Some((7, 0)))));
        assert!(!auto(&kde(Some("6"), Some((6, 0)))));
        assert!(!auto(&kde(Some("5"), Some((5, 27)))));
        assert!(
            !auto(&kde(Some("5"), Some((6, 4)))),
            "Plasma 5 session wins"
        );
        assert!(!auto(&kde(Some("6"), None)), "version unknown");
        // niri on; GNOME, wlroots and anything else off.
        assert!(auto(&facts("niri")));
        assert!(!auto(&facts("GNOME")));
        assert!(!auto(&facts("ubuntu:GNOME")));
        for other in ["sway", "Hyprland", "wlroots", "COSMIC", "X-Cinnamon", ""] {
            assert!(!auto(&facts(other)), "{other}");
        }
        // Overrides.
        for f in [
            kde(Some("5"), None),
            facts("GNOME"),
            facts("sway"),
            facts(""),
        ] {
            assert!(decide(PauseOverride::EnvOn, &f).0, "env 1 forces on");
        }
        for f in [kde(Some("6"), Some((6, 8))), facts("niri")] {
            assert!(!decide(PauseOverride::EnvOff, &f).0);
            assert!(
                !decide(PauseOverride::ConfigOff, &f).0,
                "config false: off everywhere"
            );
        }
        assert_eq!(KWIN_MIN, (6, 1));
    }

    #[test]
    fn the_compositor_comes_from_the_desktop_name_then_the_processes() {
        assert_eq!(compositor(&facts("KDE")), Compositor::KWin);
        assert_eq!(compositor(&facts("plasma")), Compositor::KWin);
        assert_eq!(compositor(&facts("niri")), Compositor::Niri);
        assert_eq!(compositor(&facts("GNOME-Classic:GNOME")), Compositor::Gnome);
        assert_eq!(compositor(&facts("sway")), Compositor::Other);
        // The name says nothing: the running compositor.
        let unknown = |f: fn(&mut DesktopFacts)| {
            let mut d = facts("");
            f(&mut d);
            compositor(&d)
        };
        assert_eq!(unknown(|d| d.niri_socket = true), Compositor::Niri);
        assert_eq!(unknown(|d| d.niri_running = true), Compositor::Niri);
        assert_eq!(unknown(|d| d.kwin_running = true), Compositor::KWin);
        assert_eq!(unknown(|d| d.gnome_shell_running = true), Compositor::Gnome);
        assert_eq!(unknown(|_| {}), Compositor::Other);
        // A name it knows wins over a stray process.
        let mut sway = facts("sway");
        sway.niri_running = true;
        assert_eq!(compositor(&sway), Compositor::Other);
        let mut niri_kwin = facts("niri");
        niri_kwin.kwin_running = true;
        assert_eq!(compositor(&niri_kwin), Compositor::Niri);
    }

    #[test]
    fn kwin_versions_parse_from_support_information_and_the_binary() {
        let info = "KWin Support Information:\n...\nVersion\n=======\nKWin version: 6.8.1\nQt Version: 6.10.0\n";
        assert_eq!(parse_kwin_version(info), Some((6, 8)));
        assert_eq!(parse_kwin_version("KWin version: 6.0.90\n"), Some((6, 0)));
        assert_eq!(parse_kwin_version("kwin 6.4.5\n"), Some((6, 4)));
        assert_eq!(parse_kwin_version("kwin_wayland 6.1.0\n"), Some((6, 1)));
        assert_eq!(parse_kwin_version("kwin 5.27.11\n"), Some((5, 27)));
        assert_eq!(parse_kwin_version("Qt Version: 6.10.0\n"), None);
        assert_eq!(parse_kwin_version(""), None);
    }

    #[test]
    fn the_process_scan_finds_compositors_by_comm() {
        let dir = std::env::temp_dir().join(format!("pause-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (pid, comm) in [("10", "niri\n"), ("20", "bash\n"), ("30", "kwin_wayland\n")] {
            std::fs::create_dir_all(dir.join(pid)).unwrap();
            std::fs::write(dir.join(pid).join("comm"), comm).unwrap();
        }
        std::fs::create_dir_all(dir.join("self")).unwrap();
        assert_eq!(scan_processes(&dir, 100), (true, true, false));
        std::fs::write(dir.join("30").join("comm"), "gnome-shell\n").unwrap();
        assert_eq!(scan_processes(&dir, 100), (true, false, true));
        assert_eq!(
            scan_processes(&dir.join("missing"), 100),
            (false, false, false)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn kwin_version_retries_dbus_once_on_plasma_6_before_the_binary() {
        use std::cell::Cell;
        let run =
            |session: Option<&str>, dbus: Vec<Option<(u32, u32)>>, bin: Option<(u32, u32)>| {
                let calls = Cell::new(0usize);
                let bin_calls = Cell::new(0usize);
                let got = rt().block_on(kwin_version_with(
                    session,
                    Duration::from_millis(1),
                    || {
                        let n = calls.get();
                        calls.set(n + 1);
                        let answer = dbus.get(n).copied().flatten();
                        async move { answer }
                    },
                    || {
                        bin_calls.set(bin_calls.get() + 1);
                        async move { bin }
                    },
                ));
                (got, calls.get(), bin_calls.get())
            };
        assert_eq!(
            run(Some("6"), vec![Some((6, 8))], Some((6, 9))),
            (Some((6, 8)), 1, 0)
        );
        assert_eq!(
            run(Some("6"), vec![None, Some((6, 8))], Some((6, 9))),
            (Some((6, 8)), 2, 0),
            "KWin was still starting: the second D-Bus read wins"
        );
        assert_eq!(
            run(Some("6"), vec![None, None], Some((6, 9))),
            (Some((6, 9)), 2, 1)
        );
        assert_eq!(
            run(None, vec![None], Some((6, 9))),
            (Some((6, 9)), 1, 1),
            "no Plasma 6 session: no retry"
        );
        assert_eq!(run(Some("5"), vec![None], None), (None, 1, 1));
    }

    #[test]
    fn on_niri_streaming_answers_a_resume_elsewhere_only_a_buffer_does() {
        for niri in [true, false] {
            let slot = Box::leak(Box::new(Mutex::new(Ctl {
                enabled: true,
                streaming_answers: niri,
                ..Ctl::default()
            })));
            let stream = live(slot);
            assert_eq!(stream.streaming_answers_for_test(), niri);
        }
    }

    fn niri_facts() -> DesktopFacts {
        DesktopFacts {
            desktop: "niri".into(),
            niri_socket: true,
            ..DesktopFacts::default()
        }
    }

    #[test]
    fn the_applied_decision_keeps_the_niri_flag_when_forced_on() {
        let niri = niri_facts();
        assert!(applied_decision(PauseOverride::Auto, &niri).0);
        assert!(applied_decision(PauseOverride::Auto, &niri).1);
        assert_eq!(
            applied_decision(PauseOverride::EnvOn, &niri),
            (true, true, "forced on by STAT_TRACKER_SCREENCAST_PAUSE")
        );
        let kde = DesktopFacts {
            kwin_version: Some((6, 8)),
            ..facts("KDE")
        };
        assert!(!applied_decision(PauseOverride::Auto, &kde).1);
        assert!(!applied_decision(PauseOverride::EnvOn, &facts("sway")).1);
    }

    /// The startup path itself: what lands in the control slot, and which
    /// session reader runs.
    #[test]
    fn decide_and_apply_sets_the_niri_flag_and_reads_the_session_when_forced() {
        use std::cell::Cell;
        let run = |over: PauseOverride, facts: DesktopFacts| {
            let slot = leak_ctl(false);
            let full_ran = Cell::new(false);
            let basic_ran = Cell::new(false);
            let on = rt().block_on(decide_and_apply_in(
                slot,
                over,
                || {
                    full_ran.set(true);
                    std::future::ready(facts.clone())
                },
                || {
                    basic_ran.set(true);
                    std::future::ready(facts.clone())
                },
            ));
            let ctl = ctl(slot);
            (
                on,
                ctl.enabled,
                ctl.streaming_answers,
                full_ran.get(),
                basic_ran.get(),
            )
        };
        // Forced on, on niri: the session is still read (basic, no KWin
        // version), and Streaming answers.
        assert_eq!(
            run(PauseOverride::EnvOn, niri_facts()),
            (true, true, true, false, true)
        );
        // Automatic on niri: the full read.
        assert_eq!(
            run(PauseOverride::Auto, niri_facts()),
            (true, true, true, true, false)
        );
        // Automatic on KWin 6.8: on, only buffers answer.
        let kde = DesktopFacts {
            kwin_version: Some((6, 8)),
            ..facts("KDE")
        };
        assert_eq!(
            run(PauseOverride::Auto, kde),
            (true, true, false, true, false)
        );
        // Off by config: off, the session still read for the log.
        assert_eq!(
            run(PauseOverride::ConfigOff, niri_facts()),
            (false, false, true, false, true)
        );
    }
}
