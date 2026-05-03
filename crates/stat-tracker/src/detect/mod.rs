pub mod match_end;

use std::path::PathBuf;

use evdev::{Device, EventStream, EventSummary, KeyCode};

#[derive(Debug, Clone, PartialEq)]
pub enum MatchOutcome {
    Victory,
    Defeat,
    Draw,
    Unknown,
}

pub fn find_keyboard_device() -> Result<Device, Box<dyn std::error::Error>> {
    let mut devices: Vec<(PathBuf, Device)> = evdev::enumerate().collect();
    devices.sort_by(|a, b| a.0.cmp(&b.0));

    for (_path, device) in devices {
        if let Some(keys) = device.supported_keys() {
            if keys.contains(KeyCode::KEY_TAB)
                && keys.contains(KeyCode::KEY_A)
                && keys.contains(KeyCode::KEY_ENTER)
            {
                tracing::info!(name = ?device.name(), "found keyboard device");
                return Ok(device);
            }
        }
    }
    Err("no keyboard device found — ensure user is in the 'input' group".into())
}

pub fn open_keyboard_stream() -> Result<EventStream, Box<dyn std::error::Error>> {
    let device = find_keyboard_device()?;
    Ok(device.into_event_stream()?)
}

pub async fn wait_next_tab(stream: &mut EventStream) -> Result<(), Box<dyn std::error::Error>> {
    loop {
        let event = stream.next_event().await?;
        if let EventSummary::Key(_, KeyCode::KEY_TAB, 1) = event.destructure() {
            return Ok(());
        }
    }
}
