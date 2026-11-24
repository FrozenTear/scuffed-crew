//! Capture settings: `capture_route` and the `STAT_TRACKER_CAPTURE`
//! override (with the environment.d files that set it), the screen-stream
//! rate and pause keys, and `debug_portal_frames`.

use serde::{Deserialize, Deserializer, Serialize};

use super::Config;

/// `capture_route` in config.toml. Missing means [`Self::Auto`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CaptureRoute {
    #[default]
    Auto,
    Screencast,
    Screenshot,
    /// KDE direct: KWin's ScreenShot2 interface.
    Kwin,
}

impl CaptureRoute {
    /// `auto`, `screencast`, `screenshot`, `kwin`, and the env names
    /// `portal`, `portal-screenshot` and `kde-direct`. Case does not matter. [`Self::parse`] returns
    /// `None` for an unknown word. Config load does not: an unknown
    /// `capture_route` falls back to automatic so the daemon still starts.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "screencast" | "portal" => Some(Self::Screencast),
            "screenshot" | "portal-screenshot" => Some(Self::Screenshot),
            "kwin" | "kde-direct" => Some(Self::Kwin),
            _ => None,
        }
    }

    pub const fn is_auto(&self) -> bool {
        matches!(self, Self::Auto)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Screencast => "screencast",
            Self::Screenshot => "screenshot",
            Self::Kwin => "kwin",
        }
    }
}

impl<'de> Deserialize<'de> for CaptureRoute {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Ok(Self::parse(&raw).unwrap_or_else(|| {
            // A short word is a typo worth showing. Anything longer may be a
            // token pasted into the wrong key, so it stays out of the log.
            let shown =
                if raw.len() <= 16 && raw.chars().all(|c| c.is_ascii_alphabetic() || c == '-') {
                    raw.as_str()
                } else {
                    "(value hidden)"
                };
            tracing::warn!(
                capture_route = %shown,
                "unknown capture_route; using auto"
            );
            Self::Auto
        }))
    }
}

/// Env var that wins over [`Config::capture_route`] when it is non-empty.
pub const CAPTURE_ENV: &str = "STAT_TRACKER_CAPTURE";

/// Default screen-stream cap. QA can set `screencast_max_fps` to compare.
pub const SCREENCAST_MAX_FPS_DEFAULT: u32 = 4;
pub const SCREENCAST_MAX_FPS_MIN: u32 = 1;
pub const SCREENCAST_MAX_FPS_MAX: u32 = 5;

/// Clamp a configured stream cap. Missing means [`SCREENCAST_MAX_FPS_DEFAULT`].
pub fn clamp_screencast_max_fps(raw: Option<u32>) -> u32 {
    match raw {
        None => SCREENCAST_MAX_FPS_DEFAULT,
        Some(n) => n.clamp(SCREENCAST_MAX_FPS_MIN, SCREENCAST_MAX_FPS_MAX),
    }
}

impl Config {
    /// Screen-stream cap in frames per second, after clamping.
    pub fn screencast_max_fps(&self) -> u32 {
        clamp_screencast_max_fps(self.screencast_max_fps)
    }

    /// The pause-between-grabs setting with [`SCREENCAST_PAUSE_ENV`]
    /// applied. The capture module decides `Auto` for the compositor.
    pub fn screencast_pause_override(&self) -> PauseOverride {
        screencast_pause_override(
            self.screencast_pause_between_grabs,
            std::env::var(SCREENCAST_PAUSE_ENV).ok().as_deref(),
        )
    }
}

/// `1`, `true`, `on`, or `yes` pauses the screen stream between grabs on
/// any compositor (for testing). `0`, `false`, `off`, or `no` keeps it
/// running. Any other value, or unset, leaves the config key in charge.
pub const SCREENCAST_PAUSE_ENV: &str = "STAT_TRACKER_SCREENCAST_PAUSE";

/// Where pausing the screen stream between grabs was decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseOverride {
    /// The env var forces it on, on any compositor.
    EnvOn,
    /// The env var turns it off.
    EnvOff,
    /// `screencast_pause_between_grabs = false`: off everywhere.
    ConfigOff,
    /// Missing or `true`: on where the compositor is known to handle it
    /// (KWin 6.1 or newer, niri).
    Auto,
}

/// The env override first, then the file key.
pub fn screencast_pause_override(file: Option<bool>, env: Option<&str>) -> PauseOverride {
    let from_env = env.map(|v| v.trim().to_ascii_lowercase());
    match from_env.as_deref() {
        Some("0" | "false" | "off" | "no") => PauseOverride::EnvOff,
        Some("1" | "true" | "on" | "yes") => PauseOverride::EnvOn,
        _ if file == Some(false) => PauseOverride::ConfigOff,
        _ => PauseOverride::Auto,
    }
}

impl Config {
    /// File flag or `STAT_TRACKER_DEBUG_PORTAL_FRAMES`. The env value is not
    /// stored on the struct, so a Settings save cannot persist it.
    pub fn debug_portal_frames_enabled(&self) -> bool {
        self.debug_portal_frames || Self::env_truthy("STAT_TRACKER_DEBUG_PORTAL_FRAMES")
    }

    /// One `STAT_TRACKER_CAPTURE=` line from an `environment.d` file.
    /// `path` is the file name the Settings screen should name.
    pub fn capture_env_note(process: Option<&str>, files: &[(String, String)]) -> Option<String> {
        let from_file = winning_env_assignment(files);
        let process = process.map(str::trim).filter(|s| !s.is_empty());
        match (process, from_file) {
            (Some(value), Some((path, _))) => Some(format!(
                "STAT_TRACKER_CAPTURE={value} is overriding this choice. Edit {path}."
            )),
            (Some(value), None) => Some(format!(
                "STAT_TRACKER_CAPTURE={value} is overriding this choice. For the tracker service, set it in a file under .config/environment.d (for example scuffed-capture.conf)."
            )),
            (None, Some((path, value))) => Some(format!(
                "{path} sets STAT_TRACKER_CAPTURE={value}, but this session has not loaded it. Log out and back in for it to override this choice."
            )),
            (None, None) => None,
        }
    }
}

/// Last `STAT_TRACKER_CAPTURE` assignment across `files` in the order given.
/// Callers pass `environment.d` entries in alphabetical order. A later file
/// wins, and the last assignment in a file wins.
pub fn winning_env_assignment(files: &[(String, String)]) -> Option<(&str, &str)> {
    let mut found: Option<(&str, &str)> = None;
    for (path, text) in files {
        if let Some(value) = last_capture_assignment(text) {
            found = Some((path.as_str(), value));
        }
    }
    found
}

fn last_capture_assignment(text: &str) -> Option<&str> {
    let mut found = None;
    for line in text.lines() {
        let line = line.trim();
        let line = line.strip_prefix("export").unwrap_or(line).trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let Some(rest) = line.strip_prefix("STAT_TRACKER_CAPTURE=") else {
            continue;
        };
        let value = rest.trim().trim_matches('"').trim_matches('\'');
        if !value.is_empty() {
            found = Some(value);
        }
    }
    found
}

/// `~/.config/environment.d/*.conf`, alphabetical. The value is the last
/// assignment in that file.
pub fn environment_d_capture_files() -> Vec<(String, String)> {
    // Settings draws this on every frame. The directory does not change
    // that often, so a short cache keeps the read off the hot path.
    const TTL: std::time::Duration = std::time::Duration::from_secs(2);
    type EnvCache = Option<(std::time::Instant, Vec<(String, String)>)>;
    static CACHE: std::sync::Mutex<EnvCache> = std::sync::Mutex::new(None);
    let mut guard = CACHE.lock().unwrap_or_else(|err| err.into_inner());
    if let Some((at, files)) = guard.as_ref()
        && at.elapsed() < TTL
    {
        return files.clone();
    }
    let files = read_environment_d_capture_files();
    *guard = Some((std::time::Instant::now(), files.clone()));
    files
}

fn read_environment_d_capture_files() -> Vec<(String, String)> {
    let Some(dir) = dirs::config_dir() else {
        return Vec::new();
    };
    let dir = dir.join("environment.d");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut paths: Vec<std::path::PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("conf"))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .filter_map(|path| {
            let text = std::fs::read_to_string(&path).ok()?;
            last_capture_assignment(&text)?;
            Some((path.display().to_string(), text))
        })
        .collect()
}
