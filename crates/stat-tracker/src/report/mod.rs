//! Issue report bundles (`crates/stat-tracker/docs/report-bundle.md`,
//! revision 2).
//!
//! This module covers log-only reports (`crash`, `performance`, `other`):
//! the crash marker ([`marker`]), the log scrub ([`scrub`]), and the zip
//! with its manifest ([`bundle`]). Game reports with blanked crops build on
//! the same pieces later.
//!
//! The desktop app calls [`bundle::LogReportInput::from_config`] and
//! [`bundle::build_log_report`] for the preview, then uploads or saves the
//! returned bytes unchanged.

pub mod bundle;
pub mod marker;
pub mod scrub;

#[cfg(test)]
mod tests;

pub use bundle::{
    LogOnlyCategory, LogReportInput, ReportError, ReportFile, ReportZip, Resolution, SessionType,
    SystemInfo, build_log_report,
};
pub use marker::{LastCrash, LastExit};
pub use scrub::{LogWindow, ScrubContext};

/// True when `daemon.pid` names a live tracker (`system.daemon_running`).
/// Read-only: a stale pid file is left for the GUI's own check to remove.
pub fn daemon_running(data_dir: &std::path::Path) -> bool {
    std::fs::read_to_string(data_dir.join("daemon.pid"))
        .ok()
        .and_then(|t| crate::proc_id::parse_pid_record(&t))
        .is_some_and(|r| crate::proc_id::pid_is_live_tracker_started(r.pid, r.start_ticks))
}
