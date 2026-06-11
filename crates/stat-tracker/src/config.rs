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
}

fn default_session_window_secs() -> u64 {
    1800
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
    pub fn load() -> Result<Self, Box<dyn std::error::Error>> {
        let config_dir = dirs::config_dir()
            .ok_or("no config directory found")?
            .join("scuffed-stat-tracker");

        let config_path = config_dir.join("config.toml");

        let mut config = if config_path.exists() {
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
            let _ = std::fs::create_dir_all(&config_dir);
            if let Ok(toml) = toml::to_string_pretty(&config) {
                let _ = std::fs::write(&config_path, toml);
                tracing::info!(path = %config_path.display(), "wrote initial config.toml");
            }
        }

        Ok(config)
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
        }
    }
}
