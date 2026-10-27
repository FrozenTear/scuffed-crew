use std::path::PathBuf;

use serde::{Deserialize, Serialize};

mod capture;
mod edit;
mod file;
mod parse_error;
mod poll;
mod reader;
mod report_kit;
mod sync;

pub use capture::{
    CAPTURE_ENV, CaptureRoute, PauseOverride, SCREENCAST_MAX_FPS_DEFAULT, SCREENCAST_MAX_FPS_MAX,
    SCREENCAST_MAX_FPS_MIN, SCREENCAST_PAUSE_ENV, clamp_screencast_max_fps,
    environment_d_capture_files, screencast_pause_override, winning_env_assignment,
};
pub use edit::SetupDiskPatch;
pub use poll::{
    AutoDetectConfig, CLASSIC_POLL_INTERVAL, LEGACY_DEFAULT_POLL_SECS, NEW_READER_POLL_INTERVAL,
    POLL_INTERVAL_MS_MAX, POLL_INTERVAL_MS_MIN,
};
pub use reader::{
    OCR_ENV, OCR_THREADS_ENV, OcrControl, OcrSetting, ReaderSetting, ShadowRecognizerControl,
    ocr_threads_override,
};
pub use report_kit::{DEFAULT_MAX_GAMES, DEFAULT_MAX_MB, Retention};
pub use sync::{
    SYNC_URL_HTTPS_REQUIRED, SYNC_URL_INVALID, SyncConfig, SyncUrlReject, validate_sync_server_url,
};

/// The settings in `config.toml`, in `$XDG_CONFIG_HOME/scuffed-stat-tracker/`
/// (usually `~/.config/scuffed-stat-tracker/`; see [`Config::config_path`]).
///
/// The Config table in `crates/stat-tracker/README.md` is the reference for
/// what each key does. The field docs here are the schema: what the key is,
/// its default, its env or CLI override, where it matters whether it is left
/// out of the file, and the code that applies it. They do not repeat the
/// README.
///
/// Field order is the file's key order, and the serde attributes are its
/// format; `config/tests_pins.rs` pins both byte for byte.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Config {
    pub data_dir: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capture_output: Option<String>,
    /// Portal image source: `auto` (also when missing), `screencast`,
    /// `screenshot` or `kwin` (KDE direct). A non-empty [`CAPTURE_ENV`] wins over
    /// this key. A full save leaves `auto` out; when this is the only change it
    /// is edited in place, and the new word is written, `auto` included. The
    /// daemon passes it to `stat_tracker::capture` in `CapturePrefs`.
    #[serde(default, skip_serializing_if = "CaptureRoute::is_auto")]
    pub capture_route: CaptureRoute,
    /// Save stream/screenshot Tab pairs under `{data_dir}/debug/portal-compare/`.
    /// Off by default and not written while off. Also
    /// `STAT_TRACKER_DEBUG_PORTAL_FRAMES=1`, which is not
    /// stored here ([`Self::debug_portal_frames_enabled`]).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub debug_portal_frames: bool,
    /// Screen-stream cap in frames per second. Missing means
    /// [`SCREENCAST_MAX_FPS_DEFAULT`]; clamped to 1..=5 ([`Self::screencast_max_fps`]).
    /// Not written unless set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screencast_max_fps: Option<u32>,
    /// Pause the screen stream between grabs. Missing or `true` lets
    /// `stat_tracker::capture` decide per compositor; `false` keeps the stream
    /// running everywhere. [`SCREENCAST_PAUSE_ENV`] wins over this key and is not
    /// stored ([`Self::screencast_pause_override`]). Not written unless set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screencast_pause_between_grabs: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub player_name: Option<String>,
    /// `[sync]`. Missing, or a table without a non-empty `token`, means sync
    /// is off (`sync::deserialize_sync`).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "sync::deserialize_sync"
    )]
    pub sync: Option<SyncConfig>,
    #[serde(default)]
    pub auto_detect: AutoDetectConfig,
    #[serde(default = "default_session_window_secs")]
    pub session_window_secs: u64,
    /// Quiet time after the last activity before a finished game is closed and
    /// uploaded. Missing means [`FINISHED_GAME_CLOSE_DEFAULT_SECS`]. The daemon
    /// never waits less than its 75-second post-match grace. Config-file only:
    /// Settings keeps the value from the file.
    #[serde(default = "default_finished_game_close_secs")]
    pub finished_game_close_secs: u64,
    /// Process names (as in `/proc/<pid>/comm`) that must be running for
    /// captures and auto-detect polling. An empty list disables the gate.
    #[serde(default = "default_game_process_names")]
    pub game_process_names: Vec<String>,
    /// Debug image dumps under `{data_dir}/debug/`, including the unblanked
    /// `debug/accepted/` and `debug/rejected/` rings. Off by default: the
    /// dumps slow Tab captures and the rings show other players' names. Also
    /// env `STAT_TRACKER_DEBUG_OCR=1` ([`Self::debug_ocr_enabled`]).
    #[serde(default)]
    pub debug_ocr: bool,
    /// Most games whose report pictures are kept in `report-kit/`. Missing
    /// means [`DEFAULT_MAX_GAMES`] (50). Oldest go first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report_kit_max_games: Option<usize>,
    /// Total size cap of `report-kit/` in MB. Missing means
    /// [`DEFAULT_MAX_MB`] (500). The newest game is kept
    /// even when it alone is larger.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report_kit_max_mb: Option<u64>,
    /// Parallel OCR workers. Missing means automatic; clamped to 1..=8
    /// ([`Self::ocr_threads_resolved`]). Overlay: CLI `--ocr-threads N`, then env
    /// `STAT_TRACKER_OCR_THREADS`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ocr_threads: Option<u32>,
    /// The shadow recognizer's log under `{data_dir}/shadow/` (log only). Off by
    /// default, and a full save leaves `false` out. The daemon also runs the
    /// log whenever the reader in effect is the new one (`reader = "new"`, or
    /// a granted `ocr = "off"`), whatever this says
    /// (`stat_tracker::shadow::worker::ShadowWorker::start_for_reader`). Env
    /// `SCUFFED_SHADOW_RECOGNIZER=1` also turns it on and is never stored
    /// ([`Self::shadow_recognizer_control`]). When this is the only change it
    /// is edited in place, and `true` or `false` is written.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub shadow_recognizer: bool,
    /// `prerelease` opts the desktop updater into alphas and rcs. Any other
    /// set value means stable. Missing or empty follows the installed build:
    /// a release install is not offered an alpha, and an alpha or rc install
    /// keeps getting newer alphas and rcs. Edited in place when this is the
    /// only change.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub update_channel: String,
    /// First-run guide finished or skipped. Missing means not completed, so
    /// the desktop app shows the guide. The guide writes this with the same
    /// in-place edit as `shadow_recognizer`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub setup_completed: bool,
    /// Not used. Reader packs come from the signed-in site
    /// (`GET /api/tracker/packs`, see `stat_tracker::packs`) and the setup guide
    /// always shows its pack step. Kept so a config.toml that still sets it
    /// parses and saves back unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reader_pack_url: Option<String>,
    /// Which reader supplies saved and uploaded stats: `"new"` (also when
    /// missing, the default from alpha.12) or `"ocr-v1"` (Tesseract). A full
    /// save leaves `"new"` out and writes `"ocr-v1"`, so a switch back survives
    /// a reload. Not consulted while `ocr = "off"` is granted
    /// (`stat_tracker::ocr_switch`). Edited in place when it is the only change
    /// (or with `shadow_recognizer`). What each value reads, including which
    /// screens come from the reader packs: the README `reader` row.
    #[serde(default, skip_serializing_if = "ReaderSetting::is_default")]
    pub reader: ReaderSetting,
    /// Whether classic OCR (Tesseract) may run: `"on"` (also when missing) or
    /// `"off"`. `off` implies the new reader and is granted only when nothing
    /// would go unread; otherwise the tracker keeps OCR on for the run and logs
    /// why (`stat_tracker::ocr_switch`). Read when the tracker starts. Env
    /// [`OCR_ENV`] overrides it for one run and is never stored
    /// ([`Self::ocr_control`]). A full save leaves `"on"` out; when this is the
    /// only change it is edited in place, and `"on"` or `"off"` is written.
    #[serde(default, skip_serializing_if = "OcrSetting::is_on")]
    pub ocr: OcrSetting,
    /// Letter-template names (the `names-v1` pack) for the player's own row: on
    /// unless the file says `false`, with either [`Self::reader`]. Not written
    /// while on.
    #[serde(
        default = "default_name_reader",
        skip_serializing_if = "name_reader_is_on"
    )]
    pub name_reader: bool,
}

fn default_name_reader() -> bool {
    true
}

fn name_reader_is_on(on: &bool) -> bool {
    *on
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

impl Config {
    /// True when `update_channel` is the prerelease opt-in.
    pub fn wants_prerelease_updates(&self) -> bool {
        self.update_channel
            .trim()
            .eq_ignore_ascii_case("prerelease")
    }

    fn env_truthy(key: &str) -> bool {
        Self::truthy(std::env::var(key).ok().as_deref())
    }

    fn truthy(value: Option<&str>) -> bool {
        matches!(value, Some("1" | "true" | "TRUE" | "yes" | "YES"))
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
            capture_route: CaptureRoute::Auto,
            debug_portal_frames: false,
            screencast_max_fps: None,
            screencast_pause_between_grabs: None,
            player_name: None,
            sync: None,
            auto_detect: AutoDetectConfig::default(),
            session_window_secs: default_session_window_secs(),
            finished_game_close_secs: default_finished_game_close_secs(),
            game_process_names: default_game_process_names(),
            debug_ocr: false,
            report_kit_max_games: None,
            report_kit_max_mb: None,
            ocr_threads: None,
            shadow_recognizer: false,
            update_channel: String::new(),
            setup_completed: false,
            reader_pack_url: None,
            reader: ReaderSetting::New,
            ocr: OcrSetting::On,
            name_reader: true,
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod tests_edit;

#[cfg(test)]
mod tests_format;

#[cfg(test)]
mod tests_overlay;

#[cfg(test)]
mod tests_parse_error;

#[cfg(test)]
mod tests_pins;

#[cfg(test)]
mod tests_sync_off;

#[cfg(test)]
mod tests_sync_url;
