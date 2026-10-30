//! Pure model for the 0.6 Settings page (redesign variant A).
//!
//! Everyday sections sit in the left column and the Advanced rows in the
//! right one from [`TWO_COLUMN_MIN`] px of page width. Below that the page
//! is one column, no wider than [`SINGLE_COLUMN_MAX`] and centred between
//! the gutters. Advanced rows fold; [`AdvancedOpen`] remembers which are
//! open. Changes apply as they are made, so the page counts what waits for
//! a tracker restart ([`changes_waiting`]) against one list of keys only
//! this app reads ([`APP_ONLY_KEYS`]). Nothing here draws.

use stat_tracker::config::{
    CaptureRoute, Config, PauseOverride, SCREENCAST_MAX_FPS_DEFAULT, SCREENCAST_MAX_FPS_MAX,
    SCREENCAST_MAX_FPS_MIN, ocr_threads_override,
};

pub use stat_tracker::config::OCR_THREADS_ENV;

use crate::settings::SessionKind;

/// Settings page width where it becomes two columns: the window less the
/// page gutters, not the window width the top bar breakpoints use.
pub const TWO_COLUMN_MIN: f32 = 1100.0;
/// Widest one column gets (spec `.setw`, 760 px). The classic page keeps
/// its own cap ([`crate::layout::SETTINGS_SINGLE_MAX`]).
pub const SINGLE_COLUMN_MAX: f32 = 760.0;
/// Gap between the two columns (spec 24 px).
pub const COLUMN_GAP: f32 = 24.0;
/// Widest the two-column page gets, so controls stay near their labels on
/// an ultrawide window. Left-aligned on the gutter (one column centres).
pub const PAGE_MAX: f32 = 2.0 * SINGLE_COLUMN_MAX + COLUMN_GAP;
/// Memory each OCR worker holds (one Tesseract instance with its model).
pub const OCR_WORKER_MB: usize = 23;
/// Most OCR workers the tracker runs ([`Config::ocr_threads_resolved`]).
pub const OCR_WORKERS_MAX: u32 = 8;

/// One or two columns for `content` px of width.
pub fn columns_for(content: f32) -> usize {
    if content.is_finite() && content >= TWO_COLUMN_MIN {
        2
    } else {
        1
    }
}

/// Width the page draws at for `content` px: the column cap in one
/// column, the two-column cap in two.
pub fn page_width(content: f32) -> f32 {
    let content = if content.is_finite() {
        content.max(0.0)
    } else {
        0.0
    };
    let cap = if columns_for(content) == 2 {
        PAGE_MAX
    } else {
        SINGLE_COLUMN_MAX
    };
    content.min(cap)
}

/// Config keys only this app reads, so a change to one never waits for a
/// tracker restart: the update channel, the setup flag, the session
/// window (Overview and Companion grouping), the reader pack address
/// (not used yet) and Report pictures (the app's misread report will read
/// it; nothing reads it yet, and the tracker never will). The waiting
/// count, the After restart tags and the legend all come from this list.
pub const APP_ONLY_KEYS: [&str; 5] = [
    "update_channel",
    "setup_completed",
    "session_window_secs",
    "reader_pack_url",
    "share_report_pictures",
];

/// Config keys whose Settings row is not shown because nothing reads the
/// key yet: the rank reader, stat picture upload, report pictures, the
/// on-screen capture check, the upload switch and extra in-game names.
/// The keys still load and save as before, and a hand-edited value never
/// shows in a summary, an After restart tag or the waiting count.
pub const HIDDEN_KEYS: [&str; 6] = [
    "rank_reader",
    "share_stat_pictures",
    "share_report_pictures",
    "capture_only_on_screen",
    "live_upload",
    "player_names",
];

/// Whether the Settings row for config key `key` is shown.
pub fn row_shown(key: &str) -> bool {
    !HIDDEN_KEYS.contains(&key)
}

/// `a` and `b` are the same in-game name: the same after the core's name
/// rule ([`stat_tracker::config::own_name`], which drops a `#1234` tag)
/// and Unicode lowercasing, as `Config::own_player_names` compares them.
pub fn same_name(a: &str, b: &str) -> bool {
    let key = |s: &str| {
        stat_tracker::config::own_name(s)
            .unwrap_or(s.trim())
            .to_lowercase()
    };
    key(a) == key(b)
}

/// True when `key` reaches the tracker only after it restarts: the tracker
/// reads config.toml once, at start.
pub fn needs_restart(key: &str) -> bool {
    !APP_ONLY_KEYS.contains(&key)
}

/// Left column, top to bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EverydaySection {
    PlayerSessions,
    Companion,
    /// Holds the Sharing rows too (spec: inside the account card).
    WebsiteAccount,
    Updates,
}

impl EverydaySection {
    pub const ALL: [EverydaySection; 4] = [
        EverydaySection::PlayerSessions,
        EverydaySection::Companion,
        EverydaySection::WebsiteAccount,
        EverydaySection::Updates,
    ];

    pub fn title(self) -> &'static str {
        match self {
            EverydaySection::PlayerSessions => "Player and sessions",
            EverydaySection::Companion => "Companion",
            EverydaySection::WebsiteAccount => "Website account",
            EverydaySection::Updates => "Updates",
        }
    }
}

/// Folding rows under ADVANCED, top to bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AdvancedSection {
    CaptureReading,
    TrackerService,
    StoredData,
    WebsiteConnection,
    Troubleshooting,
}

impl AdvancedSection {
    pub const ALL: [AdvancedSection; 5] = [
        AdvancedSection::CaptureReading,
        AdvancedSection::TrackerService,
        AdvancedSection::StoredData,
        AdvancedSection::WebsiteConnection,
        AdvancedSection::Troubleshooting,
    ];

    pub fn title(self) -> &'static str {
        match self {
            AdvancedSection::CaptureReading => "Capture and reading",
            AdvancedSection::TrackerService => "Tracker service",
            AdvancedSection::StoredData => "Stored data",
            AdvancedSection::WebsiteConnection => "Website connection",
            AdvancedSection::Troubleshooting => "Troubleshooting",
        }
    }

    /// The config keys this row's controls write.
    pub fn keys(self) -> &'static [&'static str] {
        match self {
            AdvancedSection::CaptureReading => &[
                "capture_route",
                "capture_output",
                "game_process_names",
                "screencast_pause_between_grabs",
                "screencast_max_fps",
                "reader",
                "ocr",
                "name_reader",
                "ocr_threads",
                "share_report_pictures",
                "capture_only_on_screen",
            ],
            AdvancedSection::TrackerService => &["auto_detect"],
            AdvancedSection::StoredData => &[],
            AdvancedSection::WebsiteConnection => &["sync"],
            AdvancedSection::Troubleshooting => &[
                "debug_ocr",
                "debug_save_pregame_screens",
                "shadow_recognizer",
            ],
        }
    }

    /// One line under the row: the current values, so a closed row says
    /// how it is set without opening it. It names only what the open row
    /// holds.
    pub fn summary(self, v: &AdvancedValues) -> String {
        match self {
            AdvancedSection::CaptureReading => {
                let mut line = format!(
                    "{} capture on {}, {}",
                    v.route,
                    v.monitor,
                    lower_first(v.reader.label())
                );
                if row_shown("share_report_pictures") {
                    line.push_str(&format!(", report pictures {}", on_off(v.report_pictures)));
                }
                line
            }
            AdvancedSection::TrackerService => {
                let login = if v.autostart {
                    "Starts on login"
                } else {
                    "Does not start on login"
                };
                if v.auto_detect {
                    format!(
                        "{login}, watches for match start and end, checks every {}, waits {} s after a match",
                        duration_line(v.poll_ms),
                        v.cooldown_secs,
                    )
                } else {
                    format!("{login}, match watching off")
                }
            }
            // No size until measured, and none for an empty or missing store.
            AdvancedSection::StoredData => match v.store_bytes.filter(|b| *b > 0) {
                Some(bytes) => format!(
                    "{} on this computer, {}. Compact, Delete",
                    games_line(v.games),
                    crate::settings_open::size_line(bytes)
                ),
                None => format!("{} on this computer. Compact, Delete", games_line(v.games)),
            },
            AdvancedSection::WebsiteConnection => match &v.sync_host {
                Some(host) => {
                    format!("Uploads to {host}. Manual server and token, has its own Apply")
                }
                None => "Uploads off. Manual server and token, has its own Apply".to_string(),
            },
            AdvancedSection::Troubleshooting => {
                let mut parts = vec![
                    format!("Debug images {}", on_off(v.debug_images)),
                    format!("screens around each game {}", on_off(v.pregame_screens)),
                ];
                if let Some(frames) = v.compare_frames {
                    parts.push(format!("KDE compare frames {}", on_off(frames)));
                }
                parts.push(format!("extra number reader {}", on_off(v.extra_reader)));
                parts.push("report a problem, about, uninstall".to_string());
                parts.join(", ")
            }
        }
    }

    /// Rows whose changes reach the tracker only after it restarts: any
    /// row with a key the tracker reads.
    ///
    /// Interpretation of spec p6 and p7, which tag only Capture and reading
    /// (and the Stream frame rate and Report pictures rows): the tag follows
    /// the keys, so Tracker service, Website connection and Troubleshooting
    /// carry it too, and so does In-game names. Each holds a key the
    /// tracker reads once at start, and an untagged row there would say its
    /// change applies at once when it does not.
    pub fn after_restart(self) -> bool {
        self.keys()
            .iter()
            .any(|key| needs_restart(key) && row_shown(key))
    }

    fn bit(self) -> u8 {
        match self {
            AdvancedSection::CaptureReading => 1,
            AdvancedSection::TrackerService => 1 << 1,
            AdvancedSection::StoredData => 1 << 2,
            AdvancedSection::WebsiteConnection => 1 << 3,
            AdvancedSection::Troubleshooting => 1 << 4,
        }
    }
}

/// Values the closed Advanced rows summarise, from the saved settings.
#[derive(Debug, Clone, PartialEq)]
pub struct AdvancedValues {
    pub route: &'static str,
    /// A monitor name, or "the Auto monitor" ([`monitor_phrase`]).
    pub monitor: String,
    pub reader: ReaderChoice,
    pub autostart: bool,
    pub auto_detect: bool,
    pub poll_ms: u64,
    pub cooldown_secs: u64,
    pub games: usize,
    /// Size of the stored data, once Settings has measured it.
    pub store_bytes: Option<u64>,
    /// The website's host while uploads are on.
    pub sync_host: Option<String>,
    pub debug_images: bool,
    /// Save screens around each game (`debug_save_pregame_screens`).
    pub pregame_screens: bool,
    /// KDE compare frames, on KDE only (`None` elsewhere).
    pub compare_frames: Option<bool>,
    pub extra_reader: bool,
    /// Report pictures (`share_report_pictures`).
    pub report_pictures: bool,
}

fn lower_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn on_off(v: bool) -> &'static str {
    if v { "on" } else { "off" }
}

/// `800 ms`, `1.25 s`, `4 s`.
pub fn duration_line(ms: u64) -> String {
    if ms < 1000 {
        format!("{ms} ms")
    } else if ms.is_multiple_of(1000) {
        format!("{} s", ms / 1000)
    } else {
        let text = format!("{:.2}", ms as f64 / 1000.0);
        format!("{} s", text.trim_end_matches('0'))
    }
}

fn games_line(n: usize) -> String {
    if n == 1 {
        "1 game".to_string()
    } else {
        format!("{n} games")
    }
}

/// The host part of a website address, for the summary and the account
/// card: `https://ow.example.no/` gives `ow.example.no`.
pub fn url_host(url: &str) -> String {
    let rest = url.trim();
    let rest = rest.split_once("://").map_or(rest, |(_, r)| r);
    let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    host.to_string()
}

/// Which Advanced rows are open. All start closed, so the page opens on
/// the everyday settings. More than one row can be open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AdvancedOpen(u8);

impl AdvancedOpen {
    pub fn is_open(self, section: AdvancedSection) -> bool {
        self.0 & section.bit() != 0
    }

    pub fn toggle(&mut self, section: AdvancedSection) {
        self.0 ^= section.bit();
    }

    #[cfg(test)]
    pub fn any_open(self) -> bool {
        self.0 != 0
    }
}

/// The three reader setups the Reader radio list offers. They map onto two
/// config keys: `reader = "new"` and `ocr = "on"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReaderChoice {
    NewOnly,
    NewWithOcrBackup,
    ClassicOnly,
}

impl ReaderChoice {
    pub const ALL: [ReaderChoice; 3] = [
        ReaderChoice::NewOnly,
        ReaderChoice::NewWithOcrBackup,
        ReaderChoice::ClassicOnly,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ReaderChoice::NewOnly => "New reader only",
            ReaderChoice::NewWithOcrBackup => "New reader with OCR backup",
            ReaderChoice::ClassicOnly => "Classic OCR only",
        }
    }

    /// What the two switches mean. OCR off always means the new reader, so
    /// new-reader-off with OCR off is new reader only.
    pub fn from_switches(new_reader: bool, classic_ocr: bool) -> Self {
        match (new_reader, classic_ocr) {
            (_, false) => ReaderChoice::NewOnly,
            (true, true) => ReaderChoice::NewWithOcrBackup,
            (false, true) => ReaderChoice::ClassicOnly,
        }
    }

    /// The choice a config stands for.
    pub fn of(config: &Config) -> Self {
        Self::from_switches(config.reader.is_new(), config.ocr.is_on())
    }

    /// `reader = "new"` for this choice.
    pub fn new_reader(self) -> bool {
        !matches!(self, ReaderChoice::ClassicOnly)
    }

    /// `ocr = "on"` for this choice. Turning it off goes through the reader
    /// pack check, which can refuse.
    pub fn classic_ocr(self) -> bool {
        !matches!(self, ReaderChoice::NewOnly)
    }
}

/// Session window presets for the dropdown, in seconds. The config still
/// stores seconds, so any value saved before (or typed in config.toml)
/// keeps working; one that is not a preset shows as its own entry.
pub const SESSION_WINDOW_PRESETS: [u64; 6] = [900, 1800, 3600, 7200, 10_800, 21_600];

/// One entry in the Session window dropdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionWindow(pub u64);

impl std::fmt::Display for SessionWindow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let secs = self.0;
        if secs == 0 {
            write!(f, "Off")
        } else if secs >= 3600 && secs.is_multiple_of(3600) {
            let h = secs / 3600;
            if h == 1 {
                write!(f, "1 hour")
            } else {
                write!(f, "{h} hours")
            }
        } else if secs >= 60 && secs.is_multiple_of(60) {
            write!(f, "{} min", secs / 60)
        } else {
            write!(f, "{secs} s")
        }
    }
}

/// The dropdown entries: the presets, plus the stored value when it is not
/// one (0 included, shown as "Off"), in order.
pub fn session_window_options(current: Option<u64>) -> Vec<SessionWindow> {
    let mut all: Vec<u64> = SESSION_WINDOW_PRESETS.to_vec();
    if let Some(c) = current
        && !all.contains(&c)
    {
        all.push(c);
        all.sort_unstable();
    }
    all.into_iter().map(SessionWindow).collect()
}

/// The chip lists on the Settings page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChipList {
    /// In-game names. Config holds one (`player_name`).
    Names,
    /// Game processes (`game_process_names`).
    Processes,
}

/// Comma or newline list. Empty → empty vec (process gate off, as in Config).
pub fn parse_process_names(raw: &str) -> Vec<String> {
    raw.split([',', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .collect()
}

/// The form text after removing chip `index`.
pub fn without_chip(raw: &str, index: usize) -> String {
    let mut chips = parse_process_names(raw);
    if index < chips.len() {
        chips.remove(index);
    }
    chips.join(", ")
}

/// The form text after adding what was typed. A draft with commas or
/// line breaks adds each name; blank names and names already listed
/// (ignoring case) are skipped.
pub fn with_chip(raw: &str, draft: &str) -> String {
    let mut chips = parse_process_names(raw);
    for chip in parse_process_names(draft) {
        if !chips.iter().any(|c| c.eq_ignore_ascii_case(&chip)) {
            chips.push(chip);
        }
    }
    chips.join(", ")
}

/// An in-game name as typed, through [`stat_tracker::config::own_name`]:
/// trimmed and without a `#1234` tag (the help says to leave it out, and the
/// scoreboard never shows it). `None` when fewer than
/// [`stat_tracker::config::MIN_OWN_NAME_CHARS`] characters are left, the
/// names `own_player_names` drops.
pub fn name_chip(raw: &str) -> Option<String> {
    stat_tracker::config::own_name(raw).map(str::to_string)
}

/// `config` as the Settings form would write it back. A hand-edited value
/// the form normalises (a cleartext sync URL it drops, an empty
/// `capture_output`) then never counts as a change.
fn as_form_would_save(config: &Config) -> Config {
    crate::settings::SettingsForm::from_config_and_shadow(
        config,
        Config::shadow_control(config.shadow_recognizer, None),
    )
    .to_config(config)
}

/// How many settings differ between `running` (what the tracker started
/// with) and `saved` (config.toml now), leaving out [`APP_ONLY_KEYS`]. Each
/// changed setting counts once: one Reader pick changes `reader` and `ocr`
/// but counts as one.
pub fn changes_waiting(running: &Config, saved: &Config) -> usize {
    let running = as_form_would_save(running);
    let saved = as_form_would_save(saved);
    let reader = usize::from(ReaderChoice::of(&running) != ReaderChoice::of(&saved));
    let (Ok(a), Ok(b)) = (serde_json::to_value(&running), serde_json::to_value(&saved)) else {
        return usize::from(running != saved);
    };
    match (a, b) {
        (serde_json::Value::Object(mut a), serde_json::Value::Object(mut b)) => {
            for key in APP_ONLY_KEYS
                .iter()
                .chain(&HIDDEN_KEYS)
                .chain(&["reader", "ocr"])
            {
                a.remove(*key);
                b.remove(*key);
            }
            reader + leaf_diffs(&serde_json::Value::Object(a), &serde_json::Value::Object(b))
        }
        (a, b) => usize::from(a != b),
    }
}

fn leaf_diffs(a: &serde_json::Value, b: &serde_json::Value) -> usize {
    use serde_json::Value;
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            let mut keys: Vec<&String> = x.keys().chain(y.keys()).collect();
            keys.sort();
            keys.dedup();
            keys.into_iter()
                .map(|k| {
                    leaf_diffs(
                        x.get(k).unwrap_or(&Value::Null),
                        y.get(k).unwrap_or(&Value::Null),
                    )
                })
                .sum()
        }
        (x, y) => usize::from(x != y),
    }
}

/// The count and the words after it: (1, " change waiting").
pub fn changes_waiting_words(n: usize) -> (String, &'static str) {
    (
        n.to_string(),
        if n == 1 {
            " change waiting"
        } else {
            " changes waiting"
        },
    )
}

/// A setting the 0.6 page applies straight to config.toml. The rest go
/// through the shared form ([`crate::settings::SettingsForm`]); these have
/// no form field, and the classic page keeps them as they are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigEdit {
    /// Upload games after each match (`live_upload`).
    LiveUpload(bool),
    /// Read my rank after Comp games (`rank_reader`).
    RankReader(bool),
    /// Share stat pictures (`share_stat_pictures`).
    ShareStatPictures(bool),
    /// Report pictures (`share_report_pictures`).
    ShareReportPictures(bool),
    /// Only capture while Overwatch is on screen: Auto (`true`) or Off.
    CaptureOnlyOnScreen(bool),
    /// Another in-game name, kept in `player_names`.
    AddPlayerName(String),
    /// Drop these names from `player_names`, every spelling of each
    /// ([`same_name`]).
    RemovePlayerNames(Vec<String>),
    /// Stream frame rate. `None` is the default.
    FrameRate(Option<u32>),
    /// Pause between grabs. On leaves the choice to the tracker per desktop.
    PauseBetweenGrabs(bool),
    NameReader(bool),
    /// OCR workers. `None` is Auto.
    OcrWorkers(Option<u32>),
    /// Save screens around each game (`debug_save_pregame_screens`).
    SavePregameScreens(bool),
}

impl ConfigEdit {
    /// Write this edit into `config`. A value equal to the default is
    /// stored as missing, so config.toml stays short.
    pub fn apply(self, config: &mut Config) {
        match self {
            ConfigEdit::FrameRate(fps) => {
                config.screencast_max_fps = fps
                    .map(|f| f.clamp(SCREENCAST_MAX_FPS_MIN, SCREENCAST_MAX_FPS_MAX))
                    .filter(|&f| f != SCREENCAST_MAX_FPS_DEFAULT);
            }
            ConfigEdit::PauseBetweenGrabs(on) => {
                config.screencast_pause_between_grabs = if on {
                    config.screencast_pause_between_grabs.filter(|&v| v)
                } else {
                    Some(false)
                };
            }
            ConfigEdit::NameReader(on) => config.name_reader = on,
            ConfigEdit::LiveUpload(on) => config.live_upload = on,
            ConfigEdit::RankReader(on) => config.rank_reader = on,
            ConfigEdit::ShareStatPictures(on) => config.share_stat_pictures = on,
            ConfigEdit::ShareReportPictures(on) => config.share_report_pictures = on,
            ConfigEdit::CaptureOnlyOnScreen(on) => config.capture_only_on_screen = on,
            ConfigEdit::SavePregameScreens(on) => config.debug_save_pregame_screens = on,
            ConfigEdit::AddPlayerName(name) => {
                // The core's rule: trimmed, no #1234 tag, at least 3
                // characters. Anything else is not a name.
                let Some(name) = stat_tracker::config::own_name(&name) else {
                    return;
                };
                let known = config
                    .player_name
                    .iter()
                    .chain(config.player_names.iter())
                    .any(|n| same_name(n, name));
                if !known {
                    config.player_names.push(name.to_string());
                }
            }
            ConfigEdit::RemovePlayerNames(names) => {
                config
                    .player_names
                    .retain(|n| !names.iter().any(|gone| same_name(n, gone)));
            }
            ConfigEdit::OcrWorkers(n) => {
                config.ocr_threads = n.map(|n| n.clamp(1, OCR_WORKERS_MAX));
            }
        }
    }
}

/// One entry in the Stream frame rate dropdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameRate(pub u32);

impl std::fmt::Display for FrameRate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0 == SCREENCAST_MAX_FPS_DEFAULT {
            write!(f, "{} fps (default)", self.0)
        } else {
            write!(f, "{} fps", self.0)
        }
    }
}

/// Every frame rate the tracker takes, 1 to 5.
pub fn frame_rate_options() -> Vec<FrameRate> {
    (SCREENCAST_MAX_FPS_MIN..=SCREENCAST_MAX_FPS_MAX)
        .map(FrameRate)
        .collect()
}

/// What Automatic pausing does on this desktop, as far as the app can
/// tell. The tracker decides once at start (`decide` in its capture
/// stream's `pause` module): on with KWin 6.1 or newer and on niri; off on
/// GNOME, Plasma 5, older KWin and other compositors. The app cannot read
/// KWin's version, so Plasma 6 is [`AutoPause::Unknown`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoPause {
    On,
    /// Off here, and why, for the line under the switch.
    Off(&'static str),
    /// It depends on what only the tracker reads.
    Unknown(&'static str),
}

/// [`AutoPause`] from `XDG_CURRENT_DESKTOP` and `KDE_SESSION_VERSION`,
/// with the tracker's desktop-name rules.
pub fn auto_pause(desktop: &str, kde_session_version: Option<&str>) -> AutoPause {
    let has = |names: &[&str]| {
        desktop.split(':').any(|token| {
            let token = token.trim().to_ascii_lowercase();
            names.iter().any(|name| token.contains(name))
        })
    };
    if has(&["kde", "plasma", "kwin"]) {
        return if kde_session_version.map(str::trim) == Some("5") {
            AutoPause::Off("Off on Plasma 5 (Automatic).")
        } else {
            AutoPause::Unknown("Automatic: on with KWin 6.1 or newer, off on older KWin.")
        };
    }
    if has(&["niri"]) {
        return AutoPause::On;
    }
    if has(&["gnome"]) {
        return AutoPause::Off("Off on GNOME (Automatic).");
    }
    if has(&[
        "sway", "hyprland", "wayfire", "river", "labwc", "wlroots", "cosmic",
    ]) {
        return AutoPause::Off("Off on this desktop (Automatic).");
    }
    AutoPause::Unknown("Automatic: the tracker decides from the running compositor.")
}

/// The Pause between grabs switch: whether it shows On, whether the env
/// var fixes it outside the app (the switch is then locked), whether it
/// can be pressed, and the line that says what Automatic does here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PauseRow {
    pub on: bool,
    pub locked: bool,
    /// False where On would change nothing: Automatic is off here.
    pub can_toggle: bool,
    pub note: Option<&'static str>,
    /// The stream pauses here for sure, so the summary may say so.
    pub pauses: bool,
}

pub fn pause_row(pause: PauseOverride, auto: AutoPause) -> PauseRow {
    let fixed = |on: bool, locked: bool| PauseRow {
        on,
        locked,
        can_toggle: !locked,
        note: None,
        pauses: on,
    };
    match (pause, auto) {
        (PauseOverride::Auto, AutoPause::On) => fixed(true, false),
        (PauseOverride::Auto, AutoPause::Off(why)) => PauseRow {
            on: false,
            locked: false,
            can_toggle: false,
            note: Some(why),
            pauses: false,
        },
        (PauseOverride::Auto, AutoPause::Unknown(why)) => PauseRow {
            on: true,
            locked: false,
            can_toggle: true,
            note: Some(why),
            pauses: false,
        },
        (PauseOverride::ConfigOff, _) => fixed(false, false),
        (PauseOverride::EnvOn, _) => fixed(true, true),
        (PauseOverride::EnvOff, _) => fixed(false, true),
    }
}

/// One entry in the OCR workers dropdown. `None` is Auto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OcrWorkers(pub Option<u32>);

impl std::fmt::Display for OcrWorkers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            None => write!(f, "Auto"),
            Some(n) => write!(f, "{n}"),
        }
    }
}

/// Auto, then 1 to 8.
pub fn ocr_worker_options() -> Vec<OcrWorkers> {
    std::iter::once(OcrWorkers(None))
        .chain((1..=OCR_WORKERS_MAX).map(|n| OcrWorkers(Some(n))))
        .collect()
}

/// The count [`OCR_THREADS_ENV`] fixes, when its value is one the tracker
/// takes. Any other value (empty, "0", "abc", " 3") is ignored by the
/// tracker, so it does not lock the row.
pub fn ocr_env_lock(env: Option<&str>) -> Option<u32> {
    env.and_then(ocr_threads_override)
}

/// Help under OCR workers: the memory each one takes, and the count in
/// use when Auto picks it.
pub fn ocr_workers_help(resolved: usize, auto: bool) -> String {
    if auto {
        format!("About {OCR_WORKER_MB} MB each. Auto uses {resolved} on this computer.")
    } else {
        format!("About {OCR_WORKER_MB} MB each.")
    }
}

/// The monitor Auto captures, as far as this app can tell from the output
/// names: the only output, or the only one that is not a built-in panel
/// (eDP, LVDS, DSI). With several external monitors the tracker picks the
/// largest, which only it can see, so this is `None` ("Auto").
pub fn auto_monitor(outputs: &[String]) -> Option<&str> {
    let builtin = |name: &str| ["eDP", "LVDS", "DSI"].iter().any(|p| name.starts_with(p));
    if outputs.len() == 1 {
        return Some(&outputs[0]);
    }
    let mut external = outputs.iter().filter(|o| !builtin(o));
    match (external.next(), external.next()) {
        (Some(only), None) => Some(only),
        _ => None,
    }
}

/// "Auto (DP-3)" when the Auto monitor is known, else "Auto".
pub fn auto_monitor_label(outputs: &[String]) -> String {
    match auto_monitor(outputs) {
        Some(name) => format!("Auto ({name})"),
        None => "Auto".to_string(),
    }
}

/// The monitor for a summary line: the chosen one, the Auto one when it is
/// known, or "the Auto monitor".
pub fn monitor_phrase(chosen: &str, outputs: &[String]) -> String {
    let chosen = chosen.trim();
    if !chosen.is_empty() {
        return chosen.to_string();
    }
    auto_monitor(outputs).map_or_else(|| "the Auto monitor".to_string(), str::to_string)
}

/// Whether the screen stream rows (Pause between grabs, Stream frame rate)
/// do anything here: the stream runs for Screen stream, and for Automatic
/// on KDE and GNOME. Never on X11.
pub fn stream_rows_apply(desktop: &str, session: SessionKind, route: CaptureRoute) -> bool {
    if session == SessionKind::X11 {
        return false;
    }
    match route {
        CaptureRoute::Screencast => true,
        CaptureRoute::Auto => {
            stat_tracker::capture::is_kde(desktop) || stat_tracker::capture::is_gnome(desktop)
        }
        CaptureRoute::Screenshot | CaptureRoute::Kwin => false,
    }
}

/// The version the strip shows: the installed tracker's, else this
/// build's tracker version. Never this UI crate's own version.
pub fn strip_version(installed: Option<&str>) -> String {
    installed
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map_or_else(crate::update::daemon_package_version, str::to_string)
}

/// A row whose value can differ from the default, for its changed dot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultRow {
    SessionWindow,
    CaptureMethod,
    Monitor,
    GameProcesses,
    PauseBetweenGrabs,
    FrameRate,
    Reader,
    NameReader,
    OcrWorkers,
}

/// True when `config` holds something else than a fresh install for `row`.
pub fn differs_from_default(config: &Config, row: DefaultRow) -> bool {
    let d = Config::default();
    match row {
        DefaultRow::SessionWindow => config.session_window_secs != d.session_window_secs,
        DefaultRow::CaptureMethod => config.capture_route != d.capture_route,
        DefaultRow::Monitor => config
            .capture_output
            .as_deref()
            .is_some_and(|o| !o.trim().is_empty()),
        DefaultRow::GameProcesses => config.game_process_names != d.game_process_names,
        // Off by the file or by the env var; the env var is in effect too.
        DefaultRow::PauseBetweenGrabs => matches!(
            config.screencast_pause_override(),
            PauseOverride::ConfigOff | PauseOverride::EnvOff
        ),
        DefaultRow::FrameRate => config.screencast_max_fps() != SCREENCAST_MAX_FPS_DEFAULT,
        DefaultRow::Reader => ReaderChoice::of(config) != ReaderChoice::of(&d),
        DefaultRow::NameReader => config.name_reader != d.name_reader,
        // An explicit count, from the file or the env overlay of the shown
        // config, whatever this computer's Auto count is.
        DefaultRow::OcrWorkers => config.ocr_threads.is_some(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_columns_from_1100() {
        assert_eq!(columns_for(0.0), 1);
        assert_eq!(columns_for(760.0), 1);
        assert_eq!(columns_for(1099.9), 1);
        assert_eq!(columns_for(1100.0), 2);
        assert_eq!(columns_for(3000.0), 2);
        assert_eq!(columns_for(f32::NAN), 1);
        assert_eq!(columns_for(f32::INFINITY), 1);
    }

    #[test]
    fn the_page_is_capped_in_both_modes() {
        assert_eq!(page_width(976.0), SINGLE_COLUMN_MAX);
        assert_eq!(page_width(700.0), 700.0);
        assert_eq!(page_width(1200.0), 1200.0);
        assert_eq!(page_width(3400.0), PAGE_MAX);
        assert_eq!(PAGE_MAX, 1544.0);
        assert_eq!(page_width(f32::NAN), 0.0);
    }

    #[test]
    fn sections_follow_the_spec_order() {
        let everyday: Vec<_> = EverydaySection::ALL.iter().map(|s| s.title()).collect();
        assert_eq!(
            everyday,
            [
                "Player and sessions",
                "Companion",
                "Website account",
                "Updates"
            ]
        );
        let advanced: Vec<_> = AdvancedSection::ALL.iter().map(|s| s.title()).collect();
        assert_eq!(
            advanced,
            [
                "Capture and reading",
                "Tracker service",
                "Stored data",
                "Website connection",
                "Troubleshooting"
            ]
        );
    }

    #[test]
    fn rows_with_a_tracker_key_are_tagged_after_restart() {
        for section in AdvancedSection::ALL {
            assert_eq!(
                section.after_restart(),
                section != AdvancedSection::StoredData,
                "{section:?}"
            );
        }
        assert!(!needs_restart("session_window_secs"));
        assert!(!needs_restart("update_channel"));
        assert!(needs_restart("player_name"));
        assert!(needs_restart("auto_detect"));
    }

    #[test]
    fn every_listed_key_is_a_config_key() {
        let mut config = Config::default();
        config.screencast_max_fps = Some(3);
        config.screencast_pause_between_grabs = Some(false);
        config.ocr_threads = Some(2);
        config.name_reader = false;
        config.shadow_recognizer = true;
        config.sync = Some(stat_tracker::config::SyncConfig {
            server_url: "https://example.org".into(),
            token: "t".into(),
        });
        config.capture_output = Some("DP-3".into());
        config.capture_route = CaptureRoute::Screencast;
        config.reader_pack_url = Some("https://example.org/packs".into());
        config.update_channel = "prerelease".into();
        config.setup_completed = true;
        config.debug_ocr = true;
        config.debug_save_pregame_screens = true;
        config.reader = stat_tracker::config::ReaderSetting::OcrV1;
        config.ocr = stat_tracker::config::OcrSetting::Off;
        config.share_report_pictures = true;
        config.capture_only_on_screen = false;
        // Keys left out while they hold their default are set above.
        let serde_json::Value::Object(map) = serde_json::to_value(&config).unwrap() else {
            panic!("config is a table");
        };
        let all = AdvancedSection::ALL
            .iter()
            .flat_map(|s| s.keys().iter())
            .chain(APP_ONLY_KEYS.iter());
        for key in all {
            assert!(map.contains_key(*key), "{key}");
        }
    }

    #[test]
    fn advanced_rows_start_closed_and_fold_independently() {
        let mut open = AdvancedOpen::default();
        assert!(!open.any_open());
        open.toggle(AdvancedSection::CaptureReading);
        open.toggle(AdvancedSection::StoredData);
        assert!(open.is_open(AdvancedSection::CaptureReading));
        assert!(open.is_open(AdvancedSection::StoredData));
        assert!(!open.is_open(AdvancedSection::TrackerService));
        open.toggle(AdvancedSection::CaptureReading);
        open.toggle(AdvancedSection::StoredData);
        assert!(!open.any_open());
    }

    #[test]
    fn reader_choice_round_trips_through_the_switches() {
        for choice in ReaderChoice::ALL {
            assert_eq!(
                ReaderChoice::from_switches(choice.new_reader(), choice.classic_ocr()),
                choice
            );
        }
        assert_eq!(
            ReaderChoice::from_switches(false, false),
            ReaderChoice::NewOnly
        );
    }

    fn values() -> AdvancedValues {
        AdvancedValues {
            route: "Automatic",
            monitor: "DP-3".into(),
            reader: ReaderChoice::NewWithOcrBackup,
            autostart: true,
            auto_detect: true,
            poll_ms: 1500,
            cooldown_secs: 120,
            games: 857,
            store_bytes: Some(38_240_000),
            sync_host: Some("ow.example.no".into()),
            debug_images: false,
            pregame_screens: false,
            compare_frames: None,
            extra_reader: false,
            report_pictures: false,
        }
    }

    /// A hidden row's key (report pictures, the on-screen capture check)
    /// is inert: a hand-edited value shows in no summary, tag or count.
    #[test]
    fn hidden_keys_never_show_in_summaries_tags_or_counts() {
        let mut v = values();
        v.report_pictures = true;
        assert!(
            !AdvancedSection::CaptureReading
                .summary(&v)
                .contains("report pictures")
        );
        let running = Config::default();
        let mut saved = running.clone();
        saved.capture_only_on_screen = false;
        saved.share_report_pictures = true;
        saved.live_upload = false;
        saved.player_names = vec!["Alt".into()];
        saved.rank_reader = true;
        saved.share_stat_pictures = true;
        assert_eq!(changes_waiting(&running, &saved), 0);
        for key in HIDDEN_KEYS {
            assert!(!row_shown(key), "{key}");
        }
        // Capture and reading keeps its tag from the keys it shows.
        assert!(AdvancedSection::CaptureReading.after_restart());
    }

    #[test]
    fn advanced_summaries_name_only_what_the_rows_hold() {
        let mut v = values();
        assert_eq!(
            AdvancedSection::CaptureReading.summary(&v),
            "Automatic capture on DP-3, new reader with OCR backup"
        );
        assert_eq!(
            AdvancedSection::TrackerService.summary(&v),
            "Starts on login, watches for match start and end, checks every 1.5 s, waits 120 s after a match"
        );
        assert_eq!(
            AdvancedSection::StoredData.summary(&v),
            "857 games on this computer, 38.2 MB. Compact, Delete"
        );
        assert_eq!(
            AdvancedSection::WebsiteConnection.summary(&v),
            "Uploads to ow.example.no. Manual server and token, has its own Apply"
        );
        assert!(!AdvancedSection::Troubleshooting.summary(&v).contains("KDE"));
        assert!(
            AdvancedSection::Troubleshooting
                .summary(&v)
                .starts_with("Debug images off, screens around each game off, ")
        );
        v.pregame_screens = true;
        assert!(
            AdvancedSection::Troubleshooting
                .summary(&v)
                .contains("screens around each game on")
        );
        v.auto_detect = false;
        v.compare_frames = Some(true);
        v.sync_host = None;
        assert_eq!(
            AdvancedSection::TrackerService.summary(&v),
            "Starts on login, match watching off"
        );
        assert!(
            AdvancedSection::Troubleshooting
                .summary(&v)
                .contains("KDE compare frames on")
        );
        assert!(
            AdvancedSection::WebsiteConnection
                .summary(&v)
                .starts_with("Uploads off")
        );
    }

    #[test]
    fn durations_read_naturally() {
        assert_eq!(duration_line(4000), "4 s");
        assert_eq!(duration_line(1500), "1.5 s");
        assert_eq!(duration_line(1250), "1.25 s");
        assert_eq!(duration_line(80), "80 ms");
    }

    #[test]
    fn host_comes_out_of_the_address() {
        assert_eq!(url_host("https://ow.example.no/"), "ow.example.no");
        assert_eq!(url_host("http://127.0.0.1:3030/api"), "127.0.0.1:3030");
        assert_eq!(url_host("crew.example"), "crew.example");
    }

    #[test]
    fn session_window_keeps_any_stored_value_and_lists_zero() {
        let opts = session_window_options(Some(1800));
        assert_eq!(opts.len(), SESSION_WINDOW_PRESETS.len());
        assert_eq!(SessionWindow(1800).to_string(), "30 min");
        assert_eq!(SessionWindow(10_800).to_string(), "3 hours");
        let odd = session_window_options(Some(2700));
        assert!(odd.contains(&SessionWindow(2700)));
        assert!(odd.windows(2).all(|w| w[0].0 < w[1].0));
        let zero = session_window_options(Some(0));
        assert_eq!(zero[0], SessionWindow(0));
        assert_eq!(SessionWindow(0).to_string(), "Off");
    }

    #[test]
    fn chips_round_trip_the_comma_text() {
        assert_eq!(
            parse_process_names("Overwatch.exe, ow.exe"),
            ["Overwatch.exe", "ow.exe"]
        );
        assert_eq!(without_chip("Overwatch.exe, ow.exe", 0), "ow.exe");
        assert_eq!(without_chip("Overwatch.exe", 5), "Overwatch.exe");
        assert_eq!(with_chip("Overwatch.exe", "overwatch.EXE"), "Overwatch.exe");
        assert_eq!(with_chip("", "  "), "");
        assert_eq!(with_chip("", " , ,"), "");
    }

    #[test]
    fn a_draft_with_commas_adds_each_name_once() {
        assert_eq!(
            with_chip("Overwatch.exe", "foo, Overwatch.exe"),
            "Overwatch.exe, foo"
        );
        assert_eq!(with_chip("", "a.exe, A.EXE\nb.exe"), "a.exe, b.exe");
        let added = with_chip("Overwatch.exe", "foo, overwatch.EXE");
        assert_eq!(parse_process_names(&added), ["Overwatch.exe", "foo"]);
    }

    #[test]
    fn a_name_chip_drops_the_battle_tag() {
        assert_eq!(name_chip("FrozenTear#2112").as_deref(), Some("FrozenTear"));
        assert_eq!(name_chip(" FrozenTear ").as_deref(), Some("FrozenTear"));
        assert_eq!(name_chip("A#B").as_deref(), Some("A#B"));
        assert_eq!(name_chip("#1234"), None);
        assert_eq!(name_chip("  "), None);
        assert_eq!(name_chip("Ab#1234"), None, "too short to match");
        assert_eq!(name_chip("Öst").as_deref(), Some("Öst"));
    }

    #[test]
    fn process_names_split_comma_and_newline() {
        assert_eq!(
            parse_process_names("Overwatch.exe\nwine\n"),
            vec!["Overwatch.exe", "wine"]
        );
        assert!(parse_process_names("  ,  \n").is_empty());
    }

    #[test]
    fn changes_waiting_counts_tracker_settings_only() {
        let base = Config::default();
        assert_eq!(changes_waiting(&base, &base), 0);
        let mut now = base.clone();
        now.auto_detect.cooldown_secs += 1;
        assert_eq!(changes_waiting(&base, &now), 1);
        now.game_process_names.push("ow2.exe".into());
        assert_eq!(changes_waiting(&base, &now), 2);
        // Only this app reads these.
        let mut app_only = base.clone();
        app_only.update_channel = "alpha".into();
        app_only.session_window_secs = 10_800;
        app_only.reader_pack_url = Some("https://example.org".into());
        app_only.setup_completed = !base.setup_completed;
        assert_eq!(changes_waiting(&base, &app_only), 0);
    }

    #[test]
    fn one_reader_pick_is_one_change() {
        let base = Config::default();
        for choice in ReaderChoice::ALL {
            let mut now = base.clone();
            now.reader = if choice.new_reader() {
                stat_tracker::config::ReaderSetting::New
            } else {
                stat_tracker::config::ReaderSetting::OcrV1
            };
            now.ocr = if choice.classic_ocr() {
                stat_tracker::config::OcrSetting::On
            } else {
                stat_tracker::config::OcrSetting::Off
            };
            let want = usize::from(choice != ReaderChoice::of(&base));
            assert_eq!(changes_waiting(&base, &now), want, "{choice:?}");
        }
    }

    #[test]
    fn values_the_form_normalises_are_not_changes() {
        let hand = Config {
            capture_output: Some(String::new()),
            sync: Some(stat_tracker::config::SyncConfig {
                server_url: "http://crew.example".into(),
                token: "t".into(),
            }),
            ..Config::default()
        };
        let saved = as_form_would_save(&hand);
        assert_eq!(changes_waiting(&hand, &saved), 0);
    }

    #[test]
    fn changes_waiting_words_split_the_count() {
        assert_eq!(
            changes_waiting_words(1),
            ("1".to_string(), " change waiting")
        );
        assert_eq!(
            changes_waiting_words(3),
            ("3".to_string(), " changes waiting")
        );
    }

    #[test]
    fn config_edits_store_defaults_as_missing() {
        let mut c = Config::default();
        ConfigEdit::FrameRate(Some(3)).apply(&mut c);
        assert_eq!(c.screencast_max_fps, Some(3));
        ConfigEdit::FrameRate(Some(SCREENCAST_MAX_FPS_DEFAULT)).apply(&mut c);
        assert_eq!(c.screencast_max_fps, None);
        ConfigEdit::FrameRate(Some(9)).apply(&mut c);
        assert_eq!(c.screencast_max_fps, Some(SCREENCAST_MAX_FPS_MAX));
        ConfigEdit::FrameRate(None).apply(&mut c);
        assert_eq!(c.screencast_max_fps, None);

        ConfigEdit::PauseBetweenGrabs(false).apply(&mut c);
        assert_eq!(c.screencast_pause_between_grabs, Some(false));
        ConfigEdit::PauseBetweenGrabs(true).apply(&mut c);
        assert_eq!(c.screencast_pause_between_grabs, None);
        c.screencast_pause_between_grabs = Some(true);
        ConfigEdit::PauseBetweenGrabs(true).apply(&mut c);
        assert_eq!(c.screencast_pause_between_grabs, Some(true));

        ConfigEdit::NameReader(false).apply(&mut c);
        assert!(!c.name_reader);
        ConfigEdit::OcrWorkers(Some(12)).apply(&mut c);
        assert_eq!(c.ocr_threads, Some(OCR_WORKERS_MAX));
        ConfigEdit::OcrWorkers(None).apply(&mut c);
        assert_eq!(c.ocr_threads, None);
    }

    #[test]
    fn frame_rates_and_workers_list_every_choice() {
        let rates: Vec<String> = frame_rate_options().iter().map(|r| r.to_string()).collect();
        assert_eq!(rates.len(), 5);
        assert_eq!(
            rates[(SCREENCAST_MAX_FPS_DEFAULT - 1) as usize],
            format!("{SCREENCAST_MAX_FPS_DEFAULT} fps (default)")
        );
        assert_eq!(rates[0], "1 fps");
        let workers = ocr_worker_options();
        assert_eq!(workers.len(), 9);
        assert_eq!(workers[0].to_string(), "Auto");
        assert_eq!(workers[8].to_string(), "8");
    }

    #[test]
    fn pause_switch_follows_the_override() {
        let niri = auto_pause("niri", None);
        assert_eq!(niri, AutoPause::On);
        assert_eq!(
            pause_row(PauseOverride::Auto, niri),
            PauseRow {
                on: true,
                locked: false,
                can_toggle: true,
                note: None,
                pauses: true,
            }
        );
        assert!(!pause_row(PauseOverride::ConfigOff, niri).on);
        assert!(pause_row(PauseOverride::ConfigOff, niri).can_toggle);
        assert!(pause_row(PauseOverride::EnvOn, niri).locked);
        assert!(pause_row(PauseOverride::EnvOn, niri).pauses);
        assert!(!pause_row(PauseOverride::EnvOff, niri).on);
        assert!(!pause_row(PauseOverride::EnvOff, niri).can_toggle);
    }

    /// Automatic does not pause on GNOME, Plasma 5 or wlroots desktops, so
    /// the switch does not claim it does there.
    #[test]
    fn automatic_pause_shows_what_this_desktop_gets() {
        for (desktop, version) in [
            ("GNOME", None),
            ("ubuntu:GNOME", None),
            ("KDE", Some("5")),
            ("sway", None),
        ] {
            let row = pause_row(PauseOverride::Auto, auto_pause(desktop, version));
            assert!(!row.on && !row.pauses && !row.can_toggle, "{desktop}");
            assert!(
                row.note.is_some_and(|n| n.starts_with("Off on")),
                "{desktop}"
            );
        }
        let plasma6 = pause_row(PauseOverride::Auto, auto_pause("KDE", Some("6")));
        assert!(plasma6.on && !plasma6.pauses, "unknown is not claimed");
        assert!(plasma6.note.is_some_and(|n| n.contains("KWin 6.1")));
        assert!(matches!(auto_pause("", None), AutoPause::Unknown(_)));
    }

    #[test]
    fn only_a_value_the_tracker_takes_locks_ocr_workers() {
        assert_eq!(ocr_env_lock(None), None);
        assert_eq!(ocr_env_lock(Some("")), None);
        assert_eq!(ocr_env_lock(Some("0")), None);
        assert_eq!(ocr_env_lock(Some("abc")), None);
        assert_eq!(ocr_env_lock(Some(" 3")), None);
        assert_eq!(ocr_env_lock(Some("3")), Some(3));
        assert_eq!(
            ocr_workers_help(4, true),
            "About 23 MB each. Auto uses 4 on this computer."
        );
        assert_eq!(ocr_workers_help(2, false), "About 23 MB each.");
    }

    #[test]
    fn auto_monitor_follows_the_tracker_rule() {
        let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            auto_monitor_label(&names(&["eDP-1", "DP-3"])),
            "Auto (DP-3)"
        );
        assert_eq!(auto_monitor_label(&names(&["eDP-1"])), "Auto (eDP-1)");
        assert_eq!(auto_monitor_label(&names(&["DP-1", "DP-3"])), "Auto");
        assert_eq!(auto_monitor_label(&[]), "Auto");
        assert_eq!(
            monitor_phrase("", &names(&["eDP-1", "HDMI-A-1"])),
            "HDMI-A-1"
        );
        assert_eq!(monitor_phrase(" DP-2 ", &[]), "DP-2");
        assert_eq!(monitor_phrase("", &[]), "the Auto monitor");
    }

    #[test]
    fn stream_rows_apply_only_where_the_stream_runs() {
        assert!(stream_rows_apply(
            "KDE",
            SessionKind::Wayland,
            CaptureRoute::Auto
        ));
        assert!(stream_rows_apply(
            "GNOME",
            SessionKind::Wayland,
            CaptureRoute::Auto
        ));
        assert!(!stream_rows_apply(
            "niri",
            SessionKind::Wayland,
            CaptureRoute::Auto
        ));
        assert!(stream_rows_apply(
            "niri",
            SessionKind::Wayland,
            CaptureRoute::Screencast
        ));
        assert!(!stream_rows_apply(
            "KDE",
            SessionKind::Wayland,
            CaptureRoute::Screenshot
        ));
        assert!(!stream_rows_apply(
            "KDE",
            SessionKind::Wayland,
            CaptureRoute::Kwin
        ));
        assert!(!stream_rows_apply(
            "KDE",
            SessionKind::X11,
            CaptureRoute::Screencast
        ));
    }

    #[test]
    fn the_strip_never_shows_the_ui_crate_version() {
        assert_eq!(strip_version(Some("0.6.0")), "0.6.0");
        let fallback = strip_version(None);
        assert_eq!(fallback, crate::update::daemon_package_version());
        assert_ne!(fallback, env!("CARGO_PKG_VERSION"));
        assert_eq!(strip_version(Some("  ")), fallback);
    }

    #[test]
    fn changed_dots_compare_with_a_fresh_install() {
        let base = Config::default();
        for row in [
            DefaultRow::SessionWindow,
            DefaultRow::CaptureMethod,
            DefaultRow::Monitor,
            DefaultRow::GameProcesses,
            DefaultRow::PauseBetweenGrabs,
            DefaultRow::FrameRate,
            DefaultRow::Reader,
            DefaultRow::NameReader,
            DefaultRow::OcrWorkers,
        ] {
            assert!(!differs_from_default(&base, row), "{row:?}");
        }
        let mut c = base.clone();
        c.session_window_secs = 10_800;
        c.capture_output = Some("DP-3".into());
        c.screencast_max_fps = Some(3);
        c.ocr_threads = Some(2);
        assert!(differs_from_default(&c, DefaultRow::SessionWindow));
        assert!(differs_from_default(&c, DefaultRow::Monitor));
        assert!(differs_from_default(&c, DefaultRow::FrameRate));
        assert!(differs_from_default(&c, DefaultRow::OcrWorkers));
        assert!(!differs_from_default(&c, DefaultRow::NameReader));
    }

    #[test]
    fn copy_has_no_plan_codes() {
        let mut all: Vec<String> = Vec::new();
        all.extend(EverydaySection::ALL.iter().map(|s| s.title().to_string()));
        for s in AdvancedSection::ALL {
            all.push(s.title().into());
            all.push(s.summary(&values()));
        }
        all.extend(ReaderChoice::ALL.iter().map(|c| c.label().to_string()));
        for line in all {
            assert!(!line.contains("S11"), "{line}");
        }
    }
}
