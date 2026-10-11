//! Crash marker (report-bundle spec section 7).
//!
//! At startup the daemon writes `{data_dir}/run.marker` with its pid and
//! start time. A clean shutdown (SIGTERM, Ctrl+C, a normal return from
//! `main`, or the GUI's update restart, which stops the daemon with SIGTERM)
//! drops [`RunMarkerGuard`], which removes the file. A crash (SIGKILL, abort,
//! a panic that unwinds `main`, power loss) leaves it behind.
//!
//! The next start finds that stale marker. When its pid is not running, or is
//! running but is not a `scuffed-stat-tracker` process (pids are reused), the
//! marker is moved to `{data_dir}/last_crash.marker` (overwritten) with one
//! WARN line. A marker whose pid is a live tracker is left alone: another
//! daemon owns it.
//!
//! The marker holds only a pid and a timestamp. It is never uploaded. The app
//! uses it for `system.last_exit` and to set the crash report's log window.

use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, SecondsFormat, Utc};

pub const RUN_MARKER: &str = "run.marker";
pub const LAST_CRASH_MARKER: &str = "last_crash.marker";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunMarker {
    pub pid: u32,
    pub started_at: DateTime<Utc>,
}

impl RunMarker {
    pub fn format(&self) -> String {
        format!(
            "pid={}\nstarted_at={}\n",
            self.pid,
            self.started_at.to_rfc3339_opts(SecondsFormat::Secs, true)
        )
    }

    /// `None` unless both lines are present and valid.
    pub fn parse(text: &str) -> Option<Self> {
        let mut pid = None;
        let mut started_at = None;
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            match k.trim() {
                "pid" => pid = v.trim().parse::<u32>().ok(),
                "started_at" => {
                    started_at = DateTime::parse_from_rfc3339(v.trim())
                        .ok()
                        .map(|t| t.with_timezone(&Utc))
                }
                _ => {}
            }
        }
        Some(Self {
            pid: pid?,
            started_at: started_at?,
        })
    }
}

pub fn run_marker_path(data_dir: &Path) -> PathBuf {
    data_dir.join(RUN_MARKER)
}

pub fn last_crash_marker_path(data_dir: &Path) -> PathBuf {
    data_dir.join(LAST_CRASH_MARKER)
}

/// What startup found in `run.marker`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StaleMarker {
    /// No marker: the last run exited cleanly, or there was none.
    Absent,
    /// The marker names a live tracker process. Left untouched.
    Live { pid: u32 },
    /// The last run did not exit cleanly. The marker is now
    /// `last_crash.marker`. `marker` is `None` when its text was unreadable.
    Moved { marker: Option<RunMarker> },
}

/// Move a stale `run.marker` to `last_crash.marker`.
///
/// `is_live_tracker` decides whether the recorded pid is a running tracker.
/// The daemon passes [`crate::proc_id::pid_is_live_tracker`], which checks
/// `/proc/<pid>/exe` (then argv0, then `comm`) and never counts this
/// process. Tests pass their own check.
pub fn recover_stale_run_marker(
    data_dir: &Path,
    is_live_tracker: impl Fn(u32) -> bool,
) -> io::Result<StaleMarker> {
    let path = run_marker_path(data_dir);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(StaleMarker::Absent),
        Err(e) => return Err(e),
    };
    let marker = RunMarker::parse(&text);
    if let Some(m) = marker
        && is_live_tracker(m.pid)
    {
        return Ok(StaleMarker::Live { pid: m.pid });
    }
    std::fs::rename(&path, last_crash_marker_path(data_dir))?;
    match marker {
        Some(m) => tracing::warn!(
            pid = m.pid,
            started_at = %m.started_at.to_rfc3339_opts(SecondsFormat::Secs, true),
            "the last tracker run did not exit cleanly; its marker is now last_crash.marker"
        ),
        None => tracing::warn!(
            "the last tracker run did not exit cleanly (unreadable marker); it is now last_crash.marker"
        ),
    }
    Ok(StaleMarker::Moved { marker })
}

/// Removes `run.marker` on drop when the file still names this process and
/// the thread is not unwinding from a panic.
#[derive(Debug)]
pub struct RunMarkerGuard {
    path: PathBuf,
    pid: u32,
}

impl RunMarkerGuard {
    /// Remove the marker now. Same owner check as drop.
    pub fn finish(self) {
        drop(self);
    }

    fn remove_if_owner(&self) {
        let Ok(text) = std::fs::read_to_string(&self.path) else {
            return;
        };
        match RunMarker::parse(&text) {
            Some(m) if m.pid == self.pid => {
                if let Err(e) = std::fs::remove_file(&self.path)
                    && e.kind() != io::ErrorKind::NotFound
                {
                    tracing::warn!(error = %e, "could not remove run.marker");
                }
            }
            _ => tracing::warn!("run.marker no longer names this process; not removing it"),
        }
    }
}

impl Drop for RunMarkerGuard {
    fn drop(&mut self) {
        // A panic that reaches here is a crash, not a clean exit.
        if std::thread::panicking() {
            return;
        }
        self.remove_if_owner();
    }
}

/// Write `run.marker` for `pid`, owner-only.
pub fn write_run_marker(data_dir: &Path, marker: RunMarker) -> io::Result<RunMarkerGuard> {
    let path = run_marker_path(data_dir);
    let tmp = data_dir.join(format!("{RUN_MARKER}.tmp"));
    std::fs::write(&tmp, marker.format())?;
    crate::fs_mode::tighten_private_file(&tmp);
    std::fs::rename(&tmp, &path)?;
    Ok(RunMarkerGuard {
        path,
        pid: marker.pid,
    })
}

/// Daemon startup: recover a stale marker, then write this run's marker.
/// Call after the pid guard is held, so a second daemon never gets here.
pub fn start_run(data_dir: &Path) -> io::Result<(StaleMarker, RunMarkerGuard)> {
    let stale = recover_stale_run_marker(data_dir, crate::proc_id::pid_is_live_tracker)?;
    let guard = write_run_marker(
        data_dir,
        RunMarker {
            pid: std::process::id(),
            started_at: Utc::now(),
        },
    )?;
    Ok((stale, guard))
}

/// `last_crash.marker` as the app reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LastCrash {
    /// Pid and start time of the run that did not exit cleanly. `None` when
    /// the marker text was unreadable.
    pub marker: Option<RunMarker>,
    /// When the next daemon start found the stale marker (the file's
    /// modification time). This is the closest time the tracker has to the
    /// crash itself.
    pub detected_at: Option<DateTime<Utc>>,
}

impl LastCrash {
    /// Time the crash report's second log window ends at: when the stale
    /// marker was found, else the crashed run's start time.
    pub fn window_anchor(&self) -> Option<DateTime<Utc>> {
        self.detected_at.or(self.marker.map(|m| m.started_at))
    }
}

pub fn read_last_crash(data_dir: &Path) -> Option<LastCrash> {
    let path = last_crash_marker_path(data_dir);
    let text = std::fs::read_to_string(&path).ok()?;
    let detected_at = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .map(DateTime::<Utc>::from);
    Some(LastCrash {
        marker: RunMarker::parse(&text),
        detected_at,
    })
}

/// Remove `last_crash.marker` (the member answered No, or sent the report).
pub fn dismiss_last_crash(data_dir: &Path) -> io::Result<()> {
    match std::fs::remove_file(last_crash_marker_path(data_dir)) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// `system.last_exit` in the manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LastExit {
    Clean,
    Crashed,
    Unknown,
}

impl LastExit {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::Crashed => "crashed",
            Self::Unknown => "unknown",
        }
    }
}

/// `crashed` while `last_crash.marker` exists, or while `run.marker` names a
/// process that is not a live tracker (the daemon died and has not started
/// again). `unknown` when a marker cannot be read. Otherwise `clean`.
pub fn last_exit(data_dir: &Path) -> LastExit {
    last_exit_with(data_dir, crate::proc_id::pid_is_live_tracker)
}

pub fn last_exit_with(data_dir: &Path, is_live_tracker: impl Fn(u32) -> bool) -> LastExit {
    match std::fs::metadata(last_crash_marker_path(data_dir)) {
        Ok(_) => return LastExit::Crashed,
        Err(e) if e.kind() != io::ErrorKind::NotFound => return LastExit::Unknown,
        Err(_) => {}
    }
    match std::fs::read_to_string(run_marker_path(data_dir)) {
        Ok(text) => match RunMarker::parse(&text) {
            Some(m) if is_live_tracker(m.pid) => LastExit::Clean,
            Some(_) => LastExit::Crashed,
            None => LastExit::Unknown,
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => LastExit::Clean,
        Err(_) => LastExit::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Buf(Arc<Mutex<Vec<u8>>>);

    impl io::Write for Buf {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Buf {
        type Writer = Buf;
        fn make_writer(&'a self) -> Buf {
            self.clone()
        }
    }

    fn with_log<T>(f: impl FnOnce() -> T) -> (T, String) {
        let buf = Buf::default();
        let sub = tracing_subscriber::fmt()
            .with_writer(buf.clone())
            .with_ansi(false)
            .with_max_level(tracing::Level::TRACE)
            .finish();
        let out = tracing::subscriber::with_default(sub, f);
        let text = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
        (out, text)
    }

    fn marker(pid: u32) -> RunMarker {
        RunMarker {
            pid,
            started_at: DateTime::parse_from_rfc3339("2026-10-11T10:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        }
    }

    fn sleep_bin() -> PathBuf {
        ["/usr/bin/sleep", "/bin/sleep"]
            .iter()
            .map(PathBuf::from)
            .find(|p| p.exists())
            .expect("sleep binary")
    }

    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[test]
    fn marker_text_round_trips_and_holds_only_pid_and_time() {
        let m = marker(4242);
        assert_eq!(m.format(), "pid=4242\nstarted_at=2026-10-11T10:00:00Z\n");
        assert_eq!(RunMarker::parse(&m.format()), Some(m));
        assert_eq!(RunMarker::parse("pid=1\n"), None);
        assert_eq!(RunMarker::parse("garbage"), None);
    }

    #[test]
    fn clean_shutdown_removes_run_marker() {
        let dir = tempfile::tempdir().unwrap();
        let guard = write_run_marker(dir.path(), marker(std::process::id())).unwrap();
        assert!(run_marker_path(dir.path()).exists());
        guard.finish();
        assert!(!run_marker_path(dir.path()).exists());
        assert_eq!(last_exit(dir.path()), LastExit::Clean);
    }

    #[test]
    fn guard_leaves_a_marker_another_run_rewrote() {
        let dir = tempfile::tempdir().unwrap();
        let guard = write_run_marker(dir.path(), marker(std::process::id())).unwrap();
        std::fs::write(run_marker_path(dir.path()), marker(7).format()).unwrap();
        drop(guard);
        assert!(run_marker_path(dir.path()).exists());
    }

    #[test]
    fn stale_dead_pid_moves_to_last_crash_with_one_warn() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(run_marker_path(dir.path()), marker(u32::MAX).format()).unwrap();
        // A previous last_crash.marker is overwritten.
        std::fs::write(last_crash_marker_path(dir.path()), "old").unwrap();
        let (out, log) = with_log(|| {
            recover_stale_run_marker(dir.path(), crate::proc_id::pid_is_live_tracker).unwrap()
        });
        assert_eq!(
            out,
            StaleMarker::Moved {
                marker: Some(marker(u32::MAX))
            }
        );
        assert!(!run_marker_path(dir.path()).exists());
        let moved = std::fs::read_to_string(last_crash_marker_path(dir.path())).unwrap();
        assert_eq!(moved, marker(u32::MAX).format());
        assert_eq!(log.matches("WARN").count(), 1, "{log}");
        assert_eq!(log.lines().count(), 1, "{log}");
        assert_eq!(last_exit(dir.path()), LastExit::Crashed);
        let crash = read_last_crash(dir.path()).unwrap();
        assert_eq!(crash.marker, Some(marker(u32::MAX)));
        assert!(crash.window_anchor().is_some());
        dismiss_last_crash(dir.path()).unwrap();
        assert_eq!(last_exit(dir.path()), LastExit::Clean);
    }

    #[test]
    fn reused_pid_that_is_not_a_tracker_is_stale() {
        let dir = tempfile::tempdir().unwrap();
        let child = Child(
            std::process::Command::new(sleep_bin())
                .arg("30")
                .spawn()
                .unwrap(),
        );
        let pid = child.0.id();
        std::fs::write(run_marker_path(dir.path()), marker(pid).format()).unwrap();
        let (out, log) = with_log(|| {
            recover_stale_run_marker(dir.path(), crate::proc_id::pid_is_live_tracker).unwrap()
        });
        assert!(matches!(out, StaleMarker::Moved { .. }), "{out:?}");
        assert!(last_crash_marker_path(dir.path()).exists());
        assert_eq!(log.matches("WARN").count(), 1, "{log}");
    }

    #[test]
    fn live_tracker_pid_is_left_alone() {
        // A live process whose executable is named scuffed-stat-tracker.
        let dir = tempfile::tempdir().unwrap();
        let bin_dir = tempfile::tempdir().unwrap();
        let fake = bin_dir.path().join(crate::proc_id::DAEMON_BIN);
        std::fs::copy(sleep_bin(), &fake).unwrap();
        let child = Child(std::process::Command::new(&fake).arg("30").spawn().unwrap());
        let pid = child.0.id();
        assert!(crate::proc_id::pid_is_live_tracker(pid));
        let text = marker(pid).format();
        std::fs::write(run_marker_path(dir.path()), &text).unwrap();
        let (out, log) = with_log(|| {
            recover_stale_run_marker(dir.path(), crate::proc_id::pid_is_live_tracker).unwrap()
        });
        assert_eq!(out, StaleMarker::Live { pid });
        assert_eq!(
            std::fs::read_to_string(run_marker_path(dir.path())).unwrap(),
            text
        );
        assert!(!last_crash_marker_path(dir.path()).exists());
        assert!(!log.contains("WARN"), "{log}");
        assert_eq!(last_exit(dir.path()), LastExit::Clean);
    }

    #[test]
    fn unreadable_marker_is_stale_and_absent_marker_is_clean() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            recover_stale_run_marker(dir.path(), |_| true).unwrap(),
            StaleMarker::Absent
        );
        std::fs::write(run_marker_path(dir.path()), "not a marker").unwrap();
        assert_eq!(last_exit_with(dir.path(), |_| false), LastExit::Unknown);
        let (out, log) = with_log(|| recover_stale_run_marker(dir.path(), |_| true).unwrap());
        assert_eq!(out, StaleMarker::Moved { marker: None });
        assert_eq!(log.matches("WARN").count(), 1, "{log}");
    }

    #[test]
    fn dead_run_marker_without_restart_reads_as_crashed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(run_marker_path(dir.path()), marker(9).format()).unwrap();
        assert_eq!(last_exit_with(dir.path(), |_| false), LastExit::Crashed);
        assert_eq!(last_exit_with(dir.path(), |_| true), LastExit::Clean);
    }
}
