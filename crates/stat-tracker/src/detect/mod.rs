pub mod hero_portrait;
pub mod match_end;
pub mod match_start;

use std::path::PathBuf;

use evdev::{Device, EventSummary, KeyCode};
use tokio::sync::mpsc;

#[derive(Debug, Clone, PartialEq)]
pub enum MatchOutcome {
    Victory,
    Defeat,
    Draw,
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GamePhase {
    MapVote { maps: Vec<String> },
    HeroBan,
    HeroSelect,
    InGame,
    Unknown,
}

pub struct MultiKeyboardStream {
    rx: mpsc::UnboundedReceiver<()>,
}

impl MultiKeyboardStream {
    pub fn open() -> Result<Self, Box<dyn std::error::Error>> {
        let mut devices: Vec<(PathBuf, Device)> = evdev::enumerate().collect();
        devices.sort_by(|a, b| a.0.cmp(&b.0));

        let (tx, rx) = mpsc::unbounded_channel();
        let mut count = 0usize;

        for (path, device) in devices {
            if let Some(keys) = device.supported_keys() {
                if keys.contains(KeyCode::KEY_TAB)
                    && keys.contains(KeyCode::KEY_A)
                    && keys.contains(KeyCode::KEY_ENTER)
                {
                    let name = device.name().unwrap_or("unknown").to_string();
                    let path_str = path.display().to_string();
                    tracing::info!(
                        name = %name,
                        path = %path_str,
                        "listening on keyboard device"
                    );

                    let tx = tx.clone();
                    tokio::spawn(async move {
                        let mut stream = match device.into_event_stream() {
                            Ok(s) => s,
                            Err(e) => {
                                tracing::warn!(device = %name, error = %e, "failed to open event stream");
                                return;
                            }
                        };
                        loop {
                            match stream.next_event().await {
                                Ok(event) => {
                                    if let EventSummary::Key(_, KeyCode::KEY_TAB, 1) = event.destructure() {
                                        if tx.send(()).is_err() {
                                            return;
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::debug!(device = %name, error = %e, "keyboard stream ended");
                                    return;
                                }
                            }
                        }
                    });
                    count += 1;
                }
            }
        }

        if count == 0 {
            return Err("no keyboard device found — ensure user is in the 'input' group".into());
        }

        tracing::info!(device_count = count, "keyboard monitoring active on all devices");
        Ok(MultiKeyboardStream { rx })
    }

    pub async fn wait_tab(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.rx
            .recv()
            .await
            .ok_or_else(|| -> Box<dyn std::error::Error> {
                "all keyboard devices disconnected".into()
            })
    }
}
