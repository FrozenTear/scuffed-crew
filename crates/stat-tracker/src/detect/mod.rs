pub mod game_running;
pub mod hero_portrait;
pub mod match_end;
pub mod match_start;

use std::path::PathBuf;

use evdev::{Device, EventSummary, KeyCode};
use tokio::sync::mpsc;

/// A match result. The canonical wire/storage spelling is the lowercase
/// `Display` form ("victory"/"defeat"/"draw"/"unknown") — every layer that
/// needs a string goes through `to_string()`, and parsing goes through
/// `FromStr` (strict: anything else is an error, callers decide whether that
/// means `Unknown`). Do not hand-roll translations of these names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MatchOutcome {
    Victory,
    Defeat,
    Draw,
    Unknown,
}

impl std::fmt::Display for MatchOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            MatchOutcome::Victory => "victory",
            MatchOutcome::Defeat => "defeat",
            MatchOutcome::Draw => "draw",
            MatchOutcome::Unknown => "unknown",
        })
    }
}

impl std::str::FromStr for MatchOutcome {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "victory" => Ok(MatchOutcome::Victory),
            "defeat" => Ok(MatchOutcome::Defeat),
            "draw" => Ok(MatchOutcome::Draw),
            "unknown" => Ok(MatchOutcome::Unknown),
            other => Err(format!("not a match outcome: {other:?}")),
        }
    }
}

impl MatchOutcome {
    /// Parse storage/GUI outcome strings, including legacy `win`/`loss` spellings
    /// from older local data. Unknown / empty / garbage → [`MatchOutcome::Unknown`].
    pub fn parse_lenient(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "victory" | "win" => MatchOutcome::Victory,
            "defeat" | "loss" => MatchOutcome::Defeat,
            "draw" => MatchOutcome::Draw,
            // FromStr's canonical spellings are all matched above, so any
            // other string (incl. "unknown"/"") can only be Unknown.
            _ => MatchOutcome::Unknown,
        }
    }

    pub fn is_win(self) -> bool {
        matches!(self, MatchOutcome::Victory)
    }

    pub fn is_loss(self) -> bool {
        matches!(self, MatchOutcome::Defeat)
    }

    pub fn is_decided(self) -> bool {
        !matches!(self, MatchOutcome::Unknown)
    }

    /// CSS suffix for history/dashboard rows: `win` / `loss` / `draw` / `undecided`.
    pub fn row_class(self) -> &'static str {
        match self {
            MatchOutcome::Victory => "win",
            MatchOutcome::Defeat => "loss",
            MatchOutcome::Draw => "draw",
            MatchOutcome::Unknown => "undecided",
        }
    }

    /// CSS class for outcome text colour.
    pub fn text_class(self) -> &'static str {
        match self {
            MatchOutcome::Victory => "outcome-win",
            MatchOutcome::Defeat => "outcome-loss",
            MatchOutcome::Draw => "outcome-draw",
            MatchOutcome::Unknown => "outcome-unknown",
        }
    }
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
            if let Some(keys) = device.supported_keys()
                && keys.contains(KeyCode::KEY_TAB)
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
                                if let EventSummary::Key(_, KeyCode::KEY_TAB, 1) =
                                    event.destructure()
                                    && tx.send(()).is_err()
                                {
                                    return;
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

        if count == 0 {
            return Err("no keyboard device found — ensure user is in the 'input' group".into());
        }

        tracing::info!(
            device_count = count,
            "keyboard monitoring active on all devices"
        );
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
