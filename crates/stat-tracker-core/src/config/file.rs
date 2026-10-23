//! Loading and saving `config.toml`: the CLI/env overlay of [`Config::load`],
//! the file path, and the owner-only atomic write.

use super::{Config, SyncConfig};

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

        let mut config = Self::read_at(&config_path)?;

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
        // no file yet, avoids requiring flags on every subsequent start.
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

        // SCUFFED_SHADOW_RECOGNIZER is read by `shadow_recognizer_control()`,
        // never folded into the struct, so a Settings save can't persist it.

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

    /// Parse `config.toml` text the way [`Self::load`] reads the file.
    ///
    /// No CLI or env overlay. A missing `shadow_recognizer` stays off.
    ///
    /// The error names the line and toml's reason but never quotes the file,
    /// and secret values (the sync token) are redacted from it, so it is safe
    /// for Settings and the logs (`config/parse_error.rs`).
    pub fn parse_file_contents(
        content: &str,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        toml::from_str(content)
            .map_err(|err| unreadable_config(super::parse_error::describe(content, &err)))
    }

    /// Read the on-disk file [`Self::load`] starts from, without env overlays.
    pub fn read_stored() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Self::read_at(&Self::config_path()?)
    }

    /// Re-read `config.toml` at `path`. Settings calls this on every Save.
    ///
    /// A missing file is the default config. A file that does not parse is an
    /// error, and the caller must not write over it.
    pub fn read_at(
        config_path: &std::path::Path,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        if config_path.exists() {
            // The file carries the sync bearer token. Tighten permissions on
            // files written before saves enforced 0600.
            Self::restrict_permissions(config_path);
            let content = std::fs::read_to_string(config_path)?;
            Self::parse_file_contents(&content)
        } else {
            Ok(Self::default())
        }
    }

    /// Path of the user config file (`~/.config/scuffed-stat-tracker/config.toml`).
    pub fn config_path() -> Result<std::path::PathBuf, Box<dyn std::error::Error + Send + Sync>> {
        Ok(dirs::config_dir()
            .ok_or("no config directory found")?
            .join("scuffed-stat-tracker")
            .join("config.toml"))
    }

    /// Serialize and write the config, owner-readable only. The file carries
    /// the sync bearer token.
    ///
    /// When the file already on disk parses as this same config, it is left
    /// untouched, so comments and key layout survive an unchanged Settings save.
    /// A file that does not parse is left untouched too: save returns an error
    /// instead of replacing it with defaults.
    pub fn save(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.save_at(&Self::config_path()?)
    }

    /// In-place write of this config to `path`. Same rules as [`Self::save`].
    pub fn save_at(
        &self,
        path: &std::path::Path,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.save_to_path(path)
    }

    pub(super) fn save_to_path(
        &self,
        path: &std::path::Path,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if let Some(dir) = path.parent()
            && !dir.as_os_str().is_empty()
        {
            std::fs::create_dir_all(dir)?;
        }
        let existing = std::fs::read_to_string(path).ok();
        let toml = Self::text_for_save(existing.as_deref(), self)?;
        if existing.as_deref() == Some(toml.as_str()) {
            return Ok(());
        }
        atomic_write_600(path, toml.as_bytes())
    }

    /// Best-effort chmod 600 (no-op off unix).
    pub(super) fn restrict_permissions(path: &std::path::Path) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
        #[cfg(not(unix))]
        let _ = path;
    }

    /// Find a `--key value` pair in std::env::args().
    fn arg_value(key: &str) -> Option<String> {
        let args: Vec<String> = std::env::args().collect();
        args.windows(2).find(|w| w[0] == key).map(|w| w[1].clone())
    }
}

pub(super) fn unreadable_config(message: String) -> Box<dyn std::error::Error + Send + Sync> {
    message.into()
}

/// Write `bytes` by creating a 0600 temp file in the same directory, fsyncing
/// it, and renaming it over `path`. A failed write removes the temp file.
pub(super) fn atomic_write_600(
    path: &std::path::Path,
    bytes: &[u8],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let dir = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| unreadable_config("config path has no directory".to_string()))?;
    let file_name = path
        .file_name()
        .ok_or_else(|| unreadable_config("config path has no file name".to_string()))?;
    let mut tmp_name = file_name.to_os_string();
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    tmp_name.push(format!(".{}.{unique}.tmp", std::process::id()));
    let tmp_path = dir.join(tmp_name);

    let write_result = (|| -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut file = open_private(&tmp_path)?;
        use std::io::Write;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&tmp_path, path)?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = std::fs::remove_file(&tmp_path);
    }
    write_result
}

fn open_private(
    path: &std::path::Path,
) -> Result<std::fs::File, Box<dyn std::error::Error + Send + Sync>> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)?;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        Ok(file)
    }
    #[cfg(not(unix))]
    {
        Ok(std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?)
    }
}
