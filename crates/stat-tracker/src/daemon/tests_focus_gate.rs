//! Tests for the focus gate (`focus/mod.rs`): matching, fail open, the
//! unreadable warn rate limit, the opt-out, the status in the status file
//! and the source order. Fake sources only: no compositor, X server or
//! DBus.

use super::focus::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// A source that gives `read` and counts how often it was asked.
pub(crate) struct FakeSource {
    pub(crate) read: std::sync::Mutex<FocusRead>,
    pub(crate) reads: AtomicUsize,
}

impl FakeSource {
    pub(crate) fn new(read: FocusRead) -> Arc<Self> {
        Arc::new(Self {
            read: std::sync::Mutex::new(read),
            reads: AtomicUsize::new(0),
        })
    }

    pub(crate) fn set(&self, read: FocusRead) {
        *self.read.lock().unwrap() = read;
    }
}

impl FocusSource for FakeSource {
    fn name(&self) -> &'static str {
        "fake"
    }

    fn read(&self) -> FocusRead {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.read.lock().unwrap().clone()
    }
}

pub(crate) fn window(id: &str) -> FocusRead {
    FocusRead::Window(vec![id.to_string()])
}

fn unreadable() -> FocusRead {
    FocusRead::Unreadable("fake: cannot tell".into())
}

#[test]
fn only_the_exact_overwatch_ids_match_ignoring_case() {
    for id in [
        "overwatch.exe",
        "Overwatch.exe",
        "OVERWATCH.EXE",
        "steam_app_2357570",
        "STEAM_APP_2357570",
    ] {
        assert!(matches_overwatch(id), "{id}");
    }
    for id in [
        "steam_app_123",
        "steam_app_23575701",
        "steam_app_",
        "steam_app_235757",
        "overwatch",
        "battle.net.exe",
        "xoverwatch.exe",
        "overwatch.exe ",
        "",
    ] {
        assert!(!matches_overwatch(id), "{id}");
    }
    assert_eq!(OVERWATCH_WINDOW_IDS, ["overwatch.exe", "steam_app_2357570"]);
}

#[test]
fn overwatch_reads_another_window_pauses_unreadable_reads_on() {
    let now = Instant::now();
    let mut gate = FocusGate::new(true);
    assert_eq!(gate.status(), FocusGateStatus::Unknown);
    let a = gate.apply(&window("Overwatch.exe"), now);
    assert_eq!(
        (a.status, a.decision),
        (FocusGateStatus::Focused, GateDecision::Read)
    );
    // Any of the ids may name it (instance or class).
    let both = FocusRead::Window(vec!["steam".into(), "steam_app_2357570".into()]);
    assert_eq!(gate.apply(&both, now).decision, GateDecision::Read);
    let a = gate.apply(&window("firefox"), now);
    assert_eq!(
        (a.status, a.decision),
        (FocusGateStatus::NotFocused, GateDecision::Pause)
    );
    let a = gate.apply(&FocusRead::Window(Vec::new()), now);
    assert_eq!(a.decision, GateDecision::Pause, "a window with no id");
    let a = gate.apply(&FocusRead::NoWindow, now);
    assert_eq!(
        (a.status, a.decision),
        (FocusGateStatus::NotFocused, GateDecision::Pause)
    );
    // Fail open: unreadable is a warning, not a pause.
    let a = gate.apply(&unreadable(), now);
    assert_eq!(
        (a.status, a.decision),
        (FocusGateStatus::Unreadable, GateDecision::Read)
    );
}

#[test]
fn unreadable_warns_on_entry_then_at_most_every_five_minutes() {
    let t0 = Instant::now();
    let at = |secs: u64| t0 + Duration::from_secs(secs);
    let mut gate = FocusGate::new(true);
    assert!(gate.apply(&unreadable(), at(0)).warn, "entering unreadable");
    for secs in [1, 2, 3, 60, 299] {
        assert!(!gate.apply(&unreadable(), at(secs)).warn, "{secs} s");
    }
    assert_eq!(UNREADABLE_WARN_EVERY, Duration::from_secs(300));
    assert!(gate.apply(&unreadable(), at(300)).warn, "five minutes on");
    assert!(!gate.apply(&unreadable(), at(301)).warn);
    // A good read resets: the next entry warns at once.
    assert!(!gate.apply(&window("overwatch.exe"), at(302)).warn);
    assert!(gate.apply(&unreadable(), at(303)).warn, "a new entry");
    assert!(!gate.apply(&unreadable(), at(304)).warn);
    // Not focused is a good read too.
    assert!(!gate.apply(&FocusRead::NoWindow, at(305)).warn);
    assert!(gate.apply(&unreadable(), at(306)).warn);
}

#[tokio::test]
async fn the_guard_fails_open_on_an_unreadable_source_and_no_source() {
    let source = FakeSource::new(unreadable());
    let guard = FocusGuard::new(true, Some(source.clone()), None);
    assert_eq!(guard.check(Instant::now()).await, GateDecision::Read);
    assert_eq!(guard.status(), FocusGateStatus::Unreadable);
    source.set(window("steam_app_2357570"));
    assert_eq!(guard.check(Instant::now()).await, GateDecision::Read);
    assert_eq!(guard.status(), FocusGateStatus::Focused);
    source.set(window("org.kde.konsole"));
    assert_eq!(guard.check(Instant::now()).await, GateDecision::Pause);
    assert_eq!(guard.status(), FocusGateStatus::NotFocused);
    assert_eq!(source.reads.load(Ordering::SeqCst), 3);
    // No source for this desktop: reads go on, status unreadable.
    let guard = FocusGuard::new(true, None, None);
    assert_eq!(guard.check(Instant::now()).await, GateDecision::Read);
    assert_eq!(guard.status(), FocusGateStatus::Unreadable);
}

/// A source that never answers in time.
struct SlowSource;

impl FocusSource for SlowSource {
    fn name(&self) -> &'static str {
        "slow"
    }

    fn read(&self) -> FocusRead {
        std::thread::sleep(FOCUS_READ_TIMEOUT * 4);
        window("firefox")
    }
}

#[tokio::test]
async fn a_read_that_times_out_is_unreadable_and_reads_go_on() {
    let guard = FocusGuard::new(true, Some(Arc::new(SlowSource)), None);
    assert_eq!(guard.check(Instant::now()).await, GateDecision::Read);
    assert_eq!(guard.status(), FocusGateStatus::Unreadable);
    // The hung read still runs: the next check starts no second one.
    assert_eq!(guard.check(Instant::now()).await, GateDecision::Read);
    assert_eq!(guard.status(), FocusGateStatus::Unreadable);
}

#[tokio::test]
async fn the_opt_out_never_reads_the_source() {
    let source = FakeSource::new(window("firefox"));
    let guard = FocusGuard::new(false, Some(source.clone()), None);
    for _ in 0..5 {
        assert_eq!(guard.check(Instant::now()).await, GateDecision::Read);
    }
    assert_eq!(source.reads.load(Ordering::SeqCst), 0, "not even polled");
    assert_eq!(guard.status(), FocusGateStatus::Off);
}

#[tokio::test]
async fn the_status_file_gets_the_gate_on_change() {
    let dir = tempfile::tempdir().unwrap();
    let source = FakeSource::new(window("overwatch.exe"));
    let guard = FocusGuard::new(true, Some(source.clone()), Some(dir.path().to_path_buf()));
    guard.check(Instant::now()).await;
    let status = stat_tracker::ocr_status::read_status(dir.path()).unwrap();
    assert_eq!(status.focus_gate, FocusGateStatus::Focused);
    source.set(FocusRead::NoWindow);
    guard.check(Instant::now()).await;
    let status = stat_tracker::ocr_status::read_status(dir.path()).unwrap();
    assert_eq!(status.focus_gate, FocusGateStatus::NotFocused);
}

#[test]
fn the_status_names_read_across_versions() {
    use stat_tracker::ocr_status::Status;
    for (gate, name) in [
        (FocusGateStatus::Off, "off"),
        (FocusGateStatus::Focused, "focused"),
        (FocusGateStatus::NotFocused, "not_focused"),
        (FocusGateStatus::Unreadable, "unreadable"),
        (FocusGateStatus::Unknown, "unknown"),
    ] {
        let status = Status {
            focus_gate: gate,
            ..Status::default()
        };
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["focus_gate"], name);
        let back: Status = serde_json::from_value(json).unwrap();
        assert_eq!(back.focus_gate, gate);
    }
    // A name from a newer daemon reads as unknown; the file still loads.
    let newer: Status = serde_json::from_str(r#"{"pid":7,"focus_gate":"dimmed"}"#).unwrap();
    assert_eq!((newer.pid, newer.focus_gate), (7, FocusGateStatus::Unknown));
    // A file from before the field loads too.
    let older: Status = serde_json::from_str(r#"{"pid":7}"#).unwrap();
    assert_eq!(older.focus_gate, FocusGateStatus::Unknown);
}

#[test]
fn sources_go_niri_then_kwin_on_kde_wayland_then_x11() {
    let env = |niri: bool, kde: bool, wayland: bool, display: bool| DesktopEnv {
        niri_socket: niri.then(|| "/run/user/1000/niri.sock".into()),
        display,
        wayland,
        kde,
    };
    let niri = SourceKind::Niri("/run/user/1000/niri.sock".into());
    assert_eq!(
        source_kind(&env(true, false, true, true)),
        Some(niri.clone())
    );
    assert_eq!(
        source_kind(&env(true, true, true, true)),
        Some(niri),
        "niri first"
    );
    assert_eq!(
        source_kind(&env(false, true, true, true)),
        Some(SourceKind::Kwin)
    );
    assert_eq!(
        source_kind(&env(false, true, true, false)),
        Some(SourceKind::Kwin)
    );
    // KDE on X11 uses X11.
    assert_eq!(
        source_kind(&env(false, true, false, true)),
        Some(SourceKind::X11 { wayland: false })
    );
    // Another Wayland desktop with XWayland: the X11 source, Wayland rules.
    assert_eq!(
        source_kind(&env(false, false, true, true)),
        Some(SourceKind::X11 { wayland: true })
    );
    assert_eq!(source_kind(&env(false, false, true, false)), None);
    assert_eq!(source_kind(&env(false, false, false, false)), None);
    assert!(desktop_is_kde("KDE"));
    assert!(desktop_is_kde("kde"));
    assert!(desktop_is_kde("ubuntu:KDE"));
    assert!(!desktop_is_kde("GNOME"));
    assert!(!desktop_is_kde("niri"));
    assert!(!desktop_is_kde(""));
    assert!(!desktop_is_kde("KDEx"));
}
