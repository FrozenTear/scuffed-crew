//! Pure model for the 0.6 Settings page (redesign variant A).
//!
//! Everyday sections sit in the left column and the Advanced rows in the
//! right one from [`TWO_COLUMN_MIN`] px of content width. Below that the
//! page is one column, no wider than [`SINGLE_COLUMN_MAX`]. Advanced rows
//! fold; [`AdvancedOpen`] remembers which are open. Nothing here draws.

use stat_tracker::config::{PauseOverride, SCREENCAST_MAX_FPS_DEFAULT};

/// Content width where Settings becomes two columns.
pub const TWO_COLUMN_MIN: f32 = 1100.0;
/// Widest the one-column page gets.
pub const SINGLE_COLUMN_MAX: f32 = 760.0;
/// Memory each OCR worker holds (one Tesseract instance with its model).
pub const OCR_WORKER_MB: usize = 23;
/// Env var that fixes the OCR worker count outside the app.
pub const OCR_THREADS_ENV: &str = "STAT_TRACKER_OCR_THREADS";

/// One or two columns for `content` px of width.
pub fn columns_for(content: f32) -> usize {
    if content.is_finite() && content >= TWO_COLUMN_MIN {
        2
    } else {
        1
    }
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

    /// One line under the row: the current values, so a closed row says
    /// how it is set without opening it.
    pub fn summary(self, v: &AdvancedValues) -> String {
        match self {
            AdvancedSection::CaptureReading => format!(
                "{} capture on {}, {}, report pictures off",
                v.route,
                v.monitor,
                lower_first(v.reader.label())
            ),
            AdvancedSection::TrackerService => {
                let login = if v.autostart {
                    "Starts on login"
                } else {
                    "Does not start on login"
                };
                let watch = if v.auto_detect {
                    "watches for match start and end"
                } else {
                    "match watching off"
                };
                format!(
                    "{login}, {watch}, checks every {}, waits {} s after a match, closes a finished game after {} s",
                    seconds_line(v.poll_ms),
                    v.cooldown_secs,
                    v.finished_close_secs
                )
            }
            AdvancedSection::StoredData => format!(
                "{} on this computer. Compact, Delete, older undecided games",
                games_line(v.games)
            ),
            AdvancedSection::WebsiteConnection => {
                let state = if v.sync_on { "on" } else { "off" };
                format!("Upload {state}. Manual server and token, has its own Apply")
            }
            AdvancedSection::Troubleshooting => format!(
                "Debug images {}, KDE compare frames {}, reader comparisons in the log {}, Report pictures limit {} games or {} MB, report a problem, uninstall",
                on_off(v.debug_images),
                on_off(v.compare_frames),
                on_off(v.reader_log),
                v.report_max_games,
                v.report_max_mb
            ),
        }
    }

    /// Rows whose changes reach the tracker only after it restarts.
    pub fn after_restart(self) -> bool {
        matches!(self, AdvancedSection::CaptureReading)
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

/// Values the closed Advanced rows summarise. Built from the form and the
/// app, so a change shows in the summary before Save.
#[derive(Debug, Clone, PartialEq)]
pub struct AdvancedValues {
    pub route: &'static str,
    pub monitor: String,
    pub reader: ReaderChoice,
    pub autostart: bool,
    pub auto_detect: bool,
    pub poll_ms: u64,
    pub cooldown_secs: u64,
    pub finished_close_secs: u64,
    pub games: usize,
    pub sync_on: bool,
    pub debug_images: bool,
    pub compare_frames: bool,
    pub reader_log: bool,
    pub report_max_games: usize,
    pub report_max_mb: u64,
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

/// `1.5 s`, `4 s`.
pub fn seconds_line(ms: u64) -> String {
    if ms.is_multiple_of(1000) {
        format!("{} s", ms / 1000)
    } else {
        format!("{:.1} s", ms as f64 / 1000.0)
    }
}

fn games_line(n: usize) -> String {
    if n == 1 {
        "1 game".to_string()
    } else {
        format!("{n} games")
    }
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

    pub fn any_open(self) -> bool {
        self.0 != 0
    }
}

/// The three reader setups the Reading radio offers. They map onto the two
/// existing switches: `reader = "new"` and `ocr = "on"`.
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

    /// What the form's two switches show. OCR off always means the new
    /// reader, so new-reader-off with OCR off is shown as new reader only.
    pub fn from_switches(new_reader: bool, classic_ocr: bool) -> Self {
        match (new_reader, classic_ocr) {
            (_, false) => ReaderChoice::NewOnly,
            (true, true) => ReaderChoice::NewWithOcrBackup,
            (false, true) => ReaderChoice::ClassicOnly,
        }
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
        if secs >= 3600 && secs.is_multiple_of(3600) {
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
/// one, in order.
pub fn session_window_options(current: Option<u64>) -> Vec<SessionWindow> {
    let mut all: Vec<u64> = SESSION_WINDOW_PRESETS.to_vec();
    if let Some(c) = current
        && c > 0
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

/// Chips from the form's comma text.
pub fn chips_from(raw: &str) -> Vec<String> {
    crate::settings::parse_process_names(raw)
}

/// The form text after removing chip `index`.
pub fn without_chip(raw: &str, index: usize) -> String {
    let mut chips = chips_from(raw);
    if index < chips.len() {
        chips.remove(index);
    }
    chips.join(", ")
}

/// The form text after adding `chip`. Blank and repeated chips are ignored.
pub fn with_chip(raw: &str, chip: &str) -> String {
    let mut chips = chips_from(raw);
    let chip = chip.trim().trim_matches(',').trim();
    if !chip.is_empty() && !chips.iter().any(|c| c.eq_ignore_ascii_case(chip)) {
        chips.push(chip.to_string());
    }
    chips.join(", ")
}

/// Config keys that never wait for a tracker restart: the GUI reads them
/// itself, or they only mark setup as done.
const NOT_FOR_TRACKER: [&str; 2] = ["update_channel", "setup_completed"];

/// How many settings differ between `running` (what the tracker started
/// with) and `now` (the form as it stands). Each changed value counts once,
/// so "1 change waiting" means one setting.
pub fn changes_waiting(
    running: &stat_tracker::config::Config,
    now: &stat_tracker::config::Config,
) -> usize {
    let (Ok(a), Ok(b)) = (serde_json::to_value(running), serde_json::to_value(now)) else {
        return usize::from(running != now);
    };
    match (a, b) {
        (serde_json::Value::Object(mut a), serde_json::Value::Object(mut b)) => {
            for key in NOT_FOR_TRACKER {
                a.remove(key);
                b.remove(key);
            }
            leaf_diffs(&serde_json::Value::Object(a), &serde_json::Value::Object(b))
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

/// "1 change waiting", "3 changes waiting".
pub fn changes_waiting_label(n: usize) -> String {
    if n == 1 {
        "1 change waiting".to_string()
    } else {
        format!("{n} changes waiting")
    }
}

/// "Stream frame rate" value line, and whether it differs from the default.
pub fn frame_rate_line(clamped: u32) -> (String, bool) {
    let changed = clamped != SCREENCAST_MAX_FPS_DEFAULT;
    let line = if changed {
        format!("{clamped} per second (default {SCREENCAST_MAX_FPS_DEFAULT})")
    } else {
        format!("{clamped} per second (default)")
    };
    (line, changed)
}

/// "Pause between grabs" value line, and whether it differs from the default.
pub fn pause_line(pause: PauseOverride) -> (&'static str, bool) {
    match pause {
        PauseOverride::Auto => ("Auto, where this desktop handles it (default)", false),
        PauseOverride::ConfigOff => ("Off", true),
        PauseOverride::EnvOn => ("On, set by STAT_TRACKER_SCREENCAST_PAUSE", true),
        PauseOverride::EnvOff => ("Off, set by STAT_TRACKER_SCREENCAST_PAUSE", true),
    }
}

/// "OCR workers" value line: count and memory.
pub fn ocr_workers_line(workers: usize) -> String {
    let mb = workers * OCR_WORKER_MB;
    if workers == 1 {
        format!("1 worker, about {mb} MB")
    } else {
        format!("{workers} workers, about {mb} MB")
    }
}

/// True when the env var fixes the worker count, so the row is locked.
pub fn ocr_workers_locked(env: Option<&str>) -> bool {
    env.is_some_and(|v| !v.trim().is_empty())
}

/// Copy for the header chip and the Games tab badge.
pub fn needs_look_label(count: usize) -> String {
    if count == 1 {
        "1 needs a look".to_string()
    } else {
        format!("{count} need a look")
    }
}

/// Settings the 0.6 spec shows that the tracker cannot store yet. Each is
/// drawn off and greyed, with this reason, until S11 adds the config key or
/// status message it needs.
pub const WAITS_FOR_S11: &str = "Waits for the tracker update (S11).";

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
    fn only_capture_and_reading_is_tagged_after_restart() {
        for section in AdvancedSection::ALL {
            assert_eq!(
                section.after_restart(),
                section == AdvancedSection::CaptureReading,
                "{section:?}"
            );
        }
    }

    #[test]
    fn advanced_rows_start_closed_and_fold_independently() {
        let mut open = AdvancedOpen::default();
        assert!(!open.any_open());
        for section in AdvancedSection::ALL {
            assert!(!open.is_open(section));
        }
        open.toggle(AdvancedSection::CaptureReading);
        open.toggle(AdvancedSection::StoredData);
        assert!(open.is_open(AdvancedSection::CaptureReading));
        assert!(open.is_open(AdvancedSection::StoredData));
        assert!(!open.is_open(AdvancedSection::TrackerService));
        open.toggle(AdvancedSection::CaptureReading);
        assert!(!open.is_open(AdvancedSection::CaptureReading));
        assert!(open.is_open(AdvancedSection::StoredData));
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
        assert!(!ReaderChoice::NewOnly.classic_ocr());
        assert!(ReaderChoice::NewWithOcrBackup.classic_ocr());
        assert!(!ReaderChoice::ClassicOnly.new_reader());
    }

    #[test]
    fn value_lines_mark_changes_from_default() {
        let (line, changed) = frame_rate_line(SCREENCAST_MAX_FPS_DEFAULT);
        assert!(!changed);
        assert!(line.contains("default"), "{line}");
        let (line, changed) = frame_rate_line(2);
        assert!(changed);
        assert!(line.starts_with("2 per second"), "{line}");
        assert!(!pause_line(PauseOverride::Auto).1);
        assert!(pause_line(PauseOverride::ConfigOff).1);
        assert!(
            pause_line(PauseOverride::EnvOn)
                .0
                .contains("STAT_TRACKER_SCREENCAST_PAUSE")
        );
    }

    #[test]
    fn ocr_workers_show_memory_and_the_env_lock() {
        assert_eq!(ocr_workers_line(1), "1 worker, about 23 MB");
        assert_eq!(ocr_workers_line(3), "3 workers, about 69 MB");
        assert!(!ocr_workers_locked(None));
        assert!(!ocr_workers_locked(Some("  ")));
        assert!(ocr_workers_locked(Some("2")));
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
            finished_close_secs: 180,
            games: 857,
            sync_on: true,
            debug_images: false,
            compare_frames: false,
            reader_log: false,
            report_max_games: 50,
            report_max_mb: 500,
        }
    }

    #[test]
    fn advanced_summaries_show_values() {
        let v = values();
        assert_eq!(
            AdvancedSection::CaptureReading.summary(&v),
            "Automatic capture on DP-3, new reader with OCR backup, report pictures off"
        );
        assert_eq!(
            AdvancedSection::TrackerService.summary(&v),
            "Starts on login, watches for match start and end, checks every 1.5 s, waits 120 s after a match, closes a finished game after 180 s"
        );
        assert!(
            AdvancedSection::StoredData
                .summary(&v)
                .starts_with("857 games on this computer")
        );
        assert!(
            AdvancedSection::Troubleshooting
                .summary(&v)
                .contains("Report pictures limit 50 games or 500 MB")
        );
        assert_eq!(seconds_line(4000), "4 s");
    }

    #[test]
    fn session_window_keeps_any_stored_value() {
        let opts = session_window_options(Some(1800));
        assert_eq!(opts.len(), SESSION_WINDOW_PRESETS.len());
        assert!(
            opts.iter()
                .any(|o| o.to_string() == "3 hours" && o.0 == 10_800)
        );
        assert_eq!(SessionWindow(1800).to_string(), "30 min");
        assert_eq!(SessionWindow(3600).to_string(), "1 hour");
        let odd = session_window_options(Some(2700));
        assert_eq!(odd.len(), SESSION_WINDOW_PRESETS.len() + 1);
        assert!(odd.contains(&SessionWindow(2700)));
        assert!(odd.windows(2).all(|w| w[0].0 < w[1].0));
        assert_eq!(SessionWindow(45).to_string(), "45 s");
    }

    #[test]
    fn chips_round_trip_the_comma_text() {
        assert_eq!(
            chips_from("Overwatch.exe, ow.exe"),
            ["Overwatch.exe", "ow.exe"]
        );
        assert_eq!(without_chip("Overwatch.exe, ow.exe", 0), "ow.exe");
        assert_eq!(without_chip("Overwatch.exe", 5), "Overwatch.exe");
        assert_eq!(
            with_chip("Overwatch.exe", " ow.exe "),
            "Overwatch.exe, ow.exe"
        );
        assert_eq!(with_chip("Overwatch.exe", "overwatch.EXE"), "Overwatch.exe");
        assert_eq!(with_chip("", "  "), "");
    }

    #[test]
    fn changes_waiting_counts_each_setting() {
        let base = stat_tracker::config::Config::default();
        assert_eq!(changes_waiting(&base, &base), 0);
        let mut now = base.clone();
        now.session_window_secs += 60;
        assert_eq!(changes_waiting(&base, &now), 1);
        now.auto_detect.cooldown_secs += 1;
        assert_eq!(changes_waiting(&base, &now), 2);
        let mut channel = base.clone();
        channel.update_channel = "alpha".into();
        assert_eq!(changes_waiting(&base, &channel), 0);
        assert_eq!(changes_waiting_label(1), "1 change waiting");
        assert_eq!(changes_waiting_label(3), "3 changes waiting");
    }

    #[test]
    fn needs_look_copy_counts() {
        assert_eq!(needs_look_label(1), "1 needs a look");
        assert_eq!(needs_look_label(159), "159 need a look");
    }

    #[test]
    fn copy_has_no_em_dashes() {
        let mut all: Vec<String> = Vec::new();
        all.extend(EverydaySection::ALL.iter().map(|s| s.title().to_string()));
        for s in AdvancedSection::ALL {
            all.push(s.title().into());
            all.push(s.summary(&values()));
        }
        all.extend(ReaderChoice::ALL.iter().map(|c| c.label().to_string()));
        all.push(WAITS_FOR_S11.into());
        for line in all {
            assert!(!line.contains('\u{2014}'), "{line}");
        }
    }
}
