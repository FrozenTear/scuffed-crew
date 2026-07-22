use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub data_dir: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capture_output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub player_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sync: Option<SyncConfig>,
    #[serde(default)]
    pub auto_detect: AutoDetectConfig,
    #[serde(default = "default_session_window_secs")]
    pub session_window_secs: u64,
    /// Process names (as they appear in /proc/<pid>/comm) that must be running
    /// for captures and auto-detect polling to fire. Prevents Tab presses on the
    /// desktop / in other apps from recording garbage frames. Empty list
    /// disables the gate.
    #[serde(default = "default_game_process_names")]
    pub game_process_names: Vec<String>,
    /// When true, every Tab OCR writes intermediate PNGs under `{data_dir}/debug/`.
    /// Off by default — the pipeline recomputes preprocess just to dump stages and
    /// dominated capture latency. Also enabled by env `STAT_TRACKER_DEBUG_OCR=1`.
    #[serde(default)]
    pub debug_ocr: bool,
    /// Parallel OCR workers (each keeps a ~23 MB Tesseract model resident).
    /// `None` / omit = auto (`(cores/2).clamp(2, 4)`). Set to `1` for lowest
    /// RAM, higher for faster Tab OCR. Clamped to 1..=8 at resolve time.
    /// Overlay: env `STAT_TRACKER_OCR_THREADS`, CLI `--ocr-threads N`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ocr_threads: Option<u32>,
}

fn default_session_window_secs() -> u64 {
    1800
}

fn default_game_process_names() -> Vec<String> {
    vec!["Overwatch.exe".to_string()]
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct AutoDetectConfig {
    pub enabled: bool,
    pub poll_interval_secs: u64,
    pub cooldown_secs: u64,
}

impl Default for AutoDetectConfig {
    fn default() -> Self {
        Self {
            // The poller watches for the map-vote (game start) and post-match
            // accolade (win/loss) screens — required for automatic win/loss
            // detection — at ~25-35ms of CPU per tick. Enabled by default.
            enabled: true,
            poll_interval_secs: 4,
            // Debounce between opening new games: long enough that a map-vote
            // screen lingering across several ticks only opens one game, far
            // shorter than a real match so back-to-back games are still caught.
            cooldown_secs: 120,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SyncConfig {
    pub server_url: String,
    pub token: String,
}

impl Config {
    /// Load config from file, then overlay CLI args / env vars.
    ///
    /// CLI args take precedence over the file. If a token/server are supplied
    /// without a pre-existing config file, the resolved config is written back
    /// so the user doesn't have to pass flags on every start.
    pub fn load() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let config_dir = dirs::config_dir()
            .ok_or("no config directory found")?
            .join("scuffed-stat-tracker");

        let config_path = config_dir.join("config.toml");

        let mut config = if config_path.exists() {
            // The file carries the sync bearer token — tighten permissions on
            // files written before saves enforced 0600.
            Self::restrict_permissions(&config_path);
            let content = std::fs::read_to_string(&config_path)?;
            toml::from_str::<Config>(&content)?
        } else {
            Config::default()
        };

        // CLI / env overlay: --token / SCUFFED_TOKEN and --server / SCUFFED_SERVER
        let cli_token = Self::arg_value("--token").or_else(|| std::env::var("SCUFFED_TOKEN").ok());
        let cli_server =
            Self::arg_value("--server").or_else(|| std::env::var("SCUFFED_SERVER").ok());

        if let Some(token) = cli_token {
            let server = cli_server.unwrap_or_else(|| {
                // Preserve existing server URL if only --token was given
                config
                    .sync
                    .as_ref()
                    .map(|s| s.server_url.clone())
                    .unwrap_or_default()
            });
            config.sync = Some(SyncConfig {
                server_url: server,
                token,
            });
        }

        // Auto-save if we built a usable sync config from CLI args and there was
        // no file yet — avoids requiring flags on every subsequent start.
        if !config_path.exists()
            && let Some(sync) = &config.sync
            && !sync.server_url.is_empty()
            && !sync.token.is_empty()
        {
            match config.save() {
                Ok(()) => {
                    tracing::info!(path = %config_path.display(), "wrote initial config.toml")
                }
                Err(e) => tracing::warn!(error = %e, "failed to write initial config.toml"),
            }
        }

        // Env overlay for OCR debug dumps (config flag OR STAT_TRACKER_DEBUG_OCR=1).
        if Self::env_truthy("STAT_TRACKER_DEBUG_OCR") {
            config.debug_ocr = true;
        }

        // OCR worker count: CLI > env > config file > auto (None).
        if let Some(raw) = Self::arg_value("--ocr-threads")
            .or_else(|| std::env::var("STAT_TRACKER_OCR_THREADS").ok())
        {
            match raw.parse::<u32>() {
                Ok(n) if n > 0 => config.ocr_threads = Some(n),
                Ok(_) => tracing::warn!(
                    value = %raw,
                    "STAT_TRACKER_OCR_THREADS / --ocr-threads must be >= 1; ignoring"
                ),
                Err(_) => tracing::warn!(
                    value = %raw,
                    "invalid STAT_TRACKER_OCR_THREADS / --ocr-threads; ignoring"
                ),
            }
        }

        Ok(config)
    }

    /// Path of the user config file (`~/.config/scuffed-stat-tracker/config.toml`).
    pub fn config_path() -> Result<std::path::PathBuf, Box<dyn std::error::Error + Send + Sync>> {
        Ok(dirs::config_dir()
            .ok_or("no config directory found")?
            .join("scuffed-stat-tracker")
            .join("config.toml"))
    }

    /// Serialize and write the config, owner-readable only — the file carries
    /// the sync bearer token.
    pub fn save(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let path = Self::config_path()?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let toml = toml::to_string_pretty(self)?;
        std::fs::write(&path, toml)?;
        Self::restrict_permissions(&path);
        Ok(())
    }

    /// Best-effort chmod 600 (no-op off unix).
    fn restrict_permissions(path: &std::path::Path) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
        #[cfg(not(unix))]
        let _ = path;
    }

    /// Whether OCR should write debug PNGs this process (config and/or env).
    pub fn debug_ocr_enabled(&self) -> bool {
        self.debug_ocr || Self::env_truthy("STAT_TRACKER_DEBUG_OCR")
    }

    /// Resolved OCR worker count for the Rayon pool (and thus Tesseract instances).
    /// Explicit config/env/CLI wins; otherwise auto from host parallelism.
    pub fn ocr_threads_resolved(&self) -> usize {
        if let Some(n) = self.ocr_threads {
            return (n as usize).clamp(1, 8);
        }
        Self::default_ocr_threads()
    }

    /// Auto worker count when `ocr_threads` is unset: half the cores, 2..=4.
    pub fn default_ocr_threads() -> usize {
        let total = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        (total / 2).clamp(2, 4)
    }

    fn env_truthy(key: &str) -> bool {
        matches!(
            std::env::var(key).as_deref(),
            Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
        )
    }

    /// Find a `--key value` pair in std::env::args().
    fn arg_value(key: &str) -> Option<String> {
        let args: Vec<String> = std::env::args().collect();
        args.windows(2).find(|w| w[0] == key).map(|w| w[1].clone())
    }
}

impl Default for Config {
    fn default() -> Self {
        let data_dir = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("scuffed-stat-tracker");

        Self {
            data_dir,
            capture_output: None,
            player_name: None,
            sync: None,
            auto_detect: AutoDetectConfig::default(),
            session_window_secs: default_session_window_secs(),
            game_process_names: default_game_process_names(),
            debug_ocr: false,
            ocr_threads: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ocr_threads_resolved_clamps_and_auto() {
        let mut c = Config::default();
        assert!(
            (2..=4).contains(&c.ocr_threads_resolved()),
            "auto should stay in the historical 2..=4 band"
        );
        c.ocr_threads = Some(1);
        assert_eq!(c.ocr_threads_resolved(), 1);
        c.ocr_threads = Some(3);
        assert_eq!(c.ocr_threads_resolved(), 3);
        c.ocr_threads = Some(99);
        assert_eq!(c.ocr_threads_resolved(), 8);
        c.ocr_threads = Some(0);
        assert_eq!(c.ocr_threads_resolved(), 1);
    }
}
