use std::path::PathBuf;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

mod edit;

pub use edit::SetupDiskPatch;

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
            tracing::warn!(
                capture_route = %raw,
                "unknown capture_route; using auto"
            );
            Self::Auto
        }))
    }
}

/// Env var that wins over [`Config::capture_route`] when it is non-empty.
pub const CAPTURE_ENV: &str = "STAT_TRACKER_CAPTURE";

fn default_name_reader() -> bool {
    true
}

fn name_reader_is_on(on: &bool) -> bool {
    *on
}

/// Env override for [`Config::ocr`]: `on` or `off`. Read by
/// [`Config::ocr_control`] only, never folded into the struct, so a Settings
/// save cannot write it to config.toml.
pub const OCR_ENV: &str = "STAT_TRACKER_OCR";

/// `ocr` in config.toml. Missing means [`Self::On`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OcrSetting {
    #[default]
    On,
    Off,
}

impl OcrSetting {
    pub const ON: &'static str = "on";
    pub const OFF: &'static str = "off";

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::On => Self::ON,
            Self::Off => Self::OFF,
        }
    }

    pub const fn is_on(&self) -> bool {
        matches!(self, Self::On)
    }

    pub const fn is_off(self) -> bool {
        matches!(self, Self::Off)
    }

    /// `on` or `off`, in any case, with spaces around it ignored. The file
    /// value and the env override read the same way.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            Self::ON => Some(Self::On),
            Self::OFF => Some(Self::Off),
            _ => None,
        }
    }
}

impl Serialize for OcrSetting {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for OcrSetting {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).ok_or_else(|| D::Error::custom("ocr must be \"on\" or \"off\""))
    }
}

/// Settings and daemon view of `ocr`.
///
/// `file` is the config.toml value, the only one Settings may write. `env`
/// is [`OCR_ENV`] for this process, when it is set to `on` or `off`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OcrControl {
    pub file: OcrSetting,
    pub env: Option<OcrSetting>,
}

impl OcrControl {
    /// What this process asks for: the env value when set, else the file.
    /// The daemon may still keep OCR on (`stat_tracker::ocr_switch::decide`).
    pub fn requested(self) -> OcrSetting {
        self.env.unwrap_or(self.file)
    }
}

/// `reader` in config.toml. Missing means [`Self::New`] (alpha.12 on;
/// before that, missing meant [`Self::OcrV1`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReaderSetting {
    OcrV1,
    #[default]
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

    /// The value a missing key means. A full save leaves it out and writes
    /// `"ocr-v1"`, so an explicit switch back survives a reload.
    pub const fn is_default(&self) -> bool {
        matches!(self, Self::New)
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
    /// Seconds between end-screen polls. Missing means automatic
    /// ([`AutoDetectConfig::poll_interval`]): 1.5 s with the new reader,
    /// 4 s with classic OCR. 4, the value older Settings always wrote, also
    /// counts as automatic so existing configs get the faster rate.
    ///
    /// Automatic is written as 4, not left out: alpha.7 and older need the
    /// key, and read 4 as their own default, so a downgrade keeps working.
    /// Read back, 4 is `None` again, so automatic has one value in memory.
    #[serde(
        default,
        serialize_with = "write_auto_poll_as_legacy",
        deserialize_with = "read_legacy_poll_as_auto"
    )]
    pub poll_interval_secs: Option<u64>,
    /// Milliseconds between end-screen polls. When set, this wins over
    /// `poll_interval_secs` and over the automatic 1.5 s / 4 s interval.
    /// Clamped to 500..=10000. Missing leaves the seconds key in charge.
    /// Not written unless it was set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poll_interval_ms: Option<u64>,
    pub cooldown_secs: u64,
}

impl Default for AutoDetectConfig {
    fn default() -> Self {
        Self {
            // The poller watches for the map-vote (game start) and post-match
            // accolade (win/loss) screens, required for automatic win/loss
            // detection, at ~25-35ms of CPU per tick. Enabled by default.
            enabled: true,
            poll_interval_secs: None,
            poll_interval_ms: None,
            // Debounce between opening new games: long enough that a map-vote
            // screen lingering across several ticks only opens one game, far
            // shorter than a real match so back-to-back games are still caught.
            cooldown_secs: 120,
        }
    }
}

/// Automatic poll interval with classic OCR (Tesseract reads the boards).
pub const CLASSIC_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(4);
/// Automatic poll interval with the new reader. Its end-screen checks are
/// cheap, and a 4 s tick missed short Victory or Defeat screens.
pub const NEW_READER_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(1500);
/// The value older Settings wrote on every save; treated as automatic.
pub const LEGACY_DEFAULT_POLL_SECS: u64 = 4;
/// Lower and upper bounds for [`AutoDetectConfig::poll_interval_ms`].
pub const POLL_INTERVAL_MS_MIN: u64 = 500;
pub const POLL_INTERVAL_MS_MAX: u64 = 10_000;
/// Default screen-stream cap. QA can set `screencast_max_fps` to compare.
pub const SCREENCAST_MAX_FPS_DEFAULT: u32 = 4;
pub const SCREENCAST_MAX_FPS_MIN: u32 = 1;
pub const SCREENCAST_MAX_FPS_MAX: u32 = 5;

/// Games kept by default. Config: `report_kit_max_games`.
pub const DEFAULT_MAX_GAMES: usize = 50;
/// Total size cap by default, in MB. Config: `report_kit_max_mb`.
pub const DEFAULT_MAX_MB: u64 = 500;

/// How many game kits (`report::kit`) are kept and how large they may be
/// together. The oldest games go first; the newest game is never trimmed,
/// even when it alone is over `max_bytes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Retention {
    pub max_games: usize,
    pub max_bytes: u64,
}

impl Default for Retention {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_GAMES, DEFAULT_MAX_MB)
    }
}

impl Retention {
    /// `max_games` is at least 1, so the newest game always stays.
    pub fn new(max_games: usize, max_mb: u64) -> Self {
        Self {
            max_games: max_games.max(1),
            max_bytes: max_mb.saturating_mul(1024 * 1024),
        }
    }
}

/// Clamp a configured stream cap. Missing means [`SCREENCAST_MAX_FPS_DEFAULT`].
pub fn clamp_screencast_max_fps(raw: Option<u32>) -> u32 {
    match raw {
        None => SCREENCAST_MAX_FPS_DEFAULT,
        Some(n) => n.clamp(SCREENCAST_MAX_FPS_MIN, SCREENCAST_MAX_FPS_MAX),
    }
}

fn read_legacy_poll_as_auto<'de, D: serde::Deserializer<'de>>(
    de: D,
) -> Result<Option<u64>, D::Error> {
    let v = Option::<u64>::deserialize(de)?;
    Ok(v.filter(|&s| s != LEGACY_DEFAULT_POLL_SECS))
}

fn write_auto_poll_as_legacy<S: serde::Serializer>(
    v: &Option<u64>,
    ser: S,
) -> Result<S::Ok, S::Error> {
    ser.serialize_u64(v.unwrap_or(LEGACY_DEFAULT_POLL_SECS))
}

impl AutoDetectConfig {
    /// The poll interval in effect. `poll_interval_ms`, when set, wins over
    /// `poll_interval_secs` and over the automatic rate. Otherwise an
    /// explicit `poll_interval_secs` (at least 1 s) other than
    /// [`LEGACY_DEFAULT_POLL_SECS`] wins, else the automatic rate for the
    /// reader in use.
    pub fn poll_interval(&self, new_reader: bool) -> std::time::Duration {
        if let Some(ms) = self.poll_interval_ms {
            return std::time::Duration::from_millis(
                ms.clamp(POLL_INTERVAL_MS_MIN, POLL_INTERVAL_MS_MAX),
            );
        }
        match self.poll_interval_secs {
            Some(secs) if secs != LEGACY_DEFAULT_POLL_SECS => {
                std::time::Duration::from_secs(secs.max(1))
            }
            _ if new_reader => NEW_READER_POLL_INTERVAL,
            _ => CLASSIC_POLL_INTERVAL,
        }
    }

    /// Where [`Self::poll_interval`] came from, for the startup log.
    pub fn poll_interval_source(&self) -> &'static str {
        if self.poll_interval_ms.is_some() {
            "poll_interval_ms"
        } else if matches!(
            self.poll_interval_secs,
            Some(secs) if secs != LEGACY_DEFAULT_POLL_SECS
        ) {
            "poll_interval_secs"
        } else {
            "automatic"
        }
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
/// Parsing is the URL standard (the `url` crate's `Url`, which is what
/// `reqwest::Url` re-exports), so `http://127.0.0.1.evil`
/// and `http://10.0.0.1` are rejected without a DNS lookup. Existing configs
/// still deserialize; callers refuse to send rather than crash.
pub fn validate_sync_server_url(raw: &str) -> Result<(), SyncUrlReject> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(SyncUrlReject::Invalid);
    }
    let parsed = url::Url::parse(raw).map_err(|_| SyncUrlReject::Invalid)?;
    match parsed.scheme() {
        "https" if parsed.host_str().is_some_and(|h| !h.is_empty()) => Ok(()),
        "http" if http_host_is_loopback(&parsed) => Ok(()),
        _ => Err(SyncUrlReject::HttpsRequired),
    }
}

fn http_host_is_loopback(url: &url::Url) -> bool {
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

    /// True when `update_channel` is the prerelease opt-in.
    pub fn wants_prerelease_updates(&self) -> bool {
        self.update_channel
            .trim()
            .eq_ignore_ascii_case("prerelease")
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
    /// Report kit retention from `report_kit_max_games` and `report_kit_max_mb`.
    pub fn report_kit_retention(&self) -> Retention {
        Retention::new(
            self.report_kit_max_games.unwrap_or(DEFAULT_MAX_GAMES),
            self.report_kit_max_mb.unwrap_or(DEFAULT_MAX_MB),
        )
    }

    pub fn debug_ocr_enabled(&self) -> bool {
        self.debug_ocr || Self::env_truthy("STAT_TRACKER_DEBUG_OCR")
    }

    /// File flag or `STAT_TRACKER_DEBUG_PORTAL_FRAMES`. The env value is not
    /// stored on the struct, so a Settings save cannot persist it.
    pub fn debug_portal_frames_enabled(&self) -> bool {
        self.debug_portal_frames || Self::env_truthy("STAT_TRACKER_DEBUG_PORTAL_FRAMES")
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

    /// File value of `ocr` plus this process's [`OCR_ENV`] override.
    ///
    /// The tracker service has its own environment, so Settings cannot treat
    /// `env` as the service's. `file` is what Settings may write.
    pub fn ocr_control(&self) -> OcrControl {
        Self::ocr_control_from(self.ocr, std::env::var(OCR_ENV).ok().as_deref())
    }

    /// Pure form of [`Self::ocr_control`]. `env` is the raw variable value.
    /// Only `on` and `off` count ([`OcrSetting::parse`]); anything else is
    /// no override.
    pub fn ocr_control_from(file: OcrSetting, env: Option<&str>) -> OcrControl {
        OcrControl {
            file,
            env: env.and_then(OcrSetting::parse),
        }
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

    /// Find a `--key value` pair in std::env::args().
    fn arg_value(key: &str) -> Option<String> {
        let args: Vec<String> = std::env::args().collect();
        args.windows(2).find(|w| w[0] == key).map(|w| w[1].clone())
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

fn unreadable_config(message: String) -> Box<dyn std::error::Error + Send + Sync> {
    message.into()
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
