pub mod game_running;
pub mod hero_portrait;
pub mod match_end;
pub mod match_start;
pub mod stability;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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

/// How often the hotplug scanner looks for keyboards that appeared after
/// startup. `/dev/input`'s mtime changes whenever a node is added or removed,
/// so each tick is a single `stat` unless something actually changed.
const HOTPLUG_SCAN_INTERVAL: Duration = Duration::from_secs(3);

/// Full rescan cadence even when `/dev/input`'s mtime did not move (belt and
/// braces: devtmpfs mtime is reliable, but a missed edge here costs an evening
/// of captures, see below).
const HOTPLUG_FULL_RESCAN: Duration = Duration::from_secs(60);

/// How often each open device re-probes whether another process holds it
/// exclusively (EVIOCGRAB). Global-hotkey daemons grab at *their* startup,
/// which can be after ours.
const GRAB_RECHECK_INTERVAL: Duration = Duration::from_secs(30);

/// Per-device state shared between the device tasks and the scanner, keyed by
/// `/dev/input/eventN` path.
#[derive(Debug, Clone)]
struct KeyboardState {
    name: String,
    /// Another process holds an exclusive grab: the kernel routes every event
    /// to the grabber only and our reader sees nothing. This is silent — the
    /// stream stays open and healthy, it just never yields a key.
    grabbed: bool,
}

type Registry = Arc<Mutex<HashMap<PathBuf, KeyboardState>>>;

/// Tab presses from every keyboard-like input device, merged into one stream.
///
/// **Why hotplug + grab detection (2026-09-01 zero-games night):** the first
/// version enumerated `/dev/input` once at startup. A global-hotkey daemon
/// (gsr-ui's `gsr-global-hotkeys --all`) started one second after the daemon,
/// grabbed every physical keyboard exclusively and re-emitted keys through a
/// uinput "virtual keyboard" that appeared a second later. The tracker held
/// only the grabbed nodes, received zero events for three hours of play, and
/// nothing above DEBUG level said why. Now: devices that appear later are
/// picked up within seconds, and a grabbed device is logged at WARN so the
/// journal names the cause.
pub struct MultiKeyboardStream {
    rx: mpsc::UnboundedReceiver<()>,
}

impl MultiKeyboardStream {
    pub fn open() -> Result<Self, Box<dyn std::error::Error>> {
        let (tx, rx) = mpsc::unbounded_channel();
        let registry: Registry = Arc::new(Mutex::new(HashMap::new()));

        let count = scan_and_open(&registry, &tx);
        if count == 0 {
            return Err("no keyboard device found — ensure user is in the 'input' group".into());
        }
        tracing::info!(
            device_count = count,
            "keyboard monitoring active on all devices"
        );
        log_all_grabbed(&registry, true);

        tokio::spawn(hotplug_scanner(registry, tx));
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

/// A device counts as a keyboard when it can type: Tab alone is not enough
/// (consumer-control and system-control interfaces advertise odd key sets).
fn is_keyboard(device: &Device) -> bool {
    device.supported_keys().is_some_and(|keys| {
        keys.contains(KeyCode::KEY_TAB)
            && keys.contains(KeyCode::KEY_A)
            && keys.contains(KeyCode::KEY_ENTER)
    })
}

/// Whether another process holds `device` under an exclusive grab. Probed by
/// briefly taking the grab ourselves: EVIOCGRAB fails with EBUSY when someone
/// else already has it, and our own grab (released immediately, microseconds)
/// is the only way userspace can observe that. Any other error is treated as
/// "not grabbed" so a permissions oddity never produces a false alarm.
fn probe_grabbed(device: &mut Device) -> bool {
    match device.grab() {
        Ok(()) => {
            if let Err(e) = device.ungrab() {
                tracing::warn!(error = %e, "failed to release keyboard grab probe");
            }
            false
        }
        Err(e) => e.kind() == std::io::ErrorKind::ResourceBusy,
    }
}

/// Enumerate `/dev/input`, open every keyboard not already tracked, spawn its
/// reader task. Returns how many devices are open afterwards.
fn scan_and_open(registry: &Registry, tx: &mpsc::UnboundedSender<()>) -> usize {
    let mut devices: Vec<(PathBuf, Device)> = evdev::enumerate().collect();
    devices.sort_by(|a, b| a.0.cmp(&b.0));

    for (path, mut device) in devices {
        if !is_keyboard(&device) {
            continue;
        }
        {
            let mut reg = registry.lock().expect("keyboard registry poisoned");
            if reg.contains_key(&path) {
                continue;
            }
            let name = device.name().unwrap_or("unknown").to_string();
            let grabbed = probe_grabbed(&mut device);
            tracing::info!(name = %name, path = %path.display(), "listening on keyboard device");
            if grabbed {
                log_grabbed(&name, &path);
            }
            reg.insert(path.clone(), KeyboardState { name, grabbed });
        }
        tokio::spawn(run_device(path, device, registry.clone(), tx.clone()));
    }

    registry.lock().expect("keyboard registry poisoned").len()
}

fn log_grabbed(name: &str, path: &std::path::Path) {
    tracing::warn!(
        name = %name,
        path = %path.display(),
        "keyboard is exclusively grabbed by another process (global-hotkey daemon such as gsr-ui?) — Tab presses on it cannot reach the tracker; its virtual pass-through keyboard will be used if one appears"
    );
}

/// WARN once when every open keyboard is grabbed (Tab capture is impossible
/// until a pass-through device shows up); INFO when that clears. `startup`
/// suppresses the all-clear so a healthy start stays quiet.
fn log_all_grabbed(registry: &Registry, startup: bool) {
    static ALL_GRABBED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let reg = registry.lock().expect("keyboard registry poisoned");
    let all = !reg.is_empty() && reg.values().all(|s| s.grabbed);
    let was = ALL_GRABBED.swap(all, std::sync::atomic::Ordering::Relaxed);
    if all && !was {
        tracing::warn!(
            devices = ?reg.values().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            "EVERY keyboard is grabbed by another process — Tab captures will not fire until the grabber's virtual keyboard appears or the grab is released (gsr-ui: set hotkeys to the no-grab/virtual-devices mode)"
        );
    } else if !all && was && !startup {
        tracing::info!("an ungrabbed keyboard is available again — Tab captures can fire");
    }
}

/// Read one device until it disappears, re-probing the exclusive-grab state
/// periodically so a grab taken after we opened the node is still reported.
async fn run_device(
    path: PathBuf,
    device: Device,
    registry: Registry,
    tx: mpsc::UnboundedSender<()>,
) {
    let name = device.name().unwrap_or("unknown").to_string();
    let mut stream = match device.into_event_stream() {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(device = %name, error = %e, "failed to open event stream");
            registry
                .lock()
                .expect("keyboard registry poisoned")
                .remove(&path);
            return;
        }
    };
    let mut recheck = tokio::time::interval(GRAB_RECHECK_INTERVAL);
    recheck.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    recheck.tick().await; // first tick fires immediately; the open-time probe covered it
    loop {
        tokio::select! {
            ev = stream.next_event() => match ev {
                Ok(event) => {
                    if let EventSummary::Key(_, KeyCode::KEY_TAB, 1) = event.destructure()
                        && tx.send(()).is_err()
                    {
                        break;
                    }
                }
                Err(e) => {
                    tracing::info!(device = %name, error = %e, "keyboard stream ended (unplugged?)");
                    break;
                }
            },
            _ = recheck.tick() => {
                let grabbed = probe_grabbed(stream.device_mut());
                let changed = {
                    let mut reg = registry.lock().expect("keyboard registry poisoned");
                    match reg.get_mut(&path) {
                        Some(st) if st.grabbed != grabbed => { st.grabbed = grabbed; true }
                        _ => false,
                    }
                };
                if changed {
                    if grabbed {
                        log_grabbed(&name, &path);
                    } else {
                        tracing::info!(device = %name, "keyboard grab released — Tab presses reach the tracker again");
                    }
                    log_all_grabbed(&registry, false);
                }
            }
        }
    }
    registry
        .lock()
        .expect("keyboard registry poisoned")
        .remove(&path);
    log_all_grabbed(&registry, false);
}

/// Pick up keyboards that appear after startup (USB re-plug, KVM switch, a
/// hotkey daemon's uinput pass-through device). Cheap: a `stat` of
/// `/dev/input` per tick, a full enumerate only when its mtime moved or the
/// slow full-rescan timer fired. Duplicate Tab events (a physical key echoed
/// by a non-grabbing pass-through device) are absorbed by the main loop's Tab
/// debounce.
async fn hotplug_scanner(registry: Registry, tx: mpsc::UnboundedSender<()>) {
    let dir = std::path::Path::new("/dev/input");
    let mut last_mtime = std::fs::metadata(dir).and_then(|m| m.modified()).ok();
    let mut last_full = Instant::now();
    let mut ticker = tokio::time::interval(HOTPLUG_SCAN_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        ticker.tick().await;
        let mtime = std::fs::metadata(dir).and_then(|m| m.modified()).ok();
        let due = mtime != last_mtime || last_full.elapsed() >= HOTPLUG_FULL_RESCAN;
        if !due {
            continue;
        }
        last_mtime = mtime;
        last_full = Instant::now();
        let before = registry.lock().expect("keyboard registry poisoned").len();
        let after = scan_and_open(&registry, &tx);
        if after != before {
            tracing::info!(device_count = after, "keyboard set changed");
            log_all_grabbed(&registry, false);
        }
    }
}
