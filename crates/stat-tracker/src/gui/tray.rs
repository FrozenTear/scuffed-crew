use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem};
use tray_icon::{Icon, TrayIconBuilder};

pub struct TrayHandle {
    pub _icon: tray_icon::TrayIcon,
    pub quit_id: MenuId,
}

pub fn try_create_tray() -> Option<TrayHandle> {
    let icon = create_icon();
    let menu = Menu::new();
    let quit_item = MenuItem::new("Quit", true, None);
    let quit_id = quit_item.id().clone();
    menu.append(&quit_item).ok()?;

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("Scuffed Stat Tracker")
        .with_icon(icon)
        .build()
        .ok()?;

    Some(TrayHandle {
        _icon: tray,
        quit_id,
    })
}

/// Block until the tray Quit item is activated (no 1 Hz wakeups).
pub fn wait_quit(quit_id: &MenuId) {
    let rx = MenuEvent::receiver();
    while let Ok(event) = rx.recv() {
        if event.id == *quit_id {
            return;
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
                rgba.extend_from_slice(&[0x7c, 0x3a, 0xed, 0xff]);
            } else {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
    Icon::from_rgba(rgba, size, size).expect("tray icon")
}
