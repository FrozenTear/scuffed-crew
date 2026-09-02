//! System tray via `tray-icon` (v1 — same crate as the Dioxus GUI).
//!
//! Menu: Show window · Hide window · Quit. Left-click also shows the window.
//! Hide/Show are handled by the iced daemon as close/open — not minimize.

use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    Show,
    Hide,
    Quit,
}

pub struct TrayHandle {
    pub _icon: tray_icon::TrayIcon,
    pub show_id: MenuId,
    pub hide_id: MenuId,
    pub quit_id: MenuId,
}

pub fn try_create() -> Option<TrayHandle> {
    let icon = create_icon();
    let menu = Menu::new();
    let show_item = MenuItem::new("Show window", true, None);
    let hide_item = MenuItem::new("Hide window", true, None);
    let quit_item = MenuItem::new("Quit", true, None);
    let show_id = show_item.id().clone();
    let hide_id = hide_item.id().clone();
    let quit_id = quit_item.id().clone();
    menu.append(&show_item).ok()?;
    menu.append(&hide_item).ok()?;
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
        quit_id,
    })
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

fn create_icon() -> Icon {
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
    Icon::from_rgba(rgba, size, size).expect("tray icon")
}

#[cfg(test)]
mod tests {
    use super::TrayAction;

    #[test]
    fn tray_actions_are_show_hide_quit() {
        assert_ne!(TrayAction::Show, TrayAction::Hide);
        assert_ne!(TrayAction::Show, TrayAction::Quit);
    }
}
