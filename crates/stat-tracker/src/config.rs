use std::path::PathBuf;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
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
    /// Quiet time after the last activity before a finished game is closed
    /// and uploaded. Activity is a stored capture, a recorded outcome, an
    /// accolade map, or the session open. The daemon clamps this to the
    /// 75-second post-match grace, so a shorter setting still waits that
    /// grace out. Missing means [`FINISHED_GAME_CLOSE_DEFAULT_SECS`].
    /// Config-file only: Settings keeps the value from the file.
    #[serde(default = "default_finished_game_close_secs")]
    pub finished_game_close_secs: u64,
    /// Process names (as they appear in /proc/<pid>/comm) that must be running
    /// for captures and auto-detect polling to fire. Prevents Tab presses on the
    /// desktop / in other apps from recording garbage frames. Empty list
    /// disables the gate.
    #[serde(default = "default_game_process_names")]
    pub game_process_names: Vec<String>,
    /// When true, Tab OCR writes intermediate PNGs under `{data_dir}/debug/`,
    /// and the poller writes Victory/Defeat evidence frames to `debug/poll/`
    /// on confirm (banner or second agreeing word-OCR) and first word-OCR
    /// streak — not every mid-match tick. Off by default — the Tab path
    /// recomputes preprocess just to dump stages and dominated capture
    /// latency. Also enabled by env `STAT_TRACKER_DEBUG_OCR=1`.
    #[serde(default)]
    pub debug_ocr: bool,
    /// Parallel OCR workers (each keeps a ~23 MB Tesseract model resident).
    /// `None` / omit = auto (`(cores/2).clamp(2, 4)`). Set to `1` for lowest
    /// RAM, higher for faster Tab OCR. Clamped to 1..=8 at resolve time.
    /// Overlay: env `STAT_TRACKER_OCR_THREADS`, CLI `--ocr-threads N`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ocr_threads: Option<u32>,
    /// Shadow digit recognizer. When true, a background thread reads each
    /// accepted scoreboard with a template digit matcher and appends where it
    /// disagrees with ocr-v1 to `{data_dir}/shadow/digits.jsonl` (capped at
    /// about 4 MB). Log only: stored stats, the capture gate, and uploads
    /// still use ocr-v1, unless [`Self::reader`] is [`ReaderSetting::New`].
    /// That setting forces this log on for the process. Off by default. Also
    /// enabled by env `SCUFFED_SHADOW_RECOGNIZER=1`. When hero icon templates
    /// are installed in `{data_dir}/templates/heroes/`, the same thread also
    /// reads each Tab row's hero and logs it to `{data_dir}/shadow/heroes.jsonl`;
    /// without templates that step is skipped.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub shadow_recognizer: bool,
    /// First-run guide finished or skipped. Missing means not completed, so
    /// the desktop app shows the guide. The guide writes this with the same
    /// in-place edit as `shadow_recognizer`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub setup_completed: bool,
    /// Optional https address of a reader template pack. The setup guide
    /// offers the download when this is set, and skips that step when it is
    /// empty. Config-file only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reader_pack_url: Option<String>,
    /// Which reader supplies saved and uploaded stats.
    ///
    /// Missing, or `"ocr-v1"`, keeps today's Tesseract reads. `"new"` stores
    /// the shadow reader's values when a field is read and not suspect, and
    /// falls back to ocr-v1 per field otherwise. Game start, game end, screen
    /// detection, and plausibility holds stay on ocr-v1 either way.
    #[serde(default, skip_serializing_if = "ReaderSetting::is_ocr_v1")]
    pub reader: ReaderSetting,
}

/// `reader` in config.toml. Missing means [`Self::OcrV1`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReaderSetting {
    #[default]
    OcrV1,
    New,
}

impl ReaderSetting {
    pub const OCR_V1: &'static str = "ocr-v1";
    pub const NEW: &'static str = "new";

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OcrV1 => Self::OCR_V1,
            Self::New => Self::NEW,
        }
    }

    pub const fn is_ocr_v1(&self) -> bool {
        matches!(self, Self::OcrV1)
    }

    pub const fn is_new(self) -> bool {
        matches!(self, Self::New)
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            Self::OCR_V1 => Some(Self::OcrV1),
            Self::NEW => Some(Self::New),
            _ => None,
        }
    }
}

impl Serialize for ReaderSetting {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ReaderSetting {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).ok_or_else(|| D::Error::custom("reader must be \"ocr-v1\" or \"new\""))
    }
}

fn default_session_window_secs() -> u64 {
    1800
}

/// Default quiet period before the last finished game of a session is
/// closed and uploaded, counted from the last capture.
pub const FINISHED_GAME_CLOSE_DEFAULT_SECS: u64 = 180;

fn default_finished_game_close_secs() -> u64 {
    FINISHED_GAME_CLOSE_DEFAULT_SECS
}

fn default_game_process_names() -> Vec<String> {
    vec!["Overwatch.exe".to_string()]
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
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

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct SyncConfig {
    pub server_url: String,
    pub token: String,
}

/// Settings copy and the daemon log share this sentence. Plain `http` to a
/// public host would put the bearer token on the wire for any network observer.
pub const SYNC_URL_HTTPS_REQUIRED: &str =
    "Website URL must use https. Plain http is only allowed for localhost, 127.0.0.1, and [::1].";

pub const SYNC_URL_INVALID: &str = "Website URL is not a valid http(s) address.";

/// Why a server URL must not carry the sync bearer token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncUrlReject {
    /// `http://` to a host that is not loopback, or a non-http(s) scheme.
    HttpsRequired,
    /// Empty, unparseable, or missing a host.
    Invalid,
}

impl SyncUrlReject {
    pub fn message(self) -> &'static str {
        match self {
            Self::HttpsRequired => SYNC_URL_HTTPS_REQUIRED,
            Self::Invalid => SYNC_URL_INVALID,
        }
    }
}

impl std::fmt::Display for SyncUrlReject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for SyncUrlReject {}

/// `Ok` only when attaching the bearer token cannot leave the machine in the
/// clear. `https` is allowed for any host. `http` is allowed only for
/// loopback (`localhost`, `127.0.0.0/8`, `::1`, and IPv4-mapped loopback),
/// which covers local dev (`http://localhost`, `http://127.0.0.1`,
/// `http://[::1]`, any port).
///
/// Parsing is the URL standard (via reqwest's `Url`), so `http://127.0.0.1.evil`
/// and `http://10.0.0.1` are rejected without a DNS lookup. Existing configs
/// still deserialize; callers refuse to send rather than crash.
pub fn validate_sync_server_url(raw: &str) -> Result<(), SyncUrlReject> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(SyncUrlReject::Invalid);
    }
    let parsed = reqwest::Url::parse(raw).map_err(|_| SyncUrlReject::Invalid)?;
    match parsed.scheme() {
        "https" if parsed.host_str().is_some_and(|h| !h.is_empty()) => Ok(()),
        "http" if http_host_is_loopback(&parsed) => Ok(()),
        _ => Err(SyncUrlReject::HttpsRequired),
    }
}

fn http_host_is_loopback(url: &reqwest::Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    // `host_str` wraps IPv6 in brackets (`[::1]`).
    let bare = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    if let Ok(v4) = bare.parse::<std::net::Ipv4Addr>() {
        return v4.is_loopback();
    }
    if let Ok(v6) = bare.parse::<std::net::Ipv6Addr>() {
        if v6.is_loopback() {
            return true;
        }
        if let Some(mapped) = v6.to_ipv4_mapped() {
            return mapped.is_loopback();
        }
    }
    false
}

/// Settings view of `shadow_recognizer`.
///
/// `file_on` is the config.toml value. `locked` means
/// `SCUFFED_SHADOW_RECOGNIZER` forces the extra reader on for this process.
/// A save writes `file_on` only. The checkbox can later become a reader
/// picker without a second env rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShadowRecognizerControl {
    pub file_on: bool,
    pub locked: bool,
}

impl ShadowRecognizerControl {
    /// On for this process. Same rule the daemon uses.
    pub fn enabled(self) -> bool {
        self.file_on || self.locked
    }
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
    pub fn parse_file_contents(
        content: &str,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Ok(toml::from_str(content)?)
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

    fn save_to_path(
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

    /// Text `save` would write.
    ///
    /// An `existing` document that parses as `next` is returned unchanged
    /// (byte for byte), comments included. When the only change is
    /// `shadow_recognizer`, that key is edited in place so comments, order,
    /// and every other key stay as they were. A document that does not parse
    /// is an error, not a pretty-printed replacement. A missing file (no
    /// `existing` text) is a fresh pretty-printed document.
    pub fn text_for_save(
        existing: Option<&str>,
        next: &Self,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let Some(existing) = existing else {
            return Ok(toml::to_string_pretty(next)?);
        };
        let loaded = toml::from_str::<Self>(existing).map_err(|err| {
            unreadable_config(format!(
                "config.toml could not be parsed, so it was not replaced: {err}"
            ))
        })?;
        if loaded == *next {
            return Ok(existing.to_string());
        }
        if only_shadow_recognizer_differs(&loaded, next) {
            return patch_shadow_recognizer(existing, next.shadow_recognizer);
        }
        if only_reader_differs(&loaded, next) {
            return patch_reader(existing, next.reader);
        }
        if only_reader_and_shadow_differ(&loaded, next) {
            let with_shadow = patch_shadow_recognizer(existing, next.shadow_recognizer)?;
            return patch_reader(&with_shadow, next.reader);
        }
        Ok(toml::to_string_pretty(next)?)
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

    /// Whether the shadow digit recognizer runs this process (config file
    /// and/or env `SCUFFED_SHADOW_RECOGNIZER`).
    ///
    /// The env override is read only by [`Self::shadow_recognizer_control`],
    /// so `shadow_recognizer` always holds the file value and a save never
    /// writes the override.
    pub fn shadow_recognizer_enabled(&self) -> bool {
        self.shadow_recognizer_control().enabled()
    }

    /// `reader = "new"`. Saved and uploaded stats may come from the new reader.
    pub fn uses_new_reader(&self) -> bool {
        self.reader.is_new()
    }

    /// The shadow log runs when its own flag is on, or when saved stats use
    /// the new reader. The file flag stays independent of `reader`.
    pub fn shadow_log_enabled(&self) -> bool {
        self.shadow_recognizer_enabled() || self.uses_new_reader()
    }

    /// File value plus whether `SCUFFED_SHADOW_RECOGNIZER` locks this process on.
    ///
    /// This is the only read of that variable, and it is this process's
    /// environment. The tracker service is started by systemd with
    /// `session.env`, and it does not report the shadow reader back here, so
    /// Settings cannot treat this lock as the service's. `file_on` is what
    /// Settings may write. `locked` is display-only and must not be saved.
    pub fn shadow_recognizer_control(&self) -> ShadowRecognizerControl {
        Self::shadow_control(
            self.shadow_recognizer,
            std::env::var("SCUFFED_SHADOW_RECOGNIZER").ok().as_deref(),
        )
    }

    /// Pure form of [`Self::shadow_recognizer_control`] for tests and Settings.
    /// `env` is the raw variable value, not a process lookup.
    pub fn shadow_control(file_flag: bool, env: Option<&str>) -> ShadowRecognizerControl {
        ShadowRecognizerControl {
            file_on: file_flag,
            locked: Self::truthy(env),
        }
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
        Self::truthy(std::env::var(key).ok().as_deref())
    }

    fn truthy(value: Option<&str>) -> bool {
        matches!(value, Some("1" | "true" | "TRUE" | "yes" | "YES"))
    }

    /// Find a `--key value` pair in std::env::args().
    fn arg_value(key: &str) -> Option<String> {
        let args: Vec<String> = std::env::args().collect();
        args.windows(2).find(|w| w[0] == key).map(|w| w[1].clone())
    }
}

fn unreadable_config(message: String) -> Box<dyn std::error::Error + Send + Sync> {
    message.into()
}

fn only_shadow_recognizer_differs(loaded: &Config, next: &Config) -> bool {
    if loaded.shadow_recognizer == next.shadow_recognizer {
        return false;
    }
    let mut same = loaded.clone();
    same.shadow_recognizer = next.shadow_recognizer;
    same == *next
}

fn only_reader_differs(loaded: &Config, next: &Config) -> bool {
    if loaded.reader == next.reader {
        return false;
    }
    let mut same = loaded.clone();
    same.reader = next.reader;
    same == *next
}

fn only_reader_and_shadow_differ(loaded: &Config, next: &Config) -> bool {
    if loaded.reader == next.reader || loaded.shadow_recognizer == next.shadow_recognizer {
        return false;
    }
    let mut same = loaded.clone();
    same.reader = next.reader;
    same.shadow_recognizer = next.shadow_recognizer;
    same == *next
}

/// Change `shadow_recognizer` and nothing else.
///
/// A root boolean is spliced in place, so comments, order, spacing, and
/// every other key stay byte for byte. A missing root key is inserted on
/// the root table, before the first `[table]` header (or at the end when
/// the file has no tables). A copy of the key inside a table is left
/// alone, and the root key is never inserted twice.
fn patch_shadow_recognizer(
    existing: &str,
    on: bool,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let doc = toml_edit::Document::parse(existing).map_err(|err| {
        unreadable_config(format!(
            "config.toml could not be parsed, so it was not replaced: {err}"
        ))
    })?;
    if doc.get("shadow_recognizer").is_some() {
        return splice_root_shadow_bool(existing, &doc, on);
    }
    insert_root_shadow_recognizer(existing, on)
}

fn splice_root_shadow_bool(
    existing: &str,
    doc: &toml_edit::Document<&str>,
    on: bool,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let wanted = if on { "true" } else { "false" };
    if let Some(item) = doc.get("shadow_recognizer")
        && let Some(value) = item.as_value()
        && value.as_bool().is_some()
        && let Some(span) = value.span()
        && existing.is_char_boundary(span.start)
        && existing.is_char_boundary(span.end)
    {
        let current = &existing[span.start..span.end];
        if current == "true" || current == "false" {
            let mut out = String::with_capacity(existing.len() + wanted.len());
            out.push_str(&existing[..span.start]);
            out.push_str(wanted);
            out.push_str(&existing[span.end..]);
            return Ok(out);
        }
    }
    // The root key is already there. Inserting another would be a duplicate.
    Err(unreadable_config(
        "config.toml could not be updated in place, so it was not replaced".to_string(),
    ))
}

/// Insert `shadow_recognizer` on the document root.
///
/// `toml_edit` emits root values before standard tables, so the new key
/// stays at the top level instead of falling into the last `[table]`.
fn insert_root_shadow_recognizer(
    existing: &str,
    on: bool,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let mut doc = existing.parse::<toml_edit::DocumentMut>().map_err(|err| {
        unreadable_config(format!(
            "config.toml could not be parsed, so it was not replaced: {err}"
        ))
    })?;
    if doc.get("shadow_recognizer").is_some() {
        return Err(unreadable_config(
            "config.toml already has shadow_recognizer at the root, so it was not replaced"
                .to_string(),
        ));
    }
    doc.as_table_mut()
        .insert("shadow_recognizer", toml_edit::value(on));
    Ok(doc.to_string())
}

/// Change `reader` and nothing else.
///
/// A root string is spliced in place. A missing root key is inserted on the
/// root table, before the first `[table]` header. Turning the switch off
/// writes `"ocr-v1"` only when the key is already there.
fn patch_reader(
    existing: &str,
    reader: ReaderSetting,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let doc = toml_edit::Document::parse(existing).map_err(|err| {
        unreadable_config(format!(
            "config.toml could not be parsed, so it was not replaced: {err}"
        ))
    })?;
    if doc.get("reader").is_some() {
        return splice_root_reader(existing, &doc, reader);
    }
    insert_root_reader(existing, reader)
}

fn splice_root_reader(
    existing: &str,
    doc: &toml_edit::Document<&str>,
    reader: ReaderSetting,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let wanted = format!("\"{}\"", reader.as_str());
    if let Some(item) = doc.get("reader")
        && let Some(value) = item.as_value()
        && value.as_str().is_some()
        && let Some(span) = value.span()
        && existing.is_char_boundary(span.start)
        && existing.is_char_boundary(span.end)
    {
        let current = &existing[span.start..span.end];
        if current.starts_with('"') && current.ends_with('"') {
            let mut out = String::with_capacity(existing.len() + wanted.len());
            out.push_str(&existing[..span.start]);
            out.push_str(&wanted);
            out.push_str(&existing[span.end..]);
            return Ok(out);
        }
    }
    Err(unreadable_config(
        "config.toml could not be updated in place, so it was not replaced".to_string(),
    ))
}

fn insert_root_reader(
    existing: &str,
    reader: ReaderSetting,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let mut doc = existing.parse::<toml_edit::DocumentMut>().map_err(|err| {
        unreadable_config(format!(
            "config.toml could not be parsed, so it was not replaced: {err}"
        ))
    })?;
    if doc.get("reader").is_some() {
        return Err(unreadable_config(
            "config.toml already has reader at the root, so it was not replaced".to_string(),
        ));
    }
    doc.as_table_mut()
        .insert("reader", toml_edit::value(reader.as_str()));
    Ok(doc.to_string())
}

/// Write `bytes` by creating a 0600 temp file in the same directory, fsyncing
/// it, and renaming it over `path`. A failed write removes the temp file.
fn atomic_write_600(
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
            finished_game_close_secs: default_finished_game_close_secs(),
            game_process_names: default_game_process_names(),
            debug_ocr: false,
            ocr_threads: None,
            shadow_recognizer: false,
            setup_completed: false,
            reader_pack_url: None,
            reader: ReaderSetting::OcrV1,
        }
    }
}

/// Keys the first-run guide may write. A device code is not a field here
/// and must never be added: it stays in memory for the sign-in poll.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SetupDiskPatch {
    pub setup_completed: Option<bool>,
    pub sync: Option<SyncConfig>,
}

impl Config {
    /// Edit `setup_completed` and the sync token in place, the same way
    /// [`Self::text_for_save`] edits `shadow_recognizer`.
    ///
    /// Comments, key order, and every other key stay. A file that does not
    /// parse is left untouched. A missing file becomes a fresh document.
    /// The patch has no device-code field, so this write cannot store one.
    pub fn apply_setup_patch(
        path: &std::path::Path,
        patch: &SetupDiskPatch,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        if let Some(dir) = path.parent()
            && !dir.as_os_str().is_empty()
        {
            std::fs::create_dir_all(dir)?;
        }
        let existing = if path.exists() {
            Some(std::fs::read_to_string(path)?)
        } else {
            None
        };
        let toml = Self::text_for_setup_patch(existing.as_deref(), patch)?;
        if existing.as_deref() != Some(toml.as_str()) {
            atomic_write_600(path, toml.as_bytes())?;
        } else if path.exists() {
            Self::restrict_permissions(path);
        }
        Self::parse_file_contents(&toml)
    }

    /// Text [`Self::apply_setup_patch`] would write.
    pub fn text_for_setup_patch(
        existing: Option<&str>,
        patch: &SetupDiskPatch,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let Some(existing) = existing else {
            let mut fresh = Self::default();
            if let Some(done) = patch.setup_completed {
                fresh.setup_completed = done;
            }
            if let Some(sync) = &patch.sync {
                fresh.sync = Some(sync.clone());
            }
            return Ok(toml::to_string_pretty(&fresh)?);
        };
        let _loaded = toml::from_str::<Self>(existing).map_err(|err| {
            unreadable_config(format!(
                "config.toml could not be parsed, so it was not replaced: {err}"
            ))
        })?;
        let mut text = existing.to_string();
        if let Some(done) = patch.setup_completed {
            text = patch_root_bool(&text, "setup_completed", done)?;
        }
        if let Some(sync) = &patch.sync {
            text = patch_sync_keys(&text, sync)?;
        }
        let _check = toml::from_str::<Self>(&text).map_err(|err| {
            unreadable_config(format!(
                "config.toml could not be updated in place, so it was not replaced: {err}"
            ))
        })?;
        Ok(text)
    }
}

/// Splice a root boolean the way `shadow_recognizer` is spliced, or insert
/// it on the root when the key is missing.
fn patch_root_bool(
    existing: &str,
    key: &str,
    on: bool,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let doc = toml_edit::Document::parse(existing).map_err(|err| {
        unreadable_config(format!(
            "config.toml could not be parsed, so it was not replaced: {err}"
        ))
    })?;
    if doc.get(key).is_some() {
        return splice_root_bool(existing, &doc, key, on);
    }
    insert_root_bool(existing, key, on)
}

fn splice_root_bool(
    existing: &str,
    doc: &toml_edit::Document<&str>,
    key: &str,
    on: bool,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let wanted = if on { "true" } else { "false" };
    if let Some(item) = doc.get(key)
        && let Some(value) = item.as_value()
        && value.as_bool().is_some()
        && let Some(span) = value.span()
        && existing.is_char_boundary(span.start)
        && existing.is_char_boundary(span.end)
    {
        let current = &existing[span.start..span.end];
        if current == "true" || current == "false" {
            let mut out = String::with_capacity(existing.len() + wanted.len());
            out.push_str(&existing[..span.start]);
            out.push_str(wanted);
            out.push_str(&existing[span.end..]);
            return Ok(out);
        }
    }
    Err(unreadable_config(
        "config.toml could not be updated in place, so it was not replaced".to_string(),
    ))
}

fn insert_root_bool(
    existing: &str,
    key: &str,
    on: bool,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let mut doc = existing.parse::<toml_edit::DocumentMut>().map_err(|err| {
        unreadable_config(format!(
            "config.toml could not be parsed, so it was not replaced: {err}"
        ))
    })?;
    if doc.get(key).is_some() {
        return Err(unreadable_config(
            "config.toml already has that key at the root, so it was not replaced".to_string(),
        ));
    }
    doc.as_table_mut().insert(key, toml_edit::value(on));
    Ok(doc.to_string())
}

fn patch_sync_keys(
    existing: &str,
    sync: &SyncConfig,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let doc = toml_edit::Document::parse(existing).map_err(|err| {
        unreadable_config(format!(
            "config.toml could not be parsed, so it was not replaced: {err}"
        ))
    })?;
    if doc.get("sync").and_then(|item| item.as_table()).is_none() {
        return insert_sync_table(existing, sync);
    }
    let text = upsert_table_string(existing, "sync", "server_url", &sync.server_url)?;
    upsert_table_string(&text, "sync", "token", &sync.token)
}

fn insert_sync_table(
    existing: &str,
    sync: &SyncConfig,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let mut doc = existing.parse::<toml_edit::DocumentMut>().map_err(|err| {
        unreadable_config(format!(
            "config.toml could not be parsed, so it was not replaced: {err}"
        ))
    })?;
    if doc.get("sync").is_some() {
        return Err(unreadable_config(
            "config.toml already has a sync entry, so it was not replaced".to_string(),
        ));
    }
    let mut table = toml_edit::Table::new();
    table["server_url"] = toml_edit::value(sync.server_url.as_str());
    table["token"] = toml_edit::value(sync.token.as_str());
    doc["sync"] = toml_edit::Item::Table(table);
    Ok(doc.to_string())
}

fn upsert_table_string(
    existing: &str,
    table: &str,
    key: &str,
    raw: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let doc = toml_edit::Document::parse(existing).map_err(|err| {
        unreadable_config(format!(
            "config.toml could not be parsed, so it was not replaced: {err}"
        ))
    })?;
    let Some(tbl) = doc.get(table).and_then(|item| item.as_table()) else {
        return Err(unreadable_config(
            "config.toml could not be updated in place, so it was not replaced".to_string(),
        ));
    };
    let token = toml_edit::Value::from(raw).to_string();
    if let Some(item) = tbl.get(key) {
        let Some(value) = item.as_value().filter(|value| value.as_str().is_some()) else {
            return Err(unreadable_config(
                "config.toml could not be updated in place, so it was not replaced".to_string(),
            ));
        };
        let Some(span) = value.span() else {
            return Err(unreadable_config(
                "config.toml could not be updated in place, so it was not replaced".to_string(),
            ));
        };
        if !existing.is_char_boundary(span.start) || !existing.is_char_boundary(span.end) {
            return Err(unreadable_config(
                "config.toml could not be updated in place, so it was not replaced".to_string(),
            ));
        }
        let mut out = String::with_capacity(existing.len() + token.len());
        out.push_str(&existing[..span.start]);
        out.push_str(&token);
        out.push_str(&existing[span.end..]);
        return Ok(out);
    }
    insert_key_after_table_header(existing, table, key, &token)
}

fn insert_key_after_table_header(
    existing: &str,
    table: &str,
    key: &str,
    token: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let header = format!("[{table}]");
    let Some(idx) = existing.find(&header) else {
        return Err(unreadable_config(
            "config.toml could not be updated in place, so it was not replaced".to_string(),
        ));
    };
    let line_start = existing[..idx].rfind('\n').map(|n| n + 1).unwrap_or(0);
    if !existing[line_start..idx].trim().is_empty() {
        return Err(unreadable_config(
            "config.toml could not be updated in place, so it was not replaced".to_string(),
        ));
    }
    let after = idx + header.len();
    let line_end = existing[after..]
        .find('\n')
        .map(|n| after + n + 1)
        .unwrap_or(existing.len());
    let mut out = String::with_capacity(existing.len() + key.len() + token.len() + 8);
    out.push_str(&existing[..line_end]);
    if line_end == existing.len() && !existing.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(key);
    out.push_str(" = ");
    out.push_str(token);
    out.push('\n');
    out.push_str(&existing[line_end..]);
    Ok(out)
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

    #[test]
    fn shadow_recognizer_defaults_off_and_is_not_written_when_off() {
        assert!(!Config::default().shadow_recognizer);
        let raw = toml::to_string_pretty(&Config::default()).unwrap();
        assert!(!raw.contains("shadow_recognizer"), "{raw}");
        let on: Config = toml::from_str(&format!("shadow_recognizer = true\n{raw}")).unwrap();
        assert!(on.shadow_recognizer);
    }

    #[test]
    fn shadow_env_off_values_keep_it_off() {
        for v in [
            None,
            Some("0"),
            Some("false"),
            Some("FALSE"),
            Some("no"),
            Some(""),
        ] {
            let control = Config::shadow_control(false, v);
            assert!(!control.enabled(), "{v:?} must not turn it on");
            assert!(!control.locked, "{v:?} must not lock Settings");
            assert!(!control.file_on);
        }
        for v in [
            Some("1"),
            Some("true"),
            Some("TRUE"),
            Some("yes"),
            Some("YES"),
        ] {
            let control = Config::shadow_control(false, v);
            assert!(control.locked, "{v:?} locks the toggle on");
            assert!(control.enabled(), "{v:?} turns it on for this process");
            assert!(!control.file_on, "{v:?} must not become the file value");
        }
        let control = Config::shadow_control(true, Some("0"));
        assert!(control.file_on);
        assert!(!control.locked);
        assert!(control.enabled(), "file flag still wins");
    }

    #[test]
    fn shadow_env_override_is_never_saved() {
        // The override is resolved at use, not stored: a config built from the
        // file value (what Settings saves) serializes without the key even
        // when the env would turn the recognizer on.
        let cfg = Config::default();
        let control = Config::shadow_control(cfg.shadow_recognizer, Some("1"));
        assert!(control.enabled());
        assert!(control.locked);
        assert!(!control.file_on);
        assert!(!cfg.shadow_recognizer);
        let raw = toml::to_string_pretty(&cfg).unwrap();
        assert!(!raw.contains("shadow_recognizer"), "{raw}");
        // Config::load must not fold the env var into the struct.
        let src = include_str!("config.rs");
        let load = &src[src.find("pub fn load").expect("load")..];
        let load = &load[..load.find("pub fn config_path").expect("end of load")];
        assert!(
            !load.contains(concat!("config.shadow_", "recognizer =")),
            "load() must not persist SCUFFED_SHADOW_RECOGNIZER into the config"
        );
    }

    #[test]
    fn sync_url_https_is_allowed() {
        assert!(validate_sync_server_url("https://crew.example").is_ok());
        assert!(validate_sync_server_url("https://crew.example/stats").is_ok());
        assert!(validate_sync_server_url("HTTPS://Crew.Example:443").is_ok());
        assert!(validate_sync_server_url("  https://crew.example  ").is_ok());
    }

    #[test]
    fn sync_url_http_non_loopback_is_rejected() {
        for raw in [
            "http://crew.example",
            "http://crew.example:8080/api",
            "http://10.0.0.5",
            "http://192.168.1.1:3030",
            "http://203.0.113.10",
            "http://[2001:db8::1]",
            "http://0.0.0.0",
            "http://127.0.0.1.evil.example",
            "ftp://localhost",
            "ws://localhost",
            "https://",
            "not a url",
            "",
        ] {
            assert!(
                validate_sync_server_url(raw).is_err(),
                "{raw} must not be allowed to carry the token"
            );
        }
        assert_eq!(
            validate_sync_server_url("http://crew.example"),
            Err(SyncUrlReject::HttpsRequired)
        );
        assert_eq!(
            validate_sync_server_url("http://crew.example")
                .unwrap_err()
                .message(),
            SYNC_URL_HTTPS_REQUIRED
        );
        assert_eq!(
            validate_sync_server_url("not a url"),
            Err(SyncUrlReject::Invalid)
        );
    }

    #[test]
    fn sync_url_http_loopback_is_allowed() {
        for raw in [
            "http://localhost",
            "http://localhost:3030",
            "http://LOCALHOST/api",
            "http://127.0.0.1",
            "http://127.0.0.1:3030/api",
            "http://[::1]",
            "http://[::1]:3030",
        ] {
            assert!(
                validate_sync_server_url(raw).is_ok(),
                "{raw} is loopback and must stay available for local dev"
            );
        }
    }

    #[test]
    fn existing_http_config_still_parses() {
        // Fail safe at send time. A stored cleartext URL must still load so
        // the daemon can refuse it without crashing and without dropping the
        // rest of the file.
        let raw = r#"
data_dir = "/tmp/sst-m20"
capture_output = "DP-1"
player_name = "Ada"
session_window_secs = 1800
debug_ocr = false
game_process_names = ["Overwatch.exe"]

[auto_detect]
enabled = true
poll_interval_secs = 4
cooldown_secs = 120

[sync]
server_url = "http://example.com"
token = "secret"
"#;
        let cfg: Config = toml::from_str(raw).expect("existing shape must still parse");
        assert!(
            !cfg.shadow_recognizer,
            "the shadow recognizer is off unless a config asks for it"
        );
        assert_eq!(
            cfg.finished_game_close_secs, FINISHED_GAME_CLOSE_DEFAULT_SECS,
            "an older config file has no quiet-close setting"
        );
        let sync = cfg.sync.expect("sync block");
        assert_eq!(sync.token, "secret");
        assert_eq!(
            validate_sync_server_url(&sync.server_url),
            Err(SyncUrlReject::HttpsRequired)
        );
    }

    fn sample(full: bool) -> Config {
        Config {
            data_dir: PathBuf::from("/var/lib/scuffed-outside"),
            capture_output: full.then(|| "DP-1".to_string()),
            player_name: full.then(|| "Ada".to_string()),
            sync: full.then(|| SyncConfig {
                server_url: "https://crew.example".to_string(),
                token: "super-secret-token".to_string(),
            }),
            auto_detect: AutoDetectConfig {
                enabled: false,
                poll_interval_secs: 8,
                cooldown_secs: 30,
            },
            session_window_secs: 900,
            finished_game_close_secs: 240,
            game_process_names: if full {
                vec!["Overwatch.exe".into(), "wine".into()]
            } else {
                Vec::new()
            },
            debug_ocr: full,
            ocr_threads: full.then_some(2),
            shadow_recognizer: full,
            setup_completed: full,
            reader_pack_url: full.then(|| "https://crew.example/packs/reader.zip".to_string()),
            reader: if full {
                ReaderSetting::New
            } else {
                ReaderSetting::OcrV1
            },
        }
    }

    #[test]
    fn toml_save_format_roundtrips_optional_fields_set_and_unset() {
        for full in [true, false] {
            let cfg = sample(full);
            let raw = toml::to_string_pretty(&cfg).expect("toml 1 must serialize Config");
            if full {
                assert!(raw.contains("capture_output"), "{raw}");
                assert!(raw.contains("player_name"), "{raw}");
                assert!(raw.contains("[sync]"), "{raw}");
                assert!(raw.contains("ocr_threads"), "{raw}");
            } else {
                assert!(!raw.contains("capture_output"), "{raw}");
                assert!(!raw.contains("player_name"), "{raw}");
                assert!(!raw.contains("[sync]"), "{raw}");
                assert!(!raw.contains("ocr_threads"), "{raw}");
            }
            let back: Config = toml::from_str(&raw).expect("toml 1 output must parse");
            assert_eq!(back, cfg);
        }
    }

    #[test]
    fn readme_and_bootstrap_examples_pin_this_crate_version() {
        let tag = format!("stat-tracker-v{}", env!("CARGO_PKG_VERSION"));
        let readme = include_str!("../README.md");
        let bootstrap = include_str!("../dist/bootstrap.sh");
        assert!(
            readme.contains(&format!("TAG={tag}")),
            "README pin example is stale; want TAG={tag}"
        );
        assert!(
            bootstrap.contains(&format!("TAG={tag}")),
            "bootstrap.sh pin example is stale; want TAG={tag}"
        );
        let changelog = include_str!("../CHANGELOG.md");
        let version = env!("CARGO_PKG_VERSION");
        assert!(
            changelog.contains(&format!("## {version}")),
            "CHANGELOG is missing a section for {version}"
        );
    }

    fn hand_edited_config(shadow_line: &str) -> String {
        format!(
            "\
# hand-edited tracker config. keep this comment.
data_dir = \"/tmp/sst-hand-edited\"

# scoreboard name, not a default key dump
player_name = \"the streamer\"

# extra number reader (private log only)
{shadow_line}

# a key Settings does not own
custom_note = \"leave this line alone\"

session_window_secs = 1200
"
        )
    }

    fn changed_lines<'a>(before: &'a str, after: &'a str) -> Vec<(&'a str, &'a str)> {
        let before_lines: Vec<_> = before.split_inclusive('\n').collect();
        let after_lines: Vec<_> = after.split_inclusive('\n').collect();
        let mut diffs = Vec::new();
        let count = before_lines.len().max(after_lines.len());
        for index in 0..count {
            let left = before_lines.get(index).copied().unwrap_or("");
            let right = after_lines.get(index).copied().unwrap_or("");
            if left != right {
                diffs.push((left, right));
            }
        }
        diffs
    }

    #[test]
    fn flipping_shadow_recognizer_changes_only_that_line() {
        for (before_line, after_line) in [
            ("shadow_recognizer = false", "shadow_recognizer = true"),
            ("shadow_recognizer = true", "shadow_recognizer = false"),
        ] {
            let raw = hand_edited_config(before_line);
            let mut next: Config = toml::from_str(&raw).expect("hand-edited file must parse");
            next.shadow_recognizer = !next.shadow_recognizer;
            let saved = Config::text_for_save(Some(&raw), &next).expect("shadow-only save");
            let diffs = changed_lines(&raw, &saved);
            assert_eq!(
                diffs.len(),
                1,
                "flipping the toggle must change one line, got {diffs:?}\n{saved}"
            );
            assert!(
                diffs[0].0.contains(before_line),
                "old line: {:?}",
                diffs[0].0
            );
            assert!(
                diffs[0].1.contains(after_line),
                "new line: {:?}",
                diffs[0].1
            );
            assert!(saved.contains("# hand-edited tracker config. keep this comment."));
            assert!(saved.contains("# scoreboard name, not a default key dump"));
            assert!(saved.contains("# extra number reader (private log only)"));
            assert!(saved.contains("custom_note = \"leave this line alone\""));
            assert!(saved.contains("player_name = \"the streamer\""));
            assert!(
                !saved.contains("game_process_names"),
                "a shadow toggle must not add default keys: {saved}"
            );
        }
    }

    #[test]
    fn save_skips_write_when_nothing_changed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        let raw = hand_edited_config("shadow_recognizer = true");
        std::fs::write(&path, &raw).expect("seed");
        let stamped = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .expect("open")
            .set_modified(stamped)
            .expect("stamp mtime");

        let cfg: Config = toml::from_str(&raw).expect("parse");
        cfg.save_to_path(&path).expect("unchanged save");

        assert_eq!(std::fs::read_to_string(&path).expect("read"), raw);
        assert_eq!(
            std::fs::metadata(&path)
                .expect("meta")
                .modified()
                .expect("mtime"),
            stamped,
            "save() must not touch the file when the text is unchanged"
        );
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .expect("dir")
            .map(|entry| entry.expect("entry").file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("config.toml")]);
    }

    #[test]
    fn save_refuses_to_replace_an_unparseable_config() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        let raw = "\
this is not toml
server_url = \"https://crew.example\"
token = \"secret-token-must-stay\"
";
        std::fs::write(&path, raw).expect("seed");
        let err = Config::default()
            .save_to_path(&path)
            .expect_err("a broken config must not be replaced");
        let message = err.to_string();
        assert!(
            message.contains("not replaced"),
            "error should say the file was kept: {message}"
        );
        assert!(
            !message.contains('—') && !message.contains('–'),
            "{message}"
        );
        assert_eq!(std::fs::read_to_string(&path).expect("read"), raw);
        assert!(
            std::fs::read_to_string(&path)
                .expect("read")
                .contains("secret-token-must-stay")
        );
    }

    #[test]
    fn save_replaces_the_file_atomically_with_mode_0600() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        let raw = hand_edited_config("shadow_recognizer = false");
        std::fs::write(&path, &raw).expect("seed");
        let mut next: Config = toml::from_str(&raw).expect("parse");
        next.shadow_recognizer = true;
        next.save_to_path(&path).expect("save");

        let saved = std::fs::read_to_string(&path).expect("read");
        let diffs = changed_lines(&raw, &saved);
        assert_eq!(diffs.len(), 1, "{diffs:?}\n{saved}");
        assert!(diffs[0].1.contains("shadow_recognizer = true"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).expect("meta").permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "saved config must be owner-only");
        }
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .expect("dir")
            .map(|entry| entry.expect("entry").file_name())
            .collect();
        assert_eq!(
            names,
            vec![std::ffi::OsString::from("config.toml")],
            "the temp file must be renamed, not left behind"
        );
    }

    /// Shape Settings writes: top-level keys, then `[auto_detect]` and `[sync]`.
    fn settings_shaped_without_shadow() -> String {
        "\
data_dir = \"/tmp/sst-settings-shape\"
capture_output = \"DP-1\"
player_name = \"the streamer\"
session_window_secs = 1800
finished_game_close_secs = 180
game_process_names = [\"Overwatch.exe\"]
debug_ocr = false

[auto_detect]
enabled = true
poll_interval_secs = 4
cooldown_secs = 120

[sync]
server_url = \"https://crew.example\"
token = \"not-a-real-token\"
"
        .to_string()
    }

    fn table_block(text: &str) -> &str {
        text.find("[auto_detect]")
            .map(|index| &text[index..])
            .unwrap_or(text)
    }

    fn assert_root_shadow(text: &str, on: bool) {
        let parsed = toml_edit::Document::parse(text).expect("edited file must parse");
        let root = parsed
            .get("shadow_recognizer")
            .and_then(|item| item.as_value())
            .and_then(|value| value.as_bool());
        assert_eq!(
            root,
            Some(on),
            "exactly one root shadow_recognizer:\n{text}"
        );
        let loaded = Config::parse_file_contents(text).expect("Config::load parse");
        assert_eq!(
            loaded.shadow_recognizer, on,
            "Config::load must read the root value"
        );
        let header = text
            .find("\n[")
            .map(|index| index + 1)
            .or_else(|| text.starts_with('[').then_some(0))
            .expect("table header");
        assert!(
            text[..header].contains("shadow_recognizer"),
            "the root key must sit before the first table:\n{text}"
        );
        assert_eq!(
            text[..header].matches("shadow_recognizer").count(),
            1,
            "the root key must not be duplicated:\n{text}"
        );
    }

    #[test]
    fn missing_shadow_recognizer_toggles_stay_on_the_root() {
        let original = settings_shaped_without_shadow();
        let tables = table_block(&original).to_string();
        assert!(!original.contains("shadow_recognizer"));
        let mut current = Config::parse_file_contents(&original).expect("seed parses");
        assert!(!current.shadow_recognizer);

        let mut text = original.clone();
        for on in [true, false, true] {
            current.shadow_recognizer = on;
            text = Config::text_for_save(Some(&text), &current).expect("toggle");
            assert_root_shadow(&text, on);
            assert_eq!(
                table_block(&text),
                tables,
                "tables must stay byte for byte on toggle {on}:\n{text}"
            );
            current = Config::parse_file_contents(&text).expect("reload");
        }
    }

    #[test]
    fn nested_shadow_recognizer_is_left_alone_and_root_is_inserted_once() {
        let original = "\
data_dir = \"/tmp/sst-nested-shadow\"
player_name = \"the streamer\"

[sync]
server_url = \"https://crew.example\"
token = \"not-a-real-token\"
shadow_recognizer = true
";
        let mut next = Config::parse_file_contents(original).expect("nested key is not the root");
        assert!(!next.shadow_recognizer);
        next.shadow_recognizer = true;
        let saved = Config::text_for_save(Some(original), &next).expect("insert root");
        assert_root_shadow(&saved, true);
        let doc = toml_edit::Document::parse(&saved).expect("parse");
        let nested = doc
            .get("sync")
            .and_then(|item| item.as_table())
            .and_then(|table| table.get("shadow_recognizer"))
            .and_then(|item| item.as_value())
            .and_then(|value| value.as_bool());
        assert_eq!(nested, Some(true), "the mistaken table key stays:\n{saved}");
        let again = Config::text_for_save(Some(&saved), &next).expect("second toggle");
        assert_eq!(again, saved, "a second on must not add another key");
        assert_root_shadow(&again, true);
    }

    #[test]
    fn missing_shadow_recognizer_without_tables_is_appended_at_the_end() {
        let original = "\
data_dir = \"/tmp/sst-no-tables\"
player_name = \"the streamer\"
";
        let mut next = Config::parse_file_contents(original).expect("parse");
        next.shadow_recognizer = true;
        let saved = Config::text_for_save(Some(original), &next).expect("insert");
        let doc = toml_edit::Document::parse(&saved).expect("parse");
        assert_eq!(
            doc.get("shadow_recognizer")
                .and_then(|item| item.as_value())
                .and_then(|value| value.as_bool()),
            Some(true)
        );
        assert!(
            !saved.contains('['),
            "a file with no tables stays free of headers:\n{saved}"
        );
        assert!(
            Config::parse_file_contents(&saved)
                .expect("load")
                .shadow_recognizer
        );
    }

    #[test]
    fn setup_patch_edits_completed_and_sync_in_place_and_never_writes_device_code() {
        let secret = "dc-SECRET-9f3a-not-for-disk";
        let original = "\
# hand-edited tracker config. keep this comment.
data_dir = \"/tmp/sst-hand-edited\"

# scoreboard name, not a default key dump
player_name = \"the streamer\"

# extra number reader (private log only)
shadow_recognizer = true

# a key Settings does not own
custom_note = \"leave this line alone\"

session_window_secs = 1200

[sync]
server_url = \"https://old.example\"
token = \"old-token\"
";
        let patch = SetupDiskPatch {
            setup_completed: Some(true),
            sync: Some(SyncConfig {
                server_url: "https://crew.example".to_string(),
                token: "pasted-token".to_string(),
            }),
        };
        let saved = Config::text_for_setup_patch(Some(original), &patch).expect("patch");
        assert!(saved.contains("# hand-edited tracker config. keep this comment."));
        assert!(saved.contains("shadow_recognizer = true"));
        assert!(saved.contains("custom_note = \"leave this line alone\""));
        assert!(saved.contains("player_name = \"the streamer\""));
        assert!(saved.contains("setup_completed = true"));
        assert!(saved.contains("https://crew.example"));
        assert!(saved.contains("pasted-token"));
        assert!(!saved.contains("old-token"));
        assert!(!saved.contains(secret));
        assert!(
            !saved.contains("device_code"),
            "the guide patch must not invent a device_code key:\n{saved}"
        );
        let loaded = Config::parse_file_contents(&saved).expect("parse");
        assert!(loaded.setup_completed);
        assert!(loaded.shadow_recognizer);
        let sync = loaded.sync.expect("sync");
        assert_eq!(sync.server_url, "https://crew.example");
        assert_eq!(sync.token, "pasted-token");

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, original).expect("seed");
        Config::apply_setup_patch(&path, &patch).expect("write");
        let disk = std::fs::read_to_string(&path).expect("read");
        assert!(!disk.contains(secret));
        assert!(disk.contains("pasted-token"));
        assert!(disk.contains("shadow_recognizer = true"));
    }

    #[test]
    fn setup_patch_refuses_an_unparseable_file() {
        let original = "this is not toml {";
        let err = Config::text_for_setup_patch(
            Some(original),
            &SetupDiskPatch {
                setup_completed: Some(true),
                sync: None,
            },
        )
        .expect_err("broken file");
        assert!(err.to_string().contains("not replaced"));
    }

    #[test]
    fn fresh_setup_patch_writes_completed_without_a_device_code() {
        let saved = Config::text_for_setup_patch(
            None,
            &SetupDiskPatch {
                setup_completed: Some(true),
                sync: Some(SyncConfig {
                    server_url: "https://crew.example".into(),
                    token: "tok".into(),
                }),
            },
        )
        .expect("fresh");
        let loaded = Config::parse_file_contents(&saved).expect("parse");
        assert!(loaded.setup_completed);
        assert_eq!(loaded.sync.expect("sync").token, "tok");
        assert!(!saved.contains("device_code"));
    }

    #[test]
    fn reader_defaults_to_ocr_v1_and_rejects_unknown_values() {
        assert!(Config::default().reader.is_ocr_v1());
        assert!(!Config::default().uses_new_reader());
        assert!(!Config::default().shadow_log_enabled());
        let raw = settings_shaped_without_shadow();
        let loaded = Config::parse_file_contents(&raw).expect("missing reader");
        assert!(loaded.reader.is_ocr_v1());
        let with_new = format!("reader = \"new\"\n{raw}");
        let on = Config::parse_file_contents(&with_new).expect("reader = new");
        assert!(on.uses_new_reader());
        assert!(on.shadow_log_enabled());
        assert!(!on.shadow_recognizer);
        let unknown = format!("reader = \"cv-v5\"\n{raw}");
        assert!(Config::parse_file_contents(&unknown).is_err());
    }

    #[test]
    fn missing_reader_toggles_stay_on_the_root_and_keep_comments() {
        let original = settings_shaped_without_shadow();
        let tables = table_block(&original).to_string();
        assert!(!original.contains("reader"));
        let mut current = Config::parse_file_contents(&original).expect("seed parses");
        assert!(current.reader.is_ocr_v1());

        current.reader = ReaderSetting::New;
        let on = Config::text_for_save(Some(&original), &current).expect("turn on");
        assert!(on.contains("# ") || !original.contains("# "));
        assert!(on.contains("reader = \"new\""), "{on}");
        assert_eq!(
            table_block(&on),
            tables,
            "tables must stay byte for byte:\n{on}"
        );
        assert!(
            on.find("reader").unwrap() < on.find("[auto_detect]").unwrap(),
            "reader stays on the root:\n{on}"
        );
        let loaded = Config::parse_file_contents(&on).expect("reload on");
        assert!(loaded.uses_new_reader());

        current.reader = ReaderSetting::OcrV1;
        let off = Config::text_for_save(Some(&on), &current).expect("turn off");
        assert!(off.contains("reader = \"ocr-v1\""), "{off}");
        assert_eq!(
            table_block(&off),
            tables,
            "tables must stay byte for byte:\n{off}"
        );
        assert!(
            Config::parse_file_contents(&off)
                .expect("reload off")
                .reader
                .is_ocr_v1()
        );
    }

    #[test]
    fn reader_toggle_preserves_a_hand_edited_comment() {
        let raw = "\
# hand-edited tracker config. keep this comment.
data_dir = \"/tmp/sst-hand-edited\"
player_name = \"the streamer\"
reader = \"ocr-v1\"
custom_note = \"leave this line alone\"
";
        let mut next: Config = toml::from_str(raw).expect("hand-edited file must parse");
        next.reader = ReaderSetting::New;
        let saved = Config::text_for_save(Some(raw), &next).expect("reader-only save");
        assert!(saved.contains("# hand-edited tracker config. keep this comment."));
        assert!(saved.contains("custom_note = \"leave this line alone\""));
        assert!(saved.contains("player_name = \"the streamer\""));
        assert!(saved.contains("reader = \"new\""));
        assert!(!saved.contains("reader = \"ocr-v1\""));
        assert!(
            !saved.contains("game_process_names"),
            "a reader toggle must not add default keys: {saved}"
        );
    }
}
