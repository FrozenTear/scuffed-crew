//! Tests for the focus sources' parsing and rules on canned data: niri
//! replies, X11 `WM_CLASS` bytes and a fake X11 server (the Wayland
//! session rule), and the KWin script's reports. No compositor, X server
//! or DBus.

use super::focus::kwin::*;
use super::focus::niri::parse_niri_reply;
use super::focus::x11::*;
use super::focus::*;
use super::tests_focus_gate::window;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn is_unreadable(read: &FocusRead) -> bool {
    matches!(read, FocusRead::Unreadable(_))
}

#[test]
fn niri_replies_parse() {
    let focused = r#"{"Ok":{"FocusedWindow":{"id":12,"title":"Overwatch","app_id":"overwatch.exe","pid":4242,"workspace_id":1,"is_focused":true,"is_floating":false,"is_urgent":false,"layout":{"pos_in_scrolling_layout":[1,1],"tile_size":[2560.0,1440.0],"window_size":[2560,1440],"tile_pos_in_workspace_view":null,"window_offset_in_tile":[0.0,0.0]}}}}"#;
    assert_eq!(parse_niri_reply(focused), window("overwatch.exe"));
    let steam = r#"{"Ok":{"FocusedWindow":{"id":3,"app_id":"steam_app_2357570"}}}"#;
    assert_eq!(
        parse_niri_reply(&format!("{steam}\n")),
        window("steam_app_2357570")
    );
    assert_eq!(
        parse_niri_reply(r#"{"Ok":{"FocusedWindow":null}}"#),
        FocusRead::NoWindow
    );
    assert_eq!(
        parse_niri_reply(r#"{"Ok":{"FocusedWindow":{"id":3,"app_id":null}}}"#),
        FocusRead::Window(Vec::new()),
        "a window with no app id is not Overwatch"
    );
    assert_eq!(
        parse_niri_reply(r#"{"Ok":{"FocusedWindow":{"id":3}}}"#),
        FocusRead::Window(Vec::new())
    );
    for bad in [
        r#"{"Err":"unknown request"}"#,
        "not json",
        "",
        r#"{"Ok":{"Outputs":{}}}"#,
        r#"{"Ok":{"FocusedWindow":"overwatch.exe"}}"#,
    ] {
        assert!(is_unreadable(&parse_niri_reply(bad)), "{bad}");
    }
    // The gate's verdict on these reads.
    let mut gate = FocusGate::new(true);
    let now = Instant::now();
    let read = parse_niri_reply(focused);
    assert_eq!(gate.apply(&read, now).decision, GateDecision::Read);
    let other =
        parse_niri_reply(r#"{"Ok":{"FocusedWindow":{"id":4,"app_id":"steam_app_1086940"}}}"#);
    assert_eq!(
        gate.apply(&other, now).decision,
        GateDecision::Pause,
        "another Steam game"
    );
}

#[test]
fn wm_class_bytes_parse() {
    assert_eq!(
        parse_wm_class(b"overwatch.exe\0Overwatch.exe\0"),
        ["overwatch.exe", "Overwatch.exe"]
    );
    assert_eq!(
        parse_wm_class(b"steam_app_2357570\0steam_app_2357570\0"),
        ["steam_app_2357570", "steam_app_2357570"]
    );
    assert_eq!(
        parse_wm_class(b"firefox\0firefox"),
        ["firefox", "firefox"],
        "no last NUL"
    );
    assert_eq!(parse_wm_class(b""), Vec::<String>::new());
    assert_eq!(parse_wm_class(b"\0\0"), Vec::<String>::new());
}

/// An X server in a test: the active window and each window's class.
struct FakeX11 {
    active: Result<Option<u32>, String>,
    classes: Vec<(u32, &'static [u8])>,
}

impl X11Query for FakeX11 {
    fn active_window(&self) -> Result<Option<u32>, String> {
        self.active.clone()
    }

    fn wm_class(&self, window: u32) -> Result<Vec<u8>, String> {
        self.classes
            .iter()
            .find(|(w, _)| *w == window)
            .map(|(_, class)| class.to_vec())
            .ok_or_else(|| "BadWindow".to_string())
    }
}

fn x11(active: Option<u32>) -> FakeX11 {
    FakeX11 {
        active: Ok(active),
        classes: vec![
            (0x0040_0001, b"overwatch.exe\0overwatch.exe\0"),
            (0x0060_0002, b"steam_app_2357570\0steam_app_2357570\0"),
            (0x0080_0003, b"discord\0discord\0"),
        ],
    }
}

#[test]
fn x11_on_a_wayland_session_gives_focused_or_unreadable_never_not_focused() {
    let mut gate = FocusGate::new(true);
    let now = Instant::now();
    // (a) No active X11 window: unreadable, reads go on.
    for active in [Some(0), None] {
        let read = read_x11(&x11(active), true);
        assert!(is_unreadable(&read), "{active:?}");
        let applied = gate.apply(&read, now);
        assert_eq!(applied.decision, GateDecision::Read);
        assert_eq!(applied.status, FocusGateStatus::Unreadable);
    }
    // (b) A non-Overwatch X11 window (maybe stale): unreadable, reads go on.
    let read = read_x11(&x11(Some(0x0080_0003)), true);
    assert!(is_unreadable(&read));
    assert_eq!(gate.apply(&read, now).decision, GateDecision::Read);
    // Overwatch matched: focused.
    for active in [0x0040_0001, 0x0060_0002] {
        let read = read_x11(&x11(Some(active)), true);
        let applied = gate.apply(&read, now);
        assert_eq!(applied.status, FocusGateStatus::Focused, "{active:#x}");
    }
    // A window gone before its class was read, or a dead server.
    assert!(is_unreadable(&read_x11(&x11(Some(0x0099_0009)), true)));
    let dead = FakeX11 {
        active: Err("X11 connection failed".into()),
        classes: Vec::new(),
    };
    assert!(is_unreadable(&read_x11(&dead, true)));
}

#[test]
fn x11_on_an_x11_session_can_say_not_focused() {
    let mut gate = FocusGate::new(true);
    let now = Instant::now();
    // (c) Another window: not focused, the read pauses.
    let read = read_x11(&x11(Some(0x0080_0003)), false);
    assert_eq!(
        read,
        FocusRead::Window(vec!["discord".into(), "discord".into()])
    );
    let applied = gate.apply(&read, now);
    assert_eq!(
        (applied.status, applied.decision),
        (FocusGateStatus::NotFocused, GateDecision::Pause)
    );
    // None (0) is no active window on X11, as EWMH defines it.
    assert_eq!(read_x11(&x11(Some(0)), false), FocusRead::NoWindow);
    // Not set at all is still unreadable.
    assert!(is_unreadable(&read_x11(&x11(None), false)));
    let read = read_x11(&x11(Some(0x0040_0001)), false);
    assert_eq!(gate.apply(&read, now).status, FocusGateStatus::Focused);
}

#[test]
fn kwin_reports_parse_and_match_both_ids() {
    assert_eq!(
        parse_kwin_report(("steam_app_2357570".into(), "steam_app_2357570".into())),
        KwinReport {
            class: "steam_app_2357570".into(),
            name: "steam_app_2357570".into(),
        }
    );
    assert_eq!(
        parse_kwin_report((" Overwatch.exe ".into(), "".into())).class,
        "Overwatch.exe"
    );
    let now = Instant::now();
    let mut gate = FocusGate::new(true);
    let read_of = |class: &str, name: &str| {
        let mut state = KwinState::default();
        state.note_report(parse_kwin_report((class.into(), name.into())), now);
        state.read(now)
    };
    // Wine on native Wayland: the exe name as the app id, in either field.
    for (class, name) in [
        ("overwatch.exe", "overwatch.exe"),
        ("", "Overwatch.exe"),
        ("OVERWATCH.EXE", ""),
        ("steam_app_2357570", "steam"),
    ] {
        let applied = gate.apply(&read_of(class, name), now);
        assert_eq!(applied.status, FocusGateStatus::Focused, "{class} {name}");
    }
    let applied = gate.apply(&read_of("org.kde.dolphin", "dolphin"), now);
    assert_eq!(
        applied.decision,
        GateDecision::Pause,
        "a good read of another window"
    );
    let applied = gate.apply(&read_of("steam_app_1086940", "steam_app_1086940"), now);
    assert_eq!(applied.decision, GateDecision::Pause, "another Steam game");
    assert_eq!(read_of("", ""), FocusRead::NoWindow);
}

#[test]
fn kwin_with_no_report_an_error_or_a_stale_report_is_unreadable() {
    let t0 = Instant::now();
    let mut state = KwinState::default();
    assert!(is_unreadable(&state.read(t0)), "no report yet");
    state.note_error("KWin refused the focus script (loadScript gave -1)".into());
    assert_eq!(
        state.read(t0),
        FocusRead::Unreadable("KWin refused the focus script (loadScript gave -1)".into())
    );
    state.note_report(parse_kwin_report(("overwatch.exe".into(), "".into())), t0);
    assert_eq!(
        state.read(t0 + KWIN_STALE),
        window("overwatch.exe"),
        "fresh up to the limit"
    );
    assert!(
        is_unreadable(&state.read(t0 + KWIN_STALE + Duration::from_secs(1))),
        "stale"
    );
    // A refresh's report makes it fresh again.
    let later = t0 + KWIN_STALE * 2;
    state.note_report(
        parse_kwin_report(("discord".into(), "discord".into())),
        later,
    );
    assert_eq!(
        state.read(later),
        FocusRead::Window(vec!["discord".into(), "discord".into()])
    );
    // A failure after it (a reload KWin refused) drops the report: the
    // script may not run, so the old verdict is not kept.
    state.note_error("KWin loadScript did not answer".into());
    assert_eq!(
        state.read(later),
        FocusRead::Unreadable("KWin loadScript did not answer".into())
    );
    // The refresh keeps a live script well inside the stale limit.
    assert!(KWIN_REFRESH * 2 < KWIN_STALE);
}

#[test]
fn a_fake_kwin_source_reads_its_state() {
    let state = Arc::new(Mutex::new(KwinState::default()));
    let source = KwinSource::from_state(Arc::clone(&state));
    assert!(is_unreadable(&source.read()));
    state.lock().unwrap().note_report(
        parse_kwin_report(("steam_app_2357570".into(), "steam_app_2357570".into())),
        Instant::now(),
    );
    assert!(
        matches!(source.read(), FocusRead::Window(ids) if ids.iter().all(|id| matches_overwatch(id)))
    );
}

#[test]
fn kwin_load_results_and_the_script() {
    assert_eq!(load_result(0), Ok(0));
    assert_eq!(load_result(7), Ok(7));
    assert!(load_result(-1).is_err());
    let script = kwin_script();
    for part in [
        BUS_NAME,
        OBJECT_PATH,
        INTERFACE,
        REPORT_MEMBER,
        "resourceClass",
        "resourceName",
        "workspace.windowActivated",
        "workspace.activeWindow",
        "callDBus(",
    ] {
        assert!(script.contains(part), "{part}");
    }
}

#[test]
fn niri_with_a_dead_or_silent_socket_is_unreadable() {
    use super::focus::niri::NiriSource;
    let dir = tempfile::tempdir().unwrap();
    // Nothing listens on the path.
    let dead = NiriSource::new(dir.path().join("gone.sock"));
    assert!(is_unreadable(&dead.read()), "dead socket");
    // A socket that takes the connection and never answers: the read
    // deadline ends it, unreadable.
    let path = dir.path().join("silent.sock");
    let _listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
    let silent = NiriSource::new(path);
    let started = Instant::now();
    assert!(is_unreadable(&silent.read()), "silent socket");
    assert!(started.elapsed() < FOCUS_READ_TIMEOUT * 10, "bounded");
}

#[test]
fn on_kde_wayland_the_kwin_script_is_used_and_an_empty_xwayland_is_unreadable() {
    let env = DesktopEnv::from_vars(|key| match key {
        "XDG_CURRENT_DESKTOP" => Some("KDE".into()),
        "WAYLAND_DISPLAY" => Some("wayland-0".into()),
        "DISPLAY" => Some(":1".into()),
        _ => None,
    });
    assert_eq!(source_kind(&env), Some(SourceKind::Kwin));
    // The KWin script with no report yet: unreadable, reads go on.
    let mut gate = FocusGate::new(true);
    let now = Instant::now();
    let read = KwinState::default().read(now);
    assert_eq!(gate.apply(&read, now).decision, GateDecision::Read);
    assert_eq!(gate.status(), FocusGateStatus::Unreadable);
    // XWayland on the same session with no active X11 window: unreadable.
    let read = read_x11(&x11(Some(0)), true);
    assert!(is_unreadable(&read));
    assert_eq!(gate.apply(&read, now).decision, GateDecision::Read);
}

#[test]
fn only_kwin_itself_may_report() {
    assert!(report_from_kwin(Some(":1.42"), Some(":1.42")));
    assert!(
        !report_from_kwin(Some(":1.99"), Some(":1.42")),
        "another client"
    );
    assert!(!report_from_kwin(None, Some(":1.42")), "no sender");
    assert!(
        !report_from_kwin(Some(":1.42"), None),
        "KWin owner not known yet"
    );
}

#[test]
fn the_script_path_is_cut_to_its_file_name() {
    let script = std::path::Path::new("/home/someone/.local/share/tracker").join(SCRIPT_FILE);
    let why = format!(
        "KWin loadScript: org.kde.kwin.Error: cannot open {}",
        script.display()
    );
    let redacted = redact_path(&why, &script);
    assert_eq!(
        redacted,
        format!("KWin loadScript: org.kde.kwin.Error: cannot open {SCRIPT_FILE}")
    );
    assert!(!redacted.contains("/home/"));
    assert_eq!(redact_path("no bus", &script), "no bus");
}
