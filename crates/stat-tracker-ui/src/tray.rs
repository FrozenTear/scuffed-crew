//! System tray via `tray-icon` (v1 — same crate as the Dioxus GUI).
//!
//! Menu: Show window · Hide window · Quit. Left-click also shows the window.
//! Hide/Show are handled by the iced daemon as close/open — not minimize.
//!
//! Tray creation is optional. `libappindicator-sys` 0.9 panics if it cannot
//! `dlopen` Ayatana/AppIndicator; we probe those sonames first and swallow
//! any remaining panic so the Iced window still starts.

use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

/// Sonames `libappindicator-sys` 0.9 tries via `libloading` (then panics).
pub const APPINDICATOR_SONAMES: &[&str] = &[
    "libayatana-appindicator3.so.1",
    "libappindicator3.so.1",
    "libayatana-appindicator3.so",
    "libappindicator3.so",
];

/// Logged when the tray cannot be created. Hide-to-tray needs this lib.
pub const MISSING_TRAY_WARNING: &str = "system tray unavailable \
(libayatana-appindicator3 / libappindicator3 not loaded). \
Starting without a tray — Hide-to-tray will not work. \
Install the Ayatana AppIndicator package if your distro ships it (AerynOS may not).";

/// Short GUI toast when the window starts without a tray.
pub const MISSING_TRAY_TOAST: &str = "No system tray (AppIndicator library missing). \
Hide-to-tray is disabled; closing the window quits.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    Show,
    Hide,
    ToggleOverlay,
    Quit,
}

pub struct TrayHandle {
    pub _icon: tray_icon::TrayIcon,
    pub show_id: MenuId,
    pub hide_id: MenuId,
    pub overlay_id: MenuId,
    pub quit_id: MenuId,
}

/// True when a known AppIndicator soname can be `dlopen`ed.
///
/// Linux-only: other targets do not use `libappindicator-sys`.
pub fn appindicator_present() -> bool {
    #[cfg(not(target_os = "linux"))]
    {
        true
    }
    #[cfg(target_os = "linux")]
    {
        APPINDICATOR_SONAMES.iter().any(|name| {
            // SAFETY: libloading 0.7 `Library::new` is dlopen of a soname; we
            // only check success and drop the handle (same probe as
            // libappindicator-sys 0.9).
            unsafe { libloading::Library::new(*name).is_ok() }
        })
    }
}

/// Create the tray, or `None` if AppIndicator / `tray-icon` cannot load.
///
/// Must not panic: a missing system lib used to abort `stat-tracker-gui`.
pub fn try_create() -> Option<TrayHandle> {
    if !appindicator_present() {
        tracing::warn!("{MISSING_TRAY_WARNING}");
        return None;
    }
    match swallow_panic(create_inner) {
        Some(Some(handle)) => Some(handle),
        Some(None) | None => {
            tracing::warn!("{MISSING_TRAY_WARNING}");
            None
        }
    }
}

fn create_inner() -> Option<TrayHandle> {
    let icon = create_icon()?;
    let menu = Menu::new();
    let show_item = MenuItem::new("Show window", true, None);
    let hide_item = MenuItem::new("Hide window", true, None);
    let overlay_item = MenuItem::new("Hide / show overlay", true, None);
    let quit_item = MenuItem::new("Quit", true, None);
    let show_id = show_item.id().clone();
    let hide_id = hide_item.id().clone();
    let overlay_id = overlay_item.id().clone();
    let quit_id = quit_item.id().clone();
    menu.append(&show_item).ok()?;
    menu.append(&hide_item).ok()?;
    menu.append(&overlay_item).ok()?;
    menu.append(&quit_item).ok()?;

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("Scuffed Tracker")
        .with_icon(icon)
        .build()
        .ok()?;

    Some(TrayHandle {
        _icon: tray,
        show_id,
        hide_id,
        overlay_id,
        quit_id,
    })
}

/// Run `f`, returning `None` if it panics (and without printing a panic hook).
///
/// `libappindicator-sys` uses `once_cell::Lazy` that `panic!`s on a missing
/// `.so`. Catching that keeps the GUI alive; we never touch the tray again.
pub(crate) fn swallow_panic<T, F>(f: F) -> Option<T>
where
    F: FnOnce() -> T,
{
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    std::panic::set_hook(prev);
    result.ok()
}

/// Drain pending tray events. Call from the UI tick after pumping GTK.
pub fn poll(handle: &TrayHandle) -> Option<TrayAction> {
    let menu_rx = MenuEvent::receiver();
    while let Ok(event) = menu_rx.try_recv() {
        if event.id == handle.quit_id {
            return Some(TrayAction::Quit);
        }
        if event.id == handle.show_id {
            return Some(TrayAction::Show);
        }
        if event.id == handle.hide_id {
            return Some(TrayAction::Hide);
        }
        if event.id == handle.overlay_id {
            return Some(TrayAction::ToggleOverlay);
        }
    }

    let tray_rx = TrayIconEvent::receiver();
    while let Ok(event) = tray_rx.try_recv() {
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = event
        {
            return Some(TrayAction::Show);
        }
    }
    None
}

/// Pump GTK so tray-icon menu events arrive (Linux). No-op if GTK is not up.
pub fn pump_gtk() {
    if gtk::is_initialized() {
        while gtk::events_pending() {
            let _ = gtk::main_iteration_do(false);
        }
    }
}

fn create_icon() -> Option<Icon> {
    let size = 32u32;
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    let center = size as f32 / 2.0;
    let radius = center - 2.0;
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 - center;
            let dy = y as f32 - center;
            if dx * dx + dy * dy <= radius * radius {
                rgba.extend_from_slice(&[0x8f, 0x73, 0xff, 0xff]);
            } else {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
    Icon::from_rgba(rgba, size, size).ok()
}

#[cfg(test)]
mod tests {
    use super::{
        APPINDICATOR_SONAMES, MISSING_TRAY_TOAST, MISSING_TRAY_WARNING, TrayAction,
        appindicator_present, swallow_panic,
    };

    #[test]
    fn tray_actions_are_show_hide_quit() {
        assert_ne!(TrayAction::Show, TrayAction::Hide);
        assert_ne!(TrayAction::Show, TrayAction::Quit);
        assert_ne!(TrayAction::ToggleOverlay, TrayAction::Quit);
    }

    #[test]
    fn swallow_panic_returns_none_on_panic() {
        assert!(
            swallow_panic(|| -> u8 { panic!("Failed to load ayatana-appindicator3") }).is_none()
        );
        assert_eq!(swallow_panic(|| 7u8), Some(7));
    }

    #[test]
    fn missing_tray_copy_mentions_hide_and_ayatana() {
        assert!(MISSING_TRAY_WARNING.contains("Hide-to-tray"));
        assert!(MISSING_TRAY_WARNING.contains("libayatana-appindicator3"));
        assert!(MISSING_TRAY_WARNING.contains("AerynOS"));
        assert!(MISSING_TRAY_TOAST.contains("Hide-to-tray"));
        assert!(MISSING_TRAY_TOAST.contains("closing the window quits"));
    }

    #[test]
    fn appindicator_sonames_match_libappindicator_sys() {
        assert_eq!(
            APPINDICATOR_SONAMES,
            [
                "libayatana-appindicator3.so.1",
                "libappindicator3.so.1",
                "libayatana-appindicator3.so",
                "libappindicator3.so",
            ]
        );
        let _ = appindicator_present();
    }

    #[test]
    fn try_create_does_not_abort_when_appindicator_missing_or_present() {
        let _ = super::try_create();
    }

    #[test]
    fn try_create_is_none_when_probe_finds_no_soname() {
        if !appindicator_present() {
            assert!(super::try_create().is_none());
        }
    }
}
