use std::path::PathBuf;

use serde::{Deserialize, Serialize};

mod capture;
mod edit;
mod file;
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
pub use reader::{OCR_ENV, OcrControl, OcrSetting, ReaderSetting, ShadowRecognizerControl};
pub use report_kit::{DEFAULT_MAX_GAMES, DEFAULT_MAX_MB, Retention};
pub use sync::{
    SYNC_URL_HTTPS_REQUIRED, SYNC_URL_INVALID, SyncConfig, SyncUrlReject, validate_sync_server_url,
};

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Config {
    pub data_dir: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capture_output: Option<String>,
    /// Which portal image source to use. Missing or `auto` keeps the usual
    /// probe: wlroots and X11 stay direct, and a portal desktop uses the
    /// screen stream. `screencast` forces the stream. `screenshot` forces
    /// the older screenshot portal. `kwin` asks KWin directly (KDE direct,
    /// Plasma 6.8 or later), and falls back to automatic when KWin does not
    /// answer. A non-empty `STAT_TRACKER_CAPTURE` wins over this key.
    /// Edited in place when this is the only change.
    #[serde(default, skip_serializing_if = "CaptureRoute::is_auto")]
    pub capture_route: CaptureRoute,
    /// Save a few Tab frames from the screen stream and the screenshot
    /// portal side by side under `{data_dir}/debug/portal-compare/`. Off
    /// by default. Also enabled by `STAT_TRACKER_DEBUG_PORTAL_FRAMES=1`,
    /// which is not written back to the file. The pictures stay on this
    /// machine. Leave this off while a voice app is sharing the screen:
    /// the screenshot half can hit the KWin cursor crash.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub debug_portal_frames: bool,
    /// Cap on the screen-stream rate, in frames per second. Missing means
    /// [`SCREENCAST_MAX_FPS_DEFAULT`] (4). Values outside 1..=5 are clamped. This is the PipeWire
    /// max-framerate and the tracker's own throttle. Not written unless set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screencast_max_fps: Option<u32>,
    /// Pause the screen stream between grabs, so the compositor does not
    /// render frames the tracker throws away. Missing or `true` means on
    /// where the compositor handles it (KWin 6.1 or newer, niri) and off
    /// elsewhere. `false` keeps the stream running the whole time, as before
    /// 0.5.0-rc.7, on every desktop. `STAT_TRACKER_SCREENCAST_PAUSE=1` forces
    /// it on anywhere and `=0` off; the env var wins over this key and is not
    /// written back. Not written unless set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screencast_pause_between_grabs: Option<bool>,
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
    /// on confirm (second agreeing word-OCR) and first word-OCR
    /// streak, not every mid-match tick. Off by default, the Tab path
    /// recomputes preprocess just to dump stages and dominated capture
    /// latency. Also enabled by env `STAT_TRACKER_DEBUG_OCR=1`. Also gates
    /// the `debug/accepted/` and `debug/rejected/` rings: those crops are
    /// unblanked and show other players' names.
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
    /// Which reader supplies saved and uploaded stats.
    ///
    /// Missing, or `"new"`, stores the new reader's values when a field is
    /// read and not suspect (the default from alpha.12, after cv-v6 read
    /// 720 of 720 held-out benchmark boards with none wrong). `"ocr-v1"`
    /// keeps the Tesseract reads; Settings writes it explicitly, so a member
    /// who switches back stays on it. Under `"new"` with classic OCR on, a
    /// Tab whose board the new reader finds nothing on (not found, team size
    /// unknown, no digit values) is read with ocr-v1 instead and stored with
    /// recognizer `ocr-v1`. On every other Tab under `"new"` the six stats
    /// come from the new reader only: a Tab whose board it does not vouch
    /// for is not stored, and a stat cell it is unsure of keeps the game's
    /// last accepted value and is marked, with no OCR number in its place.
    /// A game has no stored row until each of the six stats has
    /// had a sure read on some Tab of it: a stat never read is not stored
    /// or uploaded as 0, and a game that ends that way is not stored at
    /// all. The member's own row is found from the own-name seeds and the
    /// highlight; a name is read with OCR only on a board where neither
    /// names a row and a player name is set. `name_reader` (on unless the
    /// file says `false`) can take that read, and the ocr-v1 name match,
    /// from the names-v1 pack. The top-bar map still falls
    /// back to OCR when the banner reader abstains. Under `"new"` the
    /// result words, the FINAL SCORE line and the
    /// game-start screens are read from the words pack, with OCR only where
    /// a template or mask is not loaded; the accolade and rank words and
    /// the accolade map are still OCR.
    #[serde(default, skip_serializing_if = "ReaderSetting::is_default")]
    pub reader: ReaderSetting,
    /// Whether classic OCR (Tesseract) may run at all.
    ///
    /// Missing, or `"on"`, is today's behaviour for both values of
    /// [`Self::reader`]. `"off"` makes the tracker read everything with the
    /// new reader and never start Tesseract:
    ///
    /// | `ocr` | `reader` | what reads |
    /// | --- | --- | --- |
    /// | `on` | `ocr-v1` | Tesseract, everywhere |
    /// | `on` | `new` | the new reader for saved stats, Tesseract for the rest and as its fallback |
    /// | `off` | either | the new reader, everywhere; a field it cannot read is held, never read with OCR |
    ///
    /// `off` implies the new reader, so `reader` is not consulted (its value
    /// stays in the file). `off` is granted only when nothing would go
    /// unread: the build has to read everything without Tesseract, and the
    /// hero, map and word packs have to be loaded with what those readers
    /// need. Otherwise the tracker keeps OCR on for the run and logs why
    /// (`stat_tracker::ocr_switch`). The tracker reads this when it starts.
    /// Overlay for tests and tools: env [`OCR_ENV`] (`on` or `off`).
    #[serde(default, skip_serializing_if = "OcrSetting::is_on")]
    pub ocr: OcrSetting,
    /// Letter-template names for the configured player's own row.
    ///
    /// Missing, or `true`, is on (rc.1). `false` keeps today's Tesseract
    /// read of that row. Independent of [`Self::reader`]: ocr-v1 and the
    /// new reader both honor it. With a `names-v1` pack loaded, a row is
    /// used when its confidence meets the pack's own `min_confidence`. A
    /// blank row, or one under that threshold, is read with Tesseract for
    /// that row. With no pack installed, `stat_tracker::names::read_names`
    /// already uses Tesseract, so leaving this on does not change that
    /// path. The pack is not in the repo. A default install downloads it
    /// with the other reader packs.
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
mod tests_pins;

#[cfg(test)]
mod tests_sync_url;
