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
            enabled: false,
            poll_interval_secs: 4,
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
    pub fn load() -> Result<Self, Box<dyn std::error::Error>> {
        let config_dir = dirs::config_dir()
            .ok_or("no config directory found")?
            .join("scuffed-stat-tracker");

        let config_path = config_dir.join("config.toml");

        if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)?;
            let config: Config = toml::from_str(&content)?;
            Ok(config)
        } else {
            Ok(Self::default())
        }
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
