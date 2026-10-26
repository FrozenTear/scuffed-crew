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
    WebsiteAccount,
    Sharing,
    Updates,
}

impl EverydaySection {
    pub const ALL: [EverydaySection; 5] = [
        EverydaySection::PlayerSessions,
        EverydaySection::Companion,
        EverydaySection::WebsiteAccount,
        EverydaySection::Sharing,
        EverydaySection::Updates,
    ];

    pub fn title(self) -> &'static str {
        match self {
            EverydaySection::PlayerSessions => "Player and sessions",
            EverydaySection::Companion => "Companion",
            EverydaySection::WebsiteAccount => "Website account",
            EverydaySection::Sharing => "Sharing",
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

    /// One line under the closed row, so the row says what is inside.
    pub fn summary(self) -> &'static str {
        match self {
            AdvancedSection::CaptureReading => "Capture method, monitor, reader, OCR",
            AdvancedSection::TrackerService => "Start on login, match watching, waits",
            AdvancedSection::StoredData => "Compact or delete local games",
            AdvancedSection::WebsiteConnection => "Manual server and token",
            AdvancedSection::Troubleshooting => {
                "Debug images, reader log, report a problem, uninstall"
            }
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
                "Sharing",
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
            all.push(s.summary().into());
        }
        all.extend(ReaderChoice::ALL.iter().map(|c| c.label().to_string()));
        all.push(WAITS_FOR_S11.into());
        for line in all {
            assert!(!line.contains('\u{2014}'), "{line}");
        }
    }
}
