//! The tracker's focus gate in the top bar (UX Spec, focus status, variant
//! A): the copy, the GUI-only debounce, and the way from the unreadable
//! pill to the Settings row.
//!
//! The daemon writes `focus_gate` into its status file
//! ([`FocusGateStatus`]). The GUI reads it once a second, only from the
//! running tracker's file (the pid guard in [`observe`]), and shows it in
//! the top bar only while the header is Capturing
//! ([`crate::tracker_status::classify_header_status`]). A new not focused
//! shows after [`NOT_FOCUSED_AFTER`], a new unreadable after
//! [`UNREADABLE_AFTER`]; focused, off and unknown show at once. The
//! daemon's own pause is not debounced.

use std::time::{Duration, Instant};

use iced::advanced::widget::operation::{self, Operation, Outcome};
use iced::advanced::widget::{Id, operation::scrollable::AbsoluteOffset};
use iced::{Rectangle, Task, Vector};

use crate::app::{Message, TrackerApp};
use crate::settings_model::AdvancedSection;
use stat_tracker::ocr_status::FocusGateStatus;
use stat_tracker::ocr_switch::Status as OcrSwitchStatus;

/// Long words for the off state (the gate turned off in Settings).
pub const OFF_LONG: &str = "Capturing in any window";
/// Detail (tooltip) for the off state.
pub const OFF_DETAIL: &str = "Only capture while Overwatch is in front is off in Settings.";
/// Words for not focused, long and medium.
pub const PAUSED_LONG: &str = "Paused while Overwatch isn't in front";
/// Words for not focused under 1100px.
pub const PAUSED_SHORT: &str = "Paused";
/// Detail (tooltip) for not focused.
pub const PAUSED_DETAIL: &str = "Your game keeps going. Reading resumes when you switch back.";
/// Words in the unreadable pill, long and medium.
pub const UNREADABLE_LONG: &str = "Can't tell which window is in front";
/// The unreadable button under 1100px.
pub const UNREADABLE_SHORT: &str = "?";
/// Detail (tooltip) for unreadable, first line.
pub const UNREADABLE_DETAIL: &str = "Capturing all the time on this desktop.";
/// Detail (tooltip) for unreadable, second line: where the pill goes.
pub const UNREADABLE_ACTION: &str = "Select to change this in Settings.";

/// Settings row title for `capture_only_on_screen`.
pub const ROW_TITLE: &str = "Only capture while Overwatch is in front";
/// Settings row help.
pub const ROW_HELP: &str = "Pauses capture while another window is in front. Your game keeps going. Where this desktop can't tell, capture keeps running.";
/// Tooltip on the row's Auto and Off buttons in demo mode, where they do
/// nothing.
pub const DEMO_TOOLTIP: &str = "Settings can't be changed in demo mode.";
/// Live note under the row, only while the focus is unreadable.
pub const ROW_UNREADABLE_NOTE: &str = "Can't tell which window is in front on this desktop, so the tracker captures all the time. Choose Off and restart the tracker to stop this warning.";

/// A new not focused waits this long before it shows.
pub const NOT_FOCUSED_AFTER: Duration = Duration::from_secs(2);
/// A new unreadable waits this long before it shows: one slow read can
/// flip the value for a poll and back.
pub const UNREADABLE_AFTER: Duration = Duration::from_secs(5);
/// How long the Settings row stays marked after the pill opened it.
pub const HIGHLIGHT_FOR: Duration = Duration::from_secs(2);

/// The pane scrollable that scrolls the 0.6 Settings page.
pub const PANE_SCROLL_ID: &str = "page-pane-scroll";
/// The `capture_only_on_screen` row on the 0.6 Settings page.
pub const ROW_ID: &str = "settings-focus-gate-row";
/// Room left above the row when the pane scrolls to it.
const ROW_MARGIN: f32 = 24.0;

/// The focus value the top bar shows, debounced.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FocusDisplay {
    shown: FocusGateStatus,
    /// A value that waits for its delay, and since when it is read.
    pending: Option<(FocusGateStatus, Instant)>,
}

impl FocusDisplay {
    /// What the bar shows now.
    pub fn shown(&self) -> FocusGateStatus {
        self.shown
    }

    /// Feed the value read at `now`. `None` (no status, or a file from
    /// another tracker run) shows unknown at once.
    pub fn observe(&mut self, gate: Option<FocusGateStatus>, now: Instant) {
        let gate = gate.unwrap_or_default();
        let delay = match gate {
            FocusGateStatus::NotFocused => NOT_FOCUSED_AFTER,
            FocusGateStatus::Unreadable => UNREADABLE_AFTER,
            FocusGateStatus::Focused | FocusGateStatus::Off | FocusGateStatus::Unknown => {
                Duration::ZERO
            }
        };
        if gate == self.shown {
            self.pending = None;
            return;
        }
        let since = match self.pending {
            Some((value, since)) if value == gate => since,
            _ => now,
        };
        if now.saturating_duration_since(since) >= delay {
            self.shown = gate;
            self.pending = None;
        } else {
            self.pending = Some((gate, since));
        }
    }
}

/// The running tracker's focus value: `None` unless the status file is
/// from the tracker that runs now (`daemon_pid`).
pub fn live_focus(
    status: Option<&OcrSwitchStatus>,
    daemon_pid: Option<u32>,
) -> Option<FocusGateStatus> {
    status
        .filter(|status| daemon_pid == Some(status.pid))
        .map(|status| status.focus_gate)
}

/// The running tracker's keyboard sentence (no keyboard device could be
/// opened). Empty, or a status file from another run, is nothing to show.
pub(crate) fn daemon_input_notice(
    status: Option<&OcrSwitchStatus>,
    daemon_pid: Option<u32>,
) -> Option<String> {
    let status = status.filter(|status| daemon_pid == Some(status.pid))?;
    let notice = status.input_notice.trim();
    (!notice.is_empty()).then(|| notice.to_string())
}

impl TrackerApp {
    /// The tick's read of the status file for the capture route and
    /// keyboard notices and the top bar's focus value.
    pub(crate) fn observe_capture_and_focus(
        &mut self,
        status: Option<&OcrSwitchStatus>,
        now: Instant,
    ) {
        let pid = self.daemon.pid;
        self.ocr_switch.capture_notice = crate::settings::daemon_capture_notice(status, pid);
        self.ocr_switch.input_notice = daemon_input_notice(status, pid);
        self.page_state.focus.observe(live_focus(status, pid), now);
    }
}

/// Scroll the Settings pane so the focus row is in view. Runs after the
/// page is shown; nothing happens when the row is not there.
pub fn scroll_to_row() -> Task<Message> {
    scroll_to(ROW_ID)
}

/// The layout id of an Advanced row on the Settings page, for scrolling
/// to it.
pub fn section_row_id(section: AdvancedSection) -> &'static str {
    match section {
        AdvancedSection::CaptureReading => "settings-advanced-capture-reading",
        AdvancedSection::TrackerService => "settings-advanced-tracker-service",
        AdvancedSection::StoredData => "settings-advanced-stored-data",
        AdvancedSection::WebsiteConnection => "settings-advanced-website-connection",
        AdvancedSection::Troubleshooting => "settings-advanced-troubleshooting",
    }
}

/// Scroll the Settings pane so the container with `row_id` is in view.
/// Nothing happens when it is not there.
pub fn scroll_to(row_id: &'static str) -> Task<Message> {
    iced::advanced::widget::operate(RevealRow {
        row_id,
        pane_top: None,
        row_top: None,
    })
}

/// Finds the pane and the row, then scrolls the pane to the row.
#[derive(Debug)]
struct RevealRow {
    row_id: &'static str,
    pane_top: Option<f32>,
    row_top: Option<f32>,
}

impl<T: Send + 'static> Operation<T> for RevealRow {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<T>)) {
        operate(self);
    }

    fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
        if id == Some(&Id::new(self.row_id)) {
            self.row_top = Some(bounds.y);
        }
    }

    fn scrollable(
        &mut self,
        id: Option<&Id>,
        _bounds: Rectangle,
        content_bounds: Rectangle,
        _translation: Vector,
        _state: &mut dyn operation::Scrollable,
    ) {
        if id == Some(&Id::new(PANE_SCROLL_ID)) {
            self.pane_top = Some(content_bounds.y);
        }
    }

    fn finish(&self) -> Outcome<T> {
        match (self.pane_top, self.row_top) {
            (Some(pane), Some(row)) => Outcome::Chain(Box::new(operation::scrollable::scroll_to(
                Id::new(PANE_SCROLL_ID),
                AbsoluteOffset {
                    x: None,
                    y: Some(row_offset(pane, row)),
                },
            ))),
            _ => Outcome::None,
        }
    }
}

/// The pane offset that puts a row at `row` (layout y) a little under the
/// top of a pane whose content starts at `pane`.
pub fn row_offset(pane: f32, row: f32) -> f32 {
    (row - pane - ROW_MARGIN).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(t0: Instant, ms: u64) -> Instant {
        t0 + Duration::from_millis(ms)
    }

    #[test]
    fn every_advanced_row_has_its_own_layout_id() {
        let ids: std::collections::HashSet<_> = AdvancedSection::ALL
            .into_iter()
            .map(section_row_id)
            .collect();
        assert_eq!(ids.len(), AdvancedSection::ALL.len());
        assert!(!ids.contains(ROW_ID));
    }

    #[test]
    fn focused_off_and_unknown_show_at_once() {
        let t0 = Instant::now();
        let mut d = FocusDisplay::default();
        for gate in [
            FocusGateStatus::Off,
            FocusGateStatus::Focused,
            FocusGateStatus::Unknown,
        ] {
            d.observe(Some(gate), t0);
            assert_eq!(d.shown(), gate);
        }
    }

    #[test]
    fn not_focused_waits_two_seconds_and_unreadable_five() {
        let t0 = Instant::now();
        let mut d = FocusDisplay::default();
        d.observe(Some(FocusGateStatus::Focused), t0);
        d.observe(Some(FocusGateStatus::NotFocused), at(t0, 1_000));
        assert_eq!(d.shown(), FocusGateStatus::Focused, "one tick in");
        d.observe(Some(FocusGateStatus::NotFocused), at(t0, 2_000));
        assert_eq!(d.shown(), FocusGateStatus::Focused, "1 s held");
        d.observe(Some(FocusGateStatus::NotFocused), at(t0, 3_000));
        assert_eq!(d.shown(), FocusGateStatus::NotFocused, "2 s held");
        // Back to focused at once.
        d.observe(Some(FocusGateStatus::Focused), at(t0, 3_500));
        assert_eq!(d.shown(), FocusGateStatus::Focused);
        // Unreadable needs 5 s.
        let t1 = at(t0, 10_000);
        d.observe(Some(FocusGateStatus::Unreadable), t1);
        d.observe(Some(FocusGateStatus::Unreadable), at(t1, 4_999));
        assert_eq!(d.shown(), FocusGateStatus::Focused);
        d.observe(Some(FocusGateStatus::Unreadable), at(t1, 5_000));
        assert_eq!(d.shown(), FocusGateStatus::Unreadable);
        // Off shows at once, from any state.
        d.observe(Some(FocusGateStatus::Off), at(t1, 5_100));
        assert_eq!(d.shown(), FocusGateStatus::Off);
    }

    #[test]
    fn a_short_flip_never_shows() {
        let t0 = Instant::now();
        let mut d = FocusDisplay::default();
        d.observe(Some(FocusGateStatus::Focused), t0);
        d.observe(Some(FocusGateStatus::NotFocused), at(t0, 1_000));
        d.observe(Some(FocusGateStatus::Focused), at(t0, 2_000));
        d.observe(Some(FocusGateStatus::NotFocused), at(t0, 3_000));
        assert_eq!(d.shown(), FocusGateStatus::Focused, "the wait starts over");
        // A switch between the two waiting values starts its own wait.
        d.observe(Some(FocusGateStatus::Unreadable), at(t0, 4_500));
        d.observe(Some(FocusGateStatus::Unreadable), at(t0, 8_000));
        assert_eq!(d.shown(), FocusGateStatus::Focused);
    }

    #[test]
    fn only_the_running_trackers_file_counts() {
        let status = OcrSwitchStatus {
            pid: 41,
            focus_gate: FocusGateStatus::Unreadable,
            ..OcrSwitchStatus::default()
        };
        assert_eq!(
            live_focus(Some(&status), Some(41)),
            Some(FocusGateStatus::Unreadable)
        );
        assert_eq!(live_focus(Some(&status), Some(42)), None, "another run");
        assert_eq!(live_focus(Some(&status), None), None, "no tracker");
        assert_eq!(live_focus(None, Some(41)), None);
        // A stale file shows unknown at once, never a held pause.
        let t0 = Instant::now();
        let mut d = FocusDisplay::default();
        d.observe(Some(FocusGateStatus::Off), t0);
        d.observe(live_focus(Some(&status), Some(42)), at(t0, 1_000));
        assert_eq!(d.shown(), FocusGateStatus::Unknown);
    }

    #[test]
    fn the_row_lands_a_little_under_the_pane_top() {
        assert_eq!(row_offset(100.0, 900.0), 900.0 - 100.0 - ROW_MARGIN);
        assert_eq!(row_offset(100.0, 110.0), 0.0, "never above the top");
    }

    #[test]
    fn the_copy_has_no_long_dashes() {
        for text in [
            OFF_LONG,
            OFF_DETAIL,
            PAUSED_LONG,
            PAUSED_SHORT,
            PAUSED_DETAIL,
            UNREADABLE_LONG,
            UNREADABLE_SHORT,
            UNREADABLE_DETAIL,
            UNREADABLE_ACTION,
            ROW_TITLE,
            ROW_HELP,
            ROW_UNREADABLE_NOTE,
            DEMO_TOOLTIP,
        ] {
            assert!(
                !text.contains('\u{2014}') && !text.contains('\u{2013}'),
                "{text}"
            );
            assert!(!text.contains('\u{2019}'), "straight apostrophes: {text}");
        }
        assert!(PAUSED_SHORT.len() <= 7 && UNREADABLE_SHORT.len() <= 7);
    }

    #[test]
    fn the_keyboard_notice_is_shown_for_the_running_tracker() {
        let mut status = OcrSwitchStatus {
            pid: 41,
            input_notice: "Tab detection is off. Run: sudo usermod -aG input \"$USER\"".into(),
            ..OcrSwitchStatus::default()
        };
        assert_eq!(
            daemon_input_notice(Some(&status), Some(41)).as_deref(),
            Some(status.input_notice.as_str())
        );
        assert_eq!(daemon_input_notice(Some(&status), Some(42)), None);
        assert_eq!(daemon_input_notice(Some(&status), None), None);
        status.input_notice = "  ".into();
        assert_eq!(daemon_input_notice(Some(&status), Some(41)), None);
    }
}
