//! Daemon process identity for the pid file.
//!
//! `/proc/<pid>/comm` is truncated to 15 bytes, so a prefix check
//! (`scuffed-stat…`) also matches a reused PID whose name merely starts
//! the same way (`scuffed-station`, `scuffed-stat-tr` from a different
//! binary). The check prefers `/proc/<pid>/exe` (basename exactly
//! `scuffed-stat-tracker`, including a kernel ` (deleted)` suffix after
//! an in-place replace) and otherwise the first cmdline argument. `comm`
//! is only the fallback when both of those are unreadable, and then it
//! must be the full truncated name, not a prefix.
//!
//! The pid file's second line is the process start time (field 22 of
//! `/proc/<pid>/stat`, clock ticks since boot). A recycled PID fails that
//! check even when the new process has the same binary name.

use std::path::{Path, PathBuf};

pub const DAEMON_BIN: &str = "scuffed-stat-tracker";
/// `comm` is at most `TASK_COMM_LEN - 1` bytes. This is the truncated
/// form of [`DAEMON_BIN`], not a prefix to search for.
pub const DAEMON_COMM: &str = "scuffed-stat-tr";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PidRecord {
    pub pid: u32,
    pub start_ticks: Option<u64>,
}

pub fn parse_pid_record(text: &str) -> Option<PidRecord> {
    let mut lines = text.lines();
    let pid = lines.next()?.trim().parse().ok()?;
    let start_ticks = lines.next().and_then(|line| line.trim().parse().ok());
    Some(PidRecord { pid, start_ticks })
}

pub fn format_pid_record(pid: u32, start_ticks: Option<u64>) -> String {
    match start_ticks {
        Some(ticks) => format!("{pid}\n{ticks}\n"),
        None => format!("{pid}\n"),
    }
}

/// Basename is exactly the daemon binary. A trailing ` (deleted)` is the
/// kernel's mark on an unlinked inode and is stripped first.
pub fn exe_names_daemon(exe: &std::ffi::OsStr) -> bool {
    let lossy = exe.to_string_lossy();
    let trimmed = lossy.strip_suffix(" (deleted)").unwrap_or(lossy.as_ref());
    Path::new(trimmed)
        .file_name()
        .is_some_and(|name| name == DAEMON_BIN)
}

/// Exact `comm` match: the 15-byte kernel truncation, or the full binary
/// name when the source is not `comm`. Not a prefix.
pub fn comm_names_daemon(comm: &str) -> bool {
    let comm = comm.trim();
    comm == DAEMON_COMM || comm == DAEMON_BIN
}

/// Image matched, and the start time matches when the pid file recorded one.
pub fn accept_daemon_process(
    image_ok: bool,
    expected_start: Option<u64>,
    actual_start: Option<u64>,
) -> bool {
    if !image_ok {
        return false;
    }
    match expected_start {
        None => true,
        Some(expected) => actual_start == Some(expected),
    }
}

pub fn proc_start_ticks(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    start_ticks_from_stat(&stat)
}

pub fn start_ticks_from_stat(stat: &str) -> Option<u64> {
    // `comm` is wrapped in parentheses and may itself contain spaces or
    // parens. The last `)` closes it; field 22 (starttime) is the 20th
    // whitespace-separated field after that.
    let rest = stat.rsplit_once(')')?.1;
    rest.split_whitespace().nth(19)?.parse().ok()
}

/// Alive daemon, ignoring start time (legacy pid files that stored only a pid).
pub fn pid_is_live_tracker(pid: u32) -> bool {
    pid_is_live_tracker_started(pid, None)
}

/// Alive daemon whose `/proc` start time equals `expected_start` when that
/// is `Some`. This process is never treated as the daemon.
pub fn pid_is_live_tracker_started(pid: u32, expected_start: Option<u64>) -> bool {
    if pid == std::process::id() {
        return false;
    }
    let image_ok = image_matches(pid);
    let actual_start = proc_start_ticks(pid);
    accept_daemon_process(image_ok, expected_start, actual_start)
}

fn image_matches(pid: u32) -> bool {
    match std::fs::read_link(format!("/proc/{pid}/exe")) {
        Ok(exe) => return exe_names_daemon(exe.as_os_str()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return false,
        Err(_) => {}
    }
    if let Some(argv0) = cmdline_argv0(pid) {
        return exe_names_daemon(std::ffi::OsStr::new(&argv0));
    }
    match std::fs::read_to_string(format!("/proc/{pid}/comm")) {
        Ok(comm) => comm_names_daemon(&comm),
        Err(_) => false,
    }
}

fn cmdline_argv0(pid: u32) -> Option<String> {
    let bytes = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let argv0 = bytes.split(|b| *b == 0).next()?;
    if argv0.is_empty() {
        return None;
    }
    Some(String::from_utf8_lossy(argv0).into_owned())
}

/// Read `/proc/<pid>/exe` for tests and callers that want the path itself.
pub fn proc_exe(pid: u32) -> Option<PathBuf> {
    std::fs::read_link(format!("/proc/{pid}/exe")).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exe_match_is_the_binary_name_not_a_prefix() {
        assert!(exe_names_daemon(std::ffi::OsStr::new(
            "/home/user/.local/bin/scuffed-stat-tracker"
        )));
        assert!(exe_names_daemon(std::ffi::OsStr::new(
            "/home/user/.local/bin/scuffed-stat-tracker (deleted)"
        )));
        assert!(!exe_names_daemon(std::ffi::OsStr::new(
            "/home/user/.local/bin/scuffed-stat-tracker-helper"
        )));
        assert!(!exe_names_daemon(std::ffi::OsStr::new(
            "/usr/bin/scuffed-station"
        )));
        assert!(!exe_names_daemon(std::ffi::OsStr::new(
            "/usr/bin/scuffed-stat-tr"
        )));
        assert!(!exe_names_daemon(std::ffi::OsStr::new("stat-tracker-gui")));
    }

    #[test]
    fn comm_match_is_exact_not_a_prefix() {
        assert!(comm_names_daemon("scuffed-stat-tr"));
        assert!(comm_names_daemon("scuffed-stat-tr\n"));
        assert!(comm_names_daemon("scuffed-stat-tracker"));
        assert!(!comm_names_daemon("scuffed-station"));
        assert!(!comm_names_daemon("scuffed-stat"));
        assert!(!comm_names_daemon("scuffed-stat-tracker-helper"));
        assert!(!comm_names_daemon("stat-tracker-gu"));
        assert!(!comm_names_daemon(""));
    }

    #[test]
    fn start_time_rejects_a_reused_pid_of_the_same_binary() {
        assert!(!accept_daemon_process(false, None, None));
        assert!(accept_daemon_process(true, None, None));
        assert!(accept_daemon_process(true, Some(10), Some(10)));
        assert!(
            !accept_daemon_process(true, Some(10), Some(11)),
            "same binary name, different start time, is a reused PID"
        );
        assert!(!accept_daemon_process(true, Some(10), None));
    }

    #[test]
    fn stat_parser_reads_starttime_after_a_spaced_comm() {
        let stat = "42 (scuffed stat) S 1 1 1 0 -1 4194304 0 0 0 0 0 0 0 0 20 0 1 0 999888 1 2";
        assert_eq!(start_ticks_from_stat(stat), Some(999888));
        let live = std::fs::read_to_string("/proc/self/stat").unwrap();
        assert!(start_ticks_from_stat(&live).is_some());
        assert_eq!(
            proc_start_ticks(std::process::id()),
            start_ticks_from_stat(&live)
        );
    }

    #[test]
    fn pid_record_keeps_a_legacy_single_line_and_a_start_time() {
        assert_eq!(
            parse_pid_record("4242\n"),
            Some(PidRecord {
                pid: 4242,
                start_ticks: None,
            })
        );
        assert_eq!(
            parse_pid_record("4242\n999888\n"),
            Some(PidRecord {
                pid: 4242,
                start_ticks: Some(999888),
            })
        );
        assert_eq!(format_pid_record(7, Some(8)), "7\n8\n");
        assert!(parse_pid_record("nope").is_none());
    }

    #[test]
    fn self_and_dead_and_foreign_pids_are_not_the_daemon() {
        assert!(!pid_is_live_tracker(std::process::id()));
        assert!(!pid_is_live_tracker(u32::MAX));
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("sleep");
        let pid = child.id();
        assert!(
            !pid_is_live_tracker(pid),
            "a live process with a different exe is not the daemon"
        );
        let start = proc_start_ticks(pid);
        assert!(!pid_is_live_tracker_started(pid, start));
        let _ = child.kill();
        let _ = child.wait();
    }
}
