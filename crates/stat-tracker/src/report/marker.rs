//! Crash marker (report-bundle spec section 7).
//!
//! At startup the daemon writes `{data_dir}/run.marker` with its pid, its
//! start time, and `last_seen`, the last time it was known to be alive. A
//! [`MarkerHeartbeat`] rewrites `last_seen` once a minute by writing a temp
//! file and renaming it over the marker, so a crash mid-write leaves the
//! previous copy. A clean shutdown (SIGTERM, Ctrl+C, a normal return from
//! `main`, or the GUI's update restart, which stops the daemon with SIGTERM)
//! drops [`RunMarkerGuard`], which removes the file. A crash (SIGKILL, abort,
//! a panic that unwinds `main`, power loss) leaves it.
//!
//! The next start finds that stale marker. When its pid is not running, or is
//! running but is not a `scuffed-stat-tracker` process (pids are reused), the
//! marker is moved to `{data_dir}/last_crash.marker` (overwritten), keeping
//! the old `last_seen`, with one WARN line. A marker whose pid is a live
//! tracker is left alone: another daemon owns it.
//!
//! The marker holds only the pid, the start time, and `last_seen`. It is
//! never uploaded. The app uses it for `system.last_exit`, and `last_seen`
//! sets the crash report's log window.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, SecondsFormat, Utc};

/// Cleared while a self-restart has removed `run.marker`. The heartbeat
/// must not write the file back. Restored if the exec fails.
static MARKER_HELD: AtomicBool = AtomicBool::new(true);

/// The heartbeat's lock. `release_run_marker` takes it before clearing
/// [`MARKER_HELD`], so a touch that already passed that check cannot
/// recreate the file afterwards.
static HEARTBEAT: Mutex<Option<Arc<Mutex<Shared>>>> = Mutex::new(None);

pub fn marker_is_held() -> bool {
    MARKER_HELD.load(Ordering::Acquire)
}

/// Delete `run.marker` and stop the heartbeat from recreating it.
/// Returns the text that was there, so a failed exec can put it back.
pub fn release_run_marker(data_dir: &Path) -> Option<RunMarker> {
    let shared = HEARTBEAT
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone();
    // Hold the heartbeat lock across the flag and the delete. `touch`
    // checks the flag under this same lock.
    let _guard = shared
        .as_ref()
        .map(|shared| shared.lock().unwrap_or_else(|err| err.into_inner()));
    MARKER_HELD.store(false, Ordering::Release);
    let path = run_marker_path(data_dir);
    let parsed = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| RunMarker::parse(&text));
    let _ = std::fs::remove_file(&path);
    parsed
}

/// Put `run.marker` back after a failed re-exec and let the heartbeat
/// write it again.
pub fn restore_run_marker(data_dir: &Path, marker: Option<&RunMarker>) -> io::Result<()> {
    MARKER_HELD.store(true, Ordering::Release);
    let path = run_marker_path(data_dir);
    if path.exists() {
        return Ok(());
    }
    let marker = marker.cloned().unwrap_or(RunMarker {
        pid: std::process::id(),
        started_at: Utc::now(),
        last_seen: Utc::now(),
    });
    write_atomic(&path, &marker)
}

pub const RUN_MARKER: &str = "run.marker";
pub const LAST_CRASH_MARKER: &str = "last_crash.marker";

/// How often the daemon rewrites `last_seen`.
pub const HEARTBEAT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunMarker {
    pub pid: u32,
    pub started_at: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
}

fn rfc3339(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

impl RunMarker {
    pub fn format(&self) -> String {
        format!(
            "pid={}\nstarted_at={}\nlast_seen={}\n",
            self.pid,
            rfc3339(self.started_at),
            rfc3339(self.last_seen)
        )
    }

    /// `None` unless all three lines are present and valid.
    pub fn parse(text: &str) -> Option<Self> {
        let mut pid = None;
        let mut started_at = None;
        let mut last_seen = None;
        let time = |v: &str| {
            DateTime::parse_from_rfc3339(v.trim())
                .ok()
                .map(|t| t.with_timezone(&Utc))
        };
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            match k.trim() {
                "pid" => pid = v.trim().parse::<u32>().ok(),
                "started_at" => started_at = time(v),
                "last_seen" => last_seen = time(v),
                _ => {}
            }
        }
        Some(Self {
            pid: pid?,
            started_at: started_at?,
            last_seen: last_seen?,
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
            last_seen = %rfc3339(m.last_seen),
            "the last tracker run did not exit cleanly; its marker is now last_crash.marker"
        ),
        None => tracing::warn!(
            "the last tracker run did not exit cleanly (unreadable marker); it is now last_crash.marker"
        ),
    }
    Ok(StaleMarker::Moved { marker })
}

/// State shared by the guard and the heartbeat. The lock orders a final
/// heartbeat write against the guard's removal, so a write can never bring
/// the file back after a clean shutdown removed it.
#[derive(Debug)]
struct Shared {
    path: PathBuf,
    marker: RunMarker,
    stopped: bool,
}

/// Write `marker` to `path` through a temp file and a rename.
fn write_atomic(path: &Path, marker: &RunMarker) -> io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, marker.format())?;
    crate::fs_mode::tighten_private_file(&tmp);
    std::fs::rename(&tmp, path)
}

/// Removes `run.marker` on drop when the file still names this process and
/// the thread is not unwinding from a panic.
#[derive(Debug)]
pub struct RunMarkerGuard {
    shared: Arc<Mutex<Shared>>,
}

/// What one heartbeat tick does. A released marker is skipped, not an exit:
/// a failed re-exec restores the marker and the loop must still be running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeartbeatStep {
    Stop,
    Skip,
    Touch,
}

fn heartbeat_step(stopped: bool, held: bool) -> HeartbeatStep {
    if stopped {
        HeartbeatStep::Stop
    } else if !held {
        HeartbeatStep::Skip
    } else {
        HeartbeatStep::Touch
    }
}

/// Rewrites `last_seen` in this run's marker. Cheap to clone.
#[derive(Debug, Clone)]
pub struct MarkerHeartbeat {
    shared: Arc<Mutex<Shared>>,
}

impl MarkerHeartbeat {
    /// Set `last_seen` to `now`. Does nothing after the guard removed the
    /// marker, or when the file no longer names this process.
    pub fn touch(&self, now: DateTime<Utc>) -> io::Result<()> {
        let mut s = self.shared.lock().unwrap_or_else(|p| p.into_inner());
        if !marker_is_held() || s.stopped || !file_names(&s.path, s.marker.pid) {
            return Ok(());
        }
        let next = RunMarker {
            last_seen: now,
            ..s.marker
        };
        write_atomic(&s.path, &next)?;
        s.marker = next;
        Ok(())
    }

    /// Touch every `interval` until the guard is dropped. The daemon runs
    /// this with [`HEARTBEAT_INTERVAL`].
    pub async fn run(self, interval: std::time::Duration) {
        let mut tick = tokio::time::interval(interval);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        tick.tick().await; // the first tick is immediate; the file is fresh
        loop {
            tick.tick().await;
            match heartbeat_step(
                self.shared.lock().map(|s| s.stopped).unwrap_or(true),
                marker_is_held(),
            ) {
                HeartbeatStep::Stop => return,
                HeartbeatStep::Skip => continue,
                HeartbeatStep::Touch => {
                    if let Err(e) = self.touch(Utc::now()) {
                        tracing::warn!(error = %e, "could not update run.marker");
                    }
                }
            }
        }
    }
}

fn file_names(path: &Path, pid: u32) -> bool {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| RunMarker::parse(&t))
        .is_some_and(|m| m.pid == pid)
}

impl RunMarkerGuard {
    pub fn heartbeat(&self) -> MarkerHeartbeat {
        MarkerHeartbeat {
            shared: Arc::clone(&self.shared),
        }
    }

    /// Clean shutdown: stop the heartbeat and remove the marker, but only
    /// when it still names this process. The daemon calls this only when its
    /// main loop returned `Ok` (SIGTERM, Ctrl+C, a normal exit).
    pub fn finish(self) {
        let mut s = self.shared.lock().unwrap_or_else(|p| p.into_inner());
        s.stopped = true;
        if !file_names(&s.path, s.marker.pid) {
            if s.path.exists() {
                tracing::warn!("run.marker no longer names this process; not removing it");
            }
            return;
        }
        if let Err(e) = std::fs::remove_file(&s.path)
            && e.kind() != io::ErrorKind::NotFound
        {
            tracing::warn!(error = %e, "could not remove run.marker");
        }
    }
}

impl Drop for RunMarkerGuard {
    /// Stop the heartbeat and leave the marker. Any exit that did not go
    /// through [`Self::finish`] is not clean: a panic, or an error return
    /// such as "keyboard devices lost" (exit 1, restarted by systemd). Those
    /// are the "tracking stopped" cases a crash report is for, so the next
    /// start must find the marker and offer one.
    fn drop(&mut self) {
        let mut s = self.shared.lock().unwrap_or_else(|p| p.into_inner());
        s.stopped = true;
    }
}

/// Write `run.marker`, owner-only.
pub fn write_run_marker(data_dir: &Path, marker: RunMarker) -> io::Result<RunMarkerGuard> {
    let path = run_marker_path(data_dir);
    write_atomic(&path, &marker)?;
    let shared = Arc::new(Mutex::new(Shared {
        path,
        marker,
        stopped: false,
    }));
    *HEARTBEAT.lock().unwrap_or_else(|err| err.into_inner()) = Some(Arc::clone(&shared));
    Ok(RunMarkerGuard { shared })
}

/// Daemon startup: recover a stale marker, then write this run's marker.
/// Call after the pid guard is held, so a second daemon never gets here.
pub fn start_run(data_dir: &Path) -> io::Result<(StaleMarker, RunMarkerGuard)> {
    let stale = recover_stale_run_marker(data_dir, crate::proc_id::pid_is_live_tracker)?;
    let now = Utc::now();
    let guard = write_run_marker(
        data_dir,
        RunMarker {
            pid: std::process::id(),
            started_at: now,
            last_seen: now,
        },
    )?;
    Ok((stale, guard))
}

/// `last_crash.marker`: the run that did not exit cleanly. `None` inside
/// when the marker text was unreadable.
pub fn read_last_crash(data_dir: &Path) -> Option<Option<RunMarker>> {
    let text = std::fs::read_to_string(last_crash_marker_path(data_dir)).ok()?;
    Some(RunMarker::parse(&text))
}

/// `last_seen` of the crashed run, which sets the crash log window.
pub fn last_crash_seen(data_dir: &Path) -> Option<DateTime<Utc>> {
    read_last_crash(data_dir).flatten().map(|m| m.last_seen)
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

    fn t(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn marker(pid: u32) -> RunMarker {
        RunMarker {
            pid,
            started_at: t("2026-10-11T10:00:00Z"),
            last_seen: t("2026-10-11T10:41:00Z"),
        }
    }

    fn sleep_bin() -> PathBuf {
        ["/usr/bin/sleep", "/bin/sleep"]
            .iter()
            .map(PathBuf::from)
            .find(|p| p.exists())
            .expect("sleep binary")
    }

    /// Spawn a just-copied binary. Another test thread's fork can still hold
    /// the copy's write fd until its own exec, which fails this exec with
    /// ETXTBSY ("Text file busy"); retry until that fd is gone.
    fn spawn_retrying_busy(bin: &Path) -> std::process::Child {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            match std::process::Command::new(bin).arg("30").spawn() {
                Ok(child) => return child,
                Err(e)
                    if e.kind() == std::io::ErrorKind::ExecutableFileBusy
                        && std::time::Instant::now() < deadline =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(e) => panic!("spawn {}: {e}", bin.display()),
            }
        }
    }

    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[test]
    fn marker_text_round_trips_and_holds_only_pid_and_times() {
        let m = marker(4242);
        assert_eq!(
            m.format(),
            "pid=4242\nstarted_at=2026-10-11T10:00:00Z\nlast_seen=2026-10-11T10:41:00Z\n"
        );
        assert_eq!(RunMarker::parse(&m.format()), Some(m));
        assert_eq!(
            RunMarker::parse("pid=1\nstarted_at=2026-10-11T10:00:00Z\n"),
            None
        );
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
    fn heartbeat_rewrites_last_seen_and_never_revives_a_removed_marker() {
        let dir = tempfile::tempdir().unwrap();
        let guard = write_run_marker(dir.path(), marker(std::process::id())).unwrap();
        let beat = guard.heartbeat();
        beat.touch(t("2026-10-11T11:00:00Z")).unwrap();
        let now = RunMarker::parse(&std::fs::read_to_string(run_marker_path(dir.path())).unwrap())
            .unwrap();
        assert_eq!(now.last_seen, t("2026-10-11T11:00:00Z"));
        assert_eq!(now.started_at, marker(0).started_at, "start time is kept");
        assert!(!dir.path().join("run.marker.tmp").exists());
        guard.finish();
        beat.touch(t("2026-10-11T11:01:00Z")).unwrap();
        assert!(
            !run_marker_path(dir.path()).exists(),
            "a heartbeat after shutdown must not bring the marker back"
        );
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn heartbeat_task_ticks_once_per_interval() {
        let dir = tempfile::tempdir().unwrap();
        let guard = write_run_marker(dir.path(), marker(std::process::id())).unwrap();
        let task = tokio::spawn(guard.heartbeat().run(HEARTBEAT_INTERVAL));
        let read = || {
            RunMarker::parse(&std::fs::read_to_string(run_marker_path(dir.path())).unwrap())
                .unwrap()
                .last_seen
        };
        let before = read();
        tokio::time::sleep(HEARTBEAT_INTERVAL + std::time::Duration::from_secs(1)).await;
        assert!(read() > before, "last_seen moved within one interval");
        guard.finish();
        tokio::time::sleep(HEARTBEAT_INTERVAL * 2).await;
        assert!(task.is_finished(), "the task stops after a clean finish");
        assert!(!run_marker_path(dir.path()).exists());
    }

    #[test]
    fn a_released_marker_skips_a_tick_and_a_stopped_one_ends_it() {
        assert_eq!(heartbeat_step(true, true), HeartbeatStep::Stop);
        assert_eq!(heartbeat_step(true, false), HeartbeatStep::Stop);
        assert_eq!(heartbeat_step(false, false), HeartbeatStep::Skip);
        assert_eq!(heartbeat_step(false, true), HeartbeatStep::Touch);
    }

    #[test]
    fn an_exit_without_finish_leaves_the_marker_as_a_crash() {
        let dir = tempfile::tempdir().unwrap();
        let guard = write_run_marker(dir.path(), marker(std::process::id())).unwrap();
        let beat = guard.heartbeat();
        // An error return from the main loop drops the guard without finish.
        drop(guard);
        assert!(
            run_marker_path(dir.path()).exists(),
            "an error exit is not clean, so the marker stays"
        );
        let kept = std::fs::read_to_string(run_marker_path(dir.path())).unwrap();
        beat.touch(t("2026-10-11T12:00:00Z")).unwrap();
        assert_eq!(
            std::fs::read_to_string(run_marker_path(dir.path())).unwrap(),
            kept,
            "the heartbeat stops at the drop, so last_seen stays at the last beat"
        );
    }

    #[test]
    fn guard_leaves_a_marker_another_run_rewrote() {
        let dir = tempfile::tempdir().unwrap();
        let guard = write_run_marker(dir.path(), marker(std::process::id())).unwrap();
        std::fs::write(run_marker_path(dir.path()), marker(7).format()).unwrap();
        guard.finish();
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
        assert_eq!(moved, marker(u32::MAX).format(), "old last_seen is kept");
        assert_eq!(log.matches("WARN").count(), 1, "{log}");
        assert_eq!(log.lines().count(), 1, "{log}");
        assert_eq!(last_exit(dir.path()), LastExit::Crashed);
        assert_eq!(last_crash_seen(dir.path()), Some(marker(0).last_seen));
        dismiss_last_crash(dir.path()).unwrap();
        assert_eq!(last_exit(dir.path()), LastExit::Clean);
        assert_eq!(last_crash_seen(dir.path()), None);
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
        assert_eq!(
            std::fs::read_to_string(last_crash_marker_path(dir.path())).unwrap(),
            marker(pid).format()
        );
        assert_eq!(log.matches("WARN").count(), 1, "{log}");
    }

    #[test]
    fn live_tracker_pid_is_left_alone() {
        // A live process whose executable is named scuffed-stat-tracker.
        let dir = tempfile::tempdir().unwrap();
        let bin_dir = tempfile::tempdir().unwrap();
        let fake = bin_dir.path().join(crate::proc_id::DAEMON_BIN);
        std::fs::copy(sleep_bin(), &fake).unwrap();
        let child = Child(spawn_retrying_busy(&fake));
        let pid = child.0.id();
        // The child can still show the test binary in /proc for a moment.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !crate::proc_id::pid_is_live_tracker(pid) {
            assert!(
                std::time::Instant::now() < deadline,
                "the fake tracker never showed as live"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
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
        assert_eq!(read_last_crash(dir.path()), Some(None));
        assert_eq!(last_crash_seen(dir.path()), None);
    }

    #[test]
    fn dead_run_marker_without_restart_reads_as_crashed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(run_marker_path(dir.path()), marker(9).format()).unwrap();
        assert_eq!(last_exit_with(dir.path(), |_| false), LastExit::Crashed);
        assert_eq!(last_exit_with(dir.path(), |_| true), LastExit::Clean);
    }
}
