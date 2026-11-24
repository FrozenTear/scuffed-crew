//! The active window on X11 or XWayland: `_NET_ACTIVE_WINDOW` on the root
//! window, then that window's `WM_CLASS`. Overwatch under Wine or Proton is
//! an X11 window, so on a Wayland session this sees it through XWayland.
//!
//! The rule unless the session is positively X11
//! ([`super::DesktopEnv::x11_session`]: `XDG_SESSION_TYPE` is `x11`, or
//! unset with no `WAYLAND_DISPLAY`): XWayland's `_NET_ACTIVE_WINDOW` can be
//! empty, or stale and naming another X11 window, while a native Wayland
//! window has focus. So there this source gives only focused (Overwatch
//! matched) or unreadable, never not focused. Only a positively X11 session
//! may give not focused: another window, or a window of 0 (None), which
//! EWMH defines as no active window.
//!
//! One connection per read, as the X11 capture does.

use super::{FocusRead, FocusSource, matches_overwatch};
use x11rb::connection::Connection as _;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _};

/// Longest `WM_CLASS` taken, in 32-bit units.
const WM_CLASS_LEN: u32 = 256;

/// The two X11 reads, so the rule can run on a fake in tests.
pub(crate) trait X11Query {
    /// `_NET_ACTIVE_WINDOW` on the root: `None` when not set.
    fn active_window(&self) -> Result<Option<u32>, String>;
    /// The raw `WM_CLASS` of `window`.
    fn wm_class(&self, window: u32) -> Result<Vec<u8>, String>;
}

/// One focus read through `query`; `wayland` unless the session is
/// positively X11.
pub(crate) fn read_x11(query: &impl X11Query, wayland: bool) -> FocusRead {
    let window = match query.active_window() {
        Ok(window) => window,
        Err(why) => return FocusRead::Unreadable(why),
    };
    if let Some(read) = active_window_read(window, wayland) {
        return read;
    }
    let class = match query.wm_class(window.unwrap_or_default()) {
        Ok(class) => class,
        Err(why) => return FocusRead::Unreadable(why),
    };
    class_read(parse_wm_class(&class), wayland)
}

/// The read a `_NET_ACTIVE_WINDOW` value gives on its own, or `None` when
/// the window's class must be read. Not set: unreadable. 0: no active
/// window on X11, unreadable on a Wayland session.
pub(crate) fn active_window_read(window: Option<u32>, wayland: bool) -> Option<FocusRead> {
    match window {
        None => Some(FocusRead::Unreadable(
            "_NET_ACTIVE_WINDOW is not set".into(),
        )),
        Some(0) if wayland => Some(FocusRead::Unreadable(
            "no X11 window is active (a Wayland window may be)".into(),
        )),
        Some(0) => Some(FocusRead::NoWindow),
        Some(_) => None,
    }
}

/// The active window's class ids: Overwatch is a window read; another
/// window is a window read on X11 but unreadable on a Wayland session,
/// where the XWayland value may be stale.
pub(crate) fn class_read(ids: Vec<String>, wayland: bool) -> FocusRead {
    if wayland && !ids.iter().any(|id| matches_overwatch(id)) {
        return FocusRead::Unreadable(
            "the active X11 window is not Overwatch, and a Wayland window may have focus".into(),
        );
    }
    FocusRead::Window(ids)
}

/// `WM_CLASS`: the instance and the class, each ended by a NUL (the last
/// NUL may be missing). Empty parts are left out.
pub(crate) fn parse_wm_class(bytes: &[u8]) -> Vec<String> {
    bytes
        .split(|&b| b == 0)
        .filter(|part| !part.is_empty())
        .map(|part| String::from_utf8_lossy(part).into_owned())
        .collect()
}

pub(crate) struct X11Source {
    wayland: bool,
}

impl X11Source {
    pub(crate) fn new(wayland: bool) -> Self {
        Self { wayland }
    }
}

/// A live X server connection for one read.
struct LiveX11 {
    conn: x11rb::rust_connection::RustConnection,
    root: u32,
}

impl X11Query for LiveX11 {
    fn active_window(&self) -> Result<Option<u32>, String> {
        let err = |e: &dyn std::fmt::Display| format!("X11: {e}");
        let atom = self
            .conn
            .intern_atom(true, b"_NET_ACTIVE_WINDOW")
            .map_err(|e| err(&e))?
            .reply()
            .map_err(|e| err(&e))?
            .atom;
        if atom == x11rb::NONE {
            return Err("the window manager does not set _NET_ACTIVE_WINDOW".into());
        }
        let reply = self
            .conn
            .get_property(false, self.root, atom, AtomEnum::WINDOW, 0, 1)
            .map_err(|e| err(&e))?
            .reply()
            .map_err(|e| err(&e))?;
        Ok(reply.value32().and_then(|mut values| values.next()))
    }

    fn wm_class(&self, window: u32) -> Result<Vec<u8>, String> {
        let err = |e: &dyn std::fmt::Display| format!("X11 WM_CLASS: {e}");
        let reply = self
            .conn
            .get_property(
                false,
                window,
                AtomEnum::WM_CLASS,
                AtomEnum::STRING,
                0,
                WM_CLASS_LEN,
            )
            .map_err(|e| err(&e))?
            .reply()
            .map_err(|e| err(&e))?;
        Ok(reply.value)
    }
}

impl FocusSource for X11Source {
    fn name(&self) -> &'static str {
        if self.wayland { "xwayland" } else { "x11" }
    }

    fn read(&self) -> FocusRead {
        let (conn, screen) = match x11rb::connect(None) {
            Ok(connected) => connected,
            Err(err) => return FocusRead::Unreadable(format!("X11 connection failed: {err}")),
        };
        let Some(root) = conn.setup().roots.get(screen).map(|s| s.root) else {
            return FocusRead::Unreadable("X11: no root window for the screen".into());
        };
        read_x11(&LiveX11 { conn, root }, self.wayland)
    }
}
